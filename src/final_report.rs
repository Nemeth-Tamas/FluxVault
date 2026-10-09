//! Image-only, generation-bound final audit/export. Never certifies historical yield.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{audit, conversion, conversion_run, extraction, project::ProjectState};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const LIMITS: &str = "OK means recorded image, selected recovered files and recorded delivery/conversion artifacts passed integrity checks. It does not certify original disk completeness, document semantics, historical ownership, or script/DMDE yield parity. Untracked delivery files remain attention items. Forensic fragments/deleted candidates/text salvage are not counted as recovered whole files.";

#[cfg(test)]
#[path = "final_report_tests.rs"]
mod tests;

#[path = "final_report_writer.rs"]
mod writer;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Hungarian,
    English,
}
impl Language {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "hu" => Ok(Self::Hungarian),
            "en" => Ok(Self::English),
            _ => Err("--language requires hu or en".into()),
        }
    }
    fn text<'a>(self, hu: &'a str, en: &'a str) -> &'a str {
        if self == Self::Hungarian { hu } else { en }
    }
}

#[derive(Debug, Serialize)]
pub struct Export {
    pub directory: PathBuf,
    pub workbook: PathBuf,
    pub json: PathBuf,
    pub latest: PathBuf,
    pub disks: usize,
    pub attention_disks: usize,
    #[serde(skip)]
    pub(crate) disk_verification: BTreeMap<String, bool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Summary {
    floppies_audited: usize,
    images_present: usize,
    floppies_fully_ok: usize,
    floppies_attention: usize,
    imaging_ok: usize,
    imaging_not_ok: usize,
    raw_format_exceptions: usize,
    recovered_source_files: usize,
    excluded_source_metadata_files: usize,
    customer_delivery_files: usize,
    conversion_candidates: usize,
    conversions_ok: usize,
    conversions_partial: usize,
    conversions_failed: usize,
    conversion_success_percent: f64,
    modern_outputs_good: usize,
    pdf_outputs_good: usize,
    generated_integrity_checks: usize,
    invalid_generated_outputs: usize,
    untracked_delivery_files: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Disk {
    floppy: String,
    audit_status: String,
    image: String,
    image_bytes: Option<u64>,
    #[serde(rename = "ImageSHA256")]
    image_sha256: String,
    imaging_status: String,
    bad_sectors: Option<usize>,
    extraction_status: String,
    source_recovered_files: usize,
    customer_delivery_files: usize,
    convertible_files: usize,
    conversion_ok: usize,
    conversion_partial: usize,
    conversion_failed: usize,
    integrity_invalid: usize,
    evidence_status: String,
    issues: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Recovered {
    floppy: String,
    recovered_path: String,
    recovery_method: String,
    size_bytes: u64,
    #[serde(rename = "SHA256")]
    sha256: String,
    source_image_sha256: String,
    managed_inventory_verified: bool,
    delivery_eligible: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Converted {
    floppy: String,
    original_forensic_path: String,
    recovery_method: String,
    #[serde(rename = "SourceSHA256")]
    source_sha256: String,
    modern_path: String,
    pdf_path: String,
    recorded_status: String,
    status: String,
    modern_ok: bool,
    pdf_ok: bool,
    duration_sec: f64,
    issues: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Delivery {
    floppy: String,
    delivery_path: String,
    file_role: String,
    original_forensic_path: String,
    recovery_method: String,
    size_bytes: u64,
    #[serde(rename = "SHA256")]
    sha256: String,
    integrity: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Integrity {
    floppy: String,
    delivery_path: String,
    format: String,
    recorded_success: bool,
    expected_sha256: Option<String>,
    actual_sha256: Option<String>,
    integrity: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct Document {
    schema_version: u32,
    project: String,
    generated_unix_ms: u64,
    customer_delivery_certified: bool,
    limits: &'static str,
    summary: Summary,
    floppies: Vec<Disk>,
    recovered_files: Vec<Recovered>,
    conversions: Vec<Converted>,
    integrity: Vec<Integrity>,
    delivery_files: Vec<Delivery>,
    issues: Vec<String>,
}

/// Caller holds the processing owner and artifact snapshot gate. Pipeline already
/// owns both; CLI export acquires them before refreshing the underlying audit.
pub(crate) fn export(
    project: &ProjectState,
    audit: &audit::AuditResult,
    language: Language,
) -> Result<Export, String> {
    crate::cancellation::check()?;
    let document = collect(project, audit)?;
    writer::publish(project, &document, language)
}

fn key(disk: &str, path: &str) -> String {
    format!("{disk}/{}", path.replace('\\', "/").to_lowercase())
}

pub(crate) fn package_files(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    writer::package_files(root)
}

pub(crate) fn reconcile_audit(audit: &mut audit::AuditResult, report: &Export) {
    for (disk, verified) in &mut audit.disk_verification {
        *verified &= report.disk_verification.get(disk).copied().unwrap_or(false);
    }
    audit.verified_disks = audit.disk_verification.values().filter(|v| **v).count();
    audit.attention_disks = audit.disk_count - audit.verified_disks;
}

fn collect(project: &ProjectState, audit: &audit::AuditResult) -> Result<Document, String> {
    let mut report = Document {
        schema_version: 1,
        project: project.name().to_owned(),
        generated_unix_ms: crate::external_tools::current_unix_ms(),
        customer_delivery_certified: false,
        limits: LIMITS,
        summary: Summary::default(),
        floppies: Vec::new(),
        recovered_files: Vec::new(),
        conversions: Vec::new(),
        integrity: Vec::new(),
        delivery_files: Vec::new(),
        issues: Vec::new(),
    };
    let mut disks = BTreeMap::<String, Disk>::new();
    for evidence in &audit.document.disks {
        crate::cancellation::check()?;
        let mut disk = Disk {
            floppy: evidence.disk.clone(),
            image: evidence.image.clone(),
            image_bytes: if evidence.image_hash_verified {
                Some(
                    fs::metadata(crate::recovery_plan::resolve_image_path(
                        &project.images_dir(),
                        &evidence.image,
                    )?)
                    .map_err(|e| e.to_string())?
                    .len(),
                )
            } else {
                None
            },
            image_sha256: evidence.image_sha256.clone(),
            imaging_status: evidence.image_status.clone(),
            bad_sectors: Some(evidence.bad_sectors),
            extraction_status: evidence.extraction_status.clone(),
            evidence_status: evidence.evidence_status.clone(),
            ..Disk::default()
        };
        if !evidence.issue.is_empty() {
            disk.issues.push(evidence.issue.clone());
        }
        let presence = match extraction::inspect_extraction_presence(
            &project.extracted_dir(),
            evidence.disk.parse().map_err(|_| "Invalid audit disk")?,
            evidence.attempt,
        ) {
            Ok(presence) => presence,
            Err(error) => {
                disk.issues
                    .push(format!("Selected extraction cannot be verified: {error}"));
                disks.insert(disk.floppy.clone(), disk);
                continue;
            }
        };
        if let extraction::ExtractionPresence::Automatic {
            output_directory, ..
        } = presence
        {
            // Never trust a marker alone: audit replays the complete managed inventory.
            let verified = evidence.extracted_hashes_verified && evidence.image_hash_verified;
            let native = verified && extraction::verify_native_extraction(&output_directory)?;
            for file in extraction::inventory_files(&output_directory)? {
                report.recovered_files.push(Recovered {
                    floppy: disk.floppy.clone(),
                    recovered_path: file.relative_path.clone(),
                    recovery_method: conversion::report_recovery_method(
                        Path::new(&file.relative_path),
                        native,
                    )?,
                    size_bytes: file.bytes,
                    sha256: file.sha256,
                    source_image_sha256: evidence.image_sha256.clone(),
                    managed_inventory_verified: verified,
                    delivery_eligible: conversion::delivery_eligible(Path::new(
                        &file.relative_path,
                    )),
                });
                disk.source_recovered_files += 1;
            }
        } else if let extraction::ExtractionPresence::ManualRecovery {
            output_directory, ..
        } = presence
        {
            for file in manual_files(&output_directory)? {
                let relative = file
                    .strip_prefix(&output_directory)
                    .map_err(|e| e.to_string())?;
                report.recovered_files.push(Recovered {
                    floppy: disk.floppy.clone(),
                    recovered_path: relative.to_string_lossy().replace('\\', "/"),
                    recovery_method: format!(
                        "Legacy/manual: {}",
                        conversion::report_recovery_method(relative, false)?
                    ),
                    size_bytes: fs::metadata(&file).map_err(|e| e.to_string())?.len(),
                    sha256: conversion::sha256_file(&file)?,
                    source_image_sha256: evidence.image_sha256.clone(),
                    managed_inventory_verified: false,
                    delivery_eligible: conversion::delivery_eligible(relative),
                });
                disk.source_recovered_files += 1;
            }
        }
        disks.insert(disk.floppy.clone(), disk);
    }
    for exception in crate::flux_recovery::format_exceptions(project)? {
        let floppy = format!("{:03}", exception.disk);
        let disk = disks.entry(floppy.clone()).or_insert_with(|| Disk {
            floppy,
            imaging_status: "RAW_FORMAT_EXCEPTION".into(),
            extraction_status: "MISSING".into(),
            evidence_status: "RAW_FORMAT_EXCEPTION".into(),
            ..Disk::default()
        });
        disk.issues.push(format!(
            "Raw capture has unresolved format: {}",
            exception.stop_reason
        ));
        report.summary.raw_format_exceptions += 1;
    }
    let recovered = report
        .recovered_files
        .iter()
        .map(|r| (key(&r.floppy, &r.recovered_path), r))
        .collect::<BTreeMap<_, _>>();
    if recovered.len() != report.recovered_files.len() {
        return Err(
            "Competing case-folded recovered-file identities; report binding refused".into(),
        );
    }
    let mut generated = BTreeMap::<String, (String, String, String)>::new();
    let snapshot_path = conversion_run::snapshot_path(&project.reports_dir());
    if snapshot_path.exists() {
        match conversion_run::load_snapshot(&project.reports_dir(), project.root()) {
            Ok(snapshot) => {
                for row in snapshot.rows {
                    crate::cancellation::check()?;
                    let mut issues = Vec::new();
                    let source_ok = recovered
                        .get(&key(&row.job.floppy, &row.job.original_forensic_path))
                        .is_some_and(|f| f.sha256.eq_ignore_ascii_case(&row.job.source_sha256));
                    if !source_ok {
                        issues.push("Conversion source is not bound to the currently selected verified recovered file".into());
                    }
                    let mut paths = Vec::new();
                    let mut good = Vec::new();
                    for (path, output, format) in [
                        (
                            &row.job.modern_path,
                            &row.modern,
                            row.job.modern_format.as_str(),
                        ),
                        (&row.job.pdf_path, &row.pdf, "pdf"),
                    ] {
                        let check =
                            check_output(project, &row.job.floppy, path, output, format, source_ok);
                        if check.integrity != "VALID" {
                            issues.push(check.detail.clone());
                        }
                        let valid = check.integrity == "VALID";
                        good.push(valid);
                        paths.push(check.delivery_path.clone());
                        if !check.delivery_path.is_empty() {
                            let identity = check.delivery_path.replace('\\', "/").to_lowercase();
                            if generated
                                .insert(
                                    identity,
                                    (
                                        row.job.floppy.clone(),
                                        row.job.original_forensic_path.clone(),
                                        check.integrity.clone(),
                                    ),
                                )
                                .is_some()
                            {
                                return Err(
                                    "Competing conversions claim the same delivery output".into()
                                );
                            }
                        }
                        report.integrity.push(check);
                    }
                    let status = match (good[0], good[1]) {
                        (true, true) => "OK",
                        (false, false) => "FAILED",
                        _ => "PARTIAL",
                    };
                    let recorded_status = row.status().to_owned();
                    report.conversions.push(Converted {
                        floppy: row.job.floppy,
                        original_forensic_path: row.job.original_forensic_path,
                        recovery_method: row.job.recovery_method,
                        source_sha256: row.job.source_sha256,
                        modern_path: paths[0].clone(),
                        pdf_path: paths[1].clone(),
                        recorded_status,
                        status: status.into(),
                        modern_ok: good[0],
                        pdf_ok: good[1],
                        duration_sec: row.duration_seconds,
                        issues,
                    });
                }
            }
            Err(e) => report.issues.push(e),
        }
    } else if !report.recovered_files.is_empty() {
        report.issues.push("Conversion state is absent; recorded delivery/conversion completeness cannot be checked".into());
    }
    let mut invalid_report_state = !report.issues.is_empty();
    for file in &report.recovered_files {
        if file.delivery_eligible
            && conversion::requires_conversion(Path::new(&file.recovered_path))
            && !report.conversions.iter().any(|r| {
                key(&r.floppy, &r.original_forensic_path) == key(&file.floppy, &file.recovered_path)
            })
        {
            disks
                .get_mut(&file.floppy)
                .ok_or("Recovered file has no disk")?
                .issues
                .push(format!(
                    "Convertible recovered file has no current conversion result: {}",
                    file.recovered_path
                ));
        }
    }
    let mapping = delivery_mapping(&project.reports_dir())?;
    for file in &report.recovered_files {
        if file.delivery_eligible
            && !mapping.values().any(|m| {
                key(&m.floppy, &m.original_forensic_path) == key(&file.floppy, &file.recovered_path)
                    && m.source_sha256.eq_ignore_ascii_case(&file.sha256)
            })
        {
            disks
                .get_mut(&file.floppy)
                .ok_or("Recovered file has no disk")?
                .issues
                .push(format!(
                    "Recovered file has no hash-bound delivery original: {}",
                    file.recovered_path
                ));
        }
    }
    for path in walk(&project.converted_dir(), false)? {
        let relative = path
            .strip_prefix(project.converted_dir())
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let identity = relative.to_lowercase();
        let sha256 = conversion::sha256_file(&path)?;
        let (floppy, role, original, method, integrity) = if let Some(map) = mapping.get(&identity)
        {
            let source = recovered.get(&key(&map.floppy, &map.original_forensic_path));
            let valid = source.is_some_and(|s| s.sha256.eq_ignore_ascii_case(&map.source_sha256))
                && sha256.eq_ignore_ascii_case(&map.source_sha256);
            (
                map.floppy.clone(),
                "Original recovered file",
                map.original_forensic_path.clone(),
                map.recovery_method.clone(),
                if valid { "VALID" } else { "INVALID" },
            )
        } else if let Some((floppy, original, integrity)) = generated.get(&identity) {
            (
                floppy.clone(),
                "Generated conversion",
                original.clone(),
                String::new(),
                integrity.as_str(),
            )
        } else {
            (
                relative
                    .split('/')
                    .next()
                    .filter(|s| s.parse::<u32>().is_ok())
                    .unwrap_or("")
                    .to_owned(),
                "Untracked file",
                String::new(),
                String::new(),
                "UNTRACKED",
            )
        };
        if integrity != "VALID" {
            invalid_report_state |= floppy.is_empty();
            report
                .issues
                .push(format!("Delivery {relative}: {integrity}"));
        }
        report.delivery_files.push(Delivery {
            floppy,
            delivery_path: relative,
            file_role: role.into(),
            original_forensic_path: original,
            recovery_method: method,
            size_bytes: fs::metadata(&path).map_err(|e| e.to_string())?.len(),
            sha256,
            integrity: integrity.into(),
        });
    }
    // Missing originals are explicit issues too; walking only existing files is insufficient.
    let existing = report
        .delivery_files
        .iter()
        .map(|f| f.delivery_path.to_lowercase())
        .collect::<BTreeSet<_>>();
    for (path, map) in &mapping {
        if !existing.contains(path) {
            report
                .issues
                .push(format!("Delivery original is missing: {path}"));
            disks
                .entry(map.floppy.clone())
                .or_insert_with(|| Disk {
                    floppy: map.floppy.clone(),
                    ..Disk::default()
                })
                .issues
                .push(format!("Delivery original is missing: {path}"));
        }
    }
    for row in &report.conversions {
        let disk = disks.entry(row.floppy.clone()).or_insert_with(|| Disk {
            floppy: row.floppy.clone(),
            ..Disk::default()
        });
        disk.convertible_files += 1;
        match row.status.as_str() {
            "OK" => disk.conversion_ok += 1,
            "PARTIAL" => disk.conversion_partial += 1,
            _ => disk.conversion_failed += 1,
        }
        disk.issues.extend(row.issues.clone());
    }
    for row in &report.integrity {
        if row.recorded_success && row.integrity != "VALID" {
            disks
                .entry(row.floppy.clone())
                .or_insert_with(|| Disk {
                    floppy: row.floppy.clone(),
                    ..Disk::default()
                })
                .integrity_invalid += 1;
        }
    }
    for row in &report.delivery_files {
        if row.floppy.is_empty() {
            continue;
        }
        let disk = disks.entry(row.floppy.clone()).or_insert_with(|| Disk {
            floppy: row.floppy.clone(),
            ..Disk::default()
        });
        disk.customer_delivery_files += 1;
        if row.integrity != "VALID" {
            disk.issues
                .push(format!("Delivery {}: {}", row.delivery_path, row.integrity));
        }
    }
    for disk in disks.values_mut() {
        disk.audit_status = classify(disk).into();
        if disk.audit_status == "OK" && invalid_report_state {
            // A project-wide unreadable/missing state cannot silently turn into per-disk OK.
            disk.audit_status = "CHECK: REPORT STATE".into();
        }
        disk.issues.sort();
        disk.issues.dedup();
    }
    report.floppies = disks.into_values().collect();
    report
        .recovered_files
        .sort_by_key(|r| key(&r.floppy, &r.recovered_path));
    report
        .conversions
        .sort_by_key(|r| key(&r.floppy, &r.original_forensic_path));
    report
        .integrity
        .sort_by_key(|r| key(&r.floppy, &r.delivery_path));
    report
        .delivery_files
        .sort_by(|a, b| a.delivery_path.cmp(&b.delivery_path));
    report.summary = summarize(&report);
    Ok(report)
}

fn classify(disk: &Disk) -> &'static str {
    if disk.integrity_invalid > 0 || disk.conversion_failed > 0 {
        "CHECK: CONVERSION FAILED"
    } else if disk.source_recovered_files == 0 {
        "CHECK: NO RECOVERED FILES"
    } else if disk.conversion_partial > 0 {
        "PARTIAL: CONVERSION"
    } else if disk.imaging_status != "OK"
        || disk.evidence_status == "PARTIAL_IMAGE_READ"
        || disk.evidence_status == "DERIVED_RECOVERY"
    {
        "PARTIAL: IMAGE READ"
    } else if disk.evidence_status != "IMAGE_FILES_CONVERSIONS_VERIFIED" || !disk.issues.is_empty()
    {
        "CHECK: EVIDENCE"
    } else {
        "OK"
    }
}

fn check_output(
    project: &ProjectState,
    disk: &str,
    path: &Path,
    output: &conversion_run::OutputResult,
    format: &str,
    source_ok: bool,
) -> Integrity {
    let mut check = Integrity {
        floppy: disk.into(),
        delivery_path: String::new(),
        format: format.into(),
        recorded_success: output.state.successful(),
        expected_sha256: output.output_sha256.clone(),
        actual_sha256: None,
        integrity: "INVALID".into(),
        detail: output.detail.clone(),
    };
    let result = (|| -> Result<(), String> {
        let root = project
            .converted_dir()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        // Lexical confinement also identifies missing outputs; canonical check catches junctions.
        let relative = path
            .strip_prefix(project.converted_dir())
            .or_else(|_| path.strip_prefix(&root))
            .map_err(|_| "Generated output escapes Converted")?;
        safe_relative(relative)?;
        check.delivery_path = relative.to_string_lossy().replace('\\', "/");
        let actual = path
            .canonicalize()
            .map_err(|e| format!("Generated output missing: {} ({e})", check.delivery_path))?;
        if !actual.starts_with(&root) || !actual.is_file() {
            return Err("Generated output escapes Converted or is not a file".into());
        }
        let hash = conversion::sha256_file(&actual)?;
        check.actual_sha256 = Some(hash.clone());
        if !output.state.successful() {
            return Err(format!(
                "Conversion did not record success: {}",
                output.detail
            ));
        }
        if !source_ok {
            return Err("Generated output has no current verified source binding".into());
        }
        if !output
            .output_sha256
            .as_ref()
            .is_some_and(|h| h.len() == 64 && hash.eq_ignore_ascii_case(h))
        {
            return Err("Generated output SHA-256 differs or recorded binding is absent".into());
        }
        if !conversion_run::validate_output(&actual, &format.to_ascii_lowercase())? {
            return Err("Generated output container integrity failed".into());
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            check.integrity = "VALID".into();
            check.detail =
                "Current source, output SHA-256 and container checks passed; semantics unverified"
                    .into();
        }
        Err(e) => check.detail = e,
    }
    check
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Mapping {
    floppy: String,
    original_forensic_path: String,
    recovery_method: String,
    #[serde(rename = "SourceSHA256")]
    source_sha256: String,
}

fn delivery_mapping(reports: &Path) -> Result<BTreeMap<String, Mapping>, String> {
    let path = reports.join("DeliveryPathMap.csv");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let rows = audit::parse_csv(&fs::read_to_string(path).map_err(|e| e.to_string())?)?;
    let header = rows.first().ok_or("Empty delivery mapping")?;
    let mut maps = BTreeMap::new();
    for row in rows.iter().skip(1) {
        if row.len() != header.len() {
            return Err("Malformed delivery mapping row".into());
        }
        let value = header
            .iter()
            .cloned()
            .zip(row.iter().cloned().map(serde_json::Value::String))
            .collect::<serde_json::Map<_, _>>();
        let path = value
            .get("DeliveryPath")
            .and_then(|v| v.as_str())
            .ok_or("Delivery mapping lacks path")?
            .replace('\\', "/");
        safe_relative(Path::new(&path))?;
        let map: Mapping =
            serde_json::from_value(serde_json::Value::Object(value)).map_err(|e| e.to_string())?;
        if map.floppy.parse::<u32>().ok().filter(|n| *n > 0).is_none()
            || !path.starts_with(&format!("{}/", map.floppy))
        {
            return Err("Delivery mapping has invalid disk/path identity".into());
        }
        if maps.insert(path.to_lowercase(), map).is_some() {
            return Err("Duplicate delivery mapping identity".into());
        }
    }
    Ok(maps)
}

fn safe_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("Unsafe relative report/delivery path".into());
    }
    Ok(())
}

fn walk(root: &Path, skip_managed: bool) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        crate::cancellation::check()?;
        for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink()
                || !path
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&canonical)
            {
                return Err("Report inventory contains a link or escaped path".into());
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_dir() {
                if skip_managed
                    && directory == root
                    && (name == "legacy" || name.starts_with("attempt_"))
                {
                    continue;
                }
                pending.push(path);
            } else if kind.is_file() && !name.starts_with(".fluxvault-") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}
fn manual_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    walk(root, true)
}

fn summarize(report: &Document) -> Summary {
    let s = &report.floppies;
    let c = &report.conversions;
    Summary {
        floppies_audited: s.len(),
        images_present: s.iter().filter(|d| !d.image.is_empty()).count(),
        floppies_fully_ok: s.iter().filter(|d| d.audit_status == "OK").count(),
        floppies_attention: s.iter().filter(|d| d.audit_status != "OK").count(),
        imaging_ok: s.iter().filter(|d| d.imaging_status == "OK").count(),
        imaging_not_ok: s.iter().filter(|d| d.imaging_status != "OK").count(),
        raw_format_exceptions: report.summary.raw_format_exceptions,
        recovered_source_files: report.recovered_files.len(),
        excluded_source_metadata_files: report
            .recovered_files
            .iter()
            .filter(|f| !f.delivery_eligible)
            .count(),
        customer_delivery_files: report.delivery_files.len(),
        conversion_candidates: c.len(),
        conversions_ok: c.iter().filter(|r| r.status == "OK").count(),
        conversions_partial: c.iter().filter(|r| r.status == "PARTIAL").count(),
        conversions_failed: c.iter().filter(|r| r.status == "FAILED").count(),
        conversion_success_percent: if c.is_empty() {
            0.0
        } else {
            c.iter().filter(|r| r.status == "OK").count() as f64 * 100.0 / c.len() as f64
        },
        modern_outputs_good: c.iter().filter(|r| r.modern_ok).count(),
        pdf_outputs_good: c.iter().filter(|r| r.pdf_ok).count(),
        generated_integrity_checks: report.integrity.len(),
        invalid_generated_outputs: report
            .integrity
            .iter()
            .filter(|r| r.recorded_success && r.integrity != "VALID")
            .count(),
        untracked_delivery_files: report
            .delivery_files
            .iter()
            .filter(|r| r.integrity == "UNTRACKED")
            .count(),
    }
}
