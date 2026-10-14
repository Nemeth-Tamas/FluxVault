use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fv-legacy-import-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn zip(&self, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = self.0.join("source.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        path
    }
    fn target(&self) -> PathBuf {
        self.0.join("Imported")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only this fixture's literal, uniquely-created temporary directory.
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn windows_unsafe_paths_and_control_names_are_refused() {
    for name in [
        "../Images/001.bin",
        "/Images/001.bin",
        "C:/Images/001.bin",
        "Images/../001.bin",
        "Extracted/001/CON.txt",
        "Extracted/001/com1.txt",
        "Extracted/001/LPT².dat",
        "Extracted/001/a:stream",
        "Extracted/001/x.",
        "Extracted/001/x ",
        "Extracted//x",
        "Extracted/001/.fluxvault-extraction.json",
        "Images/001.bin/",
        "project.json",
        "Reports/LegacyImport.json",
        "Reports/LegacyImportFiles.csv",
    ] {
        assert!(safe_path(name, false).is_err(), "accepted {name}");
    }
    assert_eq!(
        safe_path("extracted\\001\\árvíz.doc", false).unwrap(),
        "Extracted/001/árvíz.doc"
    );
    assert_eq!(safe_path("Images/", true).unwrap(), "Images");
}

#[test]
fn plan_never_creates_destination_or_staging() {
    let f = Fixture::new();
    let source = f.zip(&[("Images/001.bin", b"saved image")]);
    let before = hash_file(&source, MAX_ARCHIVE).unwrap();
    let value = run(&f.0, &source, &f.target(), true, &|_| {}).unwrap();
    assert_eq!(value["images"], 1);
    assert_eq!(value["plan_only"], true);
    assert_eq!(value["member_payloads_verified"], false);
    assert!(!f.target().exists());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    assert_eq!(hash_file(&source, MAX_ARCHIVE).unwrap(), before);
}

#[test]
fn images_only_isolates_inputs_records_exclusions_and_preserves_index_claims_as_metadata() {
    let f = Fixture::new();
    let image = vec![9u8; 1024];
    let source = f.zip(&[
        ("Images/007.bin", &image),
        ("Logs/007.log", b"old acquisition log"),
        ("Extracted/007/reference.doc", b"never seed a recovery"),
        ("Converted/007/reference.pdf", b"not a new conversion"),
        ("Recovery/007/manual.doc", b"not a new native recovery"),
        (
            "Reports/ArchiveIndex.csv",
            b"FloppyNumber,Status\r\n007,OK\r\n",
        ),
        ("README.txt", b"historical description"),
    ]);
    let before = hash_file(&source, MAX_ARCHIVE).unwrap();
    let plan = run_images_only(&f.0, &source, &f.target(), true, &|_| {}).unwrap();
    assert_eq!(plan["files"], 2);
    assert_eq!(plan["excluded_files"], 5);
    assert_eq!(plan["archive_files"], 7);
    assert_eq!(plan["reference_payloads_imported"], false);
    assert!(!f.target().exists());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    let applied = run_images_only(&f.0, &source, &f.target(), false, &|_| {}).unwrap();
    assert_eq!(applied["import_scope"], "images_and_logs_only");
    assert_eq!(applied["selected_member_payloads_verified"], true);
    assert_eq!(applied["member_payloads_verified"], false);
    assert_eq!(applied["reference_payloads_imported"], false);
    assert_eq!(fs::read(f.target().join("Images/007.bin")).unwrap(), image);
    assert_eq!(
        fs::read(f.target().join("Logs/007.log")).unwrap(),
        b"old acquisition log"
    );
    for top in ["Extracted", "Converted", "Recovery"] {
        assert_eq!(fs::read_dir(f.target().join(top)).unwrap().count(), 0);
    }
    assert!(!f.target().join("Reports/ArchiveIndex.csv").exists());
    assert!(!f.target().join("README.txt").exists());
    let report: Value =
        serde_json::from_slice(&fs::read(f.target().join(REPORT)).unwrap()).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["excluded_members"].as_array().unwrap().len(), 5);
    assert_eq!(
        report["disks"][0]["archive_index_claims"][0]["row"]["Status"],
        "OK"
    );
    assert_eq!(report["disks"][0]["legacy_extracted_files"], 0);
    assert_eq!(report["disks"][0]["legacy_recovery_files"], 0);
    assert!(run_images_only(&f.0, &source, &f.target(), false, &|_| {}).is_err());
    assert_eq!(hash_file(&source, MAX_ARCHIVE).unwrap(), before);
}

#[test]
fn images_only_still_refuses_unsafe_or_colliding_excluded_paths() {
    for path in [
        "Extracted/../escape.doc",
        "Recovery/007/NUL.doc",
        "Reports/LegacyImport.json",
    ] {
        let f = Fixture::new();
        let source = f.zip(&[("Images/001.bin", b"image"), (path, b"not copied")]);
        assert!(run_images_only(&f.0, &source, &f.target(), false, &|_| {}).is_err());
        assert!(!f.target().exists());
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    }
    let f = Fixture::new();
    let source = f.zip(&[
        ("Images/001.bin", b"image"),
        ("Extracted/001/a", b"one"),
        ("Extracted/001/A", b"two"),
    ]);
    assert!(run_images_only(&f.0, &source, &f.target(), false, &|_| {}).is_err());
}

#[test]
fn skipped_payload_crc_is_not_certified_and_selected_corruption_refuses_publication() {
    for corrupt_selected in [false, true] {
        let f = Fixture::new();
        let source = f.zip(&[
            ("Images/001.bin", b"unique image bytes"),
            ("Extracted/001/a.doc", b"unique reference bytes"),
        ]);
        let needle: &[u8] = if corrupt_selected {
            b"unique image bytes"
        } else {
            b"unique reference bytes"
        };
        let mut bytes = fs::read(&source).unwrap();
        let offset = bytes
            .windows(needle.len())
            .position(|v| v == needle)
            .unwrap();
        bytes[offset] ^= 1; // Stored member has invalid CRC but unchanged metadata.
        fs::write(&source, bytes).unwrap();
        let before = hash_file(&source, MAX_ARCHIVE).unwrap();
        let applied = run_images_only(&f.0, &source, &f.target(), false, &|_| {});
        if corrupt_selected {
            assert!(applied.is_err());
            assert!(!f.target().exists());
        } else {
            let value = applied.unwrap();
            assert_eq!(value["member_payloads_verified"], false);
            assert_eq!(value["selected_member_payloads_verified"], true);
            assert!(!f.target().join("Extracted/001/a.doc").exists());
        }
        assert_eq!(hash_file(&source, MAX_ARCHIVE).unwrap(), before);
    }
}

#[test]
fn images_only_cancel_or_source_change_keeps_staging_without_publishing() {
    for cancel in [false, true] {
        let f = Fixture::new();
        let source = f.zip(&[
            ("Images/001.bin", b"image"),
            ("Extracted/001/a.doc", b"reference"),
        ]);
        let token = crate::cancellation::Token::default();
        let _scope = crate::cancellation::enter(token.clone());
        let error = run_images_only(&f.0, &source, &f.target(), false, &|message| {
            if cancel && message.starts_with("IMPORT / staging:") {
                token.request();
            } else if !cancel && message.starts_with("IMPORT / final source-") {
                OpenOptions::new()
                    .append(true)
                    .open(&source)
                    .unwrap()
                    .write_all(b"changed")
                    .unwrap();
            }
        })
        .unwrap_err();
        assert!(!f.target().exists());
        if cancel {
            assert!(crate::cancellation::stopped(&error));
        } else {
            assert!(error.contains("Source ZIP changed"));
        }
        assert!(fs::read_dir(&f.0).unwrap().flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".fluxvault-import-")
                && e.path().is_dir()
        }));
    }
}

#[test]
fn legacy_snapshot_identity_and_log_claim_remain_separate_across_recovery_and_audit() {
    let f = Fixture::new();
    let image = vec![0x42; 1024];
    let actual = format!("{:x}", Sha256::digest(&image));
    let claimed = "a".repeat(64);
    let log = format!(
        "BEGIN floppy=007\nGEOMETRY cylinders=1 heads=1 sectors_per_track=2 bytes_per_sector=512 total_sectors=2 total_bytes=1024\nSHA256 {claimed}\nEND status=OK bad_sectors=0 recovered_after_retry=0 track_fallbacks=0 duration_seconds=1.0\n"
    );
    let source = f.zip(&[("Images/007.bin", &image), ("Logs/007.log", log.as_bytes())]);
    run_images_only(&f.0, &source, &f.target(), false, &|_| {}).unwrap();
    let project = ProjectState::open_without_session(f.target()).unwrap();
    let attempt = crate::imaging::load_attempts_for_disk(&project.images_dir(), 7)
        .unwrap()
        .remove(0);
    assert_eq!(attempt.sha256, actual);
    assert_eq!(attempt.status, "OK"); // Historical label, not current certification.
    assert!(attempt.attention_required);
    assert!(attempt.require_legacy_sector_binding().is_err());
    let plan = crate::recovery_plan::plan_project(&project.images_dir()).unwrap();
    assert!(plan[0].reason.contains("readable-sector map is untrusted"));
    assert_eq!(
        plan[0].action,
        crate::recovery_plan::RecoveryAction::CheckEvidence
    );
    for command in ["fat", "composite"] {
        let refusal = crate::cli::run(
            &["recovery".into(), command.into(), "7".into()],
            project.root(),
        )
        .unwrap_err();
        assert!(
            refusal.contains("readable-sector map is untrusted"),
            "{refusal}"
        );
    }
    assert_eq!(
        attempt.parsed_log.as_ref().unwrap().sha256.as_deref(),
        Some(claimed.as_str())
    );
    let directory = project.extracted_dir().join("007/legacy");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("payload.txt"),
        b"snapshot-bound derived output",
    )
    .unwrap();
    // A previously generated ordinary extraction binds the actual file, not
    // the contradictory historical claim. Selection must not reject its hash.
    let payload = fs::read(directory.join("payload.txt")).unwrap();
    let inventory = json!({"schema_version":1,"source_image":"007.bin","source_sha256":actual,
        "files":[{"relative_path":"payload.txt","bytes":payload.len(),"modified_unix_ms":null,"attributes":"","sha256":format!("{:x}",Sha256::digest(&payload))}]});
    let marker = json!({"schema_version":1,"source_image":"007.bin","source_sha256":actual,
        "extracted_unix_ms":0,"file_count":1,"total_bytes":payload.len(),"native_recovery_report_sha256":null});
    fs::write(
        directory.join(".fluxvault-inventory.json"),
        inventory.to_string(),
    )
    .unwrap();
    fs::write(
        directory.join(".fluxvault-extraction.json"),
        marker.to_string(),
    )
    .unwrap();
    assert!(matches!(
        crate::extraction::inspect_extraction_presence(&project.extracted_dir(), 7, 0).unwrap(),
        crate::extraction::ExtractionPresence::Automatic { .. }
    ));
    let native = crate::fat12_recovery::recover_attempt(
        &project.images_dir(),
        &project.extracted_dir(),
        &project.recovery_dir(),
        7,
        &attempt,
        &|_| {},
    )
    .unwrap_err();
    assert!(
        native.contains("does not establish complete sector evidence"),
        "{native}"
    );
    let audit = crate::audit::run_audit(&project, &|_| {}).unwrap();
    let report: Value = serde_json::from_slice(&fs::read(audit.json_path).unwrap()).unwrap();
    assert_eq!(report["disks"][0]["image_sha256"], actual);
    assert!(
        report["disks"][0]["issue"]
            .as_str()
            .unwrap()
            .contains("Legacy acquisition-log SHA-256 claim disagrees")
    );
    assert_ne!(
        report["disks"][0]["evidence_status"],
        "IMAGE_FILES_CONVERSIONS_VERIFIED"
    );
    assert_eq!(
        fs::read_to_string(project.logs_dir().join("007.log")).unwrap(),
        log
    );
    assert_eq!(
        fs::read(project.images_dir().join("007.bin")).unwrap(),
        image
    );
}

#[test]
fn replay_verification_checks_zip_bytes_even_if_an_input_receipt_is_rebound() {
    let f = Fixture::new();
    let source = f.zip(&[
        ("Images/001.bin", b"original image"),
        ("Logs/001.log", b"original log"),
        ("Extracted/001/reference.doc", b"reference only"),
    ]);
    let source_hash = hash_file(&source, MAX_ARCHIVE).unwrap().1;
    run_images_only(&f.0, &source, &f.target(), false, &|_| {}).unwrap();
    let project = ProjectState::open_without_session(f.target()).unwrap();
    let proof = verify_replay_inputs(&project, &source, &source_hash)
        .unwrap()
        .unwrap();
    assert_eq!(proof.verified_input_files, 2);
    assert_eq!(proof.verified_input_bytes, 26);
    let receipt = project.root().join(REPORT);
    let original_report = fs::read(&receipt).unwrap();
    fs::write(project.images_dir().join("001.bin"), b"modified image").unwrap();
    assert!(
        verify_replay_inputs(&project, &source, &source_hash)
            .unwrap_err()
            .contains("changed from archived bytes")
    );
    let mut rebound: Value = serde_json::from_slice(&original_report).unwrap();
    rebound["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(b"modified image")));
    fs::write(&receipt, rebound.to_string()).unwrap();
    assert!(
        verify_replay_inputs(&project, &source, &source_hash)
            .unwrap_err()
            .contains("changed from archived bytes")
    );
    fs::write(project.images_dir().join("001.bin"), b"original image").unwrap();
    fs::write(&receipt, &original_report).unwrap();
    assert_eq!(
        verify_replay_inputs(&project, &source, &source_hash).unwrap(),
        Some(proof)
    );
    fs::write(project.logs_dir().join("001.log"), b"modified log").unwrap();
    assert!(verify_replay_inputs(&project, &source, &source_hash).is_err());
}

#[test]
fn replay_verification_refuses_forged_scope_paths_and_incomplete_source_inventories() {
    let f = Fixture::new();
    let source = f.zip(&[("Images/001.bin", b"image"), ("Logs/001.log", b"log")]);
    let source_hash = hash_file(&source, MAX_ARCHIVE).unwrap().1;
    run_images_only(&f.0, &source, &f.target(), false, &|_| {}).unwrap();
    let project = ProjectState::open_without_session(f.target()).unwrap();
    let receipt = project.root().join(REPORT);
    let original: Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    for failure in 0..7 {
        let mut changed = original.clone();
        match failure {
            0 => changed["files"][0]["path"] = json!("Images/../escape"),
            1 => changed["files"][0]["path"] = json!("Extracted/001/reference.doc"),
            2 => {
                changed["files"].as_array_mut().unwrap().pop();
            }
            3 => {
                let duplicate = changed["files"][0].clone();
                changed["files"].as_array_mut().unwrap().push(duplicate);
            }
            4 => changed["reference_payloads_imported"] = json!(true),
            5 => changed["source_sha256"] = json!("a".repeat(64)),
            6 => changed["import_scope"] = json!("unknown_mode"),
            _ => unreachable!(),
        }
        fs::write(&receipt, changed.to_string()).unwrap();
        assert!(
            verify_replay_inputs(&project, &source, &source_hash).is_err(),
            "accepted invalid replay {failure}"
        );
    }
    fs::write(&receipt, original.to_string()).unwrap();
    assert!(verify_replay_inputs(&project, &source, &"b".repeat(64)).is_err());
    assert!(
        verify_replay_inputs(&project, &source, &source_hash)
            .unwrap()
            .is_some()
    );
}

#[test]
fn copies_all_legacy_bytes_and_records_claims_without_certifying() {
    let f = Fixture::new();
    let image = vec![7u8; 1024];
    let sha = format!("{:x}", Sha256::digest(&image));
    let log = format!(
        "BEGIN floppy=007\nGEOMETRY cylinders=1 heads=1 sectors_per_track=2 bytes_per_sector=512 total_sectors=2 total_bytes=1024\nSHA256 {sha}\nEND status=OK bad_sectors=0 recovered_after_retry=0 track_fallbacks=0 duration_seconds=1.0\n"
    );
    let index = format!(
        "\u{feff}\"FloppyNumber\",\"SHA256\",\"DurationSeconds\"\r\n\"007\",\"{sha}\",\"1,5\"\r\n\"999\",\"missing\",\"2\"\r\n"
    );
    let entries = [
        ("Images/007.bin", image.as_slice()),
        ("Images/009.img", b"short image".as_slice()),
        ("Images/010.partial.img", b"partial evidence".as_slice()),
        ("Logs/007.log", log.as_bytes()),
        ("Reports/ArchiveIndex.csv", index.as_bytes()),
        ("Extracted/007/Letter.doc", b"legacy doc".as_slice()),
        ("Converted/007/Letter.pdf", b"legacy pdf".as_slice()),
        ("Recovery/007/manual.log", b"old manual receipt".as_slice()),
        ("README.txt", b"old instructions".as_slice()),
    ];
    let source = f.zip(&entries);
    let source_hash = hash_file(&source, MAX_ARCHIVE).unwrap();
    let value = run(&f.0, &source, &f.target(), false, &|_| {}).unwrap();
    assert_eq!(value["published"], true);
    assert_eq!(value["images"], 2);
    assert_eq!(value["next_disk"], 10);
    assert_eq!(value["customer_delivery_certified"], false);
    for (name, bytes) in entries {
        assert_eq!(fs::read(f.target().join(name)).unwrap(), bytes);
    }
    assert_eq!(hash_file(&source, MAX_ARCHIVE).unwrap(), source_hash);
    let project = ProjectState::open_without_session(f.target()).unwrap();
    assert_eq!(project.name(), "Imported");
    assert_eq!(project.current_disk_number(), 10);
    assert_eq!(
        crate::run_control::status(&project).unwrap()["active"],
        false
    );
    let report: Value =
        serde_json::from_slice(&fs::read(f.target().join(REPORT)).unwrap()).unwrap();
    assert_eq!(report["disks"][0]["legacy_status"], "OK");
    assert_eq!(report["disks"][0]["recorded_log_sha256_matches"], true);
    assert_eq!(
        report["disks"][0]["archive_index_claims"][0]["row"]["DurationSeconds"],
        "1,5"
    );
    assert_eq!(report["disks"][0]["legacy_extracted_files"], 1);
    assert_eq!(
        report["archive_index_rows_without_images"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !f.target()
            .join("Extracted/007/.fluxvault-extraction.json")
            .exists()
    );
    for file in report["files"].as_array().unwrap() {
        let actual = hash_file(&f.target().join(file["path"].as_str().unwrap()), MAX_FILE).unwrap();
        assert_eq!(file["bytes"], actual.0);
        assert_eq!(file["sha256"], actual.1);
    }
}

#[test]
fn existing_destination_and_nested_project_are_refused_without_changes() {
    let f = Fixture::new();
    let source = f.zip(&[("Images/001.bin", b"image")]);
    fs::create_dir(f.target()).unwrap();
    fs::write(f.target().join("sentinel"), b"keep").unwrap();
    assert!(run(&f.0, &source, &f.target(), false, &|_| {}).is_err());
    assert_eq!(fs::read(f.target().join("sentinel")).unwrap(), b"keep");
    let existing = ProjectState::create_without_session(f.0.join("Existing")).unwrap();
    assert!(destination(&f.0, &existing.root().join("nested")).is_err());
    assert!(!existing.root().join("nested").exists());
}

#[test]
fn device_paths_are_rejected() {
    let f = Fixture::new();
    for source in ["A:", "A:\\", r"\\.\A:", r"\\.\PhysicalDrive0"] {
        assert!(prepare(&f.0, Path::new(source), &f.target()).is_err());
    }
}

#[test]
fn duplicate_case_and_file_directory_prefix_collisions_are_rejected() {
    for extra in [
        ("images/001.bin", b"other".as_slice()),
        ("Extracted/001/child", b"child".as_slice()),
    ] {
        let f = Fixture::new();
        let source = f.zip(&[
            ("Images/001.bin", b"image"),
            ("Extracted/001", b"file"),
            extra,
        ]);
        assert!(prepare(&f.0, &source, &f.target()).is_err());
        assert!(!f.target().exists());
    }
}

#[test]
fn native_metadata_and_duplicate_noncanonical_labels_are_rejected() {
    for name in [
        "Images/001.json",
        "Images/1.bin",
        "Images/000.bin",
        "Images/001.img",
        "Images/001_attempt_001.img",
        "project.json",
    ] {
        let f = Fixture::new();
        let source = f.zip(&[("Images/001.bin", b"image"), (name, b"extra")]);
        assert!(
            prepare(&f.0, &source, &f.target()).is_err(),
            "accepted {name}"
        );
    }
}

#[test]
fn archive_symlinks_are_rejected() {
    let f = Fixture::new();
    let source = f.0.join("source.zip");
    let mut writer = zip::ZipWriter::new(File::create(&source).unwrap());
    writer
        .start_file("Images/001.bin", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"image").unwrap();
    writer
        .add_symlink(
            "Extracted/001/link",
            "../../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.finish().unwrap();
    assert!(prepare(&f.0, &source, &f.target()).is_err());
}

#[test]
fn malformed_archive_index_is_rejected_before_staging() {
    for csv in [
        "Wrong\r\n001\r\n",
        "FloppyNumber,FloppyNumber\r\n001,001\r\n",
        "FloppyNumber,SHA256\r\n001\r\n",
    ] {
        let f = Fixture::new();
        let source = f.zip(&[
            ("Images/001.bin", b"image"),
            ("Reports/ArchiveIndex.csv", csv.as_bytes()),
        ]);
        assert!(run(&f.0, &source, &f.target(), false, &|_| {}).is_err());
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    }
}

#[test]
fn historical_wrong_hashes_and_truncated_extent_are_attention_not_rewritten() {
    let f = Fixture::new();
    let image = vec![8u8; 512];
    let log = "BEGIN floppy=001\nGEOMETRY cylinders=1 heads=1 sectors_per_track=2 bytes_per_sector=512 total_sectors=2 total_bytes=1024\nSHA256 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nEND status=OK bad_sectors=0 recovered_after_retry=0\n";
    let index = "FloppyNumber,SHA256\r\n001,bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\r\n";
    let source = f.zip(&[
        ("Images/001.bin", &image),
        ("Logs/001.log", log.as_bytes()),
        ("Reports/ArchiveIndex.csv", index.as_bytes()),
    ]);
    let value = run(&f.0, &source, &f.target(), false, &|_| {}).unwrap();
    assert_eq!(value["historical_claim_discrepancies"], 3);
    let report: Value =
        serde_json::from_slice(&fs::read(f.target().join(REPORT)).unwrap()).unwrap();
    assert_eq!(
        report["disks"][0]["reported_extent_matches_saved_image"],
        false
    );
    assert_eq!(report["disks"][0]["recorded_log_sha256_matches"], false);
    assert_eq!(fs::read(f.target().join("Images/001.bin")).unwrap(), image);
    assert_eq!(
        fs::read_to_string(f.target().join("Reports/ArchiveIndex.csv")).unwrap(),
        index
    );
}

#[test]
fn cancel_keeps_partial_stage_and_does_not_publish() {
    let f = Fixture::new();
    let source = f.zip(&[("Images/001.bin", b"image")]);
    let token = crate::cancellation::Token::default();
    let _scope = crate::cancellation::enter(token.clone());
    let error = run(&f.0, &source, &f.target(), false, &|message| {
        if message.starts_with("IMPORT / staging:") {
            token.request();
        }
    })
    .unwrap_err();
    assert!(crate::cancellation::stopped(&error));
    assert!(!f.target().exists());
    let stages: Vec<_> = fs::read_dir(&f.0)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.path().is_dir()
                && e.file_name()
                    .to_string_lossy()
                    .starts_with(".fluxvault-import-")
        })
        .collect();
    assert_eq!(stages.len(), 1);
    let receipt: Value =
        serde_json::from_slice(&fs::read(stages[0].path().join("IMPORT-INCOMPLETE.json")).unwrap())
            .unwrap();
    assert_eq!(receipt["phase"], "interrupted");
    assert_eq!(receipt["published"], false);
}

#[test]
fn changed_source_after_copy_refuses_publication() {
    let f = Fixture::new();
    let source = f.zip(&[("Images/001.bin", b"image")]);
    let error = run(&f.0, &source, &f.target(), false, &|message| {
        if message.starts_with("IMPORT / final source-") {
            OpenOptions::new()
                .append(true)
                .open(&source)
                .unwrap()
                .write_all(b"changed")
                .unwrap();
        }
    })
    .unwrap_err();
    assert!(error.contains("Source ZIP changed"));
    assert!(!f.target().exists());
}

#[test]
fn bounded_reader_refuses_expansion_beyond_declared_size() {
    assert!(hash(&mut b"too long".as_slice(), 2, None).is_err());
    assert_eq!(hash(&mut b"".as_slice(), 0, None).unwrap().0, 0);
}

#[test]
fn destination_owner_excludes_another_import_and_releases() {
    let f = Fixture::new();
    let target = f.target();
    let first = destination_owner(&target).unwrap();
    assert!(destination_owner(&target).is_err());
    drop(first);
    assert!(destination_owner(&target).is_ok());
}

#[test]
fn staged_mutation_is_refused_and_retained() {
    let f = Fixture::new();
    let source = f.zip(&[("Images/001.bin", b"image")]);
    let error = run(&f.0, &source, &f.target(), false, &|message| {
        if message.starts_with("IMPORT / independently rechecking") {
            let stage = fs::read_dir(&f.0)
                .unwrap()
                .flatten()
                .find(|e| {
                    e.path().is_dir()
                        && e.file_name()
                            .to_string_lossy()
                            .starts_with(".fluxvault-import-")
                })
                .unwrap();
            fs::write(stage.path().join("Imported/Images/001.bin"), b"changed").unwrap();
        }
    })
    .unwrap_err();
    assert!(error.contains("Staged member changed"));
    assert!(!f.target().exists());
}

#[test]
fn bad_crc_never_publishes_project() {
    use std::io::{Seek, SeekFrom};
    let f = Fixture::new();
    let source = f.zip(&[("Images/001.bin", b"image")]);
    let offset = {
        let mut zip = zip::ZipArchive::new(File::open(&source).unwrap()).unwrap();
        zip.by_index(0).unwrap().data_start()
    };
    let mut file = OpenOptions::new().write(true).open(&source).unwrap();
    file.seek(SeekFrom::Start(offset.unwrap())).unwrap();
    file.write_all(b"X").unwrap();
    drop(file);
    assert!(run(&f.0, &source, &f.target(), false, &|_| {}).is_err());
    assert!(!f.target().exists());
    assert!(
        fs::read_dir(&f.0)
            .unwrap()
            .flatten()
            .any(|e| e.path().join("IMPORT-INCOMPLETE.json").is_file())
    );
}

#[test]
fn impossible_zero_sector_geometry_is_a_guarded_error_not_a_panic() {
    let f = Fixture::new();
    let source = f.zip(&[
        ("Images/001.bin", b"image"),
        (
            "Logs/001.log",
            b"BEGIN floppy=001\nGEOMETRY bytes_per_sector=0\nEND status=OK\n",
        ),
    ]);
    let error = run(&f.0, &source, &f.target(), false, &|_| {}).unwrap_err();
    assert!(error.contains("zero-byte sectors"));
    assert!(!f.target().exists());
}
