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
    greaseweazle::{
        GreaseweazleBackend, GreaseweazleCommand, GreaseweazleDeviceStatus, GreaseweazleProfile,
        GreaseweazleProgressEvent, ProcessGreaseweazleBackend, classify_info_output,
        parse_info_output,
    },
    project::ProjectState,
};

fn mock_gw_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock_gw"))
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
        &["--version".to_owned()],
        &audit_path,
        Duration::from_secs(2),
    );
    assert!(version.audit.success);
    assert!(version.audit.stdout.contains("1.23"));
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

    let info = parse_info_output(&execution.stdout);
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
    assert!(audit.stdout.contains("Greaseweazle V4"));

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
    assert_eq!(
        result.host_version.as_deref(),
        Some("Greaseweazle Tools v1.23")
    );

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
        classify_info_output(&execution.stdout),
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
