use super::*;
use crate::fat12::tests::{entry, image, set_fat};
fn sample(root_entries: usize, copies: &[usize]) -> Vec<u8> {
    let mut bytes = image();
    bytes[17..19].copy_from_slice(&(root_entries as u16).to_le_bytes());
    let a = b"{\\rtf1 First known text}";
    let b = b"{\\rtf1 Other known text}";
    assert_eq!(a.len(), b.len());
    entry(
        &mut bytes,
        19 * SECTOR,
        b"FIRST   RTF",
        2,
        a.len() as u32,
        false,
    );
    entry(
        &mut bytes,
        19 * SECTOR + 32,
        b"SECOND  RTF",
        3,
        b.len() as u32,
        false,
    );
    for copy in 0..2 {
        set_fat(&mut bytes, copy, 2, 0xfff);
        set_fat(&mut bytes, copy, 3, 0xfff);
    }
    for data in copies {
        bytes[data * SECTOR..data * SECTOR + a.len()].copy_from_slice(a);
        bytes[(data + 1) * SECTOR..(data + 1) * SECTOR + b.len()].copy_from_slice(b);
    }
    bytes
}
#[test]
fn custom_root_layout_recovers_from_two_disjoint_structural_contents_without_boot_guessing() {
    let mut bytes = sample(128, &[27]);
    bytes[..SECTOR].fill(0);
    let original = bytes.clone();
    let found = crate::fat12::analyze(&bytes, &[0]).unwrap();
    assert_eq!(
        found.layout_evidence.method,
        "inferred_content_supported_layout"
    );
    assert_eq!(found.layout.root_entries, 128);
    assert_eq!(found.layout.data_start, 27);
    assert_eq!(found.recovered_files.len(), 2);
    let comparison = found.layout_evidence.competition.unwrap();
    let selected = &comparison.candidates[comparison.selected_candidate.unwrap()];
    assert_eq!(selected.content_anchors.len(), 2);
    assert!(!selected.original_layout_certified && !found.customer_delivery_certified);
    assert!(comparison.candidates.len() > 1);
    assert_eq!(bytes, original);
}
#[test]
fn competing_complete_file_mappings_refuse_an_automatic_winner() {
    let bytes = sample(128, &[25, 27]);
    let comparison = inspect(&bytes, &BTreeSet::from([0])).unwrap();
    assert!(
        comparison
            .candidates
            .iter()
            .filter(|c| c.content_anchors.len() >= 2)
            .count()
            >= 2
    );
    assert!(comparison.selected_candidate.is_none());
    assert!(
        crate::fat12::analyze(&bytes, &[0])
            .unwrap_err()
            .contains("no automatic winner")
    );
}
#[test]
fn unreadable_bytes_and_metadata_never_validate_content() {
    let bytes = sample(128, &[27]);
    let blocked = inspect(&bytes, &BTreeSet::from([0, 1])).unwrap();
    assert!(blocked.candidates.is_empty());
    let blocked = inspect(&bytes, &BTreeSet::from([0, 27])).unwrap();
    assert!(blocked.selected_candidate.is_none());
    for c in blocked.candidates {
        assert!(
            c.content_anchors
                .iter()
                .all(|a| !a.source_lbas.contains(&27))
        );
    }
}
#[test]
fn known_boot_fields_are_not_overridden_by_content_or_image_size() {
    let mut bytes = sample(128, &[27]);
    bytes[13] = 3;
    assert!(crate::fat12::analyze(&bytes, &[]).is_err());
    bytes[13] = 1;
    bytes[28] = 1;
    assert!(crate::fat12::analyze(&bytes, &[]).is_err());
}

#[test]
fn fully_populated_root_boundary_needs_two_readable_content_witnesses() {
    let mut bytes = sample(16, &[20]);
    for at in (19 * SECTOR + 64..20 * SECTOR).step_by(32) {
        bytes[at] = 0xe5;
    }
    let found = crate::fat12::analyze(&bytes, &[0]).unwrap();
    assert_eq!(found.layout.root_entries, 16);
    assert_eq!(found.recovered_files.len(), 2);
    assert_eq!(
        found.layout_evidence.method,
        "inferred_content_supported_layout"
    );
    let blocked = inspect(&bytes, &BTreeSet::from([0, 20])).unwrap();
    assert!(blocked.selected_candidate.is_none());
}
#[test]
fn matching_standard_content_adds_comparison_without_changing_payloads() {
    let bytes = sample(224, &[33]);
    let read = crate::fat12::analyze(&bytes, &[]).unwrap();
    let lost = crate::fat12::analyze(&bytes, &[0]).unwrap();
    assert_eq!(lost.layout_evidence.method, "inferred_standard_layout");
    assert_eq!(
        read.recovered_files
            .iter()
            .map(|f| &f.sha256)
            .collect::<Vec<_>>(),
        lost.recovered_files
            .iter()
            .map(|f| &f.sha256)
            .collect::<Vec<_>>()
    );
    assert!(
        lost.layout_evidence
            .competition
            .unwrap()
            .selected_candidate
            .is_some()
    );
}
#[test]
fn old_evidence_is_readable_but_invalid_maps_and_malformed_images_refuse() {
    let evidence: crate::fat12::LayoutEvidence = serde_json::from_value(serde_json::json!({"method":"inferred_standard_layout","standard_profile":"ibm.1440","boot_error":"old","source_lbas":[1],"anchor_directory_offsets":[9728],"warning":"historical"})).unwrap();
    assert!(evidence.competition.is_none());
    assert!(crate::fat12::competing_layouts(&image(), &[0, 0]).is_err());
    assert!(crate::fat12::competing_layouts(&image(), &[2880]).is_err());
    for n in [0, 511, 513, MAX_IMAGE_BYTES + SECTOR] {
        assert!(inspect(&vec![0; n], &BTreeSet::new()).is_err());
    }
}

#[test]
fn truncated_searches_never_select_an_already_seen_strong_candidate() {
    let bytes = sample(16, &[20]);
    for budget in [
        Budget {
            trials: 1,
            candidates: 512,
            work: MAX_WORK,
        },
        Budget {
            trials: 8192,
            candidates: 1,
            work: MAX_WORK,
        },
        Budget {
            trials: 8192,
            candidates: 512,
            work: 1,
        },
    ] {
        let found = inspect_bounded(&bytes, &BTreeSet::from([0]), budget).unwrap();
        assert!(!found.search_complete_within_bounds);
        assert!(found.selected_candidate.is_none());
        assert_eq!(found.maximum_trials, budget.trials);
        assert_eq!(found.maximum_candidates, budget.candidates);
        assert_eq!(found.maximum_validation_work_bytes, budget.work);
    }
}

#[test]
fn invalid_containers_and_crosslinked_files_do_not_supply_two_content_witnesses() {
    let mut bytes = sample(16, &[20]);
    bytes[21 * SECTOR + 1] = b'x';
    let invalid = inspect(&bytes, &BTreeSet::from([0])).unwrap();
    assert!(invalid.selected_candidate.is_none());
    for c in invalid.candidates {
        assert!(c.content_anchors.len() < 2);
    }
    let mut bytes = sample(16, &[20]);
    bytes[19 * SECTOR + 32 + 26..19 * SECTOR + 32 + 28].copy_from_slice(&2u16.to_le_bytes());
    let linked = inspect(&bytes, &BTreeSet::from([0])).unwrap();
    assert!(linked.selected_candidate.is_none());
}

#[test]
fn complementary_passes_restore_exact_boot_and_directory_bytes_with_donor_provenance() {
    use crate::composite::{CompositeRequest, CompositeSource, run_composite};
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "fv-layout-donors-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let p = crate::project::ProjectState::create_without_session(root).unwrap();
    let original = sample(224, &[33]);
    let bad_sets = [vec![0, 19], vec![34, 35, 36]];
    let mut source_bytes = Vec::new();
    let mut sources = Vec::new();
    for (i, bad) in bad_sets.into_iter().enumerate() {
        let mut bytes = original.clone();
        for lba in &bad {
            bytes[*lba as usize * SECTOR..(*lba as usize + 1) * SECTOR].fill(0);
        }
        let path = p.images_dir().join(format!("001_attempt_{:03}.img", i + 1));
        fs::write(&path, &bytes).unwrap();
        sources.push(CompositeSource {
            attempt_number: i as u32 + 1,
            image_path: path.clone(),
            expected_sha256: Some(format!("{:x}", Sha256::digest(&bytes))),
            total_sectors: 2880,
            bad_sectors: bad,
        });
        source_bytes.push((path, bytes));
    }
    let request = CompositeRequest {
        images_directory: p.images_dir(),
        recovery_root: p.recovery_dir(),
        disk_number: 1,
        sources,
    };
    let result = run_composite(&request, &|_| {}).unwrap();
    assert_eq!(result.base_attempt, 1);
    assert_eq!(
        result
            .replacements
            .iter()
            .map(|r| r.target_lba)
            .collect::<Vec<_>>(),
        vec![0, 19]
    );
    assert!(
        result
            .replacements
            .iter()
            .all(|r| r.source_attempt == 2 && !r.source_sha256.is_empty())
    );
    let bytes = fs::read(result.derived_image.as_ref().unwrap()).unwrap();
    assert_eq!(bytes, original);
    let found = crate::fat12::analyze(&bytes, &result.unresolved_bad_sectors).unwrap();
    assert_eq!(found.layout_evidence.method, "readable_boot_sector");
    assert_eq!(found.recovered_files.len(), 2);
    assert!(run_composite(&request, &|_| {}).unwrap().reused);
    for (path, bytes) in source_bytes {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert!(
        p.root()
            .canonicalize()
            .unwrap()
            .starts_with(std::env::temp_dir().canonicalize().unwrap())
    );
    fs::remove_dir_all(p.root()).unwrap();
}
