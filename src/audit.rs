//! Read-only evidence audit for image and managed extraction integrity.
//! Conversion and customer-delivery completeness are deliberately not certified yet.

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
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
    evidence_status: String,
    issue: String,
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
        if record.image_hash_verified
            && !attempt.attention_required
            && record.extracted_hashes_verified
            && record.extracted_files > 0
        {
            record.evidence_status = "IMAGE_AND_FILES_VERIFIED".to_owned();
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
        }
        disks.push(record);
    }
    let verified_disks = disks
        .iter()
        .filter(|disk| disk.evidence_status == "IMAGE_AND_FILES_VERIFIED")
        .count();
    let attention_disks = disks.len() - verified_disks;
    let report = AuditDocument {
        schema_version: 1,
        scope: "image_and_managed_extraction_integrity_only",
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
        "\u{feff}\"Disk\",\"Attempt\",\"Image\",\"ImageStatus\",\"BadSectors\",\"ImageHashVerified\",\"ExtractionStatus\",\"ExtractedFiles\",\"ExtractedBytes\",\"ExtractedHashesVerified\",\"EvidenceStatus\",\"Issue\"\r\n",
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
                evidence_status: "CHECK".to_owned(),
                issue: "bad \"quote\"".to_owned(),
            }],
        };
        assert!(csv_report(&report).contains("\"bad \"\"quote\"\"\""));
    }
}
