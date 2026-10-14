use super::{
    tests::{ArtifactBackend, fixture},
    *,
};

fn request() -> CaptureRequest {
    CaptureRequest {
        disk_number: 7,
        profile: GreaseweazleProfile::Ibm720,
        drive: 'A',
        revolutions: 2,
    }
}
fn interrupted_capture(phase: &'static str) -> (ProjectState, PathBuf, ArtifactBackend) {
    let (project, root) = fixture();
    let mut backend = ArtifactBackend::default();
    publication::fail_at(Some(phase));
    let result = capture_with_settings(
        &project,
        request(),
        &CaptureSettings::default(),
        &mut backend,
    );
    publication::fail_at(None);
    assert!(result.unwrap_err().contains("cutpoint"), "{phase}");
    (project, root, backend)
}
fn interrupted_decode(phase: &'static str) -> (ProjectState, PathBuf, ArtifactBackend) {
    let (project, root) = fixture();
    let mut backend = ArtifactBackend::default();
    capture_with_settings(
        &project,
        request(),
        &CaptureSettings::default(),
        &mut backend,
    )
    .unwrap();
    publication::fail_at(Some(phase));
    let result = decode(
        &project,
        7,
        1,
        Some(GreaseweazleProfile::Ibm720),
        &mut backend,
    );
    publication::fail_at(None);
    assert!(result.unwrap_err().contains("cutpoint"), "{phase}");
    (project, root, backend)
}

#[test]
fn prepared_capture_cuts_reopen_and_publish_same_slot_without_another_command() {
    for phase in ["capture_prepared", "capture_payload", "capture_committed"] {
        let (project, root, backend) = interrupted_capture(phase);
        let reopened = ProjectState::open_without_session(project.root().to_owned()).unwrap();
        let resumed =
            resume_capture_publication(&reopened, request(), &CaptureSettings::default(), 1)
                .unwrap();
        assert_eq!(resumed, phase != "capture_committed");
        assert_eq!(backend.commands.len(), 1);
        let record: CaptureRecord =
            serde_json::from_slice(&fs::read(root.join("Flux/007_attempt_001.json")).unwrap())
                .unwrap();
        assert_eq!(record.status, "complete");
        assert_eq!(
            fs::read(root.join("Flux/007_attempt_001.scp")).unwrap(),
            b"SCP synthetic flux"
        );
        assert!(!root.join("Flux/007_attempt_002.scp").exists());
        assert!(
            !resume_capture_publication(&reopened, request(), &CaptureSettings::default(), 1)
                .unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn prepared_decode_cuts_publish_same_slot_without_conversion_or_media_access() {
    for phase in ["decode_prepared", "decode_payload", "decode_committed"] {
        let (project, root, backend) = interrupted_decode(phase);
        let reopened = ProjectState::open_without_session(project.root().to_owned()).unwrap();
        let resumed =
            resume_decode_publication(&reopened, 7, 1, GreaseweazleProfile::Ibm720).unwrap();
        assert_eq!(resumed.is_some(), phase != "decode_committed");
        let status = inspect_disk(&reopened, 7).unwrap();
        assert_eq!(status.decodes.len(), 1);
        let (bytes, bad) = verified_decode(&reopened, &status.decodes[0], 1440).unwrap();
        assert_eq!(bytes, vec![0x33; 737_280]);
        assert_eq!(bad, [711].into());
        assert_eq!(backend.commands.len(), 2);
        assert_eq!(
            fs::read(root.join("Flux/007_attempt_001.scp")).unwrap(),
            b"SCP synthetic flux"
        );
        assert!(
            resume_decode_publication(&reopened, 7, 1, GreaseweazleProfile::Ibm720)
                .unwrap()
                .is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn unsealed_host_output_and_reserved_attempts_never_become_completed_evidence() {
    for phase in ["capture_reserved", "capture_host_finished"] {
        let (project, root, mut backend) = interrupted_capture(phase);
        let before = fs::read(root.join("Flux/007_attempt_001.partial.json")).unwrap();
        assert!(
            !resume_capture_publication(&project, request(), &CaptureSettings::default(), 1)
                .unwrap()
        );
        assert_eq!(
            fs::read(root.join("Flux/007_attempt_001.partial.json")).unwrap(),
            before
        );
        assert!(!root.join("Flux/007_attempt_001.json").exists());
        assert_eq!(
            capture(&project, request(), &mut backend)
                .unwrap()
                .attempt_number,
            2
        );
        fs::remove_dir_all(root).unwrap();
    }
    for phase in ["decode_reserved", "decode_host_finished"] {
        let (project, root, mut backend) = interrupted_decode(phase);
        assert!(
            resume_decode_publication(&project, 7, 1, GreaseweazleProfile::Ibm720)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            decode(
                &project,
                7,
                1,
                Some(GreaseweazleProfile::Ibm720),
                &mut backend
            )
            .unwrap()
            .decode_attempt,
            2
        );
        assert!(
            root.join("Flux/Derived/007_flux_001_ibm_720_decode_001.partial.json")
                .exists()
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn prepared_capture_rejects_changed_bytes_settings_paths_and_destination_collisions() {
    for mutation in ["payload", "settings", "path", "destination", "receipt"] {
        let (project, root, _) = interrupted_capture("capture_prepared");
        let partial = root.join("Flux/007_attempt_001.partial.scp");
        let metadata = root.join("Flux/007_attempt_001.partial.json");
        let mut req = request();
        match mutation {
            "payload" => fs::write(&partial, b"replacement evidence").unwrap(),
            "settings" => req.drive = 'B',
            "destination" => {
                fs::write(root.join("Flux/007_attempt_001.scp"), b"operator file").unwrap()
            }
            _ => {
                let mut record: CaptureRecord =
                    serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
                if mutation == "path" {
                    record.flux_file = Some("../outside.scp".into());
                } else {
                    record.bytes = Some(1);
                }
                fs::write(&metadata, serde_json::to_vec(&record).unwrap()).unwrap();
            }
        }
        let before = fs::read(&partial).unwrap();
        assert!(
            resume_capture_publication(&project, req, &CaptureSettings::default(), 1).is_err(),
            "{mutation}"
        );
        assert_eq!(fs::read(&partial).unwrap(), before);
        assert!(!root.join("Flux/007_attempt_001.json").exists());
        if mutation == "destination" {
            assert_eq!(
                fs::read(root.join("Flux/007_attempt_001.scp")).unwrap(),
                b"operator file"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn prepared_decode_refuses_source_or_payload_changes_forged_names_and_competing_records() {
    for mutation in [
        "source",
        "payload",
        "path",
        "extent",
        "destination",
        "competing",
    ] {
        let (project, root, _) = interrupted_decode("decode_prepared");
        let directory = root.join("Flux/Derived");
        let stem = "007_flux_001_ibm_720_decode_001";
        let partial = directory.join(format!("{stem}.partial.img"));
        let metadata = directory.join(format!("{stem}.partial.json"));
        match mutation {
            "source" => {
                fs::write(root.join("Flux/007_attempt_001.scp"), b"changed raw source").unwrap()
            }
            "payload" => fs::write(&partial, vec![0x34; 737_280]).unwrap(),
            "destination" => {
                fs::write(directory.join(format!("{stem}.img")), b"operator file").unwrap()
            }
            _ => {
                let mut record: DecodeRecord =
                    serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
                match mutation {
                    "path" => record.output_file = "../escaped.img".into(),
                    "extent" => record.bytes = 512,
                    "competing" => {
                        record.decode_attempt = 2;
                        record.output_file = "007_flux_001_ibm_720_decode_002.img".into();
                        fs::write(
                            directory.join("007_flux_001_ibm_720_decode_002.partial.json"),
                            serde_json::to_vec(&record).unwrap(),
                        )
                        .unwrap();
                        continue_fixture(&project);
                        fs::remove_dir_all(root).unwrap();
                        continue;
                    }
                    _ => unreachable!(),
                }
                fs::write(&metadata, serde_json::to_vec(&record).unwrap()).unwrap();
            }
        }
        continue_fixture(&project);
        assert!(!directory.join(format!("{stem}.json")).exists());
        if mutation == "destination" {
            assert_eq!(
                fs::read(directory.join(format!("{stem}.img"))).unwrap(),
                b"operator file"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    fn continue_fixture(project: &ProjectState) {
        assert!(resume_decode_publication(project, 7, 1, GreaseweazleProfile::Ibm720).is_err());
    }
}

#[test]
fn recovery_reopens_prepared_work_after_budget_expiry_without_info_or_read_calls() {
    struct OfflineOnly(ArtifactBackend);
    impl GreaseweazleBackend for OfflineOnly {
        fn mode(&self) -> crate::greaseweazle::BackendMode {
            crate::greaseweazle::BackendMode::MockNoHardware
        }
        fn execute(
            &mut self,
            command: &GreaseweazleCommand,
        ) -> Result<crate::greaseweazle::GreaseweazleExecution, String> {
            assert_eq!(
                command.subcommand(),
                "convert",
                "A completed publication must never probe or reread media"
            );
            self.0.execute(command)
        }
    }
    for phase in [
        "capture_prepared",
        "capture_payload",
        "capture_committed",
        "decode_prepared",
        "decode_payload",
        "decode_committed",
    ] {
        let (project, root, _) = if phase.starts_with("capture") {
            interrupted_capture(phase)
        } else {
            interrupted_decode(phase)
        };
        let mut policy = crate::flux_recovery::RecoveryPolicy::default();
        policy.passes.truncate(1);
        fs::create_dir_all(root.join("Flux/Recovery")).unwrap();
        fs::write(root.join("Flux/Recovery/007_job.json"), serde_json::json!({
            "schema_version":1,"disk":7,"profile":"ibm.720","automatic_format":false,"drive":"A",
            "started_unix_ms":crate::external_tools::current_unix_ms(),"stage_budget_version":1,"policy":policy,
            "stages":[{"capture_attempt":1,"decode_attempt":null,"settings":CaptureSettings::default(),"capture_elapsed_ms":600_000,"capture_started_unix_ms":null}],"result":null
        }).to_string()).unwrap();
        let reopened = ProjectState::open_without_session(project.root().to_owned()).unwrap();
        let mut backend = OfflineOnly(ArtifactBackend::default());
        let result = crate::flux_recovery::recover(
            &reopened,
            7,
            GreaseweazleProfile::Ibm720,
            'A',
            policy.clone(),
            &mut backend,
            &|_| {},
        )
        .unwrap();
        assert!(result.resumed, "{phase}");
        assert_eq!(result.physical_reads_this_run, 0, "{phase}");
        assert_eq!(result.missing_lbas, [711]);
        assert!(result.image.ends_with("007_attempt_001.img"));
        assert_eq!(
            backend.0.commands.len(),
            usize::from(phase.starts_with("capture"))
        );
        let image = fs::read(&result.image).unwrap();
        let repeated = crate::flux_recovery::recover(
            &reopened,
            7,
            GreaseweazleProfile::Ibm720,
            'A',
            policy,
            &mut backend,
            &|_| {},
        )
        .unwrap();
        assert_eq!(result.image, repeated.image);
        assert_eq!(fs::read(&result.image).unwrap(), image);
        assert!(!root.join("Images/007_attempt_002.img").exists());
        assert_eq!(
            fs::read(root.join("Flux/007_attempt_001.scp")).unwrap(),
            b"SCP synthetic flux"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
