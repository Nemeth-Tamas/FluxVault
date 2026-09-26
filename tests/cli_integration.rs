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
    fs::remove_dir_all(root).unwrap();
}
