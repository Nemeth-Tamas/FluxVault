use super::*;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn compound(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut c = cfb::CompoundFile::create(Cursor::new(vec![])).unwrap();
    for (name, bytes) in streams {
        if name.starts_with("/Nested/") {
            c.create_storage_all("/Nested").unwrap();
        }
        c.create_stream(name).unwrap().write_all(bytes).unwrap();
    }
    c.flush().unwrap();
    c.into_inner().into_inner()
}
fn word(version: u16, encrypted: bool) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[..2].copy_from_slice(&0xa5ecu16.to_le_bytes());
    bytes[2..4].copy_from_slice(&version.to_le_bytes());
    if encrypted {
        bytes[10..12].copy_from_slice(&0x100u16.to_le_bytes());
    }
    bytes
}
fn book(version: u16, records: &[u8]) -> Vec<u8> {
    let mut bytes = vec![9, 8, 8, 0];
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&[5, 0, 0, 0, 0, 0]);
    bytes.extend_from_slice(records);
    bytes
}
fn temp() -> PathBuf {
    static ID: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "fluxvault-content-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    root
}

#[test]
fn supported_word_fibs_are_recognized_without_extension_or_table_stream() {
    for version in [0x65, 0x68, 0x69, 0xc1, 0xd9, 0x101, 0x10c, 0x112] {
        let detection = detect(compound(&[("/WordDocument", word(version, false))])).unwrap();
        assert_eq!(detection.family, Some(OfficeFamily::Word));
        assert!(detection.blocked_reason.is_none());
    }
    let mut legacy = word(0x65, false);
    legacy[..2].copy_from_slice(&0xa5dcu16.to_le_bytes());
    assert_eq!(
        detect(compound(&[("/WordDocument", legacy)]))
            .unwrap()
            .format,
        "word_6_95"
    );
    assert!(detect(compound(&[("/WordDocument", word(0x7777, false))])).is_none());
}

#[test]
fn word_password_and_encrypted_package_are_explicit_not_guessed_application() {
    assert!(
        detect(compound(&[("/WordDocument", word(0xc1, true))]))
            .unwrap()
            .blocked_reason
            .unwrap()
            .contains("Password-protected Word")
    );
    let encrypted = detect(compound(&[
        ("/EncryptionInfo", vec![0; 8]),
        ("/EncryptedPackage", vec![0; 8]),
    ]))
    .unwrap();
    assert_eq!(encrypted.family, None);
    assert!(
        encrypted
            .blocked_reason
            .unwrap()
            .contains("Password-protected")
    );
}

#[test]
fn biff5_and_biff8_route_and_filepass_is_not_worksheet_protection() {
    for version in [0x0500, 0x0600] {
        // Protect (0x12) and a FilePass-looking payload do not mean open-password.
        let records = [0x12, 0, 4, 0, 0x2f, 0, 4, 0, 0x0a, 0, 0, 0];
        let d = detect(compound(&[("/Book", book(version, &records))])).unwrap();
        assert_eq!(d.family, Some(OfficeFamily::Excel));
        assert!(d.blocked_reason.is_none());
        let password_record = if version == 0x0500 {
            vec![0x2f, 0, 4, 0, 0, 0, 0, 0]
        } else {
            vec![0x2f, 0, 6, 0, 0, 0, 0, 0, 0, 0]
        };
        let d = detect(compound(&[("/Workbook", book(version, &password_record))])).unwrap();
        assert!(d.blocked_reason.unwrap().contains("FilePass"));
    }
}

#[test]
fn truncated_globals_and_invalid_filepass_never_claim_proven_password() {
    for records in [&[0x2f, 0, 4, 0, 1][..], &[0x2f, 0, 0, 0], &[]] {
        let d = detect(compound(&[("/Book", book(0x0500, records))])).unwrap();
        assert_eq!(d.format, "excel_unresolved_globals");
        assert!(d.blocked_reason.is_none());
    }
    let mut bytes = book(0x0600, &[0x0a, 0, 0, 0]);
    bytes[6] = 0x10; // worksheet substream instead of workbook globals
    assert!(detect(compound(&[("/Book", bytes)])).is_none());
}

#[test]
fn global_detection_budget_is_bounded_and_ignores_records_after_eof() {
    let mut records = vec![0x0a, 0, 0, 0];
    records.extend_from_slice(&[0x2f, 0, 4, 0, 0, 0, 0, 0]);
    assert!(
        detect(compound(&[("/Book", book(0x0600, &records))]))
            .unwrap()
            .blocked_reason
            .is_none()
    );
    let records = [0x12, 0, 0, 0].repeat(RECORD_LIMIT + 1);
    let d = detect(compound(&[("/Book", book(0x0600, &records))])).unwrap();
    assert!(d.evidence.contains("budget") && d.blocked_reason.is_none());
}

#[test]
fn nested_decoys_and_conflicting_application_roots_never_choose_a_family() {
    assert!(detect(compound(&[("/Nested/WordDocument", word(0xc1, false))])).is_none());
    let d = detect(compound(&[
        ("/WordDocument", word(0xc1, false)),
        ("/Book", book(0x0500, &[0x0a, 0, 0, 0])),
    ]))
    .unwrap();
    assert!(d.family.is_none() && d.blocked_reason.is_some());
    let d = detect(compound(&[
        ("/Book", book(0x0500, &[0x0a, 0, 0, 0])),
        ("/Workbook", book(0x0600, &[0x0a, 0, 0, 0])),
    ]))
    .unwrap();
    assert!(d.blocked_reason.is_none());
    assert_eq!(d.family, Some(OfficeFamily::Excel));
    assert!(d.evidence.contains("both Book/Workbook"));
    let protected_compatibility = detect(compound(&[
        ("/Workbook", book(0x0600, &[0x0a, 0, 0, 0])),
        ("/Book", book(0x0500, &[0x2f, 0, 4, 0, 0, 0, 0, 0])),
    ]))
    .unwrap();
    assert!(
        protected_compatibility
            .blocked_reason
            .unwrap()
            .contains("FilePass")
    );
}

#[test]
fn saved_file_probe_leaves_originals_unchanged_and_limits_oversized_inputs() {
    let root = temp();
    let path = root.join("original.ole");
    let bytes = compound(&[("/WordDocument", word(0x68, false))]);
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        inspect(&path).unwrap().unwrap().family,
        Some(OfficeFamily::Word)
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(LIMIT + 1)
        .unwrap();
    assert!(inspect(&path).unwrap().is_none());
    assert!(inspect(&root).is_err());
    fs::write(&path, b"not an Office container").unwrap();
    assert!(inspect(&path).unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn planning_routes_generic_and_misnamed_sources_but_preserves_their_names_and_bytes() {
    let root = temp();
    let extracted = root.join("Extracted/001");
    fs::create_dir_all(&extracted).unwrap();
    let word_bytes = compound(&[("/WordDocument", word(0x68, false))]);
    fs::write(extracted.join("old.ole"), &word_bytes).unwrap();
    fs::write(extracted.join("backup.wbk"), &word_bytes).unwrap();
    fs::write(extracted.join("work.tmp"), &word_bytes).unwrap();
    fs::write(extracted.join("actually-word.xls"), &word_bytes).unwrap();
    fs::write(
        extracted.join("locked.xls"),
        compound(&[("/Book", book(0x0500, &[0x2f, 0, 4, 0, 0, 0, 0, 0]))]),
    )
    .unwrap();
    fs::write(extracted.join("unknown.doc"), b"legacy extension fallback").unwrap();
    let plan = crate::conversion::build_conversion_plan(
        &crate::conversion::ConversionPlanningRequest {
            extracted_root: root.join("Extracted"),
            converted_root: root.join("Converted"),
            reports_directory: root.join("Reports"),
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(plan.jobs.len(), 6);
    for name in ["old.ole", "actually-word.xls", "backup.wbk", "work.tmp"] {
        let job = plan
            .jobs
            .iter()
            .find(|j| j.source_path.ends_with(name))
            .unwrap();
        assert_eq!(job.modern_format, "DOCX");
        assert!(job.modern_path.to_string_lossy().contains("[from "));
        assert_eq!(fs::read(&job.source_path).unwrap(), word_bytes);
        let mirror = root.join("Converted").join(&job.delivery_original_path);
        assert_eq!(fs::read(mirror).unwrap(), word_bytes);
    }
    assert!(
        plan.jobs
            .iter()
            .find(|j| j.source_path.ends_with("locked.xls"))
            .unwrap()
            .content_detection
            .as_ref()
            .unwrap()
            .blocked_reason
            .is_some()
    );
    assert!(
        plan.jobs
            .iter()
            .find(|j| j.source_path.ends_with("unknown.doc"))
            .unwrap()
            .content_detection
            .is_none()
    );
    assert!(
        fs::read_to_string(plan.conversion_plan)
            .unwrap()
            .contains("DetectedContent")
    );
    let mut old_job = serde_json::to_value(&plan.jobs[0]).unwrap();
    old_job.as_object_mut().unwrap().remove("content_detection");
    assert!(
        serde_json::from_value::<crate::conversion::ConversionJob>(old_job)
            .unwrap()
            .content_detection
            .is_none()
    );
    fs::remove_dir_all(root).unwrap();
}
