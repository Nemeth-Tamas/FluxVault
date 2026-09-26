//! Read-only evidence audit for image and managed extraction integrity.
//! Conversion and customer-delivery completeness are deliberately not certified yet.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    conversion_run,
    extraction::{self, ExtractionPresence},
    imaging,
    project::ProjectState,
};

#[derive(Debug, Clone)]
pub enum AuditEvent {
    Stage(String),
    Finished(Result<AuditResult, String>),
}

#[derive(Debug, Clone)]
pub struct AuditResult {
    pub json_path: PathBuf,
    pub csv_path: PathBuf,
    pub disk_count: usize,
    pub verified_disks: usize,
    pub attention_disks: usize,
}

#[derive(Debug, Serialize)]
struct AuditDocument {
    schema_version: u32,
    scope: &'static str,
    customer_delivery_certified: bool,
    project: String,
    disks: Vec<DiskEvidence>,
}

#[derive(Debug, Serialize)]
struct DiskEvidence {
    disk: String,
    attempt: u32,
    image: String,
    image_status: String,
    bad_sectors: usize,
    image_sha256: String,
    image_hash_verified: bool,
    extraction_status: String,
    extracted_files: usize,
    extracted_bytes: u64,
    extracted_hashes_verified: bool,
    conversion_status: String,
    conversion_jobs: usize,
    converted_outputs_verified: usize,
    evidence_status: String,
    issue: String,
}

#[derive(Debug, Default)]
struct ConversionEvidence {
    jobs: usize,
    verified_outputs: usize,
    issues: Vec<String>,
}

pub fn spawn_audit(project: ProjectState) -> Receiver<AuditEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = run_audit(&project, &|stage| {
            let _ = sender.send(AuditEvent::Stage(stage.to_owned()));
        });
        let _ = sender.send(AuditEvent::Finished(result));
    });
    receiver
}

pub(crate) fn run_audit(
    project: &ProjectState,
    stage: &impl Fn(&str),
) -> Result<AuditResult, String> {
    let statistics = imaging::load_project_statistics(&project.images_dir())?;
    let conversion_evidence =
        load_conversion_evidence(&project.reports_dir(), &project.converted_dir());
    let mut disks = Vec::with_capacity(statistics.disk_count);
    for (index, summary) in statistics.disks.iter().enumerate() {
        stage(&format!(
            "Auditing disk {} of {}: {:03}",
            index + 1,
            statistics.disk_count,
            summary.disk_number
        ));
        let attempts = imaging::load_attempts_for_disk(&project.images_dir(), summary.disk_number)?;
        let Some(attempt) = attempts
            .iter()
            .find(|attempt| attempt.attempt_number == summary.best_attempt_number)
        else {
            continue;
        };
        let image_path = project.images_dir().join(&attempt.image_file);
        let mut record = DiskEvidence {
            disk: format!("{:03}", summary.disk_number),
            attempt: attempt.attempt_number,
            image: attempt.image_file.clone(),
            image_status: attempt.status.clone(),
            bad_sectors: attempt.bad_sectors.len(),
            image_sha256: attempt.sha256.clone(),
            image_hash_verified: false,
            extraction_status: "MISSING".to_owned(),
            extracted_files: 0,
            extracted_bytes: 0,
            extracted_hashes_verified: false,
            conversion_status: "NOT_RUN".to_owned(),
            conversion_jobs: 0,
            converted_outputs_verified: 0,
            evidence_status: "CHECK".to_owned(),
            issue: String::new(),
        };
        let actual_image_hash = match hash_file(&image_path) {
            Ok(hash) => hash,
            Err(error) => {
                record.issue = format!("Image cannot be read: {error}");
                disks.push(record);
                continue;
            }
        };
        if !attempt.sha256.is_empty() && !actual_image_hash.eq_ignore_ascii_case(&attempt.sha256) {
            record.issue = "Image SHA-256 differs from acquisition metadata.".to_owned();
            disks.push(record);
            continue;
        }
        record.image_sha256 = actual_image_hash.clone();
        record.image_hash_verified = !attempt.sha256.is_empty();
        let presence = extraction::inspect_extraction_presence(
            &project.extracted_dir(),
            summary.disk_number,
            attempt.attempt_number,
        );
        match presence {
            Ok(ExtractionPresence::Automatic {
                output_directory, ..
            }) => {
                match extraction::verify_managed_extraction(&output_directory, &actual_image_hash) {
                    Ok((files, bytes)) => {
                        record.extraction_status = "VERIFIED".to_owned();
                        record.extracted_files = files;
                        record.extracted_bytes = bytes;
                        record.extracted_hashes_verified = true;
                    }
                    Err(error) => {
                        record.extraction_status = "INVALID".to_owned();
                        record.issue = error;
                    }
                }
            }
            Ok(ExtractionPresence::ManualRecovery {
                file_count,
                total_bytes,
                ..
            }) => {
                record.extraction_status = "MANUAL_UNVERIFIED".to_owned();
                record.extracted_files = file_count;
                record.extracted_bytes = total_bytes;
                record.issue =
                    "Manual recovery has no managed per-file integrity inventory.".to_owned();
            }
            Ok(ExtractionPresence::InvalidAutomatic { detail, .. }) => {
                record.extraction_status = "INVALID".to_owned();
                record.issue = detail;
            }
            Ok(ExtractionPresence::Missing { .. }) => {
                record.issue = "No extraction for the selected best attempt.".to_owned();
            }
            Err(error) => record.issue = error,
        }
        let conversion_ok = match &conversion_evidence {
            Ok(Some(by_disk)) => match by_disk.get(&record.disk) {
                Some(evidence) => {
                    record.conversion_jobs = evidence.jobs;
                    record.converted_outputs_verified = evidence.verified_outputs;
                    if evidence.issues.is_empty() {
                        record.conversion_status = "VERIFIED".to_owned();
                        true
                    } else {
                        record.conversion_status = "CHECK".to_owned();
                        append_issue(&mut record.issue, &evidence.issues.join("; "));
                        false
                    }
                }
                None => {
                    record.conversion_status = "NO_CANDIDATES".to_owned();
                    true
                }
            },
            Ok(None) => false,
            Err(error) => {
                record.conversion_status = "INVALID_REPORT".to_owned();
                append_issue(&mut record.issue, error);
                false
            }
        };
        if record.image_hash_verified
            && !attempt.attention_required
            && record.extracted_hashes_verified
            && record.extracted_files > 0
            && conversion_ok
        {
            record.evidence_status = "IMAGE_FILES_CONVERSIONS_VERIFIED".to_owned();
            record.issue.clear();
        } else if attempt.attention_required {
            record.evidence_status = "PARTIAL_IMAGE_READ".to_owned();
            if record.issue.is_empty() {
                record.issue = format!("{} unresolved sectors", record.bad_sectors);
            }
        } else if record.extracted_files == 0 && record.issue.is_empty() {
            record.issue = "No recovered files are recorded.".to_owned();
        } else if !record.image_hash_verified && record.issue.is_empty() {
            record.issue = "No recorded image SHA-256 to compare against.".to_owned();
        } else if !conversion_ok {
            record.evidence_status = "CHECK_CONVERSION".to_owned();
            if record.issue.is_empty() {
                record.issue = "Conversion has not been audited or has invalid outputs.".to_owned();
            }
        }
        disks.push(record);
    }
    let verified_disks = disks
        .iter()
        .filter(|disk| disk.evidence_status == "IMAGE_FILES_CONVERSIONS_VERIFIED")
        .count();
    let attention_disks = disks.len() - verified_disks;
    let report = AuditDocument {
        schema_version: 1,
        scope: "image_managed_extraction_and_recorded_conversion_integrity",
        customer_delivery_certified: false,
        project: project.name().to_owned(),
        disks,
    };
    fs::create_dir_all(project.reports_dir()).map_err(|error| error.to_string())?;
    let json_path = project.reports_dir().join("EvidenceAudit.json");
    let csv_path = project.reports_dir().join("EvidenceAudit.csv");
    fs::write(
        &json_path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot write {}: {error}", json_path.display()))?;
    fs::write(&csv_path, csv_report(&report))
        .map_err(|error| format!("Cannot write {}: {error}", csv_path.display()))?;
    Ok(AuditResult {
        json_path,
        csv_path,
        disk_count: report.disks.len(),
        verified_disks,
        attention_disks,
    })
}

fn csv_report(report: &AuditDocument) -> String {
    let mut csv = String::from(
        "\u{feff}\"Disk\",\"Attempt\",\"Image\",\"ImageStatus\",\"BadSectors\",\"ImageHashVerified\",\"ExtractionStatus\",\"ExtractedFiles\",\"ExtractedBytes\",\"ExtractedHashesVerified\",\"ConversionStatus\",\"ConversionJobs\",\"ConvertedOutputsVerified\",\"EvidenceStatus\",\"Issue\"\r\n",
    );
    for disk in &report.disks {
        let values = [
            disk.disk.clone(),
            disk.attempt.to_string(),
            disk.image.clone(),
            disk.image_status.clone(),
            disk.bad_sectors.to_string(),
            disk.image_hash_verified.to_string(),
            disk.extraction_status.clone(),
            disk.extracted_files.to_string(),
            disk.extracted_bytes.to_string(),
            disk.extracted_hashes_verified.to_string(),
            disk.conversion_status.clone(),
            disk.conversion_jobs.to_string(),
            disk.converted_outputs_verified.to_string(),
            disk.evidence_status.clone(),
            disk.issue.clone(),
        ];
        csv.push_str(
            &values
                .iter()
                .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
    }
    csv
}

fn append_issue(existing: &mut String, additional: &str) {
    if !existing.is_empty() {
        existing.push_str("; ");
    }
    existing.push_str(additional);
}

fn load_conversion_evidence(
    reports_directory: &Path,
    converted_root: &Path,
) -> Result<Option<BTreeMap<String, ConversionEvidence>>, String> {
    let summary_path = reports_directory.join("ConversionSummary.csv");
    if !summary_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&summary_path).map_err(|error| {
        format!(
            "Cannot read conversion summary {}: {error}",
            summary_path.display()
        )
    })?;
    let rows = parse_csv(&text)?;
    let Some(header) = rows.first() else {
        return Err("Conversion summary is empty.".to_owned());
    };
    let column = |name: &str| {
        header
            .iter()
            .position(|value| value == name)
            .ok_or_else(|| format!("Conversion summary lacks {name} column"))
    };
    let disk_col = column("Floppy")?;
    let source_hash_col = column("SourceSHA256")?;
    let original_col = column("DeliveryOriginalPath")?;
    let modern_col = column("ModernPath")?;
    let modern_format_col = column("ModernFormat")?;
    let modern_ok_col = column("ModernOK")?;
    let pdf_col = column("PDFPath")?;
    let pdf_ok_col = column("PDFOK")?;
    let status_col = column("Status")?;
    let mut by_disk = BTreeMap::<String, ConversionEvidence>::new();
    for (number, row) in rows.iter().enumerate().skip(1) {
        if row.len() != header.len() {
            return Err(format!(
                "Conversion summary row {} has {} fields; expected {}",
                number + 1,
                row.len(),
                header.len()
            ));
        }
        let disk = &row[disk_col];
        if disk
            .parse::<u32>()
            .ok()
            .filter(|number| *number > 0)
            .is_none()
        {
            return Err(format!(
                "Invalid floppy number in conversion summary row {}",
                number + 1
            ));
        }
        let evidence = by_disk.entry(disk.clone()).or_default();
        evidence.jobs += 1;
        if row[status_col] != "OK" || row[modern_ok_col] != "true" || row[pdf_ok_col] != "true" {
            evidence
                .issues
                .push(format!("Conversion job {} is {}", number, row[status_col]));
            continue;
        }
        let verified = (|| -> Result<(), String> {
            let original = confined_output(converted_root, &row[original_col])?;
            let expected_hash = &row[source_hash_col];
            if expected_hash.len() != 64
                || !hash_file(&original)?.eq_ignore_ascii_case(expected_hash)
            {
                return Err(format!(
                    "Converted source copy differs: {}",
                    row[original_col]
                ));
            }
            let modern = confined_output(converted_root, &row[modern_col])?;
            let modern_extension = row[modern_format_col].to_ascii_lowercase();
            let pdf = confined_output(converted_root, &row[pdf_col])?;
            if !conversion_run::validate_output(&modern, &modern_extension)?
                || !conversion_run::validate_output(&pdf, "pdf")?
            {
                return Err(format!(
                    "Generated Office/PDF integrity failed for disk {disk} job {number}"
                ));
            }
            Ok(())
        })();
        match verified {
            Ok(()) => evidence.verified_outputs += 3,
            Err(error) => evidence.issues.push(error),
        }
    }
    Ok(Some(by_disk))
}

fn confined_output(root: &Path, relative_text: &str) -> Result<PathBuf, String> {
    let normalized = relative_text.replace('\\', "/");
    let relative = Path::new(&normalized);
    if normalized.is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!("Unsafe conversion output path: {relative_text}"));
    }
    let root = root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve Converted folder: {error}"))?;
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|error| format!("Cannot resolve conversion output {relative_text}: {error}"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(format!(
            "Conversion output escapes Converted folder: {relative_text}"
        ));
    }
    Ok(path)
}

fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut after_quote = false;
    while let Some(character) = chars.next() {
        if quoted {
            if character == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                    after_quote = true;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        match character {
            '"' if field.is_empty() && !after_quote => quoted = true,
            ',' => {
                row.push(std::mem::take(&mut field));
                after_quote = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                after_quote = false;
            }
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                after_quote = false;
            }
            _ if after_quote || character == '"' => {
                return Err("Malformed quoted CSV field.".to_owned());
            }
            _ => field.push(character),
        }
    }
    if quoted {
        return Err("Unterminated quoted CSV field.".to_owned());
    }
    if !row.is_empty() || !field.is_empty() || after_quote {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn synthetic_image_audit_is_repeatable_and_detects_hash_tampering() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-audit-fixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let image = project.images_dir().join("001_attempt_001.img");
        let source_bytes = vec![0x5a; 512];
        fs::write(&image, &source_bytes).unwrap();
        let source_hash = hash_file(&image).unwrap();
        assert_eq!(source_hash, format!("{:x}", Sha256::digest(&source_bytes)));
        let metadata = serde_json::json!({
            "fluxvault_version": "0.1.0", "status": "OK",
            "disk_number": 1, "attempt_number": 1,
            "source_backend": "synthetic-test", "source_device": "none",
            "image_file": "001_attempt_001.img", "log_file": "",
            "timestamp_unix_ms": 1,
            "geometry": {"cylinders": 1, "heads": 1, "sectors_per_track": 1,
                "bytes_per_sector": 512, "total_bytes": 512, "format_guess": "fixture"},
            "sector_retries": 0, "total_sectors": 1, "bytes_written": 512,
            "retry_recovered_sectors": 0, "bad_sector_count": 0,
            "bad_sectors": [], "sha256": source_hash,
        });
        fs::write(
            project.images_dir().join("001_attempt_001.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();

        let first = run_audit(&project, &|_| {}).unwrap();
        let first_json = fs::read(&first.json_path).unwrap();
        let first_csv = fs::read(&first.csv_path).unwrap();
        let record: serde_json::Value = serde_json::from_slice(&first_json).unwrap();
        assert_eq!(record["disks"][0]["image_hash_verified"], true);
        assert_eq!(record["disks"][0]["image_sha256"], source_hash);
        let second = run_audit(&project, &|_| {}).unwrap();
        assert_eq!(fs::read(second.json_path).unwrap(), first_json);
        assert_eq!(fs::read(second.csv_path).unwrap(), first_csv);
        assert_eq!(fs::read(&image).unwrap(), source_bytes);

        fs::write(&image, vec![0xa5; 512]).unwrap();
        let tampered = run_audit(&project, &|_| {}).unwrap();
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(tampered.json_path).unwrap()).unwrap();
        assert_eq!(record["disks"][0]["image_hash_verified"], false);
        assert!(
            record["disks"][0]["issue"]
                .as_str()
                .unwrap()
                .contains("differs from acquisition metadata")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn csv_escapes_issue_text() {
        let report = AuditDocument {
            schema_version: 1,
            scope: "test",
            customer_delivery_certified: false,
            project: "test".to_owned(),
            disks: vec![DiskEvidence {
                disk: "001".to_owned(),
                attempt: 1,
                image: "001.img".to_owned(),
                image_status: "OK".to_owned(),
                bad_sectors: 0,
                image_sha256: String::new(),
                image_hash_verified: true,
                extraction_status: "VERIFIED".to_owned(),
                extracted_files: 1,
                extracted_bytes: 1,
                extracted_hashes_verified: true,
                conversion_status: "VERIFIED".to_owned(),
                conversion_jobs: 1,
                converted_outputs_verified: 3,
                evidence_status: "CHECK".to_owned(),
                issue: "bad \"quote\"".to_owned(),
            }],
        };
        assert!(csv_report(&report).contains("\"bad \"\"quote\"\"\""));
    }

    #[test]
    fn conversion_csv_parser_handles_quotes_commas_and_newlines() {
        let rows = parse_csv("\u{feff}\"A\",\"B\"\r\n\"007\",\"Dr. \"\"Anka\"\", test\nnext\"\r\n")
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], ["007", "Dr. \"Anka\", test\nnext"]);
    }

    #[test]
    fn conversion_output_path_rejects_parent_traversal() {
        assert!(confined_output(Path::new("."), "..\\outside.docx").is_err());
    }

    #[test]
    fn missing_conversion_output_is_a_disk_issue_not_a_fatal_audit_error() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-conversion-audit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let reports = root.join("Reports");
        let converted = root.join("Converted");
        fs::create_dir_all(&reports).unwrap();
        fs::create_dir_all(&converted).unwrap();
        fs::write(reports.join("ConversionSummary.csv"),
            "\"Floppy\",\"SourceSHA256\",\"DeliveryOriginalPath\",\"ModernPath\",\"ModernFormat\",\"ModernOK\",\"PDFPath\",\"PDFOK\",\"Status\"\n\"007\",\"abc\",\"007\\missing.doc\",\"007\\missing.docx\",\"DOCX\",\"true\",\"007\\missing.pdf\",\"true\",\"OK\"\n").unwrap();
        let evidence = load_conversion_evidence(&reports, &converted)
            .unwrap()
            .unwrap();
        assert_eq!(evidence["007"].jobs, 1);
        assert_eq!(evidence["007"].verified_outputs, 0);
        assert_eq!(evidence["007"].issues.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
