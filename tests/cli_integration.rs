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
        "--gw-drive",
        "B",
        "--source-write-protected",
        "--acquisition-only",
        "--json",
    ];
    let blocked = invoke_mock_with_input(
        &project,
        &app_data,
        &["greaseweazle", "scan", "--gw-drive", "B", "--json"],
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
    let bounded = [
        "greaseweazle",
        "scan",
        "--count",
        "1",
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
            if line.contains("Type READ 001") {
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
fn native_fat12_cli_recovers_saved_partial_image_without_tools_and_reuses_it() {
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
    let source_sha = format!("{:x}", Sha256::digest(&image));
    fs::write(project.images_dir().join("001.img"), &image).unwrap();
    fs::write(project.logs_dir().join("001.log"), format!(
        "BEGIN | disk=1\nGEOMETRY | bytes_per_sector=512 | total_sectors=2880\nBAD_SECTOR | LBA=34\nEND | status=PARTIAL | bytes=1474560 | sha256={source_sha}\n")).unwrap();
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
    assert_eq!(json["recovery"]["source_sha256"], source_sha);
    let output = std::path::PathBuf::from(json["recovery"]["output_directory"].as_str().unwrap());
    assert_eq!(fs::read(output.join("GOOD.TXT")).unwrap(), b"hello");
    let second = invoke(&root, &["recovery", "extract", "1", "--json"], None);
    assert_eq!(second.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(json["recovery"]["reused"], true);
    assert!(!project.logs_dir().join("external-tools.jsonl").exists());
    assert_eq!(
        fs::read(project.images_dir().join("001.img")).unwrap(),
        image
    );
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
