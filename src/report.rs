use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use chrono::{DateTime, Local, Utc};
use rust_xlsxwriter::{
    Chart, ChartType, Color, Format, FormatAlign, FormatBorder, Workbook, XlsxError,
};

use crate::final_report::Language;
use crate::imaging::{self, AttemptSummary, ProjectStatistics};

static REPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn export_hungarian_report(
    project_name: &str,
    reports_directory: &Path,
    acquisition_directory: &Path,
    statistics: &ProjectStatistics,
) -> Result<PathBuf, String> {
    export_acquisition_report(
        project_name,
        reports_directory,
        acquisition_directory,
        statistics,
        Language::Hungarian,
    )
}

/// Customer presentation only; identifiers, paths, hashes and recorded statuses
/// remain literal. The caller owns the project's report snapshot.
pub fn export_acquisition_report(
    project_name: &str,
    reports_directory: &Path,
    acquisition_directory: &Path,
    statistics: &ProjectStatistics,
    language: Language,
) -> Result<PathBuf, String> {
    fs::create_dir_all(reports_directory).map_err(|error| {
        format!(
            "Cannot create Reports directory {}: {error}",
            reports_directory.display()
        )
    })?;

    let file_name = format!(
        "FluxVault_Jelentes_{}_{:010}-{:020}_{}.xlsx",
        Local::now().format("%Y%m%d_%H%M%S_%3f"),
        std::process::id(),
        REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        language.code(),
    );

    let output_path = reports_directory.join(file_name);

    let mut attempts = Vec::<(u32, AttemptSummary)>::new();

    for disk in &statistics.disks {
        let disk_attempts =
            imaging::load_attempts_for_disk(acquisition_directory, disk.disk_number)?;

        for attempt in disk_attempts {
            attempts.push((disk.disk_number, attempt));
        }
    }

    crate::cancellation::check()?;
    let bytes = build_workbook(project_name, statistics, &attempts, language)
        .map_err(|error| format!("Acquisition workbook generation failed: {error}"))?;
    crate::cancellation::check()?;
    // Do not let a timestamp collision overwrite any previous edition.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .map_err(|e| {
            format!(
                "Cannot create acquisition workbook {}: {e}",
                output_path.display()
            )
        })?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| {
            format!(
                "Incomplete acquisition workbook retained at {}: {e}",
                output_path.display()
            )
        })?;

    Ok(output_path)
}

fn build_workbook(
    project_name: &str,
    statistics: &ProjectStatistics,
    attempts: &[(u32, AttemptSummary)],
    language: Language,
) -> Result<Vec<u8>, XlsxError> {
    let mut workbook = Workbook::new();
    let summary_name = language.text("Összesítő", "Summary");

    let title_format = Format::new()
        .set_bold()
        .set_font_size(18)
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x17365D))
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter);

    let section_format = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x1F4E78))
        .set_border(FormatBorder::Thin);

    let header_format = Format::new()
        .set_bold()
        .set_text_wrap()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x4472C4))
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter)
        .set_border(FormatBorder::Thin);

    let label_format = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xD9EAF7))
        .set_border(FormatBorder::Thin);

    let value_format = Format::new().set_border(FormatBorder::Thin);

    let center_format = Format::new()
        .set_align(FormatAlign::Center)
        .set_border(FormatBorder::Thin);

    let ok_format = Format::new()
        .set_bold()
        .set_font_color(Color::RGB(0x006100))
        .set_background_color(Color::RGB(0xC6EFCE))
        .set_align(FormatAlign::Center)
        .set_border(FormatBorder::Thin);

    let partial_format = Format::new()
        .set_bold()
        .set_font_color(Color::RGB(0x9C6500))
        .set_background_color(Color::RGB(0xFFEB9C))
        .set_align(FormatAlign::Center)
        .set_border(FormatBorder::Thin);

    {
        let worksheet = workbook.add_worksheet().set_name(summary_name)?;

        worksheet.set_row_height(0, 30)?;

        worksheet.merge_range(
            0,
            0,
            0,
            5,
            language.text(
                "FluxVault beolvasási jelentés",
                "FluxVault acquisition report",
            ),
            &title_format,
        )?;

        worksheet.write_string_with_format(
            2,
            0,
            language.text("Projekt", "Project"),
            &label_format,
        )?;

        worksheet.write_string_with_format(2, 1, project_name, &value_format)?;

        worksheet.write_string_with_format(
            3,
            0,
            language.text("Jelentés készítve", "Report generated"),
            &label_format,
        )?;

        worksheet.write_string_with_format(
            3,
            1,
            Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            &value_format,
        )?;

        worksheet.merge_range(
            5,
            0,
            5,
            1,
            language.text("Projekt összesítő", "Project summary"),
            &section_format,
        )?;

        let summary_rows = [
            (
                language.text("Beolvasott lemezek", "Acquired disks"),
                statistics.disk_count as f64,
            ),
            (
                language.text("Összes olvasási próbálkozás", "Total read attempts"),
                statistics.total_attempts as f64,
            ),
            (
                language.text(
                    "Rögzített hibás szektor nélkül",
                    "Without recorded bad sectors",
                ),
                statistics.ok_disks as f64,
            ),
            (
                language.text("Részleges lemezek", "Partial disks"),
                statistics.partial_disks as f64,
            ),
            (
                language.text(
                    "Legutóbbi próbálkozások hibás szektorai",
                    "Bad sectors in latest attempts",
                ),
                statistics.latest_bad_sectors as f64,
            ),
            (
                language.text(
                    "Legjobb ismert állapot hibás szektorai",
                    "Bad sectors in best known attempts",
                ),
                statistics.best_known_bad_sectors as f64,
            ),
        ];

        for (index, (label, value)) in summary_rows.iter().enumerate() {
            let row = 6 + index as u32;

            worksheet.write_string_with_format(row, 0, *label, &label_format)?;

            worksheet.write_number_with_format(row, 1, *value, &center_format)?;
        }

        worksheet.write_string_with_format(
            1,
            7,
            language.text("Állapot", "Status"),
            &header_format,
        )?;

        worksheet.write_string_with_format(
            1,
            8,
            language.text("Lemezek", "Disks"),
            &header_format,
        )?;

        worksheet.write_string(
            2,
            7,
            language.text("Rögzített hiba nélkül", "No recorded bad sectors"),
        )?;
        worksheet.write_number(2, 8, statistics.ok_disks as f64)?;

        worksheet.write_string(3, 7, language.text("Részleges", "Partial"))?;
        worksheet.write_number(3, 8, statistics.partial_disks as f64)?;

        let mut chart = Chart::new(ChartType::Doughnut);

        chart
            .add_series()
            .set_categories((summary_name, 2, 7, 3, 7))
            .set_values((summary_name, 2, 8, 3, 8));

        chart
            .title()
            .set_name(language.text("Beolvasások állapota", "Acquisition status"));

        worksheet.insert_chart(5, 3, &chart)?;

        worksheet.merge_range(14, 0, 14, 2, language.text(
            "Csak beolvasási statisztika: a rögzített próbálkozásokból. Az OK vagy nulla hibás szektor nem igazolja az eredeti lemez, fájlrendszer vagy dokumentumok teljességét, illetve a fájlkinyerés/konverzió sikerét. A végső integritás- és átadási eredmények a külön FloppyFinalReport.xlsx jelentésben találhatók; az sem teljességi tanúsítvány.",
            "Acquisition statistics only, from recorded attempts. OK or zero bad sectors does not certify completeness of the original disk, filesystem or documents, or successful extraction/conversion. Final integrity and delivery results are in the separate FloppyFinalReport.xlsx; that report is not a completeness certificate either."
        ), &Format::new().set_text_wrap().set_align(FormatAlign::Top))?;
        worksheet.set_row_height(14, 115)?;

        worksheet.set_column_width(0, 42)?;
        worksheet.set_column_width(1, 18)?;
        worksheet.set_column_width(2, 3)?;
        worksheet.set_column_width(3, 16)?;
        worksheet.set_column_width(4, 16)?;
        worksheet.set_column_width(5, 16)?;
        worksheet.set_column_width(7, 26)?;
        worksheet.set_column_width(8, 12)?;
    }

    {
        let worksheet = workbook
            .add_worksheet()
            .set_name(language.text("Lemezek", "Disks"))?;

        let headers = [
            language.text("Lemez", "Disk"),
            language.text("Legjobb állapot", "Best recorded status"),
            language.text("Próbálkozások", "Attempts"),
            language.text("Legutóbbi #", "Latest attempt #"),
            language.text("Legutóbbi státusz", "Latest status"),
            language.text("Legutóbbi hibás szektor", "Latest bad sectors"),
            language.text("Legjobb #", "Best attempt #"),
            language.text("Legjobb hibás szektor", "Best bad sectors"),
            language.text("Összes szektor", "Total sectors"),
            language.text("Utolsó olvasás", "Last read"),
        ];

        for (column, header) in headers.iter().enumerate() {
            worksheet.write_string_with_format(0, column as u16, *header, &header_format)?;
        }
        worksheet.set_row_height(0, 36)?;

        for (index, disk) in statistics.disks.iter().enumerate() {
            let row = index as u32 + 1;

            worksheet.write_number_with_format(row, 0, disk.disk_number as f64, &center_format)?;

            if disk.best_bad_sectors == 0 {
                worksheet.write_string_with_format(row, 1, "OK", &ok_format)?;
            } else {
                worksheet.write_string_with_format(row, 1, "PARTIAL", &partial_format)?;
            }

            worksheet.write_number_with_format(
                row,
                2,
                disk.attempt_count as f64,
                &center_format,
            )?;

            worksheet.write_number_with_format(
                row,
                3,
                disk.latest_attempt_number as f64,
                &center_format,
            )?;

            worksheet.write_string_with_format(row, 4, &disk.latest_status, &center_format)?;

            worksheet.write_number_with_format(
                row,
                5,
                disk.latest_bad_sectors as f64,
                &center_format,
            )?;

            worksheet.write_number_with_format(
                row,
                6,
                disk.best_attempt_number as f64,
                &center_format,
            )?;

            worksheet.write_number_with_format(
                row,
                7,
                disk.best_bad_sectors as f64,
                &center_format,
            )?;

            worksheet.write_number_with_format(
                row,
                8,
                disk.total_sectors as f64,
                &center_format,
            )?;

            worksheet.write_string_with_format(
                row,
                9,
                format_timestamp(disk.latest_timestamp_unix_ms, language),
                &value_format,
            )?;
        }

        worksheet.set_freeze_panes(1, 0)?;

        if !statistics.disks.is_empty() {
            worksheet.autofilter(0, 0, statistics.disks.len() as u32, 9)?;
        }

        worksheet.set_column_width(0, 10)?;
        worksheet.set_column_width(1, 18)?;
        worksheet.set_column_width(2, 16)?;
        worksheet.set_column_width(3, 14)?;
        worksheet.set_column_width(4, 20)?;
        worksheet.set_column_width(5, 24)?;
        worksheet.set_column_width(6, 12)?;
        worksheet.set_column_width(7, 22)?;
        worksheet.set_column_width(8, 16)?;
        worksheet.set_column_width(9, 22)?;
    }

    {
        let worksheet = workbook
            .add_worksheet()
            .set_name(language.text("Próbálkozások", "Attempts"))?;

        let headers = [
            language.text("Lemez", "Disk"),
            language.text("Próbálkozás", "Attempt"),
            language.text("Státusz", "Status"),
            language.text("Hibás szektorok", "Bad sectors"),
            language.text("Újrapróbálás után mentett", "Recovered after retries"),
            language.text("Összes szektor", "Total sectors"),
            language.text("Időpont", "Timestamp"),
            "SHA-256",
            language.text("Lemezkép", "Image"),
            language.text("Metaadat", "Metadata"),
        ];

        for (column, header) in headers.iter().enumerate() {
            worksheet.write_string_with_format(0, column as u16, *header, &header_format)?;
        }
        worksheet.set_row_height(0, 36)?;

        for (index, (disk_number, attempt)) in attempts.iter().enumerate() {
            let row = index as u32 + 1;

            worksheet.write_number_with_format(row, 0, *disk_number as f64, &center_format)?;

            worksheet.write_number_with_format(
                row,
                1,
                attempt.attempt_number as f64,
                &center_format,
            )?;

            if attempt.bad_sectors.is_empty() {
                worksheet.write_string_with_format(row, 2, &attempt.status, &ok_format)?;
            } else {
                worksheet.write_string_with_format(row, 2, &attempt.status, &partial_format)?;
            }

            worksheet.write_number_with_format(
                row,
                3,
                attempt.bad_sectors.len() as f64,
                &center_format,
            )?;

            worksheet.write_number_with_format(
                row,
                4,
                attempt.retry_recovered_sectors as f64,
                &center_format,
            )?;

            worksheet.write_number_with_format(
                row,
                5,
                attempt.total_sectors as f64,
                &center_format,
            )?;

            worksheet.write_string_with_format(
                row,
                6,
                format_timestamp(attempt.timestamp_unix_ms, language),
                &value_format,
            )?;

            worksheet.write_string_with_format(row, 7, &attempt.sha256, &value_format)?;

            worksheet.write_string_with_format(row, 8, &attempt.image_file, &value_format)?;

            worksheet.write_string_with_format(
                row,
                9,
                attempt.metadata_path.display().to_string(),
                &value_format,
            )?;
        }

        worksheet.set_freeze_panes(1, 0)?;

        if !attempts.is_empty() {
            worksheet.autofilter(0, 0, attempts.len() as u32, 9)?;
        }

        worksheet.set_column_width(0, 10)?;
        worksheet.set_column_width(1, 14)?;
        worksheet.set_column_width(2, 14)?;
        worksheet.set_column_width(3, 18)?;
        worksheet.set_column_width(4, 20)?;
        worksheet.set_column_width(5, 16)?;
        worksheet.set_column_width(6, 22)?;
        worksheet.set_column_width(7, 68)?;
        worksheet.set_column_width(8, 54)?;
        worksheet.set_column_width(9, 54)?;
    }

    workbook.save_to_buffer()
}

fn format_timestamp(timestamp_unix_ms: u128, language: Language) -> String {
    let Ok(timestamp_unix_ms) = i64::try_from(timestamp_unix_ms) else {
        return language.text("Ismeretlen", "Unknown").to_owned();
    };

    let Some(timestamp_utc) = DateTime::<Utc>::from_timestamp_millis(timestamp_unix_ms) else {
        return language.text("Ismeretlen", "Unknown").to_owned();
    };

    timestamp_utc
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
