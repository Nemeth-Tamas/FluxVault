use super::*;
fn project() -> ProjectState {
    ProjectState::create_without_session(std::env::temp_dir().join(format!(
                "fv-auto-audit-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
    .unwrap()
}
fn args() -> Vec<String> {
    vec!["files".into(), "manifest".into(), "--json".into()]
}
#[test]
fn material_error_still_refreshes_and_keeps_old_archive() {
    let p = project();
    let archive = p.root().join("existing.zip");
    fs::write(&archive, b"old customer bytes").unwrap();
    let error = run(&args(), p.root(), || {
        fs::write(p.extracted_dir().join("new.txt"), b"new saved evidence").unwrap();
        Err("injected failure after mutation".into())
    })
    .unwrap_err();
    assert!(error.contains("injected failure"));
    assert!(error.contains("audited automatically"));
    assert!(!p.root().join(PENDING).exists());
    assert!(p.reports_dir().join("FinalReportLatest.json").is_file());
    assert_eq!(fs::read(archive).unwrap(), b"old customer bytes");
    fs::remove_dir_all(p.root()).unwrap();
}
#[test]
fn unchanged_failed_command_does_not_export_an_audit() {
    let p = project();
    let error = run(&args(), p.root(), || Err("validation failed".into())).unwrap_err();
    assert_eq!(error, "validation failed");
    assert!(!p.root().join(PENDING).exists());
    assert!(!p.reports_dir().join("FinalReportLatest.json").exists());
    fs::remove_dir_all(p.root()).unwrap();
}
#[test]
fn restart_pending_marker_refreshes_even_when_next_command_is_unchanged() {
    let p = project();
    write(
        &p.root().join(PENDING),
        &serde_json::to_vec(
            &json!({"schema_version":1,"engine":"fluxvault-mutation-audit-v1","project":p.root()}),
        )
        .unwrap(),
    )
    .unwrap();
    let outcome = run(&args(), p.root(), || {
        Ok(CliResponse {
            output: "{}".into(),
            exit_code: 0,
        })
    })
    .unwrap();
    let v: Value = serde_json::from_str(&outcome.output).unwrap();
    assert_eq!(v["automatic_audit_refresh"]["pending_marker_cleared"], true);
    assert!(!p.root().join(PENDING).exists());
    fs::remove_dir_all(p.root()).unwrap();
}
#[test]
fn foreign_pending_marker_is_not_overwritten_or_removed() {
    let p = project();
    let path = p.root().join(PENDING);
    fs::write(&path, b"user data").unwrap();
    assert!(run(&args(), p.root(), || panic!("must refuse before work")).is_err());
    assert_eq!(fs::read(path).unwrap(), b"user data");
    fs::remove_dir_all(p.root()).unwrap();
}
#[test]
fn inspection_help_plan_and_normal_background_tail_are_not_direct_mutations() {
    for a in [
        vec!["recovery", "crc-check", "1"],
        vec!["diagnose", "1"],
        vec!["greaseweazle", "expert", "capture", "1", "--plan"],
        vec!["acquire", "--help"],
        vec!["production", "start"],
        vec!["scan"],
    ] {
        assert!(!applies(
            &a.into_iter().map(str::to_owned).collect::<Vec<_>>()
        ));
    }
    for a in [
        vec!["greaseweazle", "expert", "decode", "1"],
        vec!["recovery", "fat", "1"],
        vec!["conversion", "retry"],
    ] {
        assert!(applies(
            &a.into_iter().map(str::to_owned).collect::<Vec<_>>()
        ));
    }
}
