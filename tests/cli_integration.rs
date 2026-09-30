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
    let mut command = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
    command
        .current_dir(cwd)
        .args(args)
        .env("APPDATA", app_data)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if device_missing {
        command.env("MOCK_GW_DEVICE_NOT_FOUND", "1");
    } else {
        command.env_remove("MOCK_GW_DEVICE_NOT_FOUND");
    }
    command.output().unwrap()
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
