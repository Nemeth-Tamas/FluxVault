use super::*;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    project: ProjectState,
    destination: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-finalize-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        let destination = root.join("delivery");
        fs::create_dir(&destination).unwrap();
        Self {
            root,
            project,
            destination,
        }
    }
    fn record(&self) -> Record {
        Record::new(
            &self.project,
            Options {
                destination: self.destination.canonicalize().unwrap(),
                workers: 4,
                allow_attention: false,
            },
        )
        .unwrap()
    }
    fn tamper(&self, change: impl FnOnce(&mut Value)) {
        let path = self.project.root().join(".fluxvault-finalize.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn empty_projects_and_internal_destinations_refused_before_tools() {
    let f = Fixture::new();
    assert!(
        run(
            &f.root,
            f.project.root().into(),
            f.destination.clone(),
            4,
            false,
            true
        )
        .unwrap_err()
        .contains("No saved disk images")
    );
    assert!(
        run(
            &f.root,
            f.project.root().into(),
            f.project.reports_dir(),
            4,
            false,
            true
        )
        .unwrap_err()
        .contains("outside the project")
    );
    assert!(!f.project.root().join(".fluxvault-finalize.json").exists());
}

#[test]
fn device_destinations_are_refused_without_access() {
    let f = Fixture::new();
    for path in [
        "A:",
        "B:\\customer",
        "\\\\?\\A:\\",
        "\\\\.\\A:",
        "a:/customer",
    ] {
        assert!(
            state::destination(&f.root, f.project.root(), path.into())
                .unwrap_err()
                .contains("never floppy")
        );
    }
}

#[test]
fn direct_package_build_also_refuses_floppy_destinations_before_inventory() {
    let f = Fixture::new();
    for path in ["A:\\", "\\\\?\\B:\\", "\\\\.\\PhysicalDrive0"] {
        let error = package::build_package(
            &PackageRequest {
                project_root: f.project.root().into(),
                destination: path.into(),
                project_name: "test".into(),
            },
            &|_| panic!("unsafe destination must fail before inventory"),
        )
        .unwrap_err();
        assert!(error.contains("never floppy"), "{error}");
    }
}

#[test]
fn cli_rejects_partial_archive_flag_in_every_other_mode_before_hardware() {
    let f = Fixture::new();
    for command in [
        vec!["scan"],
        vec!["scan", "--double"],
        vec!["scan", "--double", "--plan"],
        vec!["package", "build"],
        vec!["finalize", "status"],
        vec!["process"],
    ] {
        let mut args = command.into_iter().map(str::to_owned).collect::<Vec<_>>();
        args.push("--allow-attention".into());
        assert!(
            crate::cli::run(&args, f.project.root())
                .unwrap_err()
                .contains("only valid with finalize")
        );
    }
}

#[test]
fn clean_and_partial_archive_responses_never_certify_customer_bytes() {
    for attention in [false, true] {
        let result = response(
            json!({"evidence_attention":usize::from(attention)}),
            Some(json!({"files":2})),
            attention,
            attention,
            true,
        );
        assert_eq!(result.exit_code, if attention { 3 } else { 0 });
        let value: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(value["package"]["files"], 2);
        assert_eq!(value["customer_delivery_certified"], false);
        assert_eq!(value["physical_media_access"], false);
        assert_eq!(
            value["package_status"],
            if attention {
                "verified_archival_zip_with_attention"
            } else {
                "verified_archival_zip"
            }
        );
    }
}

#[test]
fn blocked_response_and_human_instructions_are_explicit() {
    let result = response(json!({"workbook":"report.xlsx"}), None, true, false, true);
    let value: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(value["package_status"], "blocked_by_attention");
    assert!(value["package"].is_null());
    assert_eq!(result.exit_code, 3);
    let text = response(json!({"workbook":"report.xlsx"}), None, true, false, false);
    assert!(text.output.contains("resume --allow-attention"));
}

#[test]
fn atomic_receipts_round_trip_and_retain_each_phase() {
    let f = Fixture::new();
    assert!(state::load(&f.project).unwrap().is_none());
    let mut record = f.record();
    record.save(&f.project).unwrap();
    record.phase = Phase::Packaging;
    record.processing = Some(json!({"evidence_attention":1}));
    record.save(&f.project).unwrap();
    record.phase = Phase::Interrupted;
    record.error = Some("[FV_STOPPED] test".into());
    record.save(&f.project).unwrap();
    let saved = state::load(&f.project).unwrap().unwrap();
    assert_eq!(saved.phase, Phase::Interrupted);
    assert_eq!(saved.options.workers, 4);
    assert_eq!(saved.processing.unwrap()["evidence_attention"], 1);
    assert_eq!(
        fs::read_dir(f.project.root().join(".fluxvault-finalizations"))
            .unwrap()
            .count(),
        3
    );
}

#[test]
fn malformed_receipts_never_replace_previous_state() {
    let f = Fixture::new();
    let mut record = f.record();
    record.save(&f.project).unwrap();
    f.tamper(|v| v["schema"] = json!(99));
    let path = f.project.root().join(".fluxvault-finalize.json");
    let prior = fs::read(&path).unwrap();
    assert!(state::load(&f.project).unwrap_err().contains("identity"));
    assert!(record.save(&f.project).is_err());
    assert_eq!(fs::read(path).unwrap(), prior);
}

#[test]
fn foreign_project_unknown_fields_workers_and_device_saved_paths_refused() {
    for change in [0, 1, 2, 3, 4, 5] {
        let f = Fixture::new();
        f.record().save(&f.project).unwrap();
        f.tamper(|v| match change {
            0 => v["project"] = json!(f.root),
            1 => v["unknown"] = json!(true),
            2 => v["options"]["workers"] = json!(17),
            3 => v["options"]["destination"] = json!("relative"),
            4 => v["options"]["destination"] = json!("A:\\customer"),
            _ => v["customer_delivery_certified"] = json!(true),
        });
        assert!(state::load(&f.project).is_err(), "tamper variant {change}");
    }
}

#[test]
fn oversized_and_nonregular_receipts_refused() {
    for directory in [false, true] {
        let f = Fixture::new();
        let path = f.project.root().join(".fluxvault-finalize.json");
        if directory {
            fs::create_dir(path).unwrap();
        } else {
            fs::write(path, vec![b' '; 65537]).unwrap();
        }
        assert!(state::load(&f.project).is_err());
    }
}

#[test]
fn status_is_read_only_and_does_not_touch_saved_destination() {
    let f = Fixture::new();
    let mut record = f.record();
    record.options.destination = f.root.join("does-not-exist");
    record.save(&f.project).unwrap();
    let before = fs::read(f.project.root().join(".fluxvault-finalize.json")).unwrap();
    let result = status(&f.project, true).unwrap();
    let value: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(value["active"], false);
    assert_eq!(
        value["receipt_is_historical_not_current_verification"],
        true
    );
    assert!(!record.options.destination.exists());
    assert_eq!(
        fs::read(f.project.root().join(".fluxvault-finalize.json")).unwrap(),
        before
    );
}

#[test]
fn resume_requires_receipt_and_refuses_completed_runs_without_tools() {
    let f = Fixture::new();
    assert!(
        resume(&f.project, false, true)
            .unwrap_err()
            .contains("No saved finalization")
    );
    for phase in [Phase::CompleteClean, Phase::CompleteAttention] {
        let mut record = f.record();
        record.phase = phase;
        record.save(&f.project).unwrap();
        assert!(
            resume(&f.project, false, true)
                .unwrap_err()
                .contains("already completed")
        );
    }
}

#[test]
fn resume_refuses_new_project_owner_without_touching_receipt() {
    let f = Fixture::new();
    f.record().save(&f.project).unwrap();
    let owner = crate::project_work::reserve(f.project.root()).unwrap();
    assert!(
        resume(&f.project, true, true)
            .unwrap_err()
            .contains("processing owner")
    );
    assert!(
        run(
            &f.root,
            f.project.root().into(),
            f.destination.clone(),
            4,
            true,
            true
        )
        .unwrap_err()
        .contains("processing owner")
    );
    drop(owner);
}

#[test]
fn incomplete_receipt_requires_destination_revalidation_before_tools() {
    let f = Fixture::new();
    let mut record = f.record();
    record.options.destination = f.project.reports_dir().canonicalize().unwrap();
    record.save(&f.project).unwrap();
    assert!(
        resume(&f.project, false, true)
            .unwrap_err()
            .contains("outside the project")
    );
}

#[test]
fn attention_default_blocks_package_but_opt_in_preserves_attention_exit() {
    for allowed in [false, true] {
        let f = Fixture::new();
        let mut record = f.record();
        record.options.allow_attention = allowed;
        record.save(&f.project).unwrap();
        let outcome = finish_with(
            &f.project,
            &mut record,
            true,
            || {
                Ok(CliResponse {
                    output: json!({"evidence_attention":1}).to_string(),
                    exit_code: 3,
                })
            },
            || {
                assert!(allowed);
                Ok(json!({"files":2}))
            },
        )
        .unwrap();
        assert_eq!(outcome.exit_code, 3);
        assert_eq!(
            state::load(&f.project).unwrap().unwrap().phase,
            if allowed {
                Phase::CompleteAttention
            } else {
                Phase::BlockedByAttention
            }
        );
    }
}

#[test]
fn same_owner_and_control_generation_span_processing_and_packaging() {
    let f = Fixture::new();
    let _owner = crate::project_work::reserve(f.project.root()).unwrap();
    let _control = crate::run_control::Session::start(&f.project, "finalize").unwrap();
    let before = crate::run_control::status(&f.project).unwrap();
    let mut record = f.record();
    record.save(&f.project).unwrap();
    let inspect = || {
        assert!(crate::project_work::reserve(f.project.root()).is_err());
        let control = crate::run_control::status(&f.project).unwrap();
        assert_eq!(control["active"], true);
        assert_eq!(
            control["record"]["generation"],
            before["record"]["generation"]
        );
        assert_eq!(control["record"]["operation"], "finalize");
    };
    let result = finish_with(
        &f.project,
        &mut record,
        true,
        || {
            inspect();
            Ok(CliResponse {
                output: json!({"evidence_attention":0}).to_string(),
                exit_code: 0,
            })
        },
        || {
            inspect();
            assert_eq!(
                state::load(&f.project).unwrap().unwrap().phase,
                Phase::Packaging
            );
            Ok(json!({"files":2}))
        },
    )
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        state::load(&f.project).unwrap().unwrap().phase,
        Phase::CompleteClean
    );
}

#[test]
fn cancellation_at_phase_boundary_never_starts_packaging() {
    let f = Fixture::new();
    let token = crate::cancellation::Token::default();
    let _scope = crate::cancellation::enter(token.clone());
    let mut record = f.record();
    record.save(&f.project).unwrap();
    let error = finish_with(
        &f.project,
        &mut record,
        true,
        || {
            token.request();
            Ok(CliResponse {
                output: "{}".into(),
                exit_code: 0,
            })
        },
        || panic!("stop between phases must not start packaging"),
    )
    .unwrap_err();
    assert!(crate::cancellation::stopped(&error));
    assert!(record.package.is_none());
    assert_eq!(
        state::load(&f.project).unwrap().unwrap().phase,
        Phase::Processing
    );
}

#[test]
fn failed_or_invalid_process_results_cannot_create_a_package() {
    for exit_code in [0, 2, 130] {
        let f = Fixture::new();
        let mut record = f.record();
        record.save(&f.project).unwrap();
        assert!(
            finish_with(
                &f.project,
                &mut record,
                true,
                || Ok(CliResponse {
                    output: "not-json".into(),
                    exit_code
                }),
                || panic!("failed/invalid result must not package"),
            )
            .is_err()
        );
    }
}

#[test]
fn packaging_failure_keeps_resumable_phase_and_no_success_receipt() {
    let f = Fixture::new();
    let mut record = f.record();
    record.save(&f.project).unwrap();
    let error = finish_with(
        &f.project,
        &mut record,
        true,
        || {
            Ok(CliResponse {
                output: "{}".into(),
                exit_code: 0,
            })
        },
        || Err("injected disk full".into()),
    )
    .unwrap_err();
    assert_eq!(error, "injected disk full");
    assert!(record.package.is_none());
    assert_eq!(
        state::load(&f.project).unwrap().unwrap().phase,
        Phase::Packaging
    );
}
