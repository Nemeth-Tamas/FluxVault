//! Saved-cohort replay only. No physical backend or host tool is ever invoked.
use fluxvault::{
    imaging,
    production::{self, Coordinator, Station},
    project::ProjectState,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
#[ignore = "requires FV_PRIORITY_SOURCE and FV_PRIORITY_OUTPUT; copies saved dual receipts only, never reads media"]
fn saved_dual_cohort_replays_usb_triage_and_priority_without_changing_original_evidence() {
    let source = ProjectState::open_without_session(PathBuf::from(
        std::env::var_os("FV_PRIORITY_SOURCE").unwrap(),
    ))
    .unwrap();
    let saved = production::status(&source).unwrap();
    assert_eq!(
        saved["coordinator_owner_active"], false,
        "source must be idle"
    );
    let source_control = source.root().join(".fluxvault-production.json");
    let mut source_seals = vec![(
        source_control.clone(),
        hash(&fs::read(&source_control).unwrap()),
    )];
    let output = PathBuf::from(std::env::var_os("FV_PRIORITY_OUTPUT").unwrap());
    assert!(!output.exists(), "use a fresh output folder");
    let mut copy = ProjectState::create_without_session(output).unwrap();
    let first = saved["first"].as_u64().unwrap() as u32;
    let last = saved["last"].as_u64().unwrap() as u32;
    copy.set_current_disk_number_without_session(first).unwrap();
    let mut coordinator = Coordinator::open(copy.clone(), Some(last), false).unwrap();
    let mut rows = Vec::new();
    // Reconstruct feeding order, deliberately stopping before historical GW
    // transfers, to expose their original USB partials. Queue ages here describe
    // this replay's claims, not the original session's elapsed waiting time.
    for disk in first..=last {
        let original = &saved["disks"][disk.to_string()];
        let (station, receipt) = if original["usb"].is_object() {
            (Station::Usb, &original["usb"])
        } else {
            (Station::Greaseweazle, &original["gw"])
        };
        assert!(receipt.is_object(), "cohort has a missing label {disk}");
        let attempt = receipt["attempt"].as_u64().unwrap() as u32;
        let ticket = coordinator.claim(station, disk).unwrap();
        coordinator
            .confirm(&ticket, &disk.to_string(), true)
            .unwrap();
        let original_image = source.images_dir().join(receipt["image"].as_str().unwrap());
        let original_meta = original_image.with_extension("json");
        let mut metadata: Value =
            serde_json::from_slice(&fs::read(&original_meta).unwrap()).unwrap();
        let original_log = PathBuf::from(metadata["log_file"].as_str().unwrap());
        let new_log = copy.logs_dir().join(original_log.file_name().unwrap());
        for path in [&original_image, &original_meta, &original_log] {
            source_seals.push((path.clone(), hash(&fs::read(path).unwrap())));
        }
        fs::copy(
            &original_image,
            copy.images_dir().join(original_image.file_name().unwrap()),
        )
        .unwrap();
        fs::copy(&original_log, &new_log).unwrap();
        metadata["log_file"] = json!(new_log);
        metadata["image_file"] = json!(copy.images_dir().join(original_image.file_name().unwrap()));
        fs::write(
            copy.images_dir().join(original_meta.file_name().unwrap()),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        coordinator.complete(&ticket, attempt).unwrap();
        coordinator.removed(&ticket, true).unwrap();
        rows.push(json!({"disk":disk,"station":station,"attempt":attempt,
            "source_image_sha256":receipt["sha256"],"missing_sectors":receipt["bad"].as_array().unwrap().len(),
            "copied_metadata_image_and_log_paths_rebound":true}));
    }
    let ranked = coordinator.status()["recovery_priorities"].clone();
    let expected_partials = rows
        .iter()
        .filter(|r| r["station"] == "usb" && r["missing_sectors"].as_u64().unwrap() > 0)
        .count();
    assert!(expected_partials > 0);
    assert_eq!(ranked.as_array().unwrap().len(), expected_partials);
    assert!(
        ranked
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["availability"] == "ready")
    );
    assert_eq!(
        imaging::load_project_statistics(&copy.images_dir())
            .unwrap()
            .disk_count,
        (last - first + 1) as usize
    );
    drop(coordinator);
    let mut coordinator = Coordinator::open(copy.clone(), Some(last), false).unwrap();
    assert_eq!(coordinator.status()["recovery_priorities"], ranked);
    coordinator.set_paused(true).unwrap();
    drop(coordinator);
    let copy_control = copy.root().join(".fluxvault-production.json");
    let before = fs::read(&copy_control).unwrap();
    let inspected = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .current_dir(copy.root())
        .args(["production", "queue", "--json"])
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let inspected: Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(inspected["recovery_priorities"], ranked);
    assert_eq!(inspected["paused"], true);
    assert_eq!(inspected["physical_media_access"], false);
    assert_eq!(fs::read(copy_control).unwrap(), before);
    for (path, expected) in &source_seals {
        assert_eq!(
            hash(&fs::read(path).unwrap()),
            *expected,
            "original changed: {}",
            path.display()
        );
    }
    fs::write(copy.reports_dir().join("PriorityReplay.json"), serde_json::to_vec_pretty(&json!({
        "schema":1,"offline_replay_only":true,"source":source.root(),"physical_media_access":false,
        "original_control_and_copied_sources_verified_unchanged":source_seals.len(),
        "queue_age_is_replay_generation_not_original_wait_time":true,
        "saved_label_count":rows.len(),"eligible_usb_partials":expected_partials,
        "receipts":rows,"priorities":ranked
    })).unwrap()).unwrap();
}
