use super::writer::{csv, excel_text};
use super::*;
use crate::conversion::{ConversionPlanningRequest, build_conversion_plan};
use serde_json::Value;
use std::io::Write;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

#[test]
fn both_customer_notice_languages_keep_raw_evidence_and_prior_generation_unchanged() {
    let f = Fixture::files(false);
    fs::write(f.0.converted_dir().join("UNTRACKED.doc"), b"not certified").unwrap();
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let hu = export(&f.0, &audit, Language::Hungarian).unwrap();
    let old = fs::read(&hu.workbook).unwrap();
    let en = export(&f.0, &audit, Language::English).unwrap();
    for (report, name, phrase) in [
        (&hu, "Ügyféltájékoztató", "Az ellenőrzés figyelmet igényel"),
        (&en, "Customer notices", "Checks need attention"),
    ] {
        let mut zip = ZipArchive::new(fs::File::open(&report.workbook).unwrap()).unwrap();
        let mut book = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("xl/workbook.xml").unwrap(), &mut book)
            .unwrap();
        assert!(book.contains(name));
        let mut strings = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("xl/sharedStrings.xml").unwrap(),
            &mut strings,
        )
        .unwrap();
        assert!(strings.contains(phrase), "{strings}");
        assert!(strings.contains("UNTRACKED.doc"));
        let json: Value = serde_json::from_slice(&fs::read(&report.json).unwrap()).unwrap();
        assert!(
            json["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("UNTRACKED.doc"))
        );
    }
    assert_eq!(fs::read(&hu.workbook).unwrap(), old);
}

struct Fixture(ProjectState);
impl Fixture {
    fn empty() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-final-report-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        Self(ProjectState::create_without_session(root).unwrap())
    }
    fn files(office: bool) -> Self {
        let fixture = Self::empty();
        let p = &fixture.0;
        let image = p.images_dir().join("001_attempt_001.img");
        fs::write(&image, vec![0x5a; 512]).unwrap();
        let hash = conversion::sha256_file(&image).unwrap();
        fs::write(p.images_dir().join("001_attempt_001.json"), serde_json::to_vec(&json!({
            "fluxvault_version":"0.1.0","status":"OK","disk_number":1,"attempt_number":1,
            "source_backend":"synthetic-test","source_device":"none","image_file":"001_attempt_001.img","log_file":"",
            "timestamp_unix_ms":1,"geometry":{"cylinders":1,"heads":1,"sectors_per_track":1,"bytes_per_sector":512,"total_bytes":512,"format_guess":"fixture"},
            "sector_retries":0,"total_sectors":1,"bytes_written":512,"retry_recovered_sectors":0,"bad_sector_count":0,"bad_sectors":[],"sha256":hash
        })).unwrap()).unwrap();
        let folder = p.extracted_dir().join("001/attempt_001");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join(if office { "test.rtf" } else { "=formula.txt" }),
            b"{\\rtf1\\ansi complete fixture}",
        )
        .unwrap();
        let files = extraction::inventory_files(&folder).unwrap();
        fs::write(folder.join(".fluxvault-inventory.json"), serde_json::to_vec(&json!({"schema_version":1,"source_image":"001_attempt_001.img","source_sha256":hash,"files":files})).unwrap()).unwrap();
        fs::write(folder.join(".fluxvault-extraction.json"), serde_json::to_vec(&json!({"schema_version":1,"source_image":"001_attempt_001.img","source_sha256":hash,"extracted_unix_ms":1,"file_count":1,"total_bytes":files[0].bytes})).unwrap()).unwrap();
        let planning = build_conversion_plan(
            &ConversionPlanningRequest {
                extracted_root: p.extracted_dir(),
                converted_root: p.converted_dir(),
                reports_directory: p.reports_dir(),
            },
            &|_| {},
        )
        .unwrap();
        let mut rows = Vec::new();
        for job in &planning.jobs {
            let mut zip = ZipWriter::new(fs::File::create(&job.modern_path).unwrap());
            for (name, text) in [
                ("[Content_Types].xml", "<Types/>"),
                ("word/document.xml", "<document>fixture</document>"),
            ] {
                zip.start_file(name, SimpleFileOptions::default()).unwrap();
                zip.write_all(text.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
            fs::write(&job.pdf_path, b"%PDF-1.4\nfixture\n%%EOF\n").unwrap();
            rows.push(json!({"job":job,"modern":{"state":"Ok","detail":"fixture","retryable":false,"retry_count":0,"output_sha256":conversion::sha256_file(&job.modern_path).unwrap()},
                "pdf":{"state":"Ok","detail":"fixture","retryable":false,"retry_count":0,"output_sha256":conversion::sha256_file(&job.pdf_path).unwrap()},"duration_seconds":1.0}));
        }
        let result: conversion_run::ConversionResult = serde_json::from_value(json!({"planning":planning,"ok":rows.len(),"partial":0,"failed":0,"timed_out":0,"reused_outputs":0,"retried_outputs":0,"issues":[],"summary_path":p.reports_dir().join("ConversionSummary.csv"),"failures_path":p.reports_dir().join("ConversionFailures.txt"),"rows":rows})).unwrap();
        conversion_run::save_snapshot(&p.reports_dir(), p.root(), &result).unwrap();
        // Feed the pre-existing evidence auditor's compatible CSV contract too.
        let mut csv = "\"Floppy\",\"SourceSHA256\",\"DeliveryOriginalPath\",\"ModernPath\",\"ModernFormat\",\"ModernOK\",\"PDFPath\",\"PDFOK\",\"Status\"\r\n".to_owned();
        for row in &result.rows {
            let rel = |path: &Path| {
                path.strip_prefix(p.converted_dir())
                    .or_else(|_| path.strip_prefix(p.converted_dir().canonicalize().unwrap()))
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            };
            csv.push_str(&format!(
                "\"001\",\"{}\",\"{}\",\"{}\",\"docx\",\"true\",\"{}\",\"true\",\"OK\"\r\n",
                row.job.source_sha256,
                row.job.delivery_original_path,
                rel(&row.job.modern_path),
                rel(&row.job.pdf_path)
            ));
        }
        fs::write(p.reports_dir().join("ConversionSummary.csv"), csv).unwrap();
        fixture
    }
    fn report(&self) -> Document {
        let audit = audit::run_audit(&self.0, &|_| {}).unwrap();
        collect(&self.0, &audit).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.root());
    }
}

#[test]
fn final_report_combines_verified_files_conversion_integrity_and_script_metrics() {
    let f = Fixture::files(true);
    let d = f.report();
    assert_eq!(d.floppies[0].audit_status, "OK");
    assert_eq!(d.summary.floppies_fully_ok, 1);
    assert_eq!(d.summary.recovered_source_files, 1);
    assert_eq!(d.summary.customer_delivery_files, 3);
    assert_eq!(d.summary.conversions_ok, 1);
    assert_eq!(d.summary.generated_integrity_checks, 2);
    assert_eq!(d.summary.invalid_generated_outputs, 0);
    assert!(!d.customer_delivery_certified);
    assert!(d.recovered_files[0].managed_inventory_verified);
    assert!(d.delivery_files.iter().all(|r| r.integrity == "VALID"));
}

#[test]
fn structurally_valid_but_changed_pdf_fails_recorded_hash_not_container_check() {
    let f = Fixture::files(true);
    let state = conversion_run::load_snapshot(&f.0.reports_dir(), f.0.root()).unwrap();
    fs::write(
        &state.rows[0].job.pdf_path,
        b"%PDF-1.4\nchanged valid-looking payload\n%%EOF\n",
    )
    .unwrap();
    assert!(conversion_run::validate_output(&state.rows[0].job.pdf_path, "pdf").unwrap());
    let d = f.report();
    assert_eq!(d.summary.conversions_partial, 1);
    assert_eq!(d.summary.invalid_generated_outputs, 1);
    assert_eq!(d.floppies[0].audit_status, "CHECK: CONVERSION FAILED");
    assert!(
        d.integrity
            .iter()
            .any(|r| r.detail.contains("SHA-256 differs"))
    );
}

#[test]
fn missing_original_and_untracked_delivery_never_get_silent_ok() {
    let f = Fixture::files(false);
    let d = f.report();
    assert_eq!(d.floppies[0].audit_status, "OK");
    fs::remove_file(f.0.converted_dir().join("001/=formula.txt")).unwrap();
    fs::write(
        f.0.converted_dir().join("operator-note.txt"),
        b"untagged addition",
    )
    .unwrap();
    let d = f.report();
    assert_ne!(d.floppies[0].audit_status, "OK");
    assert_eq!(d.summary.untracked_delivery_files, 1);
    assert!(d.issues.iter().any(|s| s.contains("missing")));
}

#[test]
fn mutated_selected_source_invalidates_even_unchanged_outputs() {
    let f = Fixture::files(true);
    fs::write(
        f.0.extracted_dir().join("001/attempt_001/test.rtf"),
        b"changed source",
    )
    .unwrap();
    let d = f.report();
    assert!(d.recovered_files.is_empty());
    assert_eq!(d.summary.conversions_failed, 1);
    assert_eq!(d.summary.floppies_fully_ok, 0);
}

#[test]
fn partial_conversion_is_distinct_from_a_changed_successful_output() {
    let f = Fixture::files(true);
    let mut state: Value = serde_json::from_slice(
        &fs::read(conversion_run::snapshot_path(&f.0.reports_dir())).unwrap(),
    )
    .unwrap();
    let pdf = state["result"]["rows"][0]["job"]["pdf_path"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::remove_file(pdf).unwrap();
    state["result"]["rows"][0]["pdf"] = json!({"state":"Failed","detail":"bounded conversion failure","retryable":false,"retry_count":0,"output_sha256":null});
    fs::write(
        conversion_run::snapshot_path(&f.0.reports_dir()),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let d = f.report();
    assert_eq!(d.floppies[0].audit_status, "PARTIAL: CONVERSION");
    assert_eq!(d.summary.conversions_partial, 1);
    assert_eq!(d.summary.invalid_generated_outputs, 0);
}

#[test]
fn generation_publication_keeps_complete_previous_exports_and_bound_artifact_hashes() {
    let f = Fixture::files(false);
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let hu = export(&f.0, &audit, Language::Hungarian).unwrap();
    let before = fs::read(&hu.workbook).unwrap();
    let en = export(&f.0, &audit, Language::English).unwrap();
    assert_ne!(hu.directory, en.directory);
    assert_eq!(before, fs::read(&hu.workbook).unwrap());
    let latest: Value = serde_json::from_slice(&fs::read(&en.latest).unwrap()).unwrap();
    assert_eq!(latest["language"], "en");
    for (name, hash) in latest["artifacts_sha256"].as_object().unwrap() {
        assert_eq!(
            conversion::sha256_file(&en.directory.join(name)).unwrap(),
            hash.as_str().unwrap()
        );
    }
    let mut workbook = ZipArchive::new(fs::File::open(en.workbook).unwrap()).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut workbook.by_name("xl/workbook.xml").unwrap(), &mut xml)
        .unwrap();
    for sheet in [
        "Summary",
        "Floppies",
        "Recovered Files",
        "Conversions",
        "Conversion Issues",
        "Integrity",
        "Delivery Files",
        "Scope",
    ] {
        assert!(xml.contains(sheet));
    }
    let empty_issues = fs::read_to_string(en.directory.join("ConversionIssues.csv")).unwrap();
    assert!(empty_issues.contains("OriginalForensicPath"));
}

#[test]
fn hungarian_presentation_has_localized_headers_and_scope_without_changing_machine_keys() {
    let f = Fixture::files(false);
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let hu = export(&f.0, &audit, Language::Hungarian).unwrap();
    let mut workbook = ZipArchive::new(fs::File::open(&hu.workbook).unwrap()).unwrap();
    let mut strings = String::new();
    std::io::Read::read_to_string(
        &mut workbook.by_name("xl/sharedStrings.xml").unwrap(),
        &mut strings,
    )
    .unwrap();
    assert!(
        strings.contains("Lemezkép mérete (bájt)") && strings.contains("Visszanyert fájl útvonala")
    );
    assert!(strings.contains("Az OK állapot") && !strings.contains("OK means recorded image"));
    assert!(
        fs::read_to_string(hu.directory.join("FloppyFinalAudit.txt"))
            .unwrap()
            .contains("Az OK állapot")
    );
    assert!(
        fs::read_to_string(hu.directory.join("FloppyFinalAudit.csv"))
            .unwrap()
            .contains("AuditStatus")
    );
    let before = fs::read(&hu.workbook).unwrap();
    let en = export(&f.0, &audit, Language::English).unwrap();
    assert!(
        fs::read_to_string(en.directory.join("FloppyFinalAudit.txt"))
            .unwrap()
            .contains("OK means recorded image")
    );
    assert_eq!(before, fs::read(&hu.workbook).unwrap());
}

#[test]
fn package_language_must_match_the_selected_report_without_rewriting_it() {
    let f = Fixture::files(false);
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let hu = export(&f.0, &audit, Language::Hungarian).unwrap();
    let before = fs::read(&hu.workbook).unwrap();
    let destination = f.0.root().with_extension("language-package");
    fs::create_dir(&destination).unwrap();
    let error = crate::package::build_package_in_language(
        &crate::package::PackageRequest {
            project_root: f.0.root().to_owned(),
            destination: destination.clone(),
            project_name: "Mismatch".into(),
        },
        false,
        Language::English,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.contains("report export --language en"));
    assert_eq!(before, fs::read(&hu.workbook).unwrap());
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn localized_report_explains_raw_diagnostics_without_translating_source_or_machine_values() {
    let f = Fixture::files(false);
    let mut document = f.report();
    let raw =
        "Selected extraction cannot be verified: Projekt/Összesítő/READABLE SEGMENT <unknown>.doc";
    document.floppies[0].issues = vec![raw.into()];
    document.issues = vec![raw.into()];
    let original = serde_json::to_value(&document).unwrap();
    for language in [Language::Hungarian, Language::English] {
        let report = writer::publish(&f.0, &document, language).unwrap();
        let raw_json: Value = serde_json::from_slice(&fs::read(&report.json).unwrap()).unwrap();
        assert_eq!(raw_json, original);
        let text = fs::read_to_string(report.directory.join("FloppyFinalAudit.txt")).unwrap();
        assert!(text.contains(raw));
        assert!(text.contains(language.text(
            "MŰSZAKI BIZONYÍTÉK (EREDETI NYELV)",
            "TECHNICAL EVIDENCE (ORIGINAL LANGUAGE)"
        )));
        let mut workbook = ZipArchive::new(fs::File::open(&report.workbook).unwrap()).unwrap();
        let mut strings = String::new();
        std::io::Read::read_to_string(
            &mut workbook.by_name("xl/sharedStrings.xml").unwrap(),
            &mut strings,
        )
        .unwrap();
        assert!(
            strings
                .contains(language.text("Az alábbi műszaki részletek", "Technical details below"))
        );
        assert!(strings.contains("Projekt/Összesítő/READABLE SEGMENT &lt;unknown&gt;.doc"));
        let csv = fs::read_to_string(report.directory.join("FloppyFinalAudit.csv")).unwrap();
        assert!(csv.contains("AuditStatus") && csv.contains(raw));
    }
}

#[test]
fn package_rejects_new_acquisition_language_mismatch_even_with_a_matching_final_report() {
    let f = Fixture::files(false);
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let en = export(&f.0, &audit, Language::English).unwrap();
    let statistics = crate::imaging::load_project_statistics(&f.0.images_dir()).unwrap();
    let hu = crate::report::export_hungarian_report(
        f.0.name(),
        &f.0.reports_dir(),
        &f.0.images_dir(),
        &statistics,
    )
    .unwrap();
    let before = fs::read(&hu).unwrap();
    let final_before = fs::read(&en.workbook).unwrap();
    let destination = f.0.root().with_extension("acquisition-mismatch-package");
    fs::create_dir(&destination).unwrap();
    let request = crate::package::PackageRequest {
        project_root: f.0.root().to_owned(),
        destination: destination.clone(),
        project_name: "Acquisition mismatch".into(),
    };
    let error =
        crate::package::build_package_in_language(&request, false, Language::English, &|_| {})
            .unwrap_err();
    assert!(
        error.contains("Acquisition report language is hu")
            && error.contains("report export --language en")
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    assert_eq!(before, fs::read(&hu).unwrap());
    assert_eq!(final_before, fs::read(&en.workbook).unwrap());
    let new_en = crate::report::export_acquisition_report(
        f.0.name(),
        &f.0.reports_dir(),
        &f.0.images_dir(),
        &statistics,
        Language::English,
    )
    .unwrap();
    let package =
        crate::package::build_package_in_language(&request, false, Language::English, &|_| {})
            .unwrap();
    let mut zip = ZipArchive::new(fs::File::open(package.zip_path).unwrap()).unwrap();
    assert!(
        zip.by_name(&format!(
            "Reports/{}",
            new_en.file_name().unwrap().to_string_lossy()
        ))
        .is_ok()
    );
    assert!(
        zip.by_name(&format!(
            "Reports/{}",
            hu.file_name().unwrap().to_string_lossy()
        ))
        .is_err()
    );
    assert_eq!(before, fs::read(hu).unwrap());
}

#[test]
fn cancelled_export_cannot_replace_the_last_complete_generation() {
    let f = Fixture::files(false);
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let good = export(&f.0, &audit, Language::English).unwrap();
    let prior = fs::read(&good.latest).unwrap();
    let token = crate::cancellation::Token::default();
    token.request();
    let _scope = crate::cancellation::enter(token);
    assert!(crate::cancellation::stopped(
        &export(&f.0, &audit, Language::English).unwrap_err()
    ));
    assert_eq!(fs::read(&good.latest).unwrap(), prior);
}

#[test]
fn csv_escapes_formula_like_customer_names_and_preserves_quotes_and_newlines() {
    let text = csv(
        &["Path".into()],
        &[
            vec!["=HYPERLINK(\"bad\")".into()],
            vec!["normal,quoted\nname".into()],
            vec![" @SUM(1)".into()],
        ],
    );
    let rows = audit::parse_csv(&text).unwrap();
    assert_eq!(rows[1][0], "'=HYPERLINK(\"bad\")");
    assert_eq!(rows[2][0], "normal,quoted\nname");
    assert_eq!(rows[3][0], "' @SUM(1)");
}

#[test]
fn status_precedence_does_not_hide_no_files_or_conversion_failures() {
    let mut d = Disk {
        source_recovered_files: 1,
        imaging_status: "OK".into(),
        evidence_status: "IMAGE_FILES_CONVERSIONS_VERIFIED".into(),
        ..Disk::default()
    };
    assert_eq!(classify(&d), "OK");
    d.imaging_status = "PARTIAL".into();
    assert_eq!(classify(&d), "PARTIAL: IMAGE READ");
    d.conversion_partial = 1;
    assert_eq!(classify(&d), "PARTIAL: CONVERSION");
    d.conversion_failed = 1;
    assert_eq!(classify(&d), "CHECK: CONVERSION FAILED");
    d.conversion_failed = 0;
    d.source_recovered_files = 0;
    assert_eq!(classify(&d), "CHECK: NO RECOVERED FILES");
}

#[test]
fn zero_missing_read_fs_uncertainty_and_derived_bytes_have_distinct_attention() {
    let mut d = Disk {
        source_recovered_files: 1,
        imaging_status: "OK".into(),
        bad_sectors: Some(0),
        evidence_status: "PARTIAL_IMAGE_READ".into(),
        ..Disk::default()
    };
    assert_eq!(classify(&d), "CHECK: FILESYSTEM COMPLETENESS");
    d.evidence_status = "DERIVED_RECOVERY".into();
    assert_eq!(classify(&d), "CHECK: DERIVED RECOVERY");
    d.bad_sectors = Some(1);
    assert_eq!(classify(&d), "PARTIAL: IMAGE READ");
    d.source_recovered_files = 0;
    d.bad_sectors = Some(0);
    d.native_filesystem_status = "NO REACHABLE LIVE FAT FILES; DELETED ENTRIES PRESENT".into();
    assert_eq!(classify(&d), "CHECK: NO REACHABLE LIVE FILES");
    d.native_filesystem_status = "UNSUPPORTED/UNKNOWN FILESYSTEM".into();
    assert_eq!(classify(&d), "CHECK: UNSUPPORTED FILESYSTEM");
    d.conversion_failed = 1;
    assert_eq!(classify(&d), "CHECK: CONVERSION FAILED");
}

#[test]
fn bound_native_description_exposes_deleted_markers_and_unknown_layout_without_certification() {
    let mut image = vec![0; 1474560];
    image[11..13].copy_from_slice(&512_u16.to_le_bytes());
    image[13] = 1;
    image[14..16].copy_from_slice(&1_u16.to_le_bytes());
    image[16] = 2;
    image[17..19].copy_from_slice(&224_u16.to_le_bytes());
    image[19..21].copy_from_slice(&2880_u16.to_le_bytes());
    image[21] = 0xf0;
    image[22..24].copy_from_slice(&9_u16.to_le_bytes());
    image[510..512].copy_from_slice(&[0x55, 0xaa]);
    for start in [512, 5120] {
        image[start..start + 3].copy_from_slice(&[0xf0, 0xff, 0xff]);
    }
    image[9728] = 0xe5;
    image[9729..9739].copy_from_slice(b"EST    TXT");
    let mut proof = crate::fat12_recovery::RecoveryReport {
        schema_version: 1,
        native_engine_version: 5,
        method: "native_fat12_and_validated_carving".into(),
        source_image: "fixture.img".into(),
        source_sha256: "fixture".into(),
        disk: 1,
        attempt: 1,
        analysis: Some(crate::fat12::analyze(&image, &[]).unwrap()),
        carving: None,
        filesystem_error: None,
        deleted: None,
        warning: "scope remains uncertain".into(),
    };
    let mut disk = Disk::default();
    describe_native(&mut disk, &proof);
    assert_eq!(disk.native_deleted_entries, 1);
    assert!(disk.issues[0].contains("not a count of recoverable deleted files"));
    assert_eq!(classify(&disk), "CHECK: NO REACHABLE LIVE FILES");
    proof.analysis = None;
    proof.filesystem_error = Some("unsupported boot layout".into());
    let mut unknown = Disk::default();
    describe_native(&mut unknown, &proof);
    assert_eq!(classify(&unknown), "CHECK: UNSUPPORTED FILESYSTEM");
    assert!(unknown.issues[0].contains("unsupported boot layout"));
}

#[test]
fn delivery_mapping_refuses_duplicate_and_escaping_paths() {
    let f = Fixture::empty();
    let path = f.0.reports_dir().join("DeliveryPathMap.csv");
    for rows in [
        vec!["001/../escape.txt"],
        vec!["001/test.txt", "001/TEST.txt"],
    ] {
        let header = [
            "Floppy",
            "OriginalForensicPath",
            "DeliveryPath",
            "RecoveryMethod",
            "SourceSHA256",
        ]
        .map(str::to_owned);
        let values = rows
            .into_iter()
            .map(|p| {
                vec![
                    "001".into(),
                    "test.txt".into(),
                    p.into(),
                    "fixture".into(),
                    "0".repeat(64),
                ]
            })
            .collect::<Vec<_>>();
        fs::write(&path, csv(&header, &values)).unwrap();
        assert!(delivery_mapping(&f.0.reports_dir()).is_err());
    }
}

#[test]
fn long_unicode_excel_messages_are_display_bounded_not_dropped_from_evidence() {
    let original = "😀".repeat(30_000);
    let display = excel_text(&original, Language::English);
    assert!(display.encode_utf16().count() <= 32_767);
    assert!(display.ends_with("full value in JSON/CSV]"));
    assert_eq!(original.chars().count(), 30_000);
    let hu = excel_text(&original, Language::Hungarian);
    assert!(hu.encode_utf16().count() <= 32_767);
    assert!(hu.ends_with("teljes érték a JSON/CSV-ben]"));
    assert!(hu.starts_with("😀😀"));
}

#[test]
fn excluded_os_metadata_stays_in_inventory_without_a_missing_customer_file_warning() {
    let f = Fixture::files(false);
    let folder = f.0.extracted_dir().join("001/attempt_001");
    fs::create_dir(folder.join("System Volume Information")).unwrap();
    fs::write(
        folder.join("System Volume Information/IndexerVolumeGuid"),
        b"metadata",
    )
    .unwrap();
    let files = extraction::inventory_files(&folder).unwrap();
    let image_hash =
        conversion::sha256_file(&f.0.images_dir().join("001_attempt_001.img")).unwrap();
    fs::write(folder.join(".fluxvault-inventory.json"), serde_json::to_vec(&json!({"schema_version":1,"source_image":"001_attempt_001.img","source_sha256":image_hash,"files":files})).unwrap()).unwrap();
    fs::write(folder.join(".fluxvault-extraction.json"), serde_json::to_vec(&json!({"schema_version":1,"source_image":"001_attempt_001.img","source_sha256":image_hash,"extracted_unix_ms":1,"file_count":files.len(),"total_bytes":files.iter().map(|f| f.bytes).sum::<u64>()})).unwrap()).unwrap();
    let report = f.report();
    assert_eq!(report.floppies[0].audit_status, "OK");
    assert_eq!(report.summary.recovered_source_files, 2);
    assert_eq!(report.summary.excluded_source_metadata_files, 1);
    assert!(report.recovered_files.iter().any(|f| !f.delivery_eligible));
}

#[test]
fn a_foreign_conversion_snapshot_and_missing_results_cannot_claim_no_candidates() {
    let f = Fixture::files(true);
    let path = conversion_run::snapshot_path(&f.0.reports_dir());
    let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["result"]["rows"] = json!([]);
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    let d = f.report();
    assert_eq!(d.summary.floppies_fully_ok, 0);
    assert!(
        d.floppies[0]
            .issues
            .iter()
            .any(|s| s.contains("no current conversion result"))
    );
    state["project_root"] = json!("C:\\different-project");
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    let d = f.report();
    assert_eq!(d.summary.floppies_fully_ok, 0);
    assert!(d.issues.iter().any(|s| s.contains("different project")));
}

#[test]
fn packaging_selects_only_latest_report_and_refuses_changed_generation_bytes() {
    let f = Fixture::files(false);
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let old = export(&f.0, &audit, Language::Hungarian).unwrap();
    let latest = export(&f.0, &audit, Language::English).unwrap();
    let files = package_files(f.0.root()).unwrap();
    assert_eq!(files.len(), 11);
    assert!(!files.iter().any(|(p, _)| p.starts_with(&old.directory)));
    assert!(files.iter().any(|(p, _)| p == &latest.workbook));
    fs::write(
        latest.directory.join("RecoveredFiles.csv"),
        b"changed inventory",
    )
    .unwrap();
    assert!(
        package_files(f.0.root())
            .unwrap_err()
            .contains("hash/confinement mismatch")
    );
}

#[test]
fn report_mutation_during_zip_streaming_preserves_partial_not_a_completed_package() {
    let f = Fixture::files(false);
    let audit = audit::run_audit(&f.0, &|_| {}).unwrap();
    let latest = export(&f.0, &audit, Language::Hungarian).unwrap();
    let destination = f.0.root().with_file_name(format!(
        "{}-delivery",
        f.0.root().file_name().unwrap().to_string_lossy()
    ));
    fs::create_dir(&destination).unwrap();
    let result = crate::package::build_package(
        &crate::package::PackageRequest {
            project_root: f.0.root().to_owned(),
            destination: destination.clone(),
            project_name: "report-race-fixture".into(),
        },
        &|stage| {
            if stage.starts_with("Packaging file") && stage.ends_with("/RecoveredFiles.csv") {
                fs::write(
                    latest.directory.join("RecoveredFiles.csv"),
                    b"mutated during ZIP streaming",
                )
                .unwrap();
            }
        },
    );
    assert!(result.unwrap_err().contains("changed during packaging"));
    let paths = fs::read_dir(&destination)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].to_string_lossy().ends_with(".partial.zip"));
    fs::remove_dir_all(destination).unwrap();
}

#[test]
fn legacy_manual_origin_keeps_conversion_hash_results_separate_from_custody_uncertainty() {
    let f = Fixture::files(true);
    let disk = f.0.extracted_dir().join("001");
    let bytes = fs::read(disk.join("attempt_001/test.rtf")).unwrap();
    fs::remove_dir_all(disk.join("attempt_001")).unwrap();
    fs::write(disk.join("test.rtf"), bytes).unwrap();
    let report = f.report();
    assert_eq!(report.conversions[0].status, "OK");
    assert_eq!(report.summary.invalid_generated_outputs, 0);
    assert_eq!(report.floppies[0].extraction_status, "MANUAL_UNVERIFIED");
    assert_ne!(report.floppies[0].audit_status, "OK");
    assert!(!report.recovered_files[0].managed_inventory_verified);
    assert!(!report.customer_delivery_certified);
}
