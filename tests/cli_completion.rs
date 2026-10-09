//! Static shell-generation contracts; Windows engine tests need no disk or tools.
use std::process::Command;

#[test]
fn completion_generator_is_project_independent_and_byte_preserves_script() {
    let output = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .args(["completions", "powershell"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let script = include_str!("../scripts/FluxVault.Completion.ps1").replace("\r\n", "\n");
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n")
            .trim_end(),
        script.trim_end()
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires PowerShell 7 (pwsh); static completion tests never open media"]
fn powershell_engine_covers_context_cursor_literal_safety_and_native_registration() {
    let executable = env!("CARGO_BIN_EXE_fluxvault");
    let output = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scripts/FluxVault.Completion.Tests.ps1"),
        )
        .args(["-Executable", executable])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("checks passed"));
}
