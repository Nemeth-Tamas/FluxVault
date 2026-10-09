use super::*;
use rust_xlsxwriter::{Chart, ChartType, Color, Format, Workbook};

/// Select and verify only the complete latest report, not its stale/partial history.
pub(super) fn package_files(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Latest {
        schema_version: u32,
        generation: String,
        directory: String,
        language: String,
        artifacts_sha256: BTreeMap<String, String>,
        customer_delivery_certified: bool,
    }
    fn regular(path: &Path) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() {
            return Err("Linked/nonregular final-report artifact".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Final report artifact is a reparse point".into());
            }
        }
        Ok(())
    }
    let reports = root.join("Reports");
    let pointer = reports.join("FinalReportLatest.json");
    if !pointer.try_exists().map_err(|e| e.to_string())? {
        return Ok(Vec::new());
    }
    regular(&pointer)?;
    if fs::metadata(&pointer).map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return Err("Oversized final-report pointer".into());
    }
    let bytes = fs::read(&pointer).map_err(|e| e.to_string())?;
    let latest: Latest =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid final report pointer: {e}"))?;
    const FILES: &[&str] = &[
        "FloppyFinalReport.xlsx",
        "FloppyFinalReport.json",
        "FloppyFinalAudit.csv",
        "FloppyFinalAudit.txt",
        "RecoveredFiles.csv",
        "ConversionResults.csv",
        "ConversionIssues.csv",
        "IntegrityValidation.csv",
        "DeliveryManifest.csv",
        "DeliveryManifest.sha256",
    ];
    if latest.schema_version != 1
        || latest.customer_delivery_certified
        || !matches!(latest.language.as_str(), "hu" | "en")
        || latest.generation.is_empty()
        || latest.generation.len() > 100
        || !latest
            .generation
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-')
        || latest.directory != format!("FinalReports/{}", latest.generation)
        || latest.artifacts_sha256.len() != FILES.len()
        || FILES
            .iter()
            .any(|name| !latest.artifacts_sha256.contains_key(*name))
    {
        return Err("Incomplete/invalid latest final-report identity".into());
    }
    let parent = reports.join("FinalReports");
    let directory = reports.join(&latest.directory);
    let report_root = reports.canonicalize().map_err(|e| e.to_string())?;
    let parent_root = parent.canonicalize().map_err(|e| e.to_string())?;
    let generation_root = directory.canonicalize().map_err(|e| e.to_string())?;
    if parent_root.parent() != Some(report_root.as_path())
        || generation_root.parent() != Some(parent_root.as_path())
    {
        return Err("Final report directory escapes Reports".into());
    }
    let mut files = Vec::new();
    for (name, hash) in latest.artifacts_sha256 {
        crate::cancellation::check()?;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid final-report artifact hash".into());
        }
        let path = directory.join(name);
        regular(&path)?;
        if path.canonicalize().map_err(|e| e.to_string())?.parent()
            != Some(generation_root.as_path())
            || conversion::sha256_file(&path)? != hash
        {
            return Err("Final report artifact hash/confinement mismatch; refresh the report before packaging".into());
        }
        files.push((path, hash));
    }
    use sha2::{Digest, Sha256};
    files.push((pointer, format!("{:x}", Sha256::digest(&bytes))));
    Ok(files)
}
use std::{fs::OpenOptions, io::Write};

fn table<T: Serialize + Default>(rows: &[T]) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut headers = BTreeSet::new();
    let mut objects = Vec::new();
    let serde_json::Value::Object(schema) =
        serde_json::to_value(T::default()).map_err(|e| e.to_string())?
    else {
        return Err("Report schema is not an object".into());
    };
    headers.extend(schema.keys().cloned());
    for row in rows {
        let serde_json::Value::Object(value) =
            serde_json::to_value(row).map_err(|e| e.to_string())?
        else {
            return Err("Report row is not an object".into());
        };
        headers.extend(value.keys().cloned());
        objects.push(value);
    }
    let headers = headers.into_iter().collect::<Vec<_>>();
    let values = objects
        .iter()
        .map(|o| {
            headers
                .iter()
                .map(|h| match o.get(h) {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Null) | None => String::new(),
                    Some(v) => v.to_string(),
                })
                .collect()
        })
        .collect();
    Ok((headers, values))
}

pub(super) fn csv(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut csv = "\u{feff}".to_owned();
    for row in std::iter::once(headers.to_vec()).chain(rows.iter().cloned()) {
        csv.push_str(
            &row.iter()
                .map(|v| {
                    // Quoting does not prevent spreadsheet formula injection.
                    let safe = if v.trim_start().starts_with(['=', '+', '-', '@']) {
                        format!("'{v}")
                    } else {
                        v.clone()
                    };
                    format!("\"{}\"", safe.replace('"', "\"\""))
                })
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
    }
    csv
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

pub(super) fn excel_text(value: &str) -> String {
    // XLSX cells have a 32,767 UTF-16-unit limit. Keep complete evidence in JSON/CSV.
    if value.encode_utf16().count() <= 32_767 {
        return value.to_owned();
    }
    let suffix = " [truncated display; full value in JSON/CSV]";
    let budget = 32_767 - suffix.encode_utf16().count();
    let mut used = 0;
    let mut text = String::new();
    for c in value.chars() {
        if used + c.len_utf16() > budget {
            break;
        }
        used += c.len_utf16();
        text.push(c);
    }
    text.push_str(suffix);
    text
}

pub(super) fn publish(
    project: &ProjectState,
    document: &Document,
    language: Language,
) -> Result<Export, String> {
    let parent = project.reports_dir().join("FinalReports");
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    if parent.canonicalize().map_err(|e| e.to_string())?.parent()
        != Some(
            project
                .reports_dir()
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Final report folder escapes Reports".into());
    }
    let generation = format!(
        "{}-{}-{}",
        document.generated_unix_ms,
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let partial = parent.join(format!("{generation}.partial"));
    fs::create_dir(&partial).map_err(|e| e.to_string())?;
    let mut workbook = Workbook::new();
    let header = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x17365D));
    let summary = workbook.add_worksheet();
    let summary_name = language.text("Összesítő", "Summary");
    summary.set_name(summary_name).map_err(|e| e.to_string())?;
    summary
        .merge_range(
            0,
            0,
            0,
            2,
            language.text(
                "FluxVault végső archiválási jelentés",
                "FluxVault final archival report",
            ),
            &header,
        )
        .map_err(|e| e.to_string())?;
    summary
        .write_string(1, 0, &document.project)
        .map_err(|e| e.to_string())?;
    summary
        .write_string(
            2,
            0,
            language.text(
                "Integritásellenőrzés, nem teljességi tanúsítvány",
                "Integrity checks, not a completeness certificate",
            ),
        )
        .map_err(|e| e.to_string())?;
    let metrics = summary_metrics(&document.summary, language);
    for (i, (name, value)) in metrics.iter().enumerate() {
        summary
            .write_string(i as u32 + 4, 0, name)
            .map_err(|e| e.to_string())?;
        summary
            .write_number(i as u32 + 4, 1, *value)
            .map_err(|e| e.to_string())?;
    }
    summary
        .set_column_width(0, 45)
        .and_then(|s| s.set_column_width(1, 18))
        .and_then(|s| s.set_column_width(2, 58))
        .map_err(|e| e.to_string())?;
    let mut chart = Chart::new(ChartType::Column);
    chart
        .add_series()
        .set_categories((summary_name, 12, 0, 14, 0))
        .set_values((summary_name, 12, 1, 14, 1));
    chart
        .title()
        .set_name(language.text("Konverziós eredmények", "Conversion results"));
    summary
        .insert_chart(4, 3, &chart)
        .map_err(|e| e.to_string())?;
    let issues = document
        .conversions
        .iter()
        .filter(|r| r.status != "OK")
        .cloned()
        .collect::<Vec<_>>();
    let tables = [
        (
            "FloppyFinalAudit.csv",
            language.text("Lemezek", "Floppies"),
            table(&document.floppies)?,
        ),
        (
            "RecoveredFiles.csv",
            language.text("Visszanyert fájlok", "Recovered Files"),
            table(&document.recovered_files)?,
        ),
        (
            "ConversionResults.csv",
            language.text("Konverziók", "Conversions"),
            table(&document.conversions)?,
        ),
        (
            "ConversionIssues.csv",
            language.text("Konverziós hibák", "Conversion Issues"),
            table(&issues)?,
        ),
        (
            "IntegrityValidation.csv",
            language.text("Integritás", "Integrity"),
            table(&document.integrity)?,
        ),
        (
            "DeliveryManifest.csv",
            language.text("Átadási fájlok", "Delivery Files"),
            table(&document.delivery_files)?,
        ),
    ];
    let bad = Format::new()
        .set_font_color(Color::RGB(0x9C0006))
        .set_background_color(Color::RGB(0xFFC7CE));
    let warning = Format::new()
        .set_font_color(Color::RGB(0x9C6500))
        .set_background_color(Color::RGB(0xFFEB9C));
    let good = Format::new()
        .set_font_color(Color::RGB(0x006100))
        .set_background_color(Color::RGB(0xC6EFCE));
    for (file, name, (headers, rows)) in tables {
        crate::cancellation::check()?;
        write_new(&partial.join(file), csv(&headers, &rows).as_bytes())?;
        let sheet = workbook.add_worksheet();
        sheet.set_name(name).map_err(|e| e.to_string())?;
        for (col, title) in headers.iter().enumerate() {
            sheet
                .write_string_with_format(0, col as u16, title, &header)
                .map_err(|e| e.to_string())?;
            sheet
                .set_column_width(
                    col as u16,
                    if title.contains("Path") || title.contains("Issues") {
                        55
                    } else if title.contains("SHA256") {
                        68
                    } else {
                        23
                    },
                )
                .map_err(|e| e.to_string())?;
        }
        for (row, values) in rows.iter().enumerate() {
            crate::cancellation::check()?;
            for (col, value) in values.iter().enumerate() {
                let style = if value == "OK" || value == "VALID" {
                    Some(&good)
                } else if value.starts_with("PARTIAL") {
                    Some(&warning)
                } else if value.starts_with("CHECK")
                    || matches!(value.as_str(), "FAILED" | "INVALID" | "UNTRACKED")
                {
                    Some(&bad)
                } else {
                    None
                };
                // Always literal strings: no customer path/text becomes an Excel formula.
                let value = excel_text(value);
                if let Some(style) = style {
                    sheet
                        .write_string_with_format(row as u32 + 1, col as u16, &value, style)
                        .map_err(|e| e.to_string())?;
                } else {
                    sheet
                        .write_string(row as u32 + 1, col as u16, &value)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        sheet.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
        if !headers.is_empty() {
            sheet
                .autofilter(0, 0, rows.len() as u32, headers.len() as u16 - 1)
                .map_err(|e| e.to_string())?;
        }
    }
    let limits = workbook.add_worksheet();
    limits
        .set_name(language.text("Hatókör", "Scope"))
        .map_err(|e| e.to_string())?;
    limits
        .write_string_with_format(0, 0, LIMITS, &Format::new().set_text_wrap())
        .map_err(|e| e.to_string())?;
    limits.set_row_height(0, 100).map_err(|e| e.to_string())?;
    for (i, issue) in document.issues.iter().enumerate() {
        limits
            .write_string(i as u32 + 2, 0, excel_text(issue))
            .map_err(|e| e.to_string())?;
    }
    limits.set_column_width(0, 120).map_err(|e| e.to_string())?;
    let workbook_name = "FloppyFinalReport.xlsx";
    workbook
        .save(partial.join(workbook_name))
        .map_err(|e| e.to_string())?;
    OpenOptions::new()
        .write(true)
        .open(partial.join(workbook_name))
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    write_new(
        &partial.join("FloppyFinalReport.json"),
        &serde_json::to_vec_pretty(document).map_err(|e| e.to_string())?,
    )?;
    let mut text = format!(
        "{}\n{}\n\n{}\n\n",
        language.text("FLOPPY VÉGSŐ AUDIT", "FLOPPY FINAL AUDIT"),
        document.project,
        LIMITS
    );
    for (name, value) in metrics {
        text.push_str(&format!("{name}: {value}\n"));
    }
    for disk in &document.floppies {
        text.push_str(&format!(
            "\n{} | {} | {}\n",
            disk.floppy,
            disk.audit_status,
            disk.issues.join("; ")
        ));
    }
    write_new(&partial.join("FloppyFinalAudit.txt"), text.as_bytes())?;
    let mut hashes = BTreeMap::new();
    for file in walk(&partial, false)? {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        hashes.insert(name, conversion::sha256_file(&file)?);
    }
    let manifest_hash = hashes
        .get("DeliveryManifest.csv")
        .ok_or("Missing delivery manifest")?;
    write_new(
        &partial.join("DeliveryManifest.sha256"),
        format!("{manifest_hash} *DeliveryManifest.csv\n").as_bytes(),
    )?;
    hashes.insert(
        "DeliveryManifest.sha256".into(),
        conversion::sha256_file(&partial.join("DeliveryManifest.sha256"))?,
    );
    crate::cancellation::check()?;
    let directory = parent.join(&generation);
    fs::rename(&partial, &directory).map_err(|e| e.to_string())?;
    let latest = project.reports_dir().join("FinalReportLatest.json");
    if latest.exists()
        && !fs::symlink_metadata(&latest)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
    {
        return Err("Unsafe latest report pointer".into());
    }
    let temporary = project
        .reports_dir()
        .join(format!(".final-report-{generation}.partial.json"));
    write_new(&temporary, &serde_json::to_vec_pretty(&json!({"schema_version":1,"generation":generation,"directory":format!("FinalReports/{generation}"),"language":language.text("hu","en"),"artifacts_sha256":hashes,"customer_delivery_certified":false})).map_err(|e| e.to_string())?)?;
    crate::cancellation::check()?;
    fs::rename(temporary, &latest).map_err(|e| e.to_string())?;
    Ok(Export {
        workbook: directory.join(workbook_name),
        json: directory.join("FloppyFinalReport.json"),
        directory,
        latest,
        disks: document.floppies.len(),
        attention_disks: document.summary.floppies_attention,
        disk_verification: document
            .floppies
            .iter()
            .map(|d| (d.floppy.clone(), d.audit_status == "OK"))
            .collect(),
    })
}

fn summary_metrics(s: &Summary, language: Language) -> Vec<(String, f64)> {
    let rows = [
        (
            "Auditált lemezek",
            "Floppies audited",
            s.floppies_audited as f64,
        ),
        ("Lemezképek", "Images present", s.images_present as f64),
        (
            "Integritás rendben",
            "Floppies fully OK",
            s.floppies_fully_ok as f64,
        ),
        (
            "Figyelmet igényel",
            "Floppies needing attention",
            s.floppies_attention as f64,
        ),
        ("Olvasás OK", "Imaging status OK", s.imaging_ok as f64),
        (
            "Olvasás nem OK",
            "Imaging status not OK",
            s.imaging_not_ok as f64,
        ),
        (
            "Visszanyert forrásfájlok",
            "Recovered source files",
            s.recovered_source_files as f64,
        ),
        (
            "Átadási fájlok",
            "Customer delivery files",
            s.customer_delivery_files as f64,
        ),
        (
            "Konverzió OK",
            "Conversions fully OK",
            s.conversions_ok as f64,
        ),
        (
            "Konverzió részleges",
            "Conversions partial",
            s.conversions_partial as f64,
        ),
        (
            "Konverzió sikertelen",
            "Conversions failed",
            s.conversions_failed as f64,
        ),
        (
            "Konverziós jelöltek",
            "Conversion candidates",
            s.conversion_candidates as f64,
        ),
        (
            "Konverziós siker %",
            "Conversion success %",
            s.conversion_success_percent,
        ),
        (
            "Modern kimenetek jók",
            "Modern outputs good",
            s.modern_outputs_good as f64,
        ),
        (
            "PDF kimenetek jók",
            "PDF outputs good",
            s.pdf_outputs_good as f64,
        ),
        (
            "Generált integritásellenőrzések",
            "Generated integrity checks",
            s.generated_integrity_checks as f64,
        ),
        (
            "Érvénytelen generált kimenetek",
            "Invalid generated outputs",
            s.invalid_generated_outputs as f64,
        ),
        (
            "Ismeretlen átadási fájlok",
            "Untracked delivery files",
            s.untracked_delivery_files as f64,
        ),
        (
            "Ismeretlen nyers formátumok",
            "Raw format exceptions",
            s.raw_format_exceptions as f64,
        ),
    ];
    rows.into_iter()
        .map(|(hu, en, v)| (language.text(hu, en).into(), v))
        .collect()
}
