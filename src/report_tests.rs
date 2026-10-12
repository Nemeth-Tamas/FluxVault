use super::*;
use crate::{imaging::DiskSummary, project::ProjectState};
use std::io::{Cursor, Read};
use zip::ZipArchive;

fn xml(bytes: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    out
}

fn fixture() -> (ProjectStatistics, Vec<(u32, AttemptSummary)>) {
    let statistics = ProjectStatistics {
        disk_count: 1,
        total_attempts: 1,
        partial_disks: 1,
        latest_bad_sectors: 2,
        best_known_bad_sectors: 2,
        disks: vec![DiskSummary {
            disk_number: 7,
            attempt_count: 1,
            latest_attempt_number: 1,
            latest_status: "PARTIAL: IMAGE READ".into(),
            latest_bad_sectors: 2,
            latest_timestamp_unix_ms: u128::MAX,
            best_attempt_number: 1,
            best_bad_sectors: 2,
            attention_required: true,
            total_sectors: 2880,
        }],
        ..Default::default()
    };
    let attempt = AttemptSummary {
        preferred: false,
        attempt_number: 1,
        status: "PARTIAL: IMAGE READ".into(),
        timestamp_unix_ms: u128::MAX,
        image_file: "=Project Összesítő Image & <name>.img".into(),
        metadata_path: PathBuf::from("C:/evidence/Projekt & <metadata>.json"),
        log_file: String::new(),
        parsed_log: None,
        parsed_dmde_log: None,
        legacy_image: false,
        attention_required: true,
        sha256: "a".repeat(64),
        total_sectors: 2880,
        retry_recovered_sectors: 3,
        bad_sectors: vec![4, 5],
    };
    (statistics, vec![(7, attempt)])
}

#[test]
fn acquisition_workbooks_localize_labels_and_chart_references_not_source_data() {
    let (statistics, attempts) = fixture();
    for language in [Language::Hungarian, Language::English] {
        let bytes = build_workbook(
            "=Project Összesítő & <customer>",
            &statistics,
            &attempts,
            language,
        )
        .unwrap();
        let strings = xml(&bytes, "xl/sharedStrings.xml");
        assert!(strings.contains("=Project Összesítő &amp; &lt;customer&gt;"));
        assert!(strings.contains("=Project Összesítő Image &amp; &lt;name&gt;.img"));
        assert!(strings.contains("C:/evidence/Projekt &amp; &lt;metadata&gt;.json"));
        assert!(strings.contains("PARTIAL: IMAGE READ") && strings.contains(&"a".repeat(64)));
        assert!(strings.contains(language.text("Ismeretlen", "Unknown")));
        assert!(
            strings.contains(
                language.text("Csak beolvasási statisztika", "Acquisition statistics only")
            )
        );
        assert!(
            strings.contains(language.text("Újrapróbálás után mentett", "Recovered after retries"))
        );
        let book = xml(&bytes, "xl/workbook.xml");
        for name in [
            language.text("Összesítő", "Summary"),
            language.text("Lemezek", "Disks"),
            language.text("Próbálkozások", "Attempts"),
        ] {
            assert!(book.contains(&format!("name=\"{name}\"")));
        }
        let chart = xml(&bytes, "xl/charts/chart1.xml");
        assert!(chart.contains(language.text("Összesítő", "Summary")));
        assert!(chart.contains(language.text("Beolvasások állapota", "Acquisition status")));
        assert!(chart.contains("$H$3:$H$4") && chart.contains("$I$3:$I$4"));
        for sheet in [
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "xl/worksheets/sheet3.xml",
        ] {
            assert!(
                !xml(&bytes, sheet).contains("<f>"),
                "Source text must never become a formula"
            );
        }
        let attempts_xml = xml(&bytes, "xl/worksheets/sheet3.xml");
        let retry_cell = attempts_xml
            .split("r=\"E2\"")
            .nth(1)
            .unwrap()
            .split("</c>")
            .next()
            .unwrap();
        assert!(retry_cell.contains("<v>3</v>"));
        assert!(attempts_xml.contains("ht=\"36\""));
    }
}

#[test]
fn timestamp_extremes_remain_unknown_and_ordinary_times_are_identical() {
    assert_eq!(
        format_timestamp(u128::MAX, Language::Hungarian),
        "Ismeretlen"
    );
    assert_eq!(format_timestamp(u128::MAX, Language::English), "Unknown");
    assert_eq!(
        format_timestamp(1, Language::Hungarian),
        format_timestamp(1, Language::English)
    );
}

fn project() -> ProjectState {
    let root = std::env::temp_dir().join(format!(
        "fv-acquisition-language-{}-{}-{}",
        std::process::id(),
        crate::external_tools::current_unix_ms(),
        REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    ProjectState::create_without_session(root).unwrap()
}

#[test]
fn rapid_exports_keep_both_languages_and_the_legacy_hungarian_api() {
    let p = project();
    let statistics = ProjectStatistics::default();
    let hu =
        export_hungarian_report(p.name(), &p.reports_dir(), &p.images_dir(), &statistics).unwrap();
    let before = fs::read(&hu).unwrap();
    let en = export_acquisition_report(
        p.name(),
        &p.reports_dir(),
        &p.images_dir(),
        &statistics,
        Language::English,
    )
    .unwrap();
    let hu_again =
        export_hungarian_report(p.name(), &p.reports_dir(), &p.images_dir(), &statistics).unwrap();
    assert_ne!(hu, en);
    assert_ne!(hu, hu_again);
    assert_eq!(before, fs::read(hu).unwrap());
    assert!(xml(&fs::read(en).unwrap(), "xl/workbook.xml").contains("name=\"Summary\""));
}

#[test]
fn cli_export_uses_saved_choice_for_both_workbooks_and_keeps_old_snapshots() {
    let p = project();
    let run = |language: Option<&str>| {
        let mut args = vec!["report".into(), "export".into(), "--json".into()];
        if let Some(language) = language {
            args.extend(["--language".into(), language.into()]);
        }
        let response = crate::cli::run(&args, p.root()).unwrap();
        assert_eq!(response.exit_code, 0);
        serde_json::from_str::<serde_json::Value>(&response.output).unwrap()
    };
    let hu = run(None);
    let hu_path = PathBuf::from(hu["acquisition_workbook"].as_str().unwrap());
    let hu_before = fs::read(&hu_path).unwrap();
    let hu_final = fs::read(hu["workbook"].as_str().unwrap()).unwrap();
    let en = run(Some("en"));
    for field in ["acquisition_workbook", "workbook"] {
        let bytes = fs::read(en[field].as_str().unwrap()).unwrap();
        assert!(xml(&bytes, "xl/workbook.xml").contains("name=\"Summary\""));
    }
    let remembered = run(None);
    assert!(
        xml(
            &fs::read(remembered["acquisition_workbook"].as_str().unwrap()).unwrap(),
            "xl/workbook.xml"
        )
        .contains("name=\"Summary\"")
    );
    assert_eq!(hu_before, fs::read(&hu_path).unwrap());
    assert_eq!(
        hu_final,
        fs::read(hu["workbook"].as_str().unwrap()).unwrap()
    );
}
