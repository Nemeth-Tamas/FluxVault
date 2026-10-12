//! Explicit replay of actual physical USB acquisitions, never a new drive read.
use crate::{imaging, project::ProjectState, sector_inspection};
use serde_json::Value;
use std::{fs, path::PathBuf};
#[test]
#[ignore = "requires FLUXVAULT_ACCEPTANCE_PROJECT; actual completed clean/damaged USB records only, strictly image-only"]
fn saved_usb_clean_and_damaged_customer_cohort_meets_acquisition_extent_hash_map_gate() {
    let p = ProjectState::open_without_session(PathBuf::from(
        std::env::var_os("FLUXVAULT_ACCEPTANCE_PROJECT").expect("Set FLUXVAULT_ACCEPTANCE_PROJECT"),
    ))
    .unwrap();
    let before = fs::read(p.root().join("project.json")).unwrap();
    let mut good = Vec::new();
    let mut partial = Vec::new();
    for disk in [1, 3, 5, 7, 8, 10, 12, 17] {
        let attempts = imaging::load_attempts_for_disk(&p.images_dir(), disk).unwrap();
        let a = attempts
            .iter()
            .find(|a| {
                fs::read(&a.metadata_path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .is_some_and(|v| v["source_backend"] == "windows-raw-sector")
            })
            .expect("Missing actual USB acquisition");
        let s = sector_inspection::load_snapshot(&p, disk, Some(a.attempt_number)).unwrap();
        assert!(s.map_known);
        assert_eq!(s.metadata()["geometry"]["bytes_per_sector"], 512);
        assert_eq!(s.bytes.len(), 1_474_560);
        assert_eq!(a.total_sectors, 2880);
        assert_eq!(s.metadata()["sha256"], a.sha256);
        let map = s.metadata()["bad_sectors"].as_array().unwrap();
        if map.is_empty() {
            assert_eq!(a.status, "OK");
            good.push(disk);
        } else {
            assert_eq!(a.status, "PARTIAL");
            for b in map {
                let l = b["lba"].as_u64().unwrap() as usize;
                assert!(s.bytes[l * 512..(l + 1) * 512].iter().all(|b| *b == 0));
            }
            partial.push((disk, map.len()));
        }
        s.unchanged().unwrap();
    }
    assert!(good.len() >= 3);
    assert!(partial.len() >= 3);
    assert_eq!(before, fs::read(p.root().join("project.json")).unwrap());
    eprintln!(
        "Actual saved USB clean labels {good:?}, damaged labels/counts {partial:?}: exact 1,474,560-byte images, complete log/map/hash bindings, no new media access. Adapter write-blocking is a separate gate."
    );
}
