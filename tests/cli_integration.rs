//! End-to-end CLI contract checks using a disposable project and no physical drive.

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

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
