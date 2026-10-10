//! Explicit installed-Windows-shell acceptance without changing the user's PATH.
#[cfg(windows)]
#[test]
#[ignore = "requires current release build and PowerShell 7; no media/user PATH changes"]
fn installed_aliases_work_in_powershell_cmd_and_automation_without_persistent_path_changes() {
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    let root = std::env::temp_dir().join(format!(
        "fv-install-fixture-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let result = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/InstallCli.Tests.ps1"))
        .arg("-FixtureRoot")
        .arg(&root)
        .output();
    // Only the uniquely-created test installation/project are removed.
    fs::remove_dir_all(&root).unwrap();
    let output = result.unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("installation checks passed"));
}
