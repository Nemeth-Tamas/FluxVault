//! Tests for the bounded, observable Greaseweazle process runner using mock_gw.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fluxvault::{
    external_tools::{CommandAudit, run_audited_probe},
    flux_capture::{self, CaptureRequest},
    flux_recovery::{self, RecoveryPolicy, TimeLimitScope},
    greaseweazle::{
        GreaseweazleBackend, GreaseweazleCommand, GreaseweazleDeviceStatus, GreaseweazleExecution,
        GreaseweazleProfile, GreaseweazleProgressEvent, ProcessGreaseweazleBackend,
        classify_info_output, parse_info_output,
    },
    project::ProjectState,
};

fn mock_gw_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock_gw"))
}

fn test_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// A synthetic interrupted read; all successful commands still use mock_gw.
struct TimeoutReadBackend {
    inner: ProcessGreaseweazleBackend,
    timeout: Duration,
    wait_for_deadline: bool,
    reads: usize,
}

impl GreaseweazleBackend for TimeoutReadBackend {
    fn mode(&self) -> fluxvault::greaseweazle::BackendMode {
        self.inner.mode()
    }

    fn set_operation_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
        self.inner.set_operation_timeout(timeout);
    }

    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String> {
        if command.subcommand() != "read" {
            return self.inner.execute(command);
        }
        self.reads += 1;
        let started = test_unix_ms();
        if self.wait_for_deadline {
            assert!(self.timeout <= Duration::from_secs(2));
            std::thread::sleep(self.timeout + Duration::from_millis(100));
        }
        fs::write(
            command.arguments().last().unwrap(),
            b"interrupted synthetic flux",
        )
        .unwrap();
        Ok(GreaseweazleExecution {
            mode: self.mode(),
            command: command.clone(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "Synthetic process deadline expired".into(),
            timed_out: true,
            host_version: Some("mock".into()),
            started_unix_ms: started,
            duration_ms: (test_unix_ms() - started) as u128,
        })
    }
}

fn timeout_backend(project: &ProjectState, wait_for_deadline: bool) -> TimeoutReadBackend {
    TimeoutReadBackend {
        inner: ProcessGreaseweazleBackend::new(
            mock_gw_path(),
            project.logs_dir().join("external-tools.jsonl"),
        )
        .unwrap()
        .with_stream_to_stderr(false),
        timeout: Duration::ZERO,
        wait_for_deadline,
        reads: 0,
    }
}

fn whole_job_policy() -> RecoveryPolicy {
    RecoveryPolicy {
        time_limit_scope: TimeLimitScope::WholeJob,
        ..RecoveryPolicy::default()
    }
}

struct RecordingReadBackend {
    inner: ProcessGreaseweazleBackend,
    timeout: Duration,
    read_timeouts: Vec<Duration>,
    read_commands: Vec<GreaseweazleCommand>,
    intercept_read: bool,
}

impl RecordingReadBackend {
    fn new(project: &ProjectState, intercept_read: bool) -> Self {
        Self {
            inner: ProcessGreaseweazleBackend::new(
                mock_gw_path(),
                project.logs_dir().join("external-tools.jsonl"),
            )
            .unwrap()
            .with_stream_to_stderr(false),
            timeout: Duration::ZERO,
            read_timeouts: vec![],
            read_commands: vec![],
            intercept_read,
        }
    }
}

impl GreaseweazleBackend for RecordingReadBackend {
    fn mode(&self) -> fluxvault::greaseweazle::BackendMode {
        self.inner.mode()
    }
    fn set_operation_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
        self.inner.set_operation_timeout(timeout);
    }
    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String> {
        if command.subcommand() == "read" {
            self.read_timeouts.push(self.timeout);
            self.read_commands.push(command.clone());
            if self.intercept_read {
                return Err("test stopped before physical read".into());
            }
        }
        self.inner.execute(command)
    }
}

#[test]
fn old_job_age_does_not_reduce_the_next_stage_budget_or_overwrite_failed_evidence() {
    let (project, root) = disposable_project("per-stage-legacy-resume");
    let policy = RecoveryPolicy::default();
    seed_partial_recovery(&project, &policy, 601_000);
    let journal_path = root.join("Flux/Recovery/023_job.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    legacy["policy"]
        .as_object_mut()
        .unwrap()
        .remove("time_limit_scope");
    fs::write(&journal_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let partial = root.join("Flux/023_attempt_002.partial.scp");
    fs::write(&partial, b"older interrupted flux").unwrap();
    let mut backend = RecordingReadBackend::new(&project, false);
    let result = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.status, "acquired");
    assert_eq!(result.capture_attempts, vec![1, 3]);
    assert_eq!(result.physical_reads_this_run, 1);
    assert_eq!(backend.read_timeouts, vec![Duration::from_secs(600)]);
    assert_eq!(fs::read(partial).unwrap(), b"older interrupted flux");
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(journal_path).unwrap()).unwrap();
    assert_eq!(journal["started_unix_ms"], legacy["started_unix_ms"]);
    assert_eq!(journal["stage_budget_version"], 1);
    assert!(journal["stages"][1]["capture_started_unix_ms"].is_null());
    assert!(journal["stages"][1]["capture_elapsed_ms"].as_u64().unwrap() > 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn per_stage_deadline_retains_partial_and_stops_at_the_pass_limit() {
    let (project, root) = disposable_project("per-stage-deadline");
    let policy = RecoveryPolicy {
        max_seconds: 30,
        passes: RecoveryPolicy::default().passes[..2].to_vec(),
        ..RecoveryPolicy::default()
    };
    seed_partial_recovery(&project, &policy, 601_000);
    let journal_path = root.join("Flux/Recovery/023_job.json");
    let mut job: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    job["stage_budget_version"] = serde_json::json!(1);
    job["stages"][1]["capture_elapsed_ms"] = serde_json::json!(28_000);
    fs::write(&journal_path, serde_json::to_vec(&job).unwrap()).unwrap();
    let mut backend = timeout_backend(&project, true);
    let result = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.stop_reason, "stage_time_limit");
    assert_eq!(result.status, "partial");
    assert_eq!(result.capture_attempts, vec![1]);
    assert_eq!(backend.reads, 1);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(journal_path).unwrap()).unwrap();
    assert!(saved["stages"][1]["capture_elapsed_ms"].as_u64().unwrap() >= 30_000);
    assert!(saved["stages"][1]["capture_started_unix_ms"].is_null());
    assert!(root.join("Flux/023_attempt_002.partial.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn per_stage_reseat_retry_keeps_spent_capture_time_and_old_failed_attempt() {
    let (project, root) = disposable_project("per-stage-reseat-clock");
    let policy = RecoveryPolicy::default();
    seed_partial_recovery(&project, &policy, 601_000);
    let mut failing = ProcessGreaseweazleBackend::new(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap()
    .with_stream_to_stderr(false)
    .with_env("MOCK_GW_NO_INDEX", "1");
    let error = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy.clone(),
        &mut failing,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.starts_with("No Index capture stopped:"));
    let journal_path = root.join("Flux/Recovery/023_job.json");
    let mut job: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    assert!(job["stages"][1]["capture_elapsed_ms"].as_u64().unwrap() > 0);
    assert!(job["stages"][1]["capture_started_unix_ms"].is_null());
    // Synthetic fixture simulates more time spent in this stage before reseating.
    job["stages"][1]["capture_elapsed_ms"] = serde_json::json!(20_000);
    fs::write(&journal_path, serde_json::to_vec(&job).unwrap()).unwrap();
    let failed_path = root.join("Flux/023_attempt_002.partial.json");
    let before = fs::read(&failed_path).unwrap();
    let mut backend = RecordingReadBackend::new(&project, false);
    let result = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(backend.read_timeouts, vec![Duration::from_secs(580)]);
    assert_eq!(result.capture_attempts, vec![1, 3]);
    assert_eq!(fs::read(failed_path).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exhausted_stage_escalates_without_resetting_its_clock_or_rereading_it() {
    let (project, root) = disposable_project("per-stage-escalation");
    let policy = RecoveryPolicy::default();
    seed_partial_recovery(&project, &policy, 601_000);
    let journal_path = root.join("Flux/Recovery/023_job.json");
    let mut job: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    job["stage_budget_version"] = serde_json::json!(1);
    job["stages"][1]["capture_elapsed_ms"] = serde_json::json!(600_000);
    fs::write(&journal_path, serde_json::to_vec(&job).unwrap()).unwrap();
    let mut backend = RecordingReadBackend::new(&project, false);
    let result = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.status, "acquired");
    assert_eq!(backend.read_timeouts, vec![Duration::from_secs(600)]);
    assert!(
        backend.read_commands[0]
            .arguments()
            .contains(&"--revs=5".into())
    );
    assert_eq!(result.capture_attempts, vec![1, 3]);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(journal_path).unwrap()).unwrap();
    assert_eq!(saved["stages"][1]["capture_elapsed_ms"], 600_000);
    assert!(saved["stages"][1]["decode_attempt"].is_null());
    let mut offline = ProcessGreaseweazleBackend::new_offline(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap();
    let resumed = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut offline,
        &|_| {},
    )
    .unwrap();
    assert!(resumed.resumed);
    assert_eq!(resumed.physical_reads_this_run, 0);
    fs::remove_dir_all(root).unwrap();
}

/// Seed only a disposable fixture: one verified decoded pass and a pending retry.
fn seed_partial_recovery(project: &ProjectState, policy: &RecoveryPolicy, age_ms: u64) {
    let mut backend = ProcessGreaseweazleBackend::new(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap()
    .with_stream_to_stderr(false)
    .with_env("MOCK_GW_BAD_LBAS", "24");
    flux_capture::capture(
        project,
        CaptureRequest {
            disk_number: 23,
            profile: GreaseweazleProfile::Ibm1440,
            drive: 'B',
            revolutions: 2,
        },
        &mut backend,
    )
    .unwrap();
    flux_capture::decode(project, 23, 1, None, &mut backend).unwrap();
    let journal = serde_json::json!({
        "schema_version": 1, "disk": 23, "profile": "ibm.1440",
        "automatic_format": false, "drive": "B",
        "started_unix_ms": test_unix_ms().saturating_sub(age_ms),
        "policy": policy, "result": null,
        "stages": [
            {"capture_attempt": 1, "decode_attempt": 1,
                "settings": {"cylinders": null, "retries": 0}},
            {"capture_attempt": 2, "decode_attempt": null,
                "settings": {"cylinders": [0, 1], "retries": 2}}
        ]
    });
    let directory = project.root().join("Flux/Recovery");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("023_job.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
}

#[test]
fn recovery_deadline_during_read_saves_verified_partial_and_excludes_interrupted_capture() {
    let (project, root) = disposable_project("recovery-deadline");
    let policy = RecoveryPolicy {
        max_seconds: 30,
        time_limit_scope: TimeLimitScope::WholeJob,
        ..RecoveryPolicy::default()
    };
    seed_partial_recovery(&project, &policy, 28_000);
    let mut backend = timeout_backend(&project, true);
    let result = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy.clone(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.status, "partial");
    assert_eq!(result.stop_reason, "time_limit");
    assert_eq!(result.missing_lbas, vec![24]);
    assert_eq!(result.capture_attempts, vec![1]);
    assert_eq!(result.physical_reads_this_run, 1);
    assert_eq!(backend.reads, 1);
    let partial = root.join("Flux/023_attempt_002.partial.scp");
    assert_eq!(fs::read(&partial).unwrap(), b"interrupted synthetic flux");
    assert!(root.join("Flux/023_attempt_002.partial.json").is_file());
    assert!(!root.join("Flux/023_attempt_002.scp").exists());
    let provenance: serde_json::Value =
        serde_json::from_slice(&fs::read(&result.provenance).unwrap()).unwrap();
    assert_eq!(
        provenance["incomplete_capture_attempts"],
        serde_json::json!([2])
    );
    assert_eq!(provenance["stop_reason"], "time_limit");
    assert!(provenance["sectors"].as_array().unwrap().iter().all(|s| {
        s["capture_attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a == 1)
    }));
    // Completed-result verification is offline and does not read the floppy again.
    let audit = project.logs_dir().join("external-tools.jsonl");
    let before = fs::read(&audit).unwrap();
    let mut offline =
        ProcessGreaseweazleBackend::new_offline(mock_gw_path(), audit.clone()).unwrap();
    let resumed = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut offline,
        &|_| {},
    )
    .unwrap();
    assert!(resumed.resumed);
    assert_eq!(resumed.physical_reads_this_run, 0);
    assert_eq!(resumed.capture_attempts, vec![1]);
    assert_eq!(fs::read(audit).unwrap(), before);
    assert_eq!(fs::read(partial).unwrap(), b"interrupted synthetic flux");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn expired_recovery_resumes_previous_pass_without_host_calls_or_partial_promotion() {
    let (project, root) = disposable_project("expired-partial-recovery");
    let policy = whole_job_policy();
    seed_partial_recovery(&project, &policy, 601_000);
    let partial = root.join("Flux/023_attempt_002.partial.scp");
    fs::write(&partial, b"old interrupted evidence").unwrap();
    let audit = project.logs_dir().join("external-tools.jsonl");
    let before = fs::read(&audit).unwrap();
    let mut backend =
        ProcessGreaseweazleBackend::new_offline(mock_gw_path(), audit.clone()).unwrap();
    let result = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.stop_reason, "time_limit");
    assert_eq!(result.status, "partial");
    assert_eq!(result.capture_attempts, vec![1]);
    assert_eq!(result.physical_reads_this_run, 0);
    assert_eq!(fs::read(audit).unwrap(), before);
    assert_eq!(fs::read(partial).unwrap(), b"old interrupted evidence");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn early_operation_timeout_is_not_softened_to_partial_success() {
    let (project, root) = disposable_project("early-recovery-timeout");
    let policy = RecoveryPolicy::default();
    seed_partial_recovery(&project, &policy, 0);
    let mut backend = timeout_backend(&project, false);
    let error = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut backend,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.starts_with("Capture timed out:"), "{error}");
    assert_eq!(backend.timeout, Duration::from_secs(600));
    assert!(fs::read_dir(project.images_dir()).unwrap().next().is_none());
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("Flux/Recovery/023_job.json")).unwrap())
            .unwrap();
    assert!(journal["result"].is_null());
    assert!(root.join("Flux/023_attempt_002.partial.json").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timeout_without_any_completed_pass_never_publishes_an_image() {
    let (project, root) = disposable_project("empty-recovery-timeout");
    let mut backend = timeout_backend(&project, false);
    let error = flux_recovery::recover(
        &project,
        23,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.starts_with("Capture timed out:"), "{error}");
    assert!(fs::read_dir(project.images_dir()).unwrap().next().is_none());
    assert!(root.join("Flux/023_attempt_001.partial.json").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn expired_recovery_still_refuses_tampered_completed_evidence() {
    let (project, root) = disposable_project("expired-tampered-recovery");
    let policy = whole_job_policy();
    seed_partial_recovery(&project, &policy, 601_000);
    fs::write(root.join("Flux/023_attempt_001.scp"), b"changed source").unwrap();
    let mut backend = ProcessGreaseweazleBackend::new_offline(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap();
    assert!(
        flux_recovery::recover(
            &project,
            23,
            GreaseweazleProfile::Ibm1440,
            'B',
            policy,
            &mut backend,
            &|_| {},
        )
        .is_err()
    );
    assert!(fs::read_dir(project.images_dir()).unwrap().next().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires the saved 023 deadline project via FLUXVAULT_DEADLINE_PROJECT; copies only, mock info and intercepted read"]
fn saved_customer_023_resumes_detective_with_full_stage_budget_on_an_isolated_copy() {
    let source = PathBuf::from(std::env::var_os("FLUXVAULT_DEADLINE_PROJECT").unwrap());
    let (project, root) = disposable_project("saved-023-deadline");
    for directory in ["Flux", "Flux/Derived", "Flux/Formats", "Flux/Recovery"] {
        fs::create_dir_all(root.join(directory)).unwrap();
        for entry in fs::read_dir(source.join(directory)).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file()
                && entry.file_name().to_string_lossy().starts_with("023_")
            {
                fs::copy(entry.path(), root.join(directory).join(entry.file_name())).unwrap();
            }
        }
    }
    let source_journal = fs::read(source.join("Flux/Recovery/023_job.json")).unwrap();
    let policy: RecoveryPolicy = serde_json::from_value(
        serde_json::from_slice::<serde_json::Value>(&source_journal).unwrap()["policy"].clone(),
    )
    .unwrap();
    let old_partial = fs::read(root.join("Flux/023_attempt_004.partial.json")).unwrap();
    let mut backend = RecordingReadBackend::new(&project, true);
    let error =
        flux_recovery::recover_auto(&project, 23, 'B', policy, &mut backend, &|_| {}).unwrap_err();
    assert!(
        error.contains("test stopped before physical read"),
        "{error}"
    );
    assert_eq!(backend.read_timeouts, vec![Duration::from_secs(600)]);
    assert_eq!(backend.read_commands.len(), 1);
    let arguments = backend.read_commands[0].arguments();
    assert!(arguments.contains(&"--revs=8".to_owned()));
    assert!(arguments.contains(&"--retries=5".to_owned()));
    assert!(
        arguments
            .last()
            .unwrap()
            .ends_with("023_attempt_005.partial.scp")
    );
    assert_eq!(
        fs::read(root.join("Flux/023_attempt_004.partial.json")).unwrap(),
        old_partial
    );
    let resumed: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("Flux/Recovery/023_job.json")).unwrap())
            .unwrap();
    assert_eq!(resumed["stage_budget_version"], 1);
    assert!(resumed["result"].is_null());
    for stage in resumed["stages"].as_array().unwrap().iter().take(3) {
        assert_eq!(stage["decode_attempt"], 1);
    }
    assert_eq!(
        fs::read(source.join("Flux/Recovery/023_job.json")).unwrap(),
        source_journal
    );
    assert!(!source.join("Images/023_attempt_001.img").exists());
    eprintln!(
        "Saved 023: three completed passes verified; Detective requested 600 seconds, 8 revolutions, 5 retries, fresh capture 005. Synthetic guard stopped before physical read."
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_format_is_bound_raw_only_and_resumes_packed_without_any_host_command() {
    let (project, root) = disposable_project("raw-exception");
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut backend = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone())
        .unwrap()
        .with_env("MOCK_GW_MEDIA_FORMAT", "unsupported-test-format");
    let result = flux_recovery::recover_auto(
        &project,
        1,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.status, "raw_format_exception");
    assert_eq!(result.physical_reads_this_run, 1);
    assert!(result.image.as_os_str().is_empty());
    assert!(result.image_sha256.is_empty());
    assert!(result.selected_profile.is_none());
    assert!(result.format_exception.is_some());
    assert!(fs::read_dir(project.images_dir()).unwrap().next().is_none());
    fluxvault::flux_archive::pack(&project, 1, 1, true).unwrap();
    let before = fs::read(&audit).unwrap();
    let mut offline = ProcessGreaseweazleBackend::new_offline(mock_gw_path(), audit.clone())
        .unwrap()
        .with_env("MOCK_GW_FAIL", "1");
    let resumed = flux_recovery::recover_auto(
        &project,
        1,
        'B',
        RecoveryPolicy::default(),
        &mut offline,
        &|_| {},
    )
    .unwrap();
    assert_eq!(resumed.physical_reads_this_run, 0);
    assert!(resumed.resumed);
    assert_eq!(resumed.format_exception, result.format_exception);
    assert_eq!(fs::read(&audit).unwrap(), before);
    fs::write(&result.provenance, b"tampered decision").unwrap();
    assert!(
        flux_recovery::recover_auto(
            &project,
            1,
            'B',
            RecoveryPolicy::default(),
            &mut offline,
            &|_| {}
        )
        .unwrap_err()
        .contains("path/hash changed")
    );
    assert_eq!(fs::read(&audit).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_auto_decoder_is_not_misclassified_as_a_terminal_format_exception() {
    let (project, root) = disposable_project("failed-auto-decoder");
    let mut backend = ProcessGreaseweazleBackend::new(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap()
    .with_env("MOCK_GW_FAIL_CONVERT", "1");
    let error = flux_recovery::recover_auto(
        &project,
        1,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.contains("decoder trial failed"), "{error}");
    let job: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("Flux/Recovery/001_job.json")).unwrap())
            .unwrap();
    assert_eq!(job["result"], serde_json::Value::Null);
    assert!(root.join("Flux/001_attempt_001.scp").is_file());
    assert!(fs::read_dir(project.images_dir()).unwrap().next().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_format_trials_do_not_redirect_consensus_to_the_wrong_profile() {
    let (project, root) = disposable_project("auto-consensus");
    let mut backend = ProcessGreaseweazleBackend::new(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap()
    .with_env("MOCK_GW_RECOVER_AFTER_FAST", "1");
    let policy = RecoveryPolicy {
        passes: RecoveryPolicy::default().passes[..2].to_vec(),
        ..RecoveryPolicy::default()
    };
    let result =
        flux_recovery::recover_auto(&project, 1, 'B', policy, &mut backend, &|_| {}).unwrap();
    assert_eq!(result.status, "acquired");
    assert_eq!(result.selected_profile.as_deref(), Some("ibm.1440"));
    assert_eq!(result.physical_reads_this_run, 2);
    let status = flux_capture::inspect_disk(&project, 1).unwrap();
    assert_eq!(status.preferred_profile.as_deref(), Some("ibm.1440"));
    assert!(status.decodes.iter().any(|d| d.profile == "ibm.720"));
    let consensus = flux_capture::compare_flux_captures(&project, 1).unwrap();
    assert_eq!(consensus.total_sectors, 2880);
    assert!(consensus.conflicting_reported_good_lbas.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn packed_capture_decodes_and_recovery_resumes_without_hardware_or_lost_provenance() {
    let (project, root) = disposable_project("packed-resume");
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut backend = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone()).unwrap();
    let recovered = flux_recovery::recover(
        &project,
        1,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    let image_hash = recovered.image_sha256.clone();
    fluxvault::flux_archive::pack(&project, 1, 1, true).unwrap();
    let mut offline =
        ProcessGreaseweazleBackend::new_offline(mock_gw_path(), audit.clone()).unwrap();
    let decoded = flux_capture::decode(
        &project,
        1,
        1,
        Some(GreaseweazleProfile::Ibm1440),
        &mut offline,
    )
    .unwrap();
    let status = flux_capture::inspect_disk(&project, 1).unwrap();
    assert!(status.evidence_healthy);
    assert_eq!(status.decodes.len(), 2);
    assert_eq!(decoded.sha256, image_hash);
    assert!(!root.join("Flux/001_attempt_001.scp").exists());
    assert!(!fs::read_dir(root.join("Flux")).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fluxvault-unpack-")
    }));
    let resumed = flux_recovery::recover(
        &project,
        1,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut offline,
        &|_| {},
    )
    .unwrap();
    assert_eq!(resumed.physical_reads_this_run, 0);
    assert!(resumed.resumed);
    assert_eq!(resumed.image_sha256, image_hash);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn automatic_dd_selection_is_offline_resumable_and_does_not_repeat_capture() {
    let (project, root) = disposable_project("auto-dd");
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut backend = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone())
        .unwrap()
        .with_env("MOCK_GW_MEDIA_FORMAT", "ibm.720");
    let result = flux_recovery::recover_auto(
        &project,
        9,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.status, "acquired");
    assert_eq!(result.physical_reads_this_run, 1);
    assert_eq!(fs::metadata(&result.image).unwrap().len(), 737280);
    let job: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("Flux/Recovery/009_job.json")).unwrap())
            .unwrap();
    assert_eq!(job["profile"], "ibm.720");
    assert_eq!(job["automatic_format"], true);
    let before = fs::read(&audit).unwrap();
    let mut absent = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone())
        .unwrap()
        .with_env("MOCK_GW_DEVICE_NOT_FOUND", "1");
    let resumed = flux_recovery::recover_auto(
        &project,
        9,
        'B',
        RecoveryPolicy::default(),
        &mut absent,
        &|_| {},
    )
    .unwrap();
    assert!(resumed.resumed);
    assert_eq!(resumed.physical_reads_this_run, 0);
    assert_eq!(fs::read(&audit).unwrap(), before);
    assert!(
        flux_recovery::recover(
            &project,
            9,
            GreaseweazleProfile::Ibm720,
            'B',
            RecoveryPolicy::default(),
            &mut absent,
            &|_| {}
        )
        .unwrap_err()
        .contains("different")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn offline_backend_has_no_version_board_probe_and_rejects_board_commands() {
    let (project, root) = disposable_project("offline-only");
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut acquisition = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone()).unwrap();
    flux_capture::capture(
        &project,
        CaptureRequest {
            disk_number: 1,
            profile: GreaseweazleProfile::Ibm1440,
            drive: 'B',
            revolutions: 2,
        },
        &mut acquisition,
    )
    .unwrap();
    let count = fs::read_to_string(&audit).unwrap().lines().count();
    let mut offline = ProcessGreaseweazleBackend::new_offline(mock_gw_path(), audit.clone())
        .unwrap()
        .with_env("MOCK_GW_DEVICE_NOT_FOUND", "1");
    assert!(
        offline
            .execute(&GreaseweazleCommand::info())
            .unwrap_err()
            .contains("convert only")
    );
    let (decision, _) =
        fluxvault::flux_format::identify(&project, 1, 1, &mut offline, &|_| {}).unwrap();
    assert_eq!(decision.selected_profile.as_deref(), Some("ibm.1440"));
    let text = fs::read_to_string(&audit).unwrap();
    let added: Vec<CommandAudit> = text
        .lines()
        .skip(count)
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(added.len(), 2);
    assert!(added.iter().all(|a| a.arguments[0] == "convert"));
    let reused = fs::read(&audit).unwrap();
    fluxvault::flux_format::identify(&project, 1, 1, &mut offline, &|_| {}).unwrap();
    assert_eq!(fs::read(&audit).unwrap(), reused);
    let _ = fs::remove_dir_all(root);
}

fn disposable_project(name: &str) -> (ProjectState, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-gw-runner-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let project = ProjectState::create_without_session(root.clone()).unwrap();
    (project, root)
}

#[test]
fn version_probes_are_bounded_and_audited_without_media_access() {
    let (_project, root) = disposable_project("probe-timeout");
    let audit_path = root.join("logs").join("external-tools.jsonl");
    let executable = mock_gw_path();
    let version = run_audited_probe(
        "Greaseweazle",
        &executable,
        &["info".to_owned()],
        &audit_path,
        Duration::from_secs(2),
    );
    assert!(version.audit.success);
    assert!(version.audit.stderr.contains("1.23"));
    assert!(version.audit_error.is_none());

    let hanging = run_audited_probe(
        "Greaseweazle",
        &executable,
        &["info".to_owned(), "--mock-hang".to_owned()],
        &audit_path,
        Duration::from_millis(400),
    );
    assert!(!hanging.audit.success);
    assert!(hanging.audit.stderr.contains("Probe timed out"));
    assert!(hanging.audit.duration_ms < 10_000);
    assert!(hanging.audit_error.is_none());
    assert_eq!(fs::read_to_string(&audit_path).unwrap().lines().count(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runner_executes_info_preserves_output_and_records_audit_with_version() {
    let (_project, root) = disposable_project("info-test");
    let audit_path = root.join("logs").join("external-tools.jsonl");
    let mock_gw = mock_gw_path();

    let mut backend = ProcessGreaseweazleBackend::new(mock_gw.clone(), audit_path.clone()).unwrap();
    let command = GreaseweazleCommand::info();
    let execution = backend.execute(&command).unwrap();

    assert!(execution.success);
    assert_eq!(execution.exit_code, Some(0));
    assert!(!execution.timed_out);
    assert_eq!(execution.host_version.as_deref(), Some("1.23"));

    let info = parse_info_output(&execution.output_text());
    assert_eq!(info.status, GreaseweazleDeviceStatus::Connected);
    assert_eq!(info.host_tools_version.as_deref(), Some("1.23"));
    assert_eq!(info.model.as_deref(), Some("Greaseweazle V4"));
    assert_eq!(info.firmware.as_deref(), Some("1.23"));
    assert_eq!(info.port.as_deref(), Some("COM3"));

    // Verify external-tools.jsonl audit log was written
    assert!(audit_path.exists());
    let audit_content = fs::read_to_string(&audit_path).unwrap();
    let audit: CommandAudit = serde_json::from_str(audit_content.lines().next().unwrap()).unwrap();
    assert_eq!(audit.tool, "Greaseweazle");
    assert_eq!(audit.executable, mock_gw);
    assert_eq!(audit.arguments, vec!["info"]);
    assert!(audit.success);
    assert_eq!(audit.exit_code, Some(0));
    assert_eq!(audit.version.as_deref(), Some("1.23"));
    assert!(audit.stderr.contains("Greaseweazle V4"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn runner_streams_progress_events_during_raw_capture() {
    let (project, root) = disposable_project("stream-test");
    let audit_path = project.logs_dir().join("external-tools.jsonl");
    let mock_gw = mock_gw_path();

    let events: Arc<Mutex<Vec<GreaseweazleProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let mut backend = ProcessGreaseweazleBackend::new(mock_gw, audit_path).unwrap();
    backend.set_progress_callback(Some(Box::new(move |event| {
        events_clone.lock().unwrap().push(event.clone());
    })));

    let result = flux_capture::capture(
        &project,
        CaptureRequest {
            disk_number: 1,
            profile: GreaseweazleProfile::Ibm1440,
            drive: 'A',
            revolutions: 3,
        },
        &mut backend,
    )
    .unwrap();

    assert_eq!(result.disk_number, 1);
    assert_eq!(result.attempt_number, 1);
    assert!(result.flux_path.exists());
    assert!(result.metadata_path.exists());
    assert_eq!(result.host_version.as_deref(), Some("1.23"));

    // Verify that progress events were streamed
    let recorded = events.lock().unwrap().clone();
    assert!(
        !recorded.is_empty(),
        "Expected progress events to be streamed"
    );
    assert!(
        recorded
            .iter()
            .any(|e| matches!(e, GreaseweazleProgressEvent::ReadingRange { .. })),
        "Expected ReadingRange event"
    );
    assert!(
        recorded
            .iter()
            .any(|e| matches!(e, GreaseweazleProgressEvent::Track { .. })),
        "Expected Track events"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn runner_bounded_timeout_terminates_process_tree_without_losing_partial_evidence() {
    let (project, root) = disposable_project("timeout-test");
    let audit_path = project.logs_dir().join("external-tools.jsonl");
    let mock_gw = mock_gw_path();

    let mut backend = ProcessGreaseweazleBackend::new(mock_gw.clone(), audit_path.clone())
        .unwrap()
        .with_timeout(Duration::from_millis(600))
        .with_env("MOCK_GW_HANG", "1");

    let capture_err = flux_capture::capture(
        &project,
        CaptureRequest {
            disk_number: 2,
            profile: GreaseweazleProfile::Ibm1440,
            drive: 'A',
            revolutions: 3,
        },
        &mut backend,
    )
    .unwrap_err();

    assert!(capture_err.contains("Raw capture failed; attempt evidence remains at"));

    // Verify partial evidence is preserved
    let flux_dir = project.root().join("Flux");
    let partial_metadata = flux_dir.join("002_attempt_001.partial.json");
    assert!(
        partial_metadata.exists(),
        "Partial metadata must be retained on timeout"
    );

    let metadata_content = fs::read_to_string(&partial_metadata).unwrap();
    let record: serde_json::Value = serde_json::from_str(&metadata_content).unwrap();
    assert_eq!(record["status"], "failed");
    assert!(
        record["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("timed out"),
        "Detail should mention timeout"
    );

    // Verify audit log captured the timeout
    assert!(audit_path.exists());
    let audit_content = fs::read_to_string(&audit_path).unwrap();
    assert!(audit_content.contains("Process timed out"));

    // Verify next capture advances to attempt #2 without overwriting attempt #1
    let mut clean_backend = ProcessGreaseweazleBackend::new(mock_gw, audit_path).unwrap();
    let next_result = flux_capture::capture(
        &project,
        CaptureRequest {
            disk_number: 2,
            profile: GreaseweazleProfile::Ibm1440,
            drive: 'A',
            revolutions: 3,
        },
        &mut clean_backend,
    )
    .unwrap();

    assert_eq!(
        next_result.attempt_number, 2,
        "Next capture must advance to attempt #2"
    );
    assert!(
        partial_metadata.exists(),
        "Attempt #1 partial evidence must still exist"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn runner_decode_converts_raw_flux_and_records_derived_image() {
    let (project, root) = disposable_project("decode-test");
    let audit_path = project.logs_dir().join("external-tools.jsonl");
    let mock_gw = mock_gw_path();

    let mut backend = ProcessGreaseweazleBackend::new(mock_gw, audit_path).unwrap();
    let capture_result = flux_capture::capture(
        &project,
        CaptureRequest {
            disk_number: 3,
            profile: GreaseweazleProfile::Ibm1440,
            drive: 'A',
            revolutions: 3,
        },
        &mut backend,
    )
    .unwrap();

    let decode_result = flux_capture::decode(
        &project,
        3,
        capture_result.attempt_number,
        None,
        &mut backend,
    )
    .unwrap();

    assert_eq!(decode_result.disk_number, 3);
    assert_eq!(decode_result.capture_attempt, 1);
    assert_eq!(decode_result.decode_attempt, 1);
    assert!(decode_result.image_path.exists());
    assert_eq!(decode_result.bytes, 1_474_560);
    assert_eq!(decode_result.reported_sectors, Some((2880, 2880)));
    assert_eq!(decode_result.gw_bad_lbas, Some(Vec::new()));

    // Verify status inspection sees the derived evidence
    let status = flux_capture::inspect_disk(&project, 3).unwrap();
    assert_eq!(status.captures.len(), 1);
    assert_eq!(status.decodes.len(), 1);
    assert!(status.evidence_healthy);
    assert_eq!(
        status.decodes[0].sector_quality,
        "unverified_sector_quality"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn runner_detects_device_not_found_from_mock() {
    let (_project, root) = disposable_project("not-found-test");
    let audit_path = root.join("logs").join("external-tools.jsonl");
    let mock_gw = mock_gw_path();

    let mut backend = ProcessGreaseweazleBackend::new(mock_gw, audit_path)
        .unwrap()
        .with_env("MOCK_GW_DEVICE_NOT_FOUND", "1");
    let execution = backend.execute(&GreaseweazleCommand::info()).unwrap();

    assert!(execution.success);
    assert_eq!(
        classify_info_output(&execution.output_text()),
        GreaseweazleDeviceStatus::NotFound
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn runner_strictly_blocks_unsafe_greaseweazle_commands_before_execution() {
    for forbidden in ["write", "erase", "clean", "update"] {
        assert!(
            GreaseweazleCommand::from_raw_arguments(vec![forbidden.to_owned()]).is_err(),
            "Forbidden command '{forbidden}' must be rejected by safety check"
        );
        assert!(
            GreaseweazleCommand::from_raw_arguments(vec![
                "read".to_owned(),
                format!("--{forbidden}"),
                "disk.scp".to_owned()
            ])
            .is_err(),
            "Forbidden argument in read must be rejected"
        );
    }
    assert!(
        GreaseweazleCommand::raw_flux_read(
            GreaseweazleProfile::Ibm1440,
            'A',
            3,
            Path::new("test.img"), // not .scp
        )
        .is_err()
    );
    let cmd = GreaseweazleCommand::info();
    assert_eq!(cmd.subcommand(), "info");
}

#[test]
fn automatic_recovery_stops_after_clean_fast_pass_and_reuses_completed_job() {
    let (project, root) = disposable_project("auto-clean");
    let mut backend = ProcessGreaseweazleBackend::new(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap()
    .with_stream_to_stderr(false);
    let first = flux_recovery::recover(
        &project,
        7,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(first.physical_reads_this_run, 1);
    assert_eq!(first.status, "acquired");
    assert_eq!(first.single_capture_sectors, 2880);
    assert_eq!(
        fluxvault::imaging::load_project_statistics(&project.images_dir())
            .unwrap()
            .disk_count,
        1
    );
    let repeat = flux_recovery::recover(
        &project,
        7,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(repeat.physical_reads_this_run, 0);
    assert!(repeat.resumed);
    assert_eq!(repeat.image, first.image);
    fs::write(first.image, b"tampered").unwrap();
    assert!(
        flux_recovery::recover(
            &project,
            7,
            GreaseweazleProfile::Ibm1440,
            'B',
            RecoveryPolicy::default(),
            &mut backend,
            &|_| {}
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_recovery_targets_problem_cylinders_and_stops_without_improvement() {
    let (project, root) = disposable_project("auto-stubborn");
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut backend = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone())
        .unwrap()
        .with_stream_to_stderr(false)
        .with_env("MOCK_GW_BAD_LBAS", "24");
    let result = flux_recovery::recover(
        &project,
        1,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.stop_reason, "no_improvement");
    assert_eq!(result.status, "partial");
    assert_eq!(result.missing_lbas, vec![24]);
    assert_eq!(result.physical_reads_this_run, 3);
    let entries = fs::read_to_string(audit).unwrap();
    assert!(entries.contains("--tracks=c=0,1,2:h=0-1"));
    let provenance: serde_json::Value =
        serde_json::from_slice(&fs::read(result.provenance).unwrap()).unwrap();
    assert_eq!(provenance["sectors"][24]["confidence"], "unreadable");
    assert_eq!(provenance["sectors"][36]["confidence"], "corroborated");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_recovery_merges_new_sector_and_refuses_changed_control_bytes() {
    for conflict in [false, true] {
        let (project, root) = disposable_project("auto-improvement");
        let mut backend = ProcessGreaseweazleBackend::new(
            mock_gw_path(),
            project.logs_dir().join("external-tools.jsonl"),
        )
        .unwrap()
        .with_stream_to_stderr(false)
        .with_env("MOCK_GW_RECOVER_AFTER_FAST", "1");
        if conflict {
            backend = backend.with_env("MOCK_GW_CONFLICT_LBA", "36");
        }
        let result = flux_recovery::recover(
            &project,
            1,
            GreaseweazleProfile::Ibm1440,
            'B',
            RecoveryPolicy::default(),
            &mut backend,
            &|_| {},
        );
        if conflict {
            assert!(result.unwrap_err().contains("Control sectors disagree"));
            assert!(fs::read_dir(project.images_dir()).unwrap().next().is_none());
        } else {
            let result = result.unwrap();
            assert_eq!(result.physical_reads_this_run, 2);
            assert_eq!(result.status, "acquired");
            assert!(result.missing_lbas.is_empty());
            assert!(result.corroborated_sectors > 0);
            let provenance: serde_json::Value =
                serde_json::from_slice(&fs::read(result.provenance).unwrap()).unwrap();
            assert_eq!(
                provenance["sectors"][24]["capture_attempts"],
                serde_json::json!([2])
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn no_index_exit_zero_is_a_failed_capture_and_empty_job_can_retry_after_expiry() {
    let (project, root) = disposable_project("no-index-zero-exit");
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut failing = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone())
        .unwrap()
        .with_stream_to_stderr(false)
        .with_env("MOCK_GW_NO_INDEX", "1");
    let error = flux_recovery::recover(
        &project,
        4,
        GreaseweazleProfile::Ibm1440,
        'B',
        whole_job_policy(),
        &mut failing,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.contains("No Index"));
    assert!(error.contains("Reseat"));
    let failed_path = project.root().join("Flux/004_attempt_001.partial.json");
    let original_failure = fs::read(&failed_path).unwrap();
    let failed: serde_json::Value = serde_json::from_slice(&original_failure).unwrap();
    assert_eq!(failed["status"], "failed");
    let records = fs::read_to_string(&audit).unwrap();
    let failed_read: serde_json::Value = records
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|row| row["arguments"][0] == "read")
        .unwrap();
    assert_eq!(failed_read["exit_code"], 0);
    assert_eq!(failed_read["success"], false);
    assert!(!project.root().join("Flux/004_attempt_001.scp").exists());
    let job_path = project.root().join("Flux/Recovery/004_job.json");
    let mut job: serde_json::Value = serde_json::from_slice(&fs::read(&job_path).unwrap()).unwrap();
    job["started_unix_ms"] = serde_json::json!(0);
    fs::write(&job_path, serde_json::to_vec(&job).unwrap()).unwrap();
    // Even unfinalized raw evidence blocks budget reset; never discard it to retry.
    let partial_raw = project.root().join("Flux/004_attempt_001.partial.scp");
    fs::write(&partial_raw, b"synthetic interrupted raw evidence").unwrap();
    let mut blocked = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone())
        .unwrap()
        .with_stream_to_stderr(false);
    assert!(
        flux_recovery::recover(
            &project,
            4,
            GreaseweazleProfile::Ibm1440,
            'B',
            whole_job_policy(),
            &mut blocked,
            &|_| {}
        )
        .is_err()
    );
    let not_reset: serde_json::Value =
        serde_json::from_slice(&fs::read(&job_path).unwrap()).unwrap();
    assert_eq!(not_reset["started_unix_ms"], 0);
    assert!(
        not_reset["empty_capture_budget_restarts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read(&partial_raw).unwrap(),
        b"synthetic interrupted raw evidence"
    );
    fs::remove_file(partial_raw).unwrap(); // Remove only the synthetic file this test just created.
    let mut backend = ProcessGreaseweazleBackend::new(mock_gw_path(), audit)
        .unwrap()
        .with_stream_to_stderr(false);
    let result = flux_recovery::recover(
        &project,
        4,
        GreaseweazleProfile::Ibm1440,
        'B',
        whole_job_policy(),
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.status, "acquired");
    assert_eq!(result.capture_attempts, vec![2]);
    assert_eq!(result.physical_reads_this_run, 1);
    assert_eq!(fs::read(&failed_path).unwrap(), original_failure);
    let job: serde_json::Value = serde_json::from_slice(&fs::read(job_path).unwrap()).unwrap();
    assert_eq!(job["empty_capture_budget_restarts"], serde_json::json!([0]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_recovery_resumes_saved_raw_capture_after_failed_decode() {
    let (project, root) = disposable_project("auto-resume");
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut failing = ProcessGreaseweazleBackend::new(mock_gw_path(), audit.clone())
        .unwrap()
        .with_stream_to_stderr(false)
        .with_env("MOCK_GW_FAIL_CONVERT", "1");
    assert!(
        flux_recovery::recover(
            &project,
            1,
            GreaseweazleProfile::Ibm1440,
            'B',
            RecoveryPolicy::default(),
            &mut failing,
            &|_| {}
        )
        .is_err()
    );
    // Time spent shut down must not trigger another physical pass, but a saved
    // raw capture can still be decoded offline when the board is disconnected.
    let journal_path = project.root().join("Flux/Recovery/001_job.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    journal["started_unix_ms"] = serde_json::json!(0);
    fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let mut resumed = ProcessGreaseweazleBackend::new(mock_gw_path(), audit)
        .unwrap()
        .with_stream_to_stderr(false)
        .with_env("MOCK_GW_DEVICE_NOT_FOUND", "1");
    let result = flux_recovery::recover(
        &project,
        1,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut resumed,
        &|_| {},
    )
    .unwrap();
    assert!(result.resumed);
    assert_eq!(result.physical_reads_this_run, 0);
    assert_eq!(result.capture_attempts, vec![1]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_recovery_refuses_absent_board_before_capture_and_invalid_policy() {
    let (project, root) = disposable_project("auto-no-board");
    let mut backend = ProcessGreaseweazleBackend::new(
        mock_gw_path(),
        project.logs_dir().join("external-tools.jsonl"),
    )
    .unwrap()
    .with_stream_to_stderr(false)
    .with_env("MOCK_GW_DEVICE_NOT_FOUND", "1");
    let error = flux_recovery::recover(
        &project,
        1,
        GreaseweazleProfile::Ibm1440,
        'B',
        RecoveryPolicy::default(),
        &mut backend,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.contains("no physical capture started"));
    assert!(
        !fs::read_dir(project.root().join("Flux"))
            .unwrap()
            .any(|entry| {
                entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "scp")
            })
    );
    for seconds in [0, 29, 1801, u64::MAX] {
        let policy = RecoveryPolicy {
            max_seconds: seconds,
            ..RecoveryPolicy::default()
        };
        assert!(policy.validate().is_err());
    }
    let mut policy = RecoveryPolicy::default();
    policy.passes[0].retries = 11;
    assert!(policy.validate().is_err());
    fs::remove_dir_all(root).unwrap();
}
