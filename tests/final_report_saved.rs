//! Saved pilot reporting replay. Originals are hashed before/after; no media access.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

fn hash(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    format!("{:x}", digest.finalize())
}
fn copy_tree(source: &Path, destination: &Path, hashes: &mut BTreeMap<PathBuf, String>) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink());
        let src = entry.path();
        let dst = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&src, &dst, hashes);
        } else if kind.is_file() {
            hashes.insert(src.clone(), hash(&src));
            fs::copy(src, dst).unwrap();
        }
    }
}
fn rebase(value: &mut Value, spellings: &[String], destination: &str) {
    match value {
        Value::String(s) => {
            for old in spellings {
                if let Some(tail) = s.strip_prefix(old)
                    && (tail.is_empty() || tail.starts_with(['\\', '/']))
                {
                    *s = format!("{destination}{tail}");
                    break;
                }
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                rebase(v, spellings, destination);
            }
        }
        Value::Array(values) => {
            for v in values {
                rebase(v, spellings, destination);
            }
        }
        _ => {}
    }
}

#[test]
#[ignore = "requires FV_REPORT_SOURCE and FV_REPORT_OUTPUT; copies saved Images/Extracted/Converted, never reads media"]
fn saved_nineteen_disk_pilot_exports_script_metrics_without_changing_source_evidence() {
    let source = PathBuf::from(std::env::var("FV_REPORT_SOURCE").unwrap());
    let destination = PathBuf::from(std::env::var("FV_REPORT_OUTPUT").unwrap());
    assert!(!destination.exists(), "Replay destination must be fresh");
    let project =
        fluxvault::project::ProjectState::create_without_session(destination.clone()).unwrap();
    let mut hashes = BTreeMap::new();
    for folder in ["Images", "Logs", "Extracted", "Converted"] {
        copy_tree(&source.join(folder), &destination.join(folder), &mut hashes);
    }
    for name in [
        "ConversionSummary.csv",
        "DeliveryPathMap.csv",
        "ConversionState.json",
    ] {
        let src = source.join("Reports").join(name);
        hashes.insert(src.clone(), hash(&src));
        fs::copy(src, project.reports_dir().join(name)).unwrap();
    }
    let spellings = [
        source.to_string_lossy().into_owned(),
        source
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    ];
    // Acquisition metadata from live GW records absolute workstation locations.
    // Rebind copied catalog locations only; source payloads and originals stay intact.
    for entry in fs::read_dir(project.images_dir()).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|e| e == "json") {
            let mut metadata: Value =
                serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
            rebase(
                &mut metadata,
                &spellings,
                &destination.canonicalize().unwrap().to_string_lossy(),
            );
            fs::write(entry.path(), serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
        }
    }
    let mut state: Value = serde_json::from_slice(
        &fs::read(project.reports_dir().join("ConversionState.json")).unwrap(),
    )
    .unwrap();
    rebase(
        &mut state,
        &spellings,
        &destination.canonicalize().unwrap().to_string_lossy(),
    );
    fs::write(
        project.reports_dir().join("ConversionState.json"),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
    let before = hash(&destination.join("Images/001_attempt_001.img"));
    for language in ["hu", "en"] {
        let output = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
            .current_dir(&destination)
            .args(["report", "export", "--language", language, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        let report: Value = serde_json::from_slice(
            &fs::read(response["final_report"]["json"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(report["summary"]["FloppiesAudited"], 19);
        // Saved 003 has two changed DOCX outputs and two absent originals. The
        // prior success CSV alone cannot establish their current integrity.
        assert_eq!(report["summary"]["FloppiesFullyOk"], 15);
        assert_eq!(report["summary"]["FloppiesAttention"], 4);
        assert_eq!(report["summary"]["ConversionsOk"], 162);
        assert_eq!(report["summary"]["ConversionsPartial"], 2);
        assert_eq!(report["summary"]["ConversionsFailed"], 0);
        assert_eq!(report["summary"]["InvalidGeneratedOutputs"], 2);
        assert_eq!(report["summary"]["RecoveredSourceFiles"], 182);
        assert_eq!(report["summary"]["ExcludedSourceMetadataFiles"], 1);
        assert_eq!(report["summary"]["UntrackedDeliveryFiles"], 0);
        for row in report["floppies"].as_array().unwrap() {
            let disk = row["Floppy"].as_str().unwrap();
            assert_eq!(
                row["AuditStatus"],
                if disk == "003" {
                    "CHECK: CONVERSION FAILED"
                } else if ["005", "012", "017"].contains(&disk) {
                    "PARTIAL: IMAGE READ"
                } else {
                    "OK"
                }
            );
        }
        let root = Path::new(response["final_report"]["directory"].as_str().unwrap());
        let latest: Value = serde_json::from_slice(
            &fs::read(response["final_report"]["latest"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        for (name, expected) in latest["artifacts_sha256"].as_object().unwrap() {
            assert_eq!(hash(&root.join(name)), expected.as_str().unwrap());
        }
    }
    assert_eq!(
        hash(&destination.join("Images/001_attempt_001.img")),
        before
    );
    let audited = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .current_dir(&destination)
        .args(["audit", "--json"])
        .output()
        .unwrap();
    assert_eq!(audited.status.code(), Some(3));
    let audited: Value = serde_json::from_slice(&audited.stdout).unwrap();
    assert_eq!(audited["verified"], 15);
    assert_eq!(audited["attention"], 4);
    for (path, expected) in &hashes {
        assert_eq!(
            hash(path),
            *expected,
            "Original changed: {}",
            path.display()
        );
    }
    fs::write(destination.join("Reports/ReportingReplay.json"), serde_json::to_vec_pretty(&serde_json::json!({"original_files_verified":hashes.len(),"physical_media_access":false,"source":source,"copied_project":destination,"languages":["hu","en"],"customer_delivery_certified":false})).unwrap()).unwrap();
}
