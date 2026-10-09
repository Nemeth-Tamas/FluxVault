//! Retained saved-evidence replay. Never invokes a device or host tool.
use fluxvault::{flux_diagnostics, project::ProjectState};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

fn hash(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    format!("{digest:x}", digest = digest.finalize())
}

#[test]
#[ignore = "requires FV_DIAGNOSTICS_SOURCE and FV_DIAGNOSTICS_OUTPUT; copies saved 059/066 evidence, never reads media"]
fn saved_damaged_packed_captures_and_legacy_journals_replay_without_source_changes() {
    let source = ProjectState::open_without_session(PathBuf::from(
        std::env::var_os("FV_DIAGNOSTICS_SOURCE").unwrap(),
    ))
    .unwrap();
    let output = PathBuf::from(std::env::var_os("FV_DIAGNOSTICS_OUTPUT").unwrap());
    assert!(!output.exists(), "use a fresh output project");
    let project = ProjectState::create_without_session(output).unwrap();
    let mut seals = Vec::new();
    for disk in [59, 66] {
        for directory in ["Flux", "Flux/Derived", "Flux/Recovery", "Images", "Logs"] {
            let from = source.root().join(directory);
            let to = project.root().join(directory);
            fs::create_dir_all(&to).unwrap();
            for entry in fs::read_dir(&from).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let name_text = name.to_string_lossy();
                if !entry.file_type().unwrap().is_file()
                    || !name_text.starts_with(&format!("{disk:03}_"))
                    || name_text.ends_with(".lock")
                {
                    continue;
                }
                let digest = hash(&entry.path());
                fs::copy(entry.path(), to.join(&name)).unwrap();
                assert_eq!(hash(&to.join(&name)), digest);
                seals.push((entry.path(), digest));
            }
        }
        // Rebase only absolute result locations in the copied control. Captured,
        // decoded and published provenance payloads/hashes remain unchanged.
        let journal = project
            .root()
            .join(format!("Flux/Recovery/{disk:03}_job.json"));
        let mut job: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
        for (field, directory) in [("image", "Images"), ("provenance", "Flux/Recovery")] {
            let old = PathBuf::from(job["result"][field].as_str().unwrap());
            assert_eq!(
                old.canonicalize().unwrap().parent(),
                Some(
                    source
                        .root()
                        .join(directory)
                        .canonicalize()
                        .unwrap()
                        .as_path()
                )
            );
            job["result"][field] = json!(
                project
                    .root()
                    .join(directory)
                    .join(old.file_name().unwrap())
            );
        }
        fs::write(journal, serde_json::to_vec_pretty(&job).unwrap()).unwrap();
    }
    let mut results = Vec::new();
    for (disk, missing) in [(59, 5), (66, 4)] {
        let result = flux_diagnostics::diagnose(&project, disk).unwrap();
        let report: Value = serde_json::from_slice(&fs::read(&result.report).unwrap()).unwrap();
        assert_eq!(report["physical_media_access"], false);
        assert_eq!(report["host_tools_invoked"], false);
        assert_eq!(
            report["committed_recovery"]["missing_lbas"]
                .as_array()
                .unwrap()
                .len(),
            missing
        );
        assert_eq!(report["committed_recovery"]["conflicting_lbas"], json!([]));
        assert!(
            report["captures"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["measured"] == true
                    && c["storage"] == "packed"
                    && c["measurement"]["checksum_matches"] == true)
        );
        assert_eq!(
            fs::read_to_string(&result.sectors_csv)
                .unwrap()
                .lines()
                .count(),
            2881
        );
        results.push(json!({"disk":disk,"missing_sectors":missing,"captures":result.captures,"decodes":result.decodes,"report":result.report,"report_sha256":result.report_sha256}));
    }
    for (path, digest) in &seals {
        assert_eq!(hash(path), *digest, "original changed: {}", path.display());
    }
    fs::write(project.reports_dir().join("SavedFluxDiagnosticsValidation.json"),serde_json::to_vec_pretty(&json!({"schema":1,"offline_replay_only":true,"source":source.root(),"source_artifacts_verified_unchanged":seals.len(),"copied_result_paths_rebound_only":true,"results":results})).unwrap()).unwrap();
}
