use super::*;
use crate::evidence_impact::tests::{Fixture, image, tree};
use std::{fs, path::PathBuf};

#[test]
fn damaged_payloads_and_batch_union_are_reviewed_without_writing_or_counting_twice() {
    let f = Fixture::new();
    f.save_disk(1, 1, &image(), &[36]);
    f.save_disk(2, 1, &image(), &[]);
    let before = tree(f.0.root());
    let single = inspect(&f.0, 1, None).unwrap();
    assert_eq!(single["reviewed_files"].as_array().unwrap().len(), 1);
    let file = &single["reviewed_files"][0];
    assert_eq!(
        file["classification"],
        "mapped_raw_fragments_format_repair_unavailable"
    );
    assert_eq!(file["unreadable_ranges"][0]["file_offset"], 512);
    assert!(file["observed_complete_payload_sha256"].is_null());
    assert_eq!(single["files_written"], 0);
    let batch = inspect_all(&f.0, |_, _, _| {}).unwrap();
    assert_eq!(batch["counts"]["saved_labels"], 2);
    assert_eq!(batch["counts"]["reviewed_file_records"], 1);
    assert_eq!(batch["counts"]["known_files_with_problem_dependencies"], 1);
    assert_eq!(batch["counts"]["incomplete_or_ambiguous_files"], 1);
    assert_eq!(batch["inventory_changed"], false);
    assert!(render(&batch).contains("CHAIN.TXT"));
    assert_eq!(tree(f.0.root()), before);
}

#[test]
fn unknown_layout_and_missing_directory_remain_attention_not_empty_clean_cases() {
    let f = Fixture::new();
    f.save(1, &vec![0; 1474560], &[0]);
    let result = inspect_all(&f.0, |_, _, _| {}).unwrap();
    assert_eq!(result["attention_required"], true);
    assert_eq!(result["counts"]["reviewed_file_records"], 0);
    assert!(!result["impact"]["disks"][0]["filesystem_error"].is_null());
    let g = Fixture::new();
    g.save(1, &image(), &[19]);
    let result = inspect(&g.0, 1, None).unwrap();
    assert_eq!(result["impact"]["directory_gaps"], json!([19]));
    assert!(result["reviewed_files"].as_array().unwrap().is_empty());
    assert_eq!(result["attention_required"], true);
}

#[test]
fn changed_saved_bytes_or_inconsistent_maps_refuse_without_mutating_evidence() {
    let f = Fixture::new();
    f.save(1, &image(), &[36]);
    let path = f.0.images_dir().join("001_attempt_001.img");
    let mut bytes = fs::read(&path).unwrap();
    bytes[4000] ^= 1;
    fs::write(&path, &bytes).unwrap();
    let before = tree(f.0.root());
    assert!(inspect(&f.0, 1, None).unwrap_err().contains("SHA-256"));
    assert_eq!(tree(f.0.root()), before);
}

#[test]
fn fat_copy_observations_keep_disagreement_unknown_bytes_and_claims_separate() {
    let gw = image();
    let mut usb = gw.clone();
    fat12::tests::set_fat(&mut usb, 1, 2400, 0xfff);
    let analysis = fat12::analyze(&gw, &[]).unwrap();
    let result = fat_observations(&analysis, &gw, &usb, &BTreeSet::new(), &BTreeSet::new(), 17);
    let entry = result["different_declared_cluster_entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["cluster"] == 2400)
        .unwrap();
    assert_eq!(entry["usb_value"], 4095);
    assert_eq!(entry["gw_value"], 0);
    assert_eq!(entry["gw_copy_values"], json!([0, 0]));
    assert_eq!(entry["usb_copy_values"], json!([0, 4095]));
    assert_eq!(entry["known_allocation_claim"], false);
    assert_eq!(result["winner_selected"], false);
    let missing = fat_observations(
        &analysis,
        &gw,
        &usb,
        &BTreeSet::new(),
        &BTreeSet::from([17]),
        17,
    );
    assert!(missing["different_declared_cluster_entries"][0]["usb_value"].is_null());
    assert!(
        fat_observations(
            &analysis,
            &gw,
            &usb,
            &BTreeSet::new(),
            &BTreeSet::new(),
            773
        )
        .is_null()
    );
}

#[test]
fn investigation_cli_is_read_only_has_attention_json_and_strict_options() {
    let f = Fixture::new();
    f.save(1, &image(), &[36]);
    let before = tree(f.0.root());
    let args = vec![
        "recovery".into(),
        "investigate".into(),
        "all".into(),
        "--json".into(),
    ];
    let reply = crate::cli::run(&args, f.0.root()).unwrap();
    assert_eq!(reply.exit_code, 3);
    let value: Value = serde_json::from_str(&reply.output).unwrap();
    assert_eq!(value["physical_media_access"], false);
    assert_eq!(value["files_written"], 0);
    for extra in [
        vec!["--attempt".into(), "1".into()],
        vec!["--language".into(), "hu".into()],
        vec!["--lba".into(), "3".into()],
    ] {
        let mut args = args.clone();
        args.extend(extra);
        assert!(crate::cli::run(&args, f.0.root()).is_err());
    }
    assert_eq!(tree(f.0.root()), before);
}

#[test]
fn compared_observations_exclude_missing_bytes_and_never_make_a_content_winner() {
    let a = vec![1u8; 1024];
    let mut b = a.clone();
    b[511] = 2;
    let result =
        compare_ranges(&a, &b, &BTreeSet::new(), &BTreeSet::from([1]), &[(510, 4)]).unwrap();
    assert_eq!(result["agreeing_observed_bytes"], 1);
    assert_eq!(result["different_observed_bytes"], 1);
    assert_eq!(result["unavailable_bytes"], 2);
    assert!(result["usb_ordered_bytes_sha256"].is_null());
    let equal = compare_ranges(&a, &a, &BTreeSet::new(), &BTreeSet::new(), &[(510, 4)]).unwrap();
    assert_eq!(equal["agreeing_observed_bytes"], 4);
    assert_eq!(
        equal["gw_ordered_bytes_sha256"],
        equal["usb_ordered_bytes_sha256"]
    );
    assert_eq!(equal["full_original_certified"], false);
    assert!(compare_ranges(&a, &b, &BTreeSet::new(), &BTreeSet::new(), &[(1020, 10)]).is_err());
    assert!(
        compare_ranges(
            &a,
            &b,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[(usize::MAX, 1)]
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires FLUXVAULT_INVESTIGATION_PROJECT; saved 136-disk cohort only, no physical media"]
fn saved_136_case_review_replays_all_gaps_and_133_disputed_versions() {
    let p = ProjectState::open_without_session(PathBuf::from(
        std::env::var("FLUXVAULT_INVESTIGATION_PROJECT").unwrap(),
    ))
    .unwrap();
    let result = inspect_all(&p, |_, _, _| {}).unwrap();
    assert_eq!(result["counts"]["saved_labels"], 136);
    assert_eq!(
        result["counts"]["known_files_with_problem_dependencies"],
        13
    );
    assert_eq!(result["counts"]["incomplete_or_ambiguous_files"], 16);
    assert_eq!(result["counts"]["reviewed_file_records"], 17);
    assert_eq!(result["inventory_changed"], false);
    let d = result["disks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["disk"] == 133)
        .unwrap();
    assert!(d["dispute_review"]["original_content_winner"].is_null());
    assert_eq!(
        d["dispute_review"]["sector_versions"][0]["changed_bytes"],
        170
    );
    assert_eq!(
        d["dispute_review"]["sector_versions"][1]["changed_bytes"],
        426
    );
    let file = &d["reviewed_files"][0];
    let legacy = &file["word"]["compound"]["legacy_word6_main_text_map"];
    assert_eq!(legacy["main_positions"], 1519);
    assert_eq!(legacy["unavailable_or_disputed_main_text_bytes"], 0);
    assert_eq!(legacy["text_encoding_verified"], false);
    assert_eq!(
        file["legacy_main_byte_comparison"]["comparison"]["agreeing_observed_bytes"],
        1519
    );
    assert_eq!(
        file["legacy_main_byte_comparison"]["comparison"]["different_observed_bytes"],
        0
    );
    assert_eq!(
        file["legacy_main_byte_comparison"]["comparison"]["unavailable_bytes"],
        0
    );
    assert_eq!(
        file["legacy_main_byte_comparison"]["disputed_sectors_intersect_mapped_main_ranges"],
        false
    );
    let excluded = BTreeSet::from([17u64, 773]);
    let mut end = 0;
    for piece in legacy["pieces"].as_array().unwrap() {
        assert_eq!(piece["cp_start"], end);
        end += piece["cp_count"].as_u64().unwrap();
        for e in piece["source_extents"].as_array().unwrap() {
            let at = e["source_byte_offset"].as_u64().unwrap();
            let n = e["bytes"].as_u64().unwrap();
            assert!((at..at + n).all(|b| !excluded.contains(&(b / 512))));
        }
    }
    assert_eq!(end, 1519);
    for disk in result["disks"].as_array().unwrap() {
        let image = fs::read(disk["source_image"].as_str().unwrap()).unwrap();
        assert_eq!(hash(&image), disk["source_image_sha256"]);
        let bad = disk["impact"]["sectors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["lba"].as_u64().unwrap())
            .collect::<BTreeSet<_>>();
        for file in disk["reviewed_files"].as_array().unwrap() {
            assert_eq!(file["whole_original_repaired"], false);
            for segment in file["word"]["word_text"]["segments"]
                .as_array()
                .into_iter()
                .flatten()
            {
                for e in segment["source_extents"].as_array().unwrap() {
                    let at = e["source_byte_offset"].as_u64().unwrap();
                    let n = e["bytes"].as_u64().unwrap();
                    assert!((at..at + n).all(|b| !bad.contains(&(b / 512))));
                }
            }
        }
    }
}
