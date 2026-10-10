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
