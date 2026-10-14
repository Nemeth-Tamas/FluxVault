use super::*;
use crate::{imaging, project::ProjectState};

const PAYLOAD: &[u8] = b"{\\rtf1\\ansi Snapshot candidate fixture}";
struct Fixture {
    project: ProjectState,
    root: PathBuf,
}
impl Fixture {
    fn new(bytes: Option<Vec<u8>>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-snapshot-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        fs::write(
            project.images_dir().join("001.bin"),
            bytes.unwrap_or_else(image),
        )
        .unwrap();
        Self { project, root }
    }
    fn attempt(&self) -> AttemptSummary {
        imaging::load_attempts_for_disk(&self.project.images_dir(), 1)
            .unwrap()
            .remove(0)
    }
    fn recover(&self, plan: bool) -> Result<ResultSummary, String> {
        recover(
            &self.project.images_dir(),
            &self.project.recovery_dir(),
            1,
            &self.attempt(),
            plan,
            &|_| {},
        )
    }
    fn log(&self, claim: &str) {
        fs::write(self.project.logs_dir().join("001.log"),format!("BEGIN floppy=001\nGEOMETRY cylinders=80 heads=2 sectors_per_track=9 bytes_per_sector=512 total_sectors=1440 total_bytes=737280\nSHA256 {claim}\nEND status=OK bad_sectors=0 recovered_after_retry=0 track_fallbacks=0 duration_seconds=1.0\n")).unwrap();
    }
    fn bad_log(&self, lbas: &[u64]) {
        self.log(&self.attempt().sha256);
        let path = self.project.logs_dir().join("001.log");
        let text = fs::read_to_string(&path).unwrap().replace(
            "status=OK bad_sectors=0",
            &format!("status=PARTIAL bad_sectors={}", lbas.len()),
        );
        let records = lbas
            .iter()
            .map(|l| format!("BAD_SECTOR LBA={l}\n"))
            .collect::<String>();
        fs::write(
            path,
            text.replace("END status=", &format!("{records}END status=")),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn image() -> Vec<u8> {
    let mut b = vec![0; 737_280];
    b[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    b[3..11].copy_from_slice(b"FVTEST  ");
    b[11..13].copy_from_slice(&512u16.to_le_bytes());
    b[13] = 2;
    b[14..16].copy_from_slice(&1u16.to_le_bytes());
    b[16] = 2;
    b[17..19].copy_from_slice(&112u16.to_le_bytes());
    b[19..21].copy_from_slice(&1440u16.to_le_bytes());
    b[21] = 0xf9;
    b[22..24].copy_from_slice(&3u16.to_le_bytes());
    b[24..26].copy_from_slice(&9u16.to_le_bytes());
    b[26..28].copy_from_slice(&2u16.to_le_bytes());
    b[510..512].copy_from_slice(&[0x55, 0xaa]);
    for at in [512, 4 * 512] {
        b[at..at + 5].copy_from_slice(&[0xf9, 0xff, 0xff, 0xff, 0x0f]);
    }
    let at = 7 * 512;
    b[at..at + 11].copy_from_slice(b"NOTES   RTF");
    b[at + 11] = 0x20;
    b[at + 26..at + 28].copy_from_slice(&2u16.to_le_bytes());
    b[at + 28..at + 32].copy_from_slice(&(PAYLOAD.len() as u32).to_le_bytes());
    b[14 * 512..14 * 512 + PAYLOAD.len()].copy_from_slice(PAYLOAD);
    b
}

#[test]
fn known_holes_copy_saved_bytes_without_filling_and_preserve_offset_provenance() {
    let f = Fixture::new(None);
    f.bad_log(&[14]);
    let a = f.attempt();
    assert_eq!(a.bad_sectors, vec![14]);
    let source = fs::read(f.project.images_dir().join("001.bin")).unwrap();
    let original_log = fs::read(f.project.logs_dir().join("001.log")).unwrap();
    let r = f.recover(false).unwrap();
    assert_eq!(r.report.engine_version, 2);
    assert!(r.report.acquisition_mask_known);
    assert_eq!(r.report.bad_sector_count, Some(1));
    assert_eq!(r.report.known_bad_lbas, vec![14]);
    assert_eq!(r.report.files.len(), 1);
    let c = &r.report.files[0];
    assert_eq!(c.method, "fully_mapped_damaged_snapshot_copy");
    assert_eq!(
        c.known_unreadable_ranges,
        vec![Gap {
            file_offset: 0,
            bytes: PAYLOAD.len(),
            source_lbas: vec![14]
        }]
    );
    assert_eq!(fs::read(r.output_directory.join(&c.path)).unwrap(), PAYLOAD);
    assert_eq!(
        fs::read(f.project.images_dir().join("001.bin")).unwrap(),
        source
    );
    assert_eq!(
        fs::read(f.project.logs_dir().join("001.log")).unwrap(),
        original_log
    );
    assert!(!f.project.extracted_dir().join("001").exists());
    assert!(!r.report.customer_delivery_certified);
    assert!(f.recover(false).unwrap().reused);
    let relative = format!(
        "Recovery/SnapshotCandidates/001/{}/{}",
        r.output_directory.file_name().unwrap().to_string_lossy(),
        c.path
    );
    let trace = crate::file_lineage::inspect(&f.project, 1, &relative).unwrap();
    assert_eq!(trace["file"]["acquisition_mask_known"], true);
    assert_eq!(
        trace["file"]["known_unreadable_ranges"][0]["bytes"],
        PAYLOAD.len()
    );
    assert_eq!(trace["files_written"], 0);
}

fn set_fat(bytes: &mut [u8], cluster: usize, next: u16) {
    for base in [512, 4 * 512] {
        let at = base + cluster * 3 / 2;
        let mut packed = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        packed = if cluster.is_multiple_of(2) {
            (packed & 0xf000) | (next & 0xfff)
        } else {
            (packed & 0x000f) | ((next & 0xfff) << 4)
        };
        bytes[at..at + 2].copy_from_slice(&packed.to_le_bytes());
    }
}

#[test]
fn fragmented_damaged_copy_preserves_logical_order_and_eof_clipped_holes() {
    let mut bytes = image();
    set_fat(&mut bytes, 2, 5);
    set_fat(&mut bytes, 5, 0xfff);
    bytes[7 * 512 + 28..7 * 512 + 32].copy_from_slice(&1300u32.to_le_bytes());
    bytes[14 * 512..15 * 512].fill(b'a');
    bytes[15 * 512..16 * 512].fill(0x55);
    bytes[20 * 512..21 * 512].fill(0xaa);
    let f = Fixture::new(Some(bytes.clone()));
    f.bad_log(&[15, 20]);
    let r = f.recover(false).unwrap();
    let c = &r.report.files[0];
    assert_eq!(
        c.extents,
        vec![
            Extent {
                offset: 14 * 512,
                bytes: 512
            },
            Extent {
                offset: 15 * 512,
                bytes: 512
            },
            Extent {
                offset: 20 * 512,
                bytes: 276
            }
        ]
    );
    assert_eq!(
        c.known_unreadable_ranges,
        vec![Gap {
            file_offset: 512,
            bytes: 788,
            source_lbas: vec![15, 20]
        }]
    );
    let got = fs::read(r.output_directory.join(&c.path)).unwrap();
    assert_eq!(&got[..512], vec![b'a'; 512]);
    assert_eq!(&got[512..1024], vec![0x55; 512]);
    assert_eq!(&got[1024..], vec![0xaa; 276]);
    assert_eq!(
        fs::read(f.project.images_dir().join("001.bin")).unwrap(),
        bytes
    );
}

#[test]
fn ambiguous_ownership_metadata_holes_and_unmapped_tail_are_not_full_length_copies() {
    for case in ["shared", "metadata", "tail"] {
        let mut bytes = image();
        let bad = if case == "metadata" {
            vec![7]
        } else {
            vec![14]
        };
        if case == "shared" {
            let entry = bytes[7 * 512..7 * 512 + 32].to_vec();
            bytes[7 * 512 + 32..7 * 512 + 64].copy_from_slice(&entry);
            bytes[7 * 512 + 32..7 * 512 + 43].copy_from_slice(b"OTHER   RTF");
        } else if case == "tail" {
            bytes[7 * 512 + 28..7 * 512 + 32].copy_from_slice(&1025u32.to_le_bytes());
        }
        let f = Fixture::new(Some(bytes));
        f.bad_log(&bad);
        let r = f.recover(true).unwrap();
        assert!(r.report.files.is_empty(), "{case}");
        assert!(r.report.acquisition_mask_known);
    }
}

#[test]
fn fabricated_hole_range_is_refused_even_with_unchanged_candidate_payload() {
    let f = Fixture::new(None);
    f.bad_log(&[14]);
    let r = f.recover(false).unwrap();
    let mut forged = r.report.clone();
    forged.files[0].known_unreadable_ranges[0].bytes -= 1;
    fs::write(&r.report_path, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(f.recover(false).is_err());
    assert_eq!(
        fs::read(r.output_directory.join(&r.report.files[0].path)).unwrap(),
        PAYLOAD
    );
}

#[test]
fn original_v1_report_serialization_and_generation_remain_replay_compatible() {
    let f = Fixture::new(None);
    let r = f.recover(false).unwrap();
    let value = serde_json::to_value(&r.report).unwrap();
    assert!(value.get("known_bad_lbas").is_none());
    assert!(value["files"][0].get("known_unreadable_ranges").is_none());
    assert_eq!(r.report.engine_version, 1);
    assert!(
        verified_existing(
            &f.project.images_dir(),
            &f.project.recovery_dir(),
            1,
            &f.attempt()
        )
        .unwrap()
        .is_some()
    );
    assert!(f.recover(false).unwrap().reused);
}

#[test]
fn plan_inspects_snapshot_bytes_without_creating_a_generation_or_mask() {
    let f = Fixture::new(None);
    let before = fs::read(f.project.images_dir().join("001.bin")).unwrap();
    let result = f.recover(true).unwrap();
    assert!(result.plan_only);
    assert!(!result.reused);
    assert_eq!(result.report.files.len(), 1);
    assert_eq!(result.report.physical_readability, "unknown");
    assert!(!result.report.acquisition_mask_known);
    assert_eq!(result.report.bad_sector_count, None);
    assert!(!result.report.customer_delivery_certified);
    assert!(!result.report.default_live_extraction_changed);
    assert!(!f.project.recovery_dir().join("SnapshotCandidates").exists());
    assert_eq!(
        fs::read(f.project.images_dir().join("001.bin")).unwrap(),
        before
    );
}

#[test]
fn publication_replays_extents_and_reuses_without_managed_extraction_or_source_writes() {
    let f = Fixture::new(None);
    let before = f.attempt();
    let result = f.recover(false).unwrap();
    let file = &result.report.files[0];
    assert_eq!(
        file.extents,
        vec![Extent {
            offset: 14 * 512,
            bytes: PAYLOAD.len()
        }]
    );
    assert_eq!(
        fs::read(result.output_directory.join(&file.path)).unwrap(),
        PAYLOAD
    );
    assert!(!f.project.extracted_dir().join("001").exists());
    let original_receipt = fs::read(&result.report_path).unwrap();
    let reused = f.recover(false).unwrap();
    assert!(reused.reused);
    assert_eq!(result.report, reused.report);
    assert_eq!(fs::read(&result.report_path).unwrap(), original_receipt);
    assert_eq!(f.attempt().sha256, before.sha256);
    assert!(f.attempt().parsed_log.is_none());
}

#[test]
fn known_map_and_in_progress_sources_are_not_relabelled_unknown_candidates() {
    let f = Fixture::new(None);
    f.log(&f.attempt().sha256);
    assert!(!eligible(&f.project.images_dir(), &f.attempt()));
    assert!(f.recover(false).is_err());
    fs::write(f.project.logs_dir().join("001.log"), "BEGIN floppy=001\n").unwrap();
    assert!(!eligible(&f.project.images_dir(), &f.attempt()));
    assert!(f.recover(false).is_err());
    assert!(!f.project.recovery_dir().join("SnapshotCandidates").exists());
}

#[test]
fn contradictory_historical_hash_stays_separate_and_never_authorizes_sector_recovery() {
    let f = Fixture::new(None);
    let claim = "a".repeat(64);
    f.log(&claim);
    let a = f.attempt();
    assert!(a.require_legacy_sector_binding().is_err());
    let r = f.recover(false).unwrap();
    assert_eq!(r.report.historical_log_sha256_claim, Some(claim));
    assert_eq!(r.report.source_sha256, hash(&image()));
    assert!(r.report.observed_log_file_sha256.is_some());
    assert!(!r.report.acquisition_mask_known);
    assert_eq!(f.attempt().sha256, a.sha256);
    assert!(crate::fat12_recovery::validate_sector_evidence(&a, 1440, &a.sha256).is_err());
}

#[test]
fn overclaimed_log_extent_uses_actual_snapshot_size_without_rewriting_evidence() {
    let f = Fixture::new(None);
    f.log(&f.attempt().sha256);
    let log = f.project.logs_dir().join("001.log");
    let original = fs::read_to_string(&log)
        .unwrap()
        .replace("total_sectors=1440", "total_sectors=1441")
        .replace("total_bytes=737280", "total_bytes=737792");
    fs::write(&log, &original).unwrap();
    let a = f.attempt();
    assert_eq!(a.total_sectors, 1441);
    assert!(a.attention_required);
    assert!(crate::fat12_recovery::validate_sector_evidence(&a, 1441, &a.sha256).is_ok());
    assert!(crate::fat12_recovery::validate_sector_evidence(&a, 1440, &a.sha256).is_err());
    assert!(eligible(&f.project.images_dir(), &a));
    let result = f.recover(false).unwrap();
    assert_eq!(result.report.source_bytes, 737_280);
    assert_eq!(result.report.files.len(), 1);
    assert_eq!(result.report.files[0].sha256, hash(PAYLOAD));
    assert_eq!(result.report.bad_sector_count, None);
    assert!(!result.report.acquisition_mask_known);
    assert_eq!(fs::read_to_string(log).unwrap(), original);
    assert_eq!(
        fs::read(f.project.images_dir().join("001.bin")).unwrap(),
        image()
    );
    assert_eq!(f.attempt().total_sectors, 1441);
    assert!(f.recover(false).unwrap().reused);
}

#[test]
fn extent_attention_audit_keeps_extent_and_extraction_issues_without_inventing_reader_conflicts() {
    let f = Fixture::new(None);
    f.log(&f.attempt().sha256);
    let log = f.project.logs_dir().join("001.log");
    let original = fs::read_to_string(&log)
        .unwrap()
        .replace("total_sectors=1440", "total_sectors=1441");
    fs::write(&log, &original).unwrap();
    let audit = crate::audit::run_audit(&f.project, &|_| {}).unwrap();
    let row = &audit.document.disks[0];
    assert!(row.image_hash_verified);
    assert_eq!(audit.verified_disks, 0);
    assert!(row.issue.contains("Legacy sector extent disagrees"));
    assert!(row.issue.contains("No extraction"));
    assert!(!row.issue.contains("differing USB/GW"));
    assert_ne!(row.evidence_status, "CONFIRMED_CROSS_READER_CONFLICTS");
    assert!(
        fs::read_to_string(audit.csv_path)
            .unwrap()
            .contains("Legacy sector extent disagrees")
    );
    assert_eq!(fs::read_to_string(log).unwrap(), original);
    assert_eq!(
        fs::read(f.project.images_dir().join("001.bin")).unwrap(),
        image()
    );
}

#[test]
fn historical_hash_audit_issue_is_not_overwritten_by_missing_extraction_or_reader_conflict() {
    let f = Fixture::new(None);
    f.log(&"a".repeat(64));
    let audit = crate::audit::run_audit(&f.project, &|_| {}).unwrap();
    let row = &audit.document.disks[0];
    assert!(row.image_hash_verified);
    assert_eq!(audit.verified_disks, 0);
    assert!(
        row.issue
            .contains("Legacy acquisition-log SHA-256 claim disagrees")
    );
    assert!(row.issue.contains("No extraction"));
    assert!(!row.issue.contains("differing USB/GW"));
    assert_ne!(row.evidence_status, "CONFIRMED_CROSS_READER_CONFLICTS");
}

#[test]
fn truncated_snapshot_only_carves_present_bytes_and_never_pads_missing_tail() {
    let mut truncated = image();
    truncated.truncate(15 * 512);
    let f = Fixture::new(Some(truncated.clone()));
    f.log(&f.attempt().sha256);
    let original_log = fs::read(f.project.logs_dir().join("001.log")).unwrap();
    let a = f.attempt();
    assert_eq!(a.total_sectors, 1440);
    assert!(eligible(&f.project.images_dir(), &a));
    let r = f.recover(false).unwrap();
    assert_eq!(r.report.source_bytes, truncated.len());
    assert!(r.report.filesystem_error.is_some());
    assert!(r.report.files.iter().any(|c| c.sha256 == hash(PAYLOAD)));
    for c in &r.report.files {
        assert!(
            c.extents
                .iter()
                .all(|e| e.offset + e.bytes <= truncated.len())
        );
        assert_eq!(
            fs::read(r.output_directory.join(&c.path)).unwrap(),
            payload(&truncated, c).unwrap()
        );
    }
    assert!(!r.report.acquisition_mask_known);
    assert_eq!(r.report.physical_readability, "unknown");
    assert!(!r.report.customer_delivery_certified);
    assert!(!f.project.extracted_dir().join("001").exists());
    assert_eq!(
        fs::read(f.project.images_dir().join("001.bin")).unwrap(),
        truncated
    );
    assert_eq!(
        fs::read(f.project.logs_dir().join("001.log")).unwrap(),
        original_log
    );
    assert!(f.recover(false).unwrap().reused);
}

#[test]
fn changed_report_payload_and_extra_files_refuse_reuse_and_preserve_operator_bytes() {
    for mutation in ["report", "payload", "extra", "missing"] {
        let f = Fixture::new(None);
        let r = f.recover(false).unwrap();
        let file = r.output_directory.join(&r.report.files[0].path);
        match mutation {
            "report" => {
                let mut v = r.report.clone();
                v.acquisition_mask_known = true;
                fs::write(&r.report_path, serde_json::to_vec(&v).unwrap()).unwrap();
            }
            "payload" => fs::write(&file, b"operator modification").unwrap(),
            "extra" => fs::write(r.output_directory.join("extra.txt"), b"extra").unwrap(),
            _ => fs::remove_file(&file).unwrap(),
        }
        assert!(f.recover(false).is_err(), "{mutation}");
        if mutation == "payload" {
            assert_eq!(fs::read(file).unwrap(), b"operator modification");
        }
    }
}

#[test]
fn rehashed_forged_extents_are_refused_by_independent_analysis() {
    let f = Fixture::new(None);
    let r = f.recover(false).unwrap();
    let mut forged = r.report.clone();
    forged.files[0].extents = vec![Extent {
        offset: 0,
        bytes: PAYLOAD.len(),
    }];
    let bytes = payload(
        &image(),
        &Candidate {
            sha256: hash(&image()[..PAYLOAD.len()]),
            ..forged.files[0].clone()
        },
    )
    .unwrap();
    forged.files[0].sha256 = hash(&bytes);
    fs::write(r.output_directory.join(&forged.files[0].path), &bytes).unwrap();
    fs::write(&r.report_path, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(f.recover(false).is_err());
}

#[test]
fn stale_source_identity_wrong_disk_and_unsupported_partial_extent_refuse() {
    let f = Fixture::new(None);
    let a = f.attempt();
    fs::write(f.project.images_dir().join("001.bin"), vec![0x42; 737_280]).unwrap();
    assert!(
        recover(
            &f.project.images_dir(),
            &f.project.recovery_dir(),
            1,
            &a,
            false,
            &|_| {}
        )
        .is_err()
    );
    assert!(
        recover(
            &f.project.images_dir(),
            &f.project.recovery_dir(),
            2,
            &f.attempt(),
            false,
            &|_| {}
        )
        .is_err()
    );
    let partial = Fixture::new(Some(vec![0; 513]));
    assert!(partial.recover(false).is_err());
}

#[test]
fn zero_candidates_remain_a_reusable_attention_report_not_a_filled_image() {
    let f = Fixture::new(Some(vec![0x42; 737_280]));
    let r = f.recover(false).unwrap();
    assert_eq!(r.report.files.len(), 0);
    assert_eq!(r.report.total_bytes, 0);
    assert!(r.report.filesystem_error.is_some());
    assert!(f.recover(false).unwrap().reused);
    assert_eq!(
        fs::read(f.project.images_dir().join("001.bin")).unwrap(),
        vec![0x42; 737_280]
    );
}

#[test]
fn source_changes_during_staging_keep_unpublished_candidates_and_refuse_result() {
    let f = Fixture::new(None);
    let a = f.attempt();
    let source = f.project.images_dir().join("001.bin");
    let result = recover(
        &f.project.images_dir(),
        &f.project.recovery_dir(),
        1,
        &a,
        false,
        &|_| {
            fs::write(&source, vec![0x41; 737_280]).unwrap();
        },
    );
    assert!(
        result
            .unwrap_err()
            .contains("Snapshot changed before publication")
    );
    let folder = f.project.recovery_dir().join("SnapshotCandidates/001");
    assert!(fs::read_dir(folder).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp-snapshot-")
    }));
}

#[test]
fn cli_plan_is_read_only_json_attention_is_explicit_and_unrelated_flags_refuse() {
    let f = Fixture::new(None);
    let args = |extra: &[&str]| {
        let mut a = vec![
            "recovery".into(),
            "snapshot".into(),
            "1".into(),
            "--json".into(),
        ];
        a.extend(extra.iter().map(|s| s.to_string()));
        a
    };
    let preview = crate::cli::run(&args(&["--plan"]), f.project.root()).unwrap();
    assert_eq!(preview.exit_code, 0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&preview.output).unwrap()["plan_only"],
        true
    );
    assert!(!f.project.recovery_dir().join("SnapshotCandidates").exists());
    for flag in [
        "--no-verify",
        "--include-deleted",
        "--double",
        "--images-only",
    ] {
        assert!(
            crate::cli::run(&args(&[flag]), f.project.root()).is_err(),
            "{flag}"
        );
    }
    let saved = crate::cli::run(&args(&[]), f.project.root()).unwrap();
    assert_eq!(saved.exit_code, 3);
    assert!(
        serde_json::from_str::<serde_json::Value>(&saved.output).unwrap()["report"]["acquisition_mask_known"]
            == false
    );
}

#[test]
fn snapshot_file_trace_has_exact_extents_but_never_infers_a_physical_read_pass() {
    let f = Fixture::new(None);
    let result = f.recover(false).unwrap();
    let relative = result
        .output_directory
        .join(&result.report.files[0].path)
        .strip_prefix(f.project.root().canonicalize().unwrap())
        .unwrap()
        .to_string_lossy()
        .to_string();
    let trace = crate::file_lineage::inspect(&f.project, 1, &relative).unwrap();
    assert_eq!(trace["file"]["kind"], "unverified_snapshot_candidate");
    assert_eq!(trace["file"]["acquisition_mask_known"], false);
    assert_eq!(trace["file"]["source_extents"][0]["offset"], 14 * 512);
    assert_eq!(trace["file"]["source_image"]["sha256"], f.attempt().sha256);
    assert_eq!(trace["files_written"], 0);
    assert!(crate::file_lineage::inspect(&f.project, 2, &relative).is_err());
    fs::write(
        result.output_directory.join(&result.report.files[0].path),
        b"changed",
    )
    .unwrap();
    assert!(crate::file_lineage::inspect(&f.project, 1, &relative).is_err());
}
