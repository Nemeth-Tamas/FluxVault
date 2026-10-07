//! End-to-end CLI contract checks using a disposable project and no physical drive.

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

fn invoke(cwd: &Path, args: &[&str], input: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    child.wait_with_output().unwrap()
}

#[test]
#[ignore = "requires FLUXVAULT_TEST_7Z and FLUXVAULT_TEST_LIBREOFFICE; real saved-file tools, mock Greaseweazle only"]
fn default_background_scan_runs_the_whole_cli_path_without_physical_hardware() {
    let root = std::env::temp_dir().join(format!(
        "fv-bg-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    for (tool, variable) in [
        ("greaseweazle", None),
        ("sevenzip", Some("FLUXVAULT_TEST_7Z")),
        ("libreoffice", Some("FLUXVAULT_TEST_LIBREOFFICE")),
    ] {
        let executable = variable
            .map(|v| std::env::var(v).unwrap())
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_mock_gw").into());
        assert!(
            invoke_with_mock_gw(
                &project,
                &app_data,
                &["tools", "set", tool, &executable],
                false
            )
            .status
            .success()
        );
    }
    let scanned = invoke_mock_with_input(
        &project,
        &app_data,
        &["scan", "--last-disk", "2", "--json"],
        false,
        Some(b"1\n2\n"),
        &[],
    );
    // Mock images have known readable sectors but no usable filesystem. Their
    // processing must finish with attention, not crash or claim invented files.
    assert_eq!(
        scanned.status.code(),
        Some(3),
        "{}\n{}",
        String::from_utf8_lossy(&scanned.stdout),
        String::from_utf8_lossy(&scanned.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&scanned.stdout).unwrap();
    assert_eq!(value["scanned"], 2);
    assert_eq!(
        value["processing"]["background_processing"]["processed_jobs"],
        2
    );
    assert_eq!(
        value["processing"]["background_processing"]["errors"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(value["processing"]["disks"], 2);
    assert_eq!(value["processing"]["converted_ok"], 0);
    assert_eq!(
        value["capture_storage"]["errors"].as_array().unwrap().len(),
        0
    );
    let state = invoke_with_mock_gw(
        &project,
        &app_data,
        &["processing", "status", "--json"],
        true,
    );
    assert!(state.status.success());
    let state: serde_json::Value = serde_json::from_slice(&state.stdout).unwrap();
    assert_eq!(state["owner_active"], false);
    assert_eq!(state["pending"], 0);
    assert_eq!(state["attention"], 2);
    assert!(project.join("Logs/ProcessingEvents.jsonl").is_file());
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(project.join(".fluxvault-gw-scan.json")).unwrap())
            .unwrap();
    assert_eq!(journal["background_processing"], true);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn default_scan_discovers_dd_packs_and_reuses_evidence_across_cli_processes() {
    let root = std::env::temp_dir().join(format!(
        "fv-auto-packed-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &[
                "tools",
                "set",
                "greaseweazle",
                env!("CARGO_BIN_EXE_mock_gw")
            ],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(&project, &app_data, &["disk", "select", "9"], false)
            .status
            .success()
    );
    let scanned = invoke_mock_with_input(
        &project,
        &app_data,
        &["scan", "--last-disk", "9", "--acquisition-only", "--json"],
        false,
        Some(b"9\n"),
        &[("MOCK_GW_MEDIA_FORMAT", "ibm.720")],
    );
    assert_eq!(
        scanned.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&scanned.stdout),
        String::from_utf8_lossy(&scanned.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&scanned.stdout).unwrap();
    assert_eq!(summary["capture_storage"]["managed_packing"], true);
    assert_eq!(
        summary["capture_storage"]["errors"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(project.join(".fluxvault-gw-scan.json")).unwrap())
            .unwrap();
    assert_eq!(journal["automatic_format"], true);
    assert_eq!(journal["packed_captures"], true);
    assert_eq!(journal["completed"][0]["selected_profile"], "ibm.720");
    assert!(!project.join("Flux/009_attempt_001.scp").exists());
    assert!(project.join("Flux/009_attempt_001.scp.zip").is_file());
    let audit = project.join("Logs/external-tools.jsonl");
    let before = fs::read_to_string(&audit).unwrap();
    let identified = invoke_with_mock_gw(
        &project,
        &app_data,
        &["greaseweazle", "identify", "9", "--json"],
        true,
    );
    assert_eq!(identified.status.code(), Some(0));
    let identified: serde_json::Value = serde_json::from_slice(&identified.stdout).unwrap();
    assert_eq!(identified["decision"]["selected_profile"], "ibm.720");
    assert_eq!(fs::read_to_string(&audit).unwrap(), before);
    let decoded = invoke_with_mock_gw(
        &project,
        &app_data,
        &["greaseweazle", "decode", "9", "--json"],
        true,
    );
    assert_eq!(decoded.status.code(), Some(3));
    let added = fs::read_to_string(&audit).unwrap();
    let commands: Vec<serde_json::Value> = added
        .lines()
        .skip(before.lines().count())
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["arguments"][0], "convert");
    assert!(
        commands[0]["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "--format=ibm.720")
    );
    assert_eq!(
        invoke_with_mock_gw(&project, &app_data, &["storage", "resume", "--json"], true)
            .status
            .code(),
        Some(0)
    );
    let events = fs::read_dir(project.join("Logs/Benchmark"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(events.lines().any(|line| {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        v.to_string().contains("disk_committed") && v.to_string().contains("ibm.720")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn raw_only_format_exceptions_advance_custody_keep_unknown_counts_and_block_all_clear() {
    let root = std::env::temp_dir().join(format!(
        "fv-raw-scan-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &[
                "tools",
                "set",
                "greaseweazle",
                env!("CARGO_BIN_EXE_mock_gw")
            ],
            false
        )
        .status
        .success()
    );
    let output = invoke_mock_with_input(
        &project,
        &app_data,
        &["scan", "--last-disk", "2", "--acquisition-only", "--json"],
        false,
        Some(b"1\n2\n"),
        &[("MOCK_GW_MEDIA_FORMAT", "unsupported-test-format")],
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["scanned"], 2);
    assert_eq!(value["next_disk"], 3);
    assert_eq!(value["disks"][0]["status"], "raw_format_exception");
    assert!(String::from_utf8_lossy(&output.stderr).contains("RAW-ONLY FORMAT EXCEPTION SAVED"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown missing"));
    assert!(
        fs::read_dir(project.join("Images"))
            .unwrap()
            .next()
            .is_none()
    );
    let audit = project.join("Logs/external-tools.jsonl");
    let before = fs::read(&audit).unwrap();
    let report = invoke_with_mock_gw(
        &project,
        &app_data,
        &["benchmark", "report", "--json"],
        true,
    );
    let benchmark: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    // The exporter returns its snapshot paths; inspect the committed snapshot.
    let path = benchmark["json"]
        .as_str()
        .or_else(|| benchmark["summary"].as_str())
        .unwrap();
    let measured: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(measured["status_counts"]["raw_format_exception"], 2);
    assert_eq!(
        measured["disks"][0]["outcome"]["missing_sectors"],
        serde_json::Value::Null
    );
    assert_eq!(
        measured["disks"][0]["outcome"]["image_sha256"],
        serde_json::Value::Null
    );
    for args in [
        ["status", "--json"].as_slice(),
        ["recovery", "queue", "--json"].as_slice(),
        ["audit", "--json"].as_slice(),
    ] {
        let status = invoke_with_mock_gw(&project, &app_data, args, true);
        assert_eq!(
            status.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&status.stdout)
        );
    }
    assert_eq!(fs::read(&audit).unwrap(), before);
    // Reconcile a crash before the numbering commit of disk 002. No hardware,
    // new decode, packing duplicate, fake image or second number advance.
    let journal_path = project.join(".fluxvault-gw-scan.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    let second = journal["completed"].as_array_mut().unwrap().pop().unwrap();
    journal["pending"] = serde_json::json!({"disk":2,"result":second});
    fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    assert!(
        invoke_with_mock_gw(&project, &app_data, &["disk", "select", "2"], true)
            .status
            .success()
    );
    let resumed = invoke_with_mock_gw(
        &project,
        &app_data,
        &["scan", "--acquisition-only", "--json"],
        true,
    );
    assert_eq!(
        resumed.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&resumed.stdout)
    );
    let resumed: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(resumed["resumed_advances"], 1);
    assert_eq!(resumed["next_disk"], 3);
    assert_eq!(fs::read(&audit).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn enter_only_scan_is_explicit_colored_json_clean_and_saved_workers_survive_restart() {
    let root = std::env::temp_dir().join(format!(
        "fv-enter-scan-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &[
                "tools",
                "set",
                "greaseweazle",
                env!("CARGO_BIN_EXE_mock_gw")
            ],
            false
        )
        .status
        .success()
    );
    let first = invoke_mock_with_input(
        &project,
        &app_data,
        &[
            "scan",
            "--last-disk",
            "2",
            "--no-verify",
            "--conversion-workers",
            "12",
            "--color",
            "always",
            "--acquisition-only",
            "--json",
        ],
        false,
        Some(b"999\n\n\r\n\n"),
        &[],
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(summary["scanned"], 2);
    assert_eq!(summary["next_disk"], 3);
    assert_eq!(summary["conversion_workers"], 12);
    assert_eq!(summary["identity_confirmation"], "enter_only");
    let output = String::from_utf8_lossy(&first.stderr);
    assert!(output.contains("WARNING: --no-verify"));
    assert!(output.contains("DONE 002 / REMOVE 002 / BATCH FINISHED"));
    assert!(output.contains('\x1b'));
    assert!(!output.contains("INSERT 003"));
    // Even forced color must not animate into redirected stderr or JSON.
    assert!(!output.contains('\r'));
    assert!(!first.stdout.contains(&0x1b));
    let second = invoke_mock_with_input(
        &project,
        &app_data,
        &["scan", "--acquisition-only", "--json"],
        true,
        Some(b"\n"),
        &[],
    );
    assert_eq!(second.status.code(), Some(0));
    let summary: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(summary["scanned"], 0);
    assert_eq!(summary["conversion_workers"], 12);
    assert_eq!(summary["identity_confirmation"], "numbered");
    assert!(!second.stderr.contains(&0x1b));
    assert!(!String::from_utf8_lossy(&second.stderr).contains("WARNING: --no-verify"));
    fs::remove_dir_all(root).unwrap();
}

fn invoke_with_mock_gw(cwd: &Path, app_data: &Path, args: &[&str], device_missing: bool) -> Output {
    invoke_mock_with_input(cwd, app_data, args, device_missing, None, &[])
}

fn invoke_mock_with_input(
    cwd: &Path,
    app_data: &Path,
    args: &[&str],
    device_missing: bool,
    input: Option<&[u8]>,
    environment: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
    command
        .current_dir(cwd)
        .args(args)
        .env("APPDATA", app_data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if device_missing {
        command.env("MOCK_GW_DEVICE_NOT_FOUND", "1");
    } else {
        command.env_remove("MOCK_GW_DEVICE_NOT_FOUND");
    }
    command.envs(environment.iter().copied());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

#[test]
fn cli_no_index_reseats_preserve_attempts_same_disk_custody_and_json_contract() {
    let root = std::env::temp_dir().join(format!(
        "fv-cli-reseat-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &[
                "tools",
                "set",
                "greaseweazle",
                env!("CARGO_BIN_EXE_mock_gw")
            ],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(&project, &app_data, &["disk", "select", "21"], false)
            .status
            .success()
    );
    let scan = [
        "greaseweazle",
        "scan",
        "--gw-drive",
        "B",
        "--source-write-protected",
        "--profile",
        "ibm.1440",
        "--last-disk",
        "22",
        "--no-verify",
        "--acquisition-only",
        "--json",
        "--color",
        "always",
    ];
    // Disk 21 needs one reseat. Disk 22 needs both allowed reseats, showing
    // the cap resets per disk and failed attempts are never overwritten.
    let output = invoke_mock_with_input(
        &project,
        &app_data,
        &scan,
        false,
        Some(b"\n\n\n\n\n"),
        &[
            (
                "MOCK_GW_NO_INDEX_ATTEMPTS",
                "021_attempt_001,022_attempt_001,022_attempt_002",
            ),
            ("MOCK_GW_NO_INDEX_PARTIAL", "1"),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["scanned"], 2);
    assert_eq!(value["next_disk"], 23);
    assert_eq!(value["pending_disk"], serde_json::Value::Null);
    assert_eq!(
        value["disks"][0]["capture_attempts"],
        serde_json::json!([2])
    );
    assert_eq!(
        value["disks"][1]["capture_attempts"],
        serde_json::json!([3])
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains('\x1b'));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("NO INDEX 021 / REMOVE AND REINSERT SAME DISK / RETRY 1 OF 2"));
    assert!(stderr.contains("NO INDEX 022 / REMOVE AND REINSERT SAME DISK / RETRY 2 OF 2"));
    assert!(!stderr.contains("INSERT 023"));
    let mut failures = Vec::new();
    for stem in ["021_attempt_001", "022_attempt_001", "022_attempt_002"] {
        let path = project.join(format!("Flux/{stem}.partial.json"));
        let bytes = fs::read(&path).unwrap();
        let failed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(failed["status"], "failed");
        assert!(failed["detail"].as_str().unwrap().contains("No Index"));
        failures.push((path, bytes));
        let partial_raw = project.join(format!("Flux/{stem}.partial.scp"));
        assert_eq!(
            fs::read(&partial_raw).unwrap(),
            b"synthetic interrupted raw flux"
        );
        failures.push((partial_raw, b"synthetic interrupted raw flux".to_vec()));
    }
    let audit_path = project.join("Logs/external-tools.jsonl");
    let rows: Vec<serde_json::Value> = fs::read_to_string(&audit_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let reads: Vec<_> = rows
        .iter()
        .filter(|r| r["arguments"][0] == "read")
        .collect();
    assert_eq!(reads.len(), 5);
    assert_eq!(reads.iter().filter(|r| r["success"] == false).count(), 3);
    assert!(rows.iter().all(|r| !matches!(
        r["arguments"][0].as_str(),
        Some("write" | "erase" | "clean" | "update")
    )));
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(value["benchmark"]["summary"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(report["unique_committed_disks"], 2);
    assert_eq!(report["recovery_errors"], 3);
    let resumed = invoke_mock_with_input(&project, &app_data, &scan, true, Some(b"\n"), &[]);
    assert_eq!(resumed.status.code(), Some(0)); // Endpoint reuse needs no board/read.
    assert_eq!(
        fs::read_to_string(&audit_path)
            .unwrap()
            .lines()
            .filter(
                |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["arguments"][0]
                    == "read"
            )
            .count(),
        5
    );
    for (path, bytes) in failures {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_no_index_enter_only_eof_retains_pending_disk_and_resumes_after_reseat() {
    let root = std::env::temp_dir().join(format!(
        "fv-cli-reseat-eof-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &[
                "tools",
                "set",
                "greaseweazle",
                env!("CARGO_BIN_EXE_mock_gw")
            ],
            false
        )
        .status
        .success()
    );
    let scan = [
        "scan",
        "--last-disk",
        "1",
        "--no-verify",
        "--acquisition-only",
        "--json",
    ];
    let quit = invoke_mock_with_input(
        &project,
        &app_data,
        &scan,
        false,
        Some(b"\n"),
        &[("MOCK_GW_NO_INDEX", "1")],
    );
    assert_eq!(
        quit.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&quit.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&quit.stdout).unwrap();
    assert_eq!(value["pending_disk"], 1);
    assert_eq!(value["scanned"], 0);
    assert_eq!(value["next_disk"], 1);
    let failed_path = project.join("Flux/001_attempt_001.partial.json");
    let failed_bytes = fs::read(&failed_path).unwrap();
    let resumed = invoke_mock_with_input(&project, &app_data, &scan, false, Some(b"\n"), &[]);
    assert_eq!(
        resumed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(
        value["disks"][0]["capture_attempts"],
        serde_json::json!([2])
    );
    assert_eq!(value["next_disk"], 2);
    assert_eq!(fs::read(failed_path).unwrap(), failed_bytes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_mixed_format_scan_switches_at_009_and_binds_resume_to_the_saved_map() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-mixed-pilot-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &[
                "tools",
                "set",
                "greaseweazle",
                env!("CARGO_BIN_EXE_mock_gw")
            ],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(&project, &app_data, &["disk", "select", "8"], false)
            .status
            .success()
    );
    let map = root.join("profiles.json");
    fs::write(
        &map,
        include_str!("../policies/customer-first-20-profiles.json"),
    )
    .unwrap();
    let policy = root.join("policy.json");
    fs::write(&policy, include_str!("../policies/pilot-short.json")).unwrap();
    let scan = [
        "greaseweazle",
        "scan",
        "--gw-drive",
        "B",
        "--source-write-protected",
        "--profile-map",
        map.to_str().unwrap(),
        "--policy",
        policy.to_str().unwrap(),
        "--last-disk",
        "10",
        "--acquisition-only",
        "--json",
    ];
    let first = invoke_mock_with_input(
        &project,
        &app_data,
        &scan,
        false,
        Some(b"READ 008\nQUIT\n"),
        &[],
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let failed = invoke_mock_with_input(&project, &app_data, &scan, true, Some(b"READ 009\n"), &[]);
    assert_eq!(failed.status.code(), Some(2));
    fs::write(
        &map,
        r#"{"schema_version":1,"profiles":[{"disk":9,"profile":"ibm.1440"}]}"#,
    )
    .unwrap();
    let refused =
        invoke_mock_with_input(&project, &app_data, &scan, false, Some(b"READ 009\n"), &[]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("same profile"));
    fs::write(
        &map,
        include_str!("../policies/customer-first-20-profiles.json"),
    )
    .unwrap();
    let resumed = invoke_mock_with_input(
        &project,
        &app_data,
        &["scan", "--acquisition-only", "--json"],
        false,
        Some(b"\nREAD\n008\n009 extra\n009\n010\n011\n"),
        &[],
    );
    assert_eq!(
        resumed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&resumed.stdout)
    );
    let result: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(result["next_disk"], 11);
    assert_eq!(result["total_scanned"], 3);
    assert_eq!(
        fs::metadata(result["disks"][0]["image"].as_str().unwrap())
            .unwrap()
            .len(),
        737_280
    );
    assert_eq!(
        fs::metadata(result["disks"][1]["image"].as_str().unwrap())
            .unwrap()
            .len(),
        1_474_560
    );
    let audit = fs::read_to_string(project.join("Logs/external-tools.jsonl")).unwrap();
    let reads = audit
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|v| v["arguments"][0] == "read")
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 3); // Wrong-map restart did not cause an extra physical capture.
    for (row, expected) in
        reads
            .iter()
            .zip(["--format=ibm.1440", "--format=ibm.720", "--format=ibm.1440"])
    {
        assert!(
            row["arguments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == expected)
        );
    }
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(project.join(".fluxvault-gw-scan.json")).unwrap())
            .unwrap();
    assert_eq!(journal["profile_map"]["9"], "ibm.720");
    assert!(!String::from_utf8_lossy(&resumed.stderr).contains("Type 011"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_scan_processing_preflight_fails_before_custody_or_any_gw_command() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-scan-preflight-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    // Existing but incompatible tool makes failure deterministic even when real 7-Zip is installed.
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &["tools", "set", "sevenzip", env!("CARGO_BIN_EXE_mock_gw")],
            false
        )
        .status
        .success()
    );
    let response = invoke_mock_with_input(
        &project,
        &app_data,
        &[
            "greaseweazle",
            "scan",
            "--gw-drive",
            "B",
            "--source-write-protected",
            "--last-disk",
            "20",
            "--json",
        ],
        false,
        Some(b"READ 001\n"),
        &[],
    );
    assert_eq!(response.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&response.stdout)
            .contains("preflight failed before any media read")
    );
    assert!(!String::from_utf8_lossy(&response.stderr).contains("Type READ"));
    assert!(!project.join(".fluxvault-gw-scan.json").exists());
    assert!(fs::read_dir(project.join("Flux")).unwrap().next().is_none());
    let audit = fs::read_to_string(project.join("Logs/external-tools.jsonl")).unwrap();
    assert!(!audit.contains("Greaseweazle"));
    let capture_only = invoke_mock_with_input(
        &project,
        &app_data,
        &[
            "greaseweazle",
            "scan",
            "--gw-drive",
            "B",
            "--source-write-protected",
            "--last-disk",
            "20",
            "--acquisition-only",
            "--json",
        ],
        false,
        Some(b"QUIT\n"),
        &[],
    );
    assert_eq!(capture_only.status.code(), Some(0));
    let invalid_flag = invoke_with_mock_gw(
        &project,
        &app_data,
        &["status", "--profile-map", "nonexistent.json", "--json"],
        false,
    );
    assert_eq!(invalid_flag.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid_flag.stdout).contains("only valid with greaseweazle scan")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_guided_gw_scan_numbering_restart_partial_and_tamper_contract() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-gw-scan-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    assert!(
        invoke_with_mock_gw(
            &root,
            &app_data,
            &["init", project.to_str().unwrap()],
            false
        )
        .status
        .success()
    );
    assert!(
        invoke_with_mock_gw(
            &project,
            &app_data,
            &[
                "tools",
                "set",
                "greaseweazle",
                env!("CARGO_BIN_EXE_mock_gw")
            ],
            false
        )
        .status
        .success()
    );
    let scan = [
        "greaseweazle",
        "scan",
        "--count",
        "2",
        "--last-disk",
        "4",
        "--gw-drive",
        "B",
        "--source-write-protected",
        "--acquisition-only",
        "--json",
    ];
    let blocked = invoke_mock_with_input(
        &project,
        &app_data,
        &["greaseweazle", "scan", "--gw-drive", "C", "--json"],
        false,
        Some(b"READ 1\n"),
        &[],
    );
    assert_eq!(blocked.status.code(), Some(2));
    assert!(!project.join(".fluxvault-gw-scan.json").exists());
    let first = invoke_mock_with_input(
        &project,
        &app_data,
        &scan,
        false,
        Some(b"READ\nREAD 999\nREAD 001\nREAD 002\n"),
        &[],
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(response["scanned"], 2);
    assert_eq!(response["next_disk"], 3);
    assert_eq!(response["processing"]["skipped"], true);
    assert_eq!(response["benchmark"]["unique_disks"], 2);
    assert!(Path::new(response["benchmark"]["summary"].as_str().unwrap()).exists());
    assert!(String::from_utf8_lossy(&first.stderr).contains("No read started"));
    let journal_path = project.join(".fluxvault-gw-scan.json");
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    let audit_path = project.join("Logs/external-tools.jsonl");
    let reads = |path: &Path| {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|v| v["arguments"][0] == "read")
            .count()
    };
    assert_eq!(reads(&audit_path), 2);
    let end_reached = invoke_mock_with_input(
        &project,
        &app_data,
        &[
            "greaseweazle",
            "scan",
            "--gw-drive",
            "B",
            "--source-write-protected",
            "--last-disk",
            "2",
            "--acquisition-only",
            "--json",
        ],
        true,
        Some(b"READ 003\n"),
        &[],
    );
    assert_eq!(end_reached.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&end_reached.stdout).unwrap()["scanned"],
        0
    );
    assert_eq!(reads(&audit_path), 2);
    let bounded = [
        "greaseweazle",
        "scan",
        "--count",
        "1",
        "--last-disk",
        "4",
        "--gw-drive",
        "B",
        "--source-write-protected",
        "--acquisition-only",
        "--json",
    ];

    // Simulate a crash immediately before, or immediately after, the numbering commit.
    for before_number_commit in [true, false] {
        let mut interrupted = original.clone();
        let second = interrupted["completed"]
            .as_array_mut()
            .unwrap()
            .pop()
            .unwrap();
        interrupted["pending"] = serde_json::json!({"disk":2,"result":second});
        fs::write(
            &journal_path,
            serde_json::to_vec_pretty(&interrupted).unwrap(),
        )
        .unwrap();
        if before_number_commit {
            assert!(
                invoke_with_mock_gw(&project, &app_data, &["disk", "select", "2"], true)
                    .status
                    .success()
            );
        }
        let resumed = invoke_with_mock_gw(&project, &app_data, &bounded, true);
        assert_eq!(
            resumed.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&resumed.stdout)
        );
        let response: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
        assert_eq!(response["resumed_advances"], 1);
        assert_eq!(response["next_disk"], 3);
        assert_eq!(response["total_scanned"], 2);
        assert_eq!(response["disks"][0]["physical_reads_this_run"], 0);
        assert_eq!(reads(&audit_path), 2);
    }

    let absent =
        invoke_mock_with_input(&project, &app_data, &bounded, true, Some(b"READ 3\n"), &[]);
    assert_eq!(absent.status.code(), Some(2));
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    assert_eq!(state["pending"]["disk"], 3);
    assert_eq!(state["pending"]["result"], serde_json::Value::Null);
    let partial = invoke_mock_with_input(
        &project,
        &app_data,
        &bounded,
        false,
        Some(b"READ 003\n"),
        &[("MOCK_GW_BAD_LBAS", "24")],
    );
    assert_eq!(
        partial.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&partial.stdout)
    );
    let response: serde_json::Value = serde_json::from_slice(&partial.stdout).unwrap();
    assert_eq!(response["partial_this_session"], 1);
    assert_eq!(response["next_disk"], 4);
    assert_eq!(
        response["disks"][0]["missing_lbas"],
        serde_json::json!([24])
    );

    // A changed published image blocks restart before advancing or touching a drive.
    let mut tampered: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    let third = tampered["completed"].as_array_mut().unwrap().pop().unwrap();
    let image_path = third["image"].as_str().unwrap().to_owned();
    tampered["pending"] = serde_json::json!({"disk":3,"result":third});
    fs::write(&journal_path, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
    assert!(
        invoke_with_mock_gw(&project, &app_data, &["disk", "select", "3"], true)
            .status
            .success()
    );
    let mut bytes = fs::read(&image_path).unwrap();
    bytes[0] ^= 1;
    fs::write(image_path, bytes).unwrap();
    let read_count = reads(&audit_path);
    let refused = invoke_with_mock_gw(&project, &app_data, &bounded, true);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("output changed"));
    assert_eq!(reads(&audit_path), read_count);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(project.join("project.json")).unwrap()).unwrap();
    assert_eq!(metadata["current_disk_number"], 3);
    let benchmark = invoke_with_mock_gw(
        &project,
        &app_data,
        &["benchmark", "report", "--json"],
        true,
    );
    assert_eq!(benchmark.status.code(), Some(0));
    let measured: serde_json::Value = serde_json::from_slice(&benchmark.stdout).unwrap();
    assert_eq!(measured["physical_media_access"], false);
    assert_eq!(measured["benchmark"]["unique_committed_disks"], 3);
    assert_eq!(measured["benchmark"]["recovery_errors"], 2);
    assert_eq!(measured["benchmark"]["status_counts"]["partial"], 1);
    assert_eq!(reads(&audit_path), read_count);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_gw_reservation_blocks_other_projects_and_releases_after_process_termination() {
    use std::{
        io::{BufRead, BufReader},
        sync::mpsc,
        thread,
        time::Duration,
    };
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-gw-reservation-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let first_project = root.join("first");
    let second_project = root.join("second");
    for project in [&first_project, &second_project] {
        assert!(
            invoke_with_mock_gw(
                &root,
                &app_data,
                &["init", project.to_str().unwrap()],
                false
            )
            .status
            .success()
        );
    }
    let scan = [
        "greaseweazle",
        "scan",
        "--gw-drive",
        "B",
        "--source-write-protected",
        "--acquisition-only",
        "--json",
    ];
    let mut owner = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .current_dir(&first_project)
        .args(scan)
        .env("APPDATA", &app_data)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = owner.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("Type 001") {
                let _ = sender.send(());
            }
        }
    });
    let ready = receiver.recv_timeout(Duration::from_secs(5));
    if ready.is_err() {
        owner.kill().unwrap();
        owner.wait().unwrap();
    }
    assert!(
        ready.is_ok(),
        "scan must reach its custody prompt without starting a host tool"
    );
    let mut blocked = Vec::new();
    for args in [
        &scan[..],
        &[
            "greaseweazle",
            "capture",
            "1",
            "--gw-drive",
            "B",
            "--profile",
            "ibm.1440",
            "--source-write-protected",
            "--json",
        ][..],
        &[
            "greaseweazle",
            "recover",
            "1",
            "--gw-drive",
            "B",
            "--source-write-protected",
            "--json",
        ][..],
        &["greaseweazle", "info", "--json"][..],
    ] {
        blocked.push(invoke_with_mock_gw(&second_project, &app_data, args, false));
    }
    owner.kill().unwrap();
    owner.wait().unwrap();
    reader.join().unwrap();
    for response in blocked {
        assert_eq!(response.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&response.stdout).contains("reserved by another"));
    }
    let after = invoke_mock_with_input(
        &second_project,
        &app_data,
        &scan,
        false,
        Some(b"QUIT\n"),
        &[],
    );
    assert_eq!(
        after.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&after.stdout)
    );
    for project in [&first_project, &second_project] {
        assert!(fs::read_dir(project.join("Flux")).unwrap().next().is_none());
        assert!(!project.join("Logs/external-tools.jsonl").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_only_entry_point_and_greaseweazle_preview_need_no_hardware() {
    let cwd = std::env::temp_dir();
    let help = invoke(&cwd, &[], None);
    assert_eq!(help.status.code(), Some(0));
    let help_text = String::from_utf8(help.stdout).unwrap();
    assert!(help_text.contains("fluxvault init [path]"));
    assert!(!help_text.contains("Open the GUI"));

    let preview = invoke(&cwd, &["greaseweazle", "preview", "--json"], None);
    assert_eq!(preview.status.code(), Some(0));
    let preview_json: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(preview_json["executed"], false);
    assert_eq!(preview_json["source_media_access"], "read_only");
    for example in preview_json["examples"].as_array().unwrap() {
        let arguments = example["raw_capture"].as_array().unwrap();
        assert!(arguments.iter().any(|arg| arg == "--raw"));
        assert!(arguments.iter().any(|arg| arg == "--no-clobber"));
        assert!(!arguments.iter().any(|arg| arg == "write"));
    }
}

#[test]
fn native_fat12_cli_recovers_missing_boot_without_tools_and_reuses_warned_result() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-native-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let project = fluxvault::project::ProjectState::create_without_session(root.clone()).unwrap();
    let mut image = vec![0; 2880 * 512];
    image[11..13].copy_from_slice(&512u16.to_le_bytes());
    image[13] = 1;
    image[14..16].copy_from_slice(&1u16.to_le_bytes());
    image[16] = 2;
    image[17..19].copy_from_slice(&224u16.to_le_bytes());
    image[19..21].copy_from_slice(&2880u16.to_le_bytes());
    image[22..24].copy_from_slice(&9u16.to_le_bytes());
    image[510..512].copy_from_slice(&[0x55, 0xaa]);
    for start in [512, 10 * 512] {
        image[start..start + 5].copy_from_slice(&[0xf0, 0xff, 0xff, 0xff, 0x0f]);
    }
    let entry = 19 * 512;
    image[entry..entry + 11].copy_from_slice(b"GOOD    TXT");
    image[entry + 11] = 0x20;
    image[entry + 26..entry + 28].copy_from_slice(&2u16.to_le_bytes());
    image[entry + 28..entry + 32].copy_from_slice(&5u32.to_le_bytes());
    image[33 * 512..33 * 512 + 5].copy_from_slice(b"hello");
    image[..512].fill(0); // Disposable fixture only; no physical media.
    let source_sha = format!("{:x}", Sha256::digest(&image));
    fs::write(project.images_dir().join("001.img"), &image).unwrap();
    fs::write(project.logs_dir().join("001.log"), format!(
        "BEGIN | disk=1\nGEOMETRY | bytes_per_sector=512 | total_sectors=2880\nBAD_SECTOR | LBA=0\nBAD_SECTOR | LBA=34\nEND | status=PARTIAL | bytes=1474560 | sha256={source_sha}\n")).unwrap();
    let first = invoke(&root, &["recovery", "extract", "1", "--json"], None);
    assert_eq!(
        first.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(json["physical_media_access"], false);
    assert_eq!(json["recovery"]["customer_delivery_certified"], false);
    assert_eq!(json["recovery"]["files"], 1);
    assert_eq!(
        json["recovery"]["layout_method"],
        "inferred_standard_layout"
    );
    assert!(json["recovery"]["layout_warning"].is_string());
    assert!(String::from_utf8_lossy(&first.stderr).contains("layout WARNING"));
    assert_eq!(json["recovery"]["source_sha256"], source_sha);
    let output = std::path::PathBuf::from(json["recovery"]["output_directory"].as_str().unwrap());
    assert_eq!(fs::read(output.join("GOOD.TXT")).unwrap(), b"hello");
    let second = invoke(&root, &["recovery", "extract", "1", "--json"], None);
    assert_eq!(second.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(json["recovery"]["reused"], true);
    assert!(json["recovery"]["layout_warning"].is_string());
    let text = invoke(&root, &["recovery", "extract", "1"], None);
    assert_eq!(text.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&text.stdout).contains("WARNING:"));
    assert!(!project.logs_dir().join("external-tools.jsonl").exists());
    assert_eq!(
        fs::read(project.images_dir().join("001.img")).unwrap(),
        image
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_carving_cli_handles_unknown_layout_with_offsets_and_warned_reuse() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-carving-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let project = fluxvault::project::ProjectState::create_without_session(root.clone()).unwrap();
    let mut image = vec![0; 2880 * 512];
    let payload = b"{\\rtf1 bounded recovery candidate}";
    image[800 * 512 + 3..800 * 512 + 3 + payload.len()].copy_from_slice(payload);
    let sha = format!("{:x}", Sha256::digest(&image));
    fs::write(project.images_dir().join("001.img"), &image).unwrap();
    fs::write(project.logs_dir().join("001.log"), format!("BEGIN | disk=1\nGEOMETRY | bytes_per_sector=512 | total_sectors=2880\nBAD_SECTOR | LBA=0\nEND | status=PARTIAL | bytes=1474560 | sha256={sha}\n")).unwrap();
    let first = invoke(&root, &["recovery", "extract", "1", "--json"], None);
    assert_eq!(
        first.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(result["physical_media_access"], false);
    assert_eq!(result["recovery"]["carved_files"], 1);
    assert_eq!(
        result["recovery"]["layout_method"],
        "signature_only_unknown_filesystem"
    );
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(result["recovery"]["report_path"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert!(report["analysis"].is_null());
    assert!(report["filesystem_error"].is_string());
    assert_eq!(
        report["carving"]["files"][0]["source_extents"][0]["source_byte_offset"],
        800 * 512 + 3
    );
    assert_eq!(report["carving"]["files"][0]["original_name_known"], false);
    assert_eq!(
        report["carving"]["files"][0]["customer_delivery_certified"],
        false
    );
    let second = invoke(&root, &["recovery", "extract", "1", "--json"], None);
    assert_eq!(second.status.code(), Some(3));
    let reused: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(reused["recovery"]["reused"], true);
    let plan = invoke(&root, &["conversion", "plan", "--json"], None);
    assert!(plan.status.success());
    assert_eq!(
        fs::read(project.images_dir().join("001.img")).unwrap(),
        image
    );
    assert!(!project.logs_dir().join("external-tools.jsonl").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn flux_status_verifies_saved_hashes_without_a_drive_or_host_tool() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-flux-status-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let project = root.join("project");
    assert_eq!(
        invoke(&root, &["init", project.to_str().unwrap()], None)
            .status
            .code(),
        Some(0)
    );
    let flux_file = project.join("Flux").join("007_attempt_001.scp");
    let contents = b"SCP synthetic fixture";
    fs::write(&flux_file, contents).unwrap();
    let sha256 = format!("{:x}", Sha256::digest(contents));
    let metadata = serde_json::json!({
        "schema_version": 1,
        "disk_number": 7,
        "attempt_number": 1,
        "profile": "ibm.1440",
        "drive": "A",
        "revolutions": 3,
        "status": "complete",
        "flux_file": "007_attempt_001.scp",
        "bytes": contents.len(),
        "sha256": sha256,
        "command": ["read", "--raw", "--no-clobber"],
        "detail": null
    });
    fs::write(
        project.join("Flux").join("007_attempt_001.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    let healthy = invoke(&project, &["greaseweazle", "status", "7", "--json"], None);
    assert_eq!(healthy.status.code(), Some(3));
    let healthy_json: serde_json::Value = serde_json::from_slice(&healthy.stdout).unwrap();
    assert_eq!(healthy_json["evidence_healthy"], true);
    assert_eq!(healthy_json["physical_media_access"], false);
    fs::write(&flux_file, b"changed").unwrap();
    let changed = invoke(&project, &["greaseweazle", "status", "7", "--json"], None);
    assert_eq!(changed.status.code(), Some(3));
    let changed_json: serde_json::Value = serde_json::from_slice(&changed.stdout).unwrap();
    assert_eq!(changed_json["evidence_healthy"], false);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_mock_greaseweazle_capture_decode_and_consensus_never_need_media() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-gw-chain-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let app_data = root.join("app-data");
    let project = root.join("project");
    let mock_gw = env!("CARGO_BIN_EXE_mock_gw");
    let project_path = project.to_str().unwrap();
    assert_eq!(
        invoke_with_mock_gw(&root, &app_data, &["init", project_path], false)
            .status
            .code(),
        Some(0)
    );
    let configured = invoke_with_mock_gw(
        &project,
        &app_data,
        &["tools", "set", "greaseweazle", mock_gw, "--json"],
        false,
    );
    assert_eq!(configured.status.code(), Some(0));

    let info = invoke_with_mock_gw(
        &project,
        &app_data,
        &["greaseweazle", "info", "--json"],
        false,
    );
    assert_eq!(info.status.code(), Some(0));
    let info_json: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    assert_eq!(info_json["ready"], true);

    let capture_args = [
        "greaseweazle",
        "capture",
        "7",
        "--profile",
        "ibm.1440",
        "--source-write-protected",
        "--json",
    ];
    let missing = invoke_with_mock_gw(&project, &app_data, &capture_args, true);
    assert_eq!(missing.status.code(), Some(2));
    let missing_json: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert!(
        missing_json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No Greaseweazle board was found")
    );
    assert!(fs::read_dir(project.join("Flux")).unwrap().next().is_none());

    for attempt in 1..=2 {
        let capture = invoke_with_mock_gw(&project, &app_data, &capture_args, false);
        assert_eq!(
            capture.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&capture.stderr)
        );
        let capture_json: serde_json::Value = serde_json::from_slice(&capture.stdout).unwrap();
        assert_eq!(capture_json["attempt"], attempt);
        assert_eq!(capture_json["source_media_access"], "read_only");
        let decode = invoke_with_mock_gw(
            &project,
            &app_data,
            &["greaseweazle", "decode", "7", "--json"],
            false,
        );
        assert_eq!(
            decode.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&decode.stderr)
        );
        let decode_json: serde_json::Value = serde_json::from_slice(&decode.stdout).unwrap();
        assert_eq!(decode_json["capture_attempt"], attempt);
        assert_eq!(decode_json["gw_reported_found_sectors"], 2880);
        assert_eq!(decode_json["physical_media_access"], false);
    }

    let status = invoke_with_mock_gw(
        &project,
        &app_data,
        &["greaseweazle", "status", "7", "--json"],
        false,
    );
    assert_eq!(status.status.code(), Some(3));
    let status_json: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status_json["captures"].as_array().unwrap().len(), 2);
    assert_eq!(status_json["decodes"].as_array().unwrap().len(), 2);
    assert_eq!(status_json["evidence_healthy"], true);

    let consensus = invoke_with_mock_gw(
        &project,
        &app_data,
        &["greaseweazle", "consensus", "7", "--json"],
        false,
    );
    assert_eq!(consensus.status.code(), Some(0));
    let consensus_json: serde_json::Value = serde_json::from_slice(&consensus.stdout).unwrap();
    assert_eq!(
        consensus_json["matching_reported_good_lbas"]
            .as_array()
            .unwrap()
            .len(),
        2880
    );
    assert_eq!(consensus_json["physical_media_access"], false);

    // Recovery can seed an existing full decode, and completion is resumable
    // without a connected board. No new physical read is allowed in this case.
    let recover_args = [
        "greaseweazle",
        "recover",
        "7",
        "--gw-drive",
        "B",
        "--source-write-protected",
        "--acquisition-only",
        "--json",
    ];
    let recovered = invoke_with_mock_gw(&project, &app_data, &recover_args, false);
    assert_eq!(
        recovered.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&recovered.stdout)
    );
    let recovered_json: serde_json::Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert_eq!(recovered_json["recovery"]["status"], "acquired");
    assert_eq!(recovered_json["recovery"]["physical_reads_this_run"], 0);
    assert_eq!(recovered_json["customer_delivery_certified"], false);
    let resumed = invoke_with_mock_gw(&project, &app_data, &recover_args, true);
    assert_eq!(resumed.status.code(), Some(0));
    let resumed_json: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(resumed_json["recovery"]["resumed"], true);
    assert_eq!(resumed_json["recovery"]["physical_reads_this_run"], 0);
    assert_eq!(
        resumed_json["recovery"]["image"],
        recovered_json["recovery"]["image"]
    );

    let audit = fs::read_to_string(project.join("Logs").join("external-tools.jsonl")).unwrap();
    let commands: Vec<serde_json::Value> = audit
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for subcommand in ["info", "read", "convert"] {
        assert!(commands.iter().any(|entry| {
            entry["arguments"].as_array().is_some_and(|arguments| {
                arguments.first().and_then(|arg| arg.as_str()) == Some(subcommand)
            })
        }));
    }
    assert_eq!(
        commands
            .iter()
            .filter(|entry| entry["arguments"][0] == "read")
            .count(),
        2
    );
    assert!(!commands.iter().any(|entry| {
        entry["arguments"].as_array().is_some_and(|arguments| {
            arguments
                .iter()
                .any(|argument| matches!(argument.as_str(), Some("write" | "erase" | "clean")))
        })
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn executable_discovers_project_and_guards_guided_scan_without_hardware() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-e2e-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let project = root.join("project");
    let project_text = project.to_str().unwrap();
    let init = invoke(&root, &["init", project_text], None);
    assert_eq!(init.status.code(), Some(0));
    let nested = project.join("Extracted").join("007");
    fs::create_dir_all(&nested).unwrap();
    let status = invoke(&nested, &["status", "--json"], None);
    assert_eq!(status.status.code(), Some(0));
    let status_json: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status_json["current_disk"], 1);
    assert_eq!(status_json["disks"], 0);

    let denied = invoke(&nested, &["scan", "--drive", "A:", "--json"], None);
    assert_eq!(denied.status.code(), Some(2));
    let denied_json: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(denied_json["error"]["code"], "operation_error");
    assert!(
        denied_json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("independently verified")
    );

    let denied_flux = invoke(
        &nested,
        &[
            "greaseweazle",
            "capture",
            "7",
            "--profile",
            "ibm.1440",
            "--json",
        ],
        None,
    );
    assert_eq!(denied_flux.status.code(), Some(2));
    let denied_flux_json: serde_json::Value = serde_json::from_slice(&denied_flux.stdout).unwrap();
    assert!(
        denied_flux_json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--source-write-protected")
    );
    assert!(fs::read_dir(project.join("Flux")).unwrap().next().is_none());

    let missing_capture = invoke(&nested, &["greaseweazle", "decode", "7", "--json"], None);
    assert_eq!(missing_capture.status.code(), Some(2));
    let missing_json: serde_json::Value = serde_json::from_slice(&missing_capture.stdout).unwrap();
    assert!(
        missing_json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No completed raw-flux capture")
    );

    // QUIT exits before enumeration, probing, or reading a physical drive.
    let quit = invoke(
        &nested,
        &[
            "scan",
            "--drive",
            "A:",
            "--write-blocker-verified",
            "--json",
        ],
        Some(b"QUIT\n"),
    );
    assert_eq!(quit.status.code(), Some(0));
    let quit_json: serde_json::Value = serde_json::from_slice(&quit.stdout).unwrap();
    assert_eq!(quit_json["scanned"], 0);
    assert_eq!(quit_json["next_disk"], 1);
    assert_eq!(quit_json["source_media_access"], "read_only");
    assert!(String::from_utf8_lossy(&quit.stderr).contains("Type READ"));
    let finalize = invoke(
        &nested,
        &[
            "finalize",
            "--destination",
            root.to_str().unwrap(),
            "--json",
        ],
        None,
    );
    assert_eq!(finalize.status.code(), Some(2));
    let finalize_json: serde_json::Value = serde_json::from_slice(&finalize.stdout).unwrap();
    assert!(
        finalize_json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No saved disk images")
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires installed LibreOffice; uses only a disposable synthetic RTF"]
fn conversion_state_reuses_only_bound_outputs_across_cli_processes() {
    let root = std::env::temp_dir().join(format!(
        "fluxvault-cli-conversion-e2e-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let project = root.join("project");
    assert_eq!(
        invoke(&root, &["init", project.to_str().unwrap()], None)
            .status
            .code(),
        Some(0)
    );
    let source = project.join("Extracted").join("001").join("sample.rtf");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    let source_bytes = b"{\\rtf1\\ansi Disposable cross-process test}";
    fs::write(&source, source_bytes).unwrap();

    let first = invoke(&project, &["conversion", "run", "--json"], None);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = invoke(&project, &["conversion", "run", "--json"], None);
    assert_eq!(second.status.code(), Some(0));
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_json["reused_outputs"], 2);

    let pdf = project
        .join("Converted")
        .join("001")
        .join("sample [from RTF].pdf");
    assert!(pdf.is_file());
    fs::write(&pdf, b"%PDF-1.7\nvalid-looking but altered\n%%EOF\n").unwrap();
    let altered = invoke(&project, &["conversion", "run", "--json"], None);
    assert_eq!(altered.status.code(), Some(3));
    let altered_json: serde_json::Value = serde_json::from_slice(&altered.stdout).unwrap();
    assert_eq!(altered_json["issues"].as_array().unwrap().len(), 1);
    assert!(String::from_utf8_lossy(&fs::read(&pdf).unwrap()).contains("altered"));

    fs::remove_file(&pdf).unwrap();
    let retry = invoke(&project, &["conversion", "retry", "--json"], None);
    assert_eq!(
        retry.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(pdf.is_file());
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    fs::remove_dir_all(root).unwrap();
}
