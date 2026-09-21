use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Local, Utc};
use rust_xlsxwriter::{
    Chart, ChartType, Color, Format, FormatAlign, FormatBorder, Workbook, XlsxError,
};

use crate::imaging::{self, AttemptSummary, ProjectStatistics};

pub fn export_hungarian_report(
    project_name: &str,
    reports_directory: &Path,
    acquisition_directory: &Path,
    statistics: &ProjectStatistics,
) -> Result<PathBuf, String> {
    fs::create_dir_all(reports_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a Reports mappát {}: {error}",
            reports_directory.display()
        )
    })?;

    let file_name = format!(
        "FluxVault_Jelentes_{}.xlsx",
        Local::now().format("%Y%m%d_%H%M%S")
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

    build_hungarian_workbook(project_name, statistics, &attempts, &output_path).map_err(
        |error| {
            format!(
                "Excel jelentés készítési hiba {}: {error}",
                output_path.display()
            )
        },
    )?;

    Ok(output_path)
}

fn build_hungarian_workbook(
    project_name: &str,
    statistics: &ProjectStatistics,
    attempts: &[(u32, AttemptSummary)],
    output_path: &Path,
) -> Result<(), XlsxError> {
    let mut workbook = Workbook::new();

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
        let worksheet = workbook.add_worksheet().set_name("Összesítő")?;

        worksheet.set_row_height(0, 30)?;

        worksheet.merge_range(0, 0, 0, 5, "FluxVault archiválási jelentés", &title_format)?;

        worksheet.write_string_with_format(2, 0, "Projekt", &label_format)?;

        worksheet.write_string_with_format(2, 1, project_name, &value_format)?;

        worksheet.write_string_with_format(3, 0, "Jelentés készítve", &label_format)?;

        worksheet.write_string_with_format(
            3,
            1,
            &Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            &value_format,
        )?;

        worksheet.merge_range(5, 0, 5, 1, "Projekt összesítő", &section_format)?;

        let summary_rows = [
            ("Feldolgozott lemezek", statistics.disk_count as f64),
            (
                "Összes olvasási próbálkozás",
                statistics.total_attempts as f64,
            ),
            ("Hibamentes lemezek", statistics.ok_disks as f64),
            ("Részleges lemezek", statistics.partial_disks as f64),
            (
                "Legutóbbi próbálkozások hibás szektorai",
                statistics.latest_bad_sectors as f64,
            ),
            (
                "Legjobb ismert állapot hibás szektorai",
                statistics.best_known_bad_sectors as f64,
            ),
        ];

        for (index, (label, value)) in summary_rows.iter().enumerate() {
            let row = 6 + index as u32;

            worksheet.write_string_with_format(row, 0, *label, &label_format)?;

            worksheet.write_number_with_format(row, 1, *value, &center_format)?;
        }

        worksheet.write_string_with_format(1, 7, "Állapot", &header_format)?;

        worksheet.write_string_with_format(1, 8, "Lemezek", &header_format)?;

        worksheet.write_string(2, 7, "Hibamentes")?;
        worksheet.write_number(2, 8, statistics.ok_disks as f64)?;

        worksheet.write_string(3, 7, "Részleges")?;
        worksheet.write_number(3, 8, statistics.partial_disks as f64)?;

        let mut chart = Chart::new(ChartType::Doughnut);

        chart
            .add_series()
            .set_categories(("Összesítő", 2, 7, 3, 7))
            .set_values(("Összesítő", 2, 8, 3, 8));

        chart.title().set_name("Lemezek állapota");

        worksheet.insert_chart(5, 3, &chart)?;

        worksheet.set_column_width(0, 42)?;
        worksheet.set_column_width(1, 18)?;
        worksheet.set_column_width(2, 3)?;
        worksheet.set_column_width(3, 16)?;
        worksheet.set_column_width(4, 16)?;
        worksheet.set_column_width(5, 16)?;
        worksheet.set_column_width(7, 18)?;
        worksheet.set_column_width(8, 12)?;
    }

    {
        let worksheet = workbook.add_worksheet().set_name("Lemezek")?;

        let headers = [
            "Lemez",
            "Legjobb állapot",
            "Próbálkozások",
            "Legutóbbi #",
            "Legutóbbi státusz",
            "Legutóbbi hibás szektor",
            "Legjobb #",
            "Legjobb hibás szektor",
            "Összes szektor",
            "Utolsó olvasás",
        ];

        for (column, header) in headers.iter().enumerate() {
            worksheet.write_string_with_format(0, column as u16, *header, &header_format)?;
        }

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
                &format_timestamp(disk.latest_timestamp_unix_ms),
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
        let worksheet = workbook.add_worksheet().set_name("Próbálkozások")?;

        let headers = [
            "Lemez",
            "Próbálkozás",
            "Státusz",
            "Hibás szektorok",
            "Retry után mentett",
            "Összes szektor",
            "Időpont",
            "SHA-256",
            "Lemezkép",
            "Metadata",
        ];

        for (column, header) in headers.iter().enumerate() {
            worksheet.write_string_with_format(0, column as u16, *header, &header_format)?;
        }

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
                &format_timestamp(attempt.timestamp_unix_ms),
                &value_format,
            )?;

            worksheet.write_string_with_format(row, 7, &attempt.sha256, &value_format)?;

            worksheet.write_string_with_format(row, 8, &attempt.image_file, &value_format)?;

            worksheet.write_string_with_format(
                row,
                9,
                &attempt.metadata_path.display().to_string(),
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

    workbook.save(output_path)?;

    Ok(())
}

fn format_timestamp(timestamp_unix_ms: u128) -> String {
    let Ok(timestamp_unix_ms) = i64::try_from(timestamp_unix_ms) else {
        return "Ismeretlen".to_owned();
    };

    let Some(timestamp_utc) = DateTime::<Utc>::from_timestamp_millis(timestamp_unix_ms) else {
        return "Ismeretlen".to_owned();
    };

    timestamp_utc
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
