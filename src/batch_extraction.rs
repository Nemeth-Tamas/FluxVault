use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use chrono::Local;

use crate::{
    dmde_logs::DmdeLogStatus,
    extraction::{self, ExtractionPresence, ExtractionRequest},
    imaging::{self, AttemptSummary},
    legacy_logs::ArchiverLogStatus,
    manifest::{self, ManifestRequest, ManifestResult},
    recovery_backup::{self, RecoveryBackupRequest},
};

#[derive(Debug, Clone)]
pub struct BatchExtractionRequest {
    pub seven_zip_executable: PathBuf,
    pub images_directory: PathBuf,
    pub logs_directory: PathBuf,
    pub extracted_root: PathBuf,
    pub recovery_root: PathBuf,
    pub reports_directory: PathBuf,
    pub command_audit_path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum BatchExtractionEvent {
    Stage(String),
    Progress { completed: usize, total: usize },
    Finished(Result<BatchExtractionResult, String>),
}

#[derive(Debug, Clone)]
pub struct BatchExtractionResult {
    pub total_disks: usize,
    pub extracted_disks: usize,
    pub reused_disks: usize,
    pub manual_disks: usize,
    pub recovery_disks: usize,
    pub in_progress_disks: usize,
    pub zero_file_disks: usize,
    pub summary_path: PathBuf,
    pub broken_path: PathBuf,
    pub manual_path: PathBuf,
    pub in_progress_path: PathBuf,
    pub manifest: ManifestResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchDisposition {
    Extract,
    ManualRecovered,
    InProgress,
    Recovery,
}

#[derive(Debug, Clone)]
struct SummaryRow {
    floppy: String,
    image: String,
    log_type: String,
    status: String,
    bad_sectors: Option<usize>,
    reason: String,
    extracted: bool,
}

pub fn spawn_batch_extraction(request: BatchExtractionRequest) -> Receiver<BatchExtractionEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let result = run_batch_extraction(
            &request,
            &|message| {
                let _ = sender.send(BatchExtractionEvent::Stage(message.to_owned()));
            },
            &|completed, total| {
                let _ = sender.send(BatchExtractionEvent::Progress { completed, total });
            },
        );
        let _ = sender.send(BatchExtractionEvent::Finished(result));
    });

    receiver
}

fn run_batch_extraction(
    request: &BatchExtractionRequest,
    send_stage: &impl Fn(&str),
    send_progress: &impl Fn(usize, usize),
) -> Result<BatchExtractionResult, String> {
    if !request.seven_zip_executable.is_file() {
        return Err(format!(
            "A 7-Zip futtatható fájl nem található: {}",
            request.seven_zip_executable.display()
        ));
    }

    fs::create_dir_all(&request.reports_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a Reports mappát {}: {error}",
            request.reports_directory.display()
        )
    })?;

    send_stage("Projekt acquisition állapotának felmérése...");
    let statistics = imaging::load_project_statistics(&request.images_directory)?;
    let total_disks = statistics.disks.len();
    let mut rows = Vec::with_capacity(total_disks);
    let mut broken = Vec::new();
    let mut manual = Vec::new();
    let mut in_progress = Vec::new();
    let mut extracted_disks = 0usize;
    let mut reused_disks = 0usize;
    let mut zero_file_disks = 0usize;

    for (index, disk) in statistics.disks.iter().enumerate() {
        let disk_number = disk.disk_number;
        send_stage(&format!(
            "Lemez {disk_number:03} feldolgozása ({}/{total_disks})...",
            index + 1
        ));

        let attempts = imaging::load_attempts_for_disk(&request.images_directory, disk_number)?;
        let Some(attempt) = attempts.last() else {
            send_progress(index + 1, total_disks);
            continue;
        };
        let presence = extraction::inspect_extraction_presence(
            &request.extracted_root,
            disk_number,
            attempt.attempt_number,
        )?;
        let disposition = classify_attempt(attempt, &presence);
        let mut row = base_row(disk_number, attempt);

        match disposition {
            BatchDisposition::ManualRecovered => {
                let (file_count, directory) = match presence {
                    ExtractionPresence::ManualRecovery {
                        file_count,
                        output_directory,
                        ..
                    } => (file_count, output_directory),
                    _ => unreachable!("manual disposition requires manual presence"),
                };
                row.status = "MANUALLY RECOVERED".to_owned();
                row.reason = format!(
                    "Operator recovery preserved: {} files in {}",
                    file_count,
                    directory.display()
                );
                row.extracted = true;
                manual.push(row.clone());
            }
            BatchDisposition::InProgress => {
                row.status = "IN PROGRESS".to_owned();
                row.reason =
                    "Acquisition/logging is still in progress; image left untouched".to_owned();
                in_progress.push(row.clone());
            }
            BatchDisposition::Recovery => {
                row.status = "MANUAL".to_owned();
                row.reason = recovery_reason(attempt, &presence);
                let backup_reason = row.reason.clone();
                ensure_backup(
                    request,
                    disk_number,
                    attempt,
                    &backup_reason,
                    send_stage,
                    &mut row,
                );
                broken.push(row.clone());
            }
            BatchDisposition::Extract => {
                let extraction_request = ExtractionRequest {
                    seven_zip_executable: request.seven_zip_executable.clone(),
                    image_path: request.images_directory.join(&attempt.image_file),
                    disk_number,
                    attempt_number: attempt.attempt_number,
                    extracted_root: request.extracted_root.clone(),
                    logs_directory: request.logs_directory.clone(),
                    command_audit_path: request.command_audit_path.clone(),
                };

                match extraction::run_extraction(&extraction_request, send_stage) {
                    Ok(result) if result.file_count > 0 => {
                        row.status = "OK".to_owned();
                        row.reason = if result.reused {
                            "Unchanged verified extraction reused".to_owned()
                        } else {
                            "Extracted successfully".to_owned()
                        };
                        row.extracted = true;
                        extracted_disks += 1;
                        reused_disks += usize::from(result.reused);
                    }
                    Ok(_) => {
                        row.status = "MANUAL".to_owned();
                        row.reason =
                            "Readable image produced zero recovered files; operator review required"
                                .to_owned();
                        zero_file_disks += 1;
                        let backup_reason = row.reason.clone();
                        ensure_backup(
                            request,
                            disk_number,
                            attempt,
                            &backup_reason,
                            send_stage,
                            &mut row,
                        );
                        broken.push(row.clone());
                    }
                    Err(error) => {
                        row.status = "MANUAL".to_owned();
                        row.reason = format!("FAT listing/extraction failed: {error}");
                        let backup_reason = row.reason.clone();
                        ensure_backup(
                            request,
                            disk_number,
                            attempt,
                            &backup_reason,
                            send_stage,
                            &mut row,
                        );
                        broken.push(row.clone());
                    }
                }
            }
        }

        rows.push(row);
        send_progress(index + 1, total_disks);
    }

    rows.sort_by(|left, right| left.floppy.cmp(&right.floppy));
    broken.sort_by(|left, right| left.floppy.cmp(&right.floppy));
    manual.sort_by(|left, right| left.floppy.cmp(&right.floppy));
    in_progress.sort_by(|left, right| left.floppy.cmp(&right.floppy));

    send_stage("Batch extraction jelentések írása...");
    let summary_path = request.reports_directory.join("ExtractionSummary.csv");
    let broken_path = request.reports_directory.join("BrokenForDMDE.txt");
    let manual_path = request.reports_directory.join("ManuallyRecovered.txt");
    let in_progress_path = request.reports_directory.join("ExtractionInProgress.txt");

    write_summary(&summary_path, &rows)?;
    write_list(
        &broken_path,
        "FLOPPIES REQUIRING MANUAL DMDE WORK",
        &broken,
        true,
    )?;
    write_list(
        &manual_path,
        "FLOPPIES MANUALLY RECOVERED / EXTRACTED",
        &manual,
        true,
    )?;
    write_list(
        &in_progress_path,
        "FLOPPIES SKIPPED BECAUSE IMAGING/LOGGING IS STILL IN PROGRESS",
        &in_progress,
        false,
    )?;

    send_stage("MasterFileList.csv újraépítése...");
    let manifest = manifest::build_manifest(
        &ManifestRequest {
            extracted_root: request.extracted_root.clone(),
            images_directory: request.images_directory.clone(),
            reports_directory: request.reports_directory.clone(),
        },
        send_stage,
    )?;

    Ok(BatchExtractionResult {
        total_disks,
        extracted_disks,
        reused_disks,
        manual_disks: manual.len(),
        recovery_disks: broken.len(),
        in_progress_disks: in_progress.len(),
        zero_file_disks,
        summary_path,
        broken_path,
        manual_path,
        in_progress_path,
        manifest,
    })
}

fn classify_attempt(attempt: &AttemptSummary, presence: &ExtractionPresence) -> BatchDisposition {
    if matches!(presence, ExtractionPresence::ManualRecovery { .. }) {
        return BatchDisposition::ManualRecovered;
    }

    if matches!(presence, ExtractionPresence::InvalidAutomatic { .. }) {
        return BatchDisposition::Recovery;
    }

    if attempt
        .parsed_log
        .as_ref()
        .is_some_and(|log| log.status == ArchiverLogStatus::InProgress)
        || attempt
            .parsed_dmde_log
            .as_ref()
            .is_some_and(|log| log.status == DmdeLogStatus::InProgress)
    {
        return BatchDisposition::InProgress;
    }

    if attempt.parsed_log.is_none() && attempt.parsed_dmde_log.is_none() {
        return BatchDisposition::Recovery;
    }

    if attempt.attention_required {
        return BatchDisposition::Recovery;
    }

    BatchDisposition::Extract
}

fn base_row(disk_number: u32, attempt: &AttemptSummary) -> SummaryRow {
    let log_type = if attempt.parsed_log.is_some() {
        "AUTO"
    } else if attempt.parsed_dmde_log.is_some() {
        "DMDE"
    } else {
        "NONE"
    };

    SummaryRow {
        floppy: format!("{disk_number:03}"),
        image: attempt.image_file.clone(),
        log_type: log_type.to_owned(),
        status: String::new(),
        bad_sectors: (attempt.parsed_log.is_some() || attempt.parsed_dmde_log.is_some())
            .then_some(attempt.bad_sectors.len()),
        reason: String::new(),
        extracted: false,
    }
}

fn recovery_reason(attempt: &AttemptSummary, presence: &ExtractionPresence) -> String {
    if let ExtractionPresence::InvalidAutomatic { detail, .. } = presence {
        return format!("Invalid managed extraction: {detail}");
    }
    if attempt.parsed_log.is_none() && attempt.parsed_dmde_log.is_none() {
        return if attempt.log_file.is_empty() || !Path::new(&attempt.log_file).is_file() {
            "Missing acquisition log".to_owned()
        } else {
            "Unrecognized acquisition log".to_owned()
        };
    }
    format!(
        "Non-clean image: {} bad/unreadable sectors ({})",
        attempt.bad_sectors.len(),
        attempt.status
    )
}

fn ensure_backup(
    request: &BatchExtractionRequest,
    disk_number: u32,
    attempt: &AttemptSummary,
    reason: &str,
    send_stage: &impl Fn(&str),
    row: &mut SummaryRow,
) {
    let backup = recovery_backup::ensure_first_backup(
        &RecoveryBackupRequest {
            recovery_root: request.recovery_root.clone(),
            disk_number,
            attempt_number: attempt.attempt_number,
            image_path: request.images_directory.join(&attempt.image_file),
            log_path: (!attempt.log_file.is_empty()).then(|| PathBuf::from(&attempt.log_file)),
            reason: reason.to_owned(),
        },
        send_stage,
    );

    if let Err(error) = backup {
        row.reason = format!("{}; pass1 backup failed: {error}", row.reason);
    }
}

fn write_summary(path: &Path, rows: &[SummaryRow]) -> Result<(), String> {
    let mut text = String::from(
        "\u{feff}\"Floppy\",\"Image\",\"LogType\",\"Status\",\"BadSectors\",\"Reason\",\"Extracted\"\r\n",
    );
    for row in rows {
        text.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\r\n",
            csv(&row.floppy),
            csv(&row.image),
            csv(&row.log_type),
            csv(&row.status),
            row.bad_sectors
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv(&row.reason),
            if row.extracted { "True" } else { "False" }
        ));
    }
    fs::write(path, text)
        .map_err(|error| format!("Nem sikerült kiírni {}: {error}", path.display()))
}

fn write_list(
    path: &Path,
    title: &str,
    rows: &[SummaryRow],
    include_bad: bool,
) -> Result<(), String> {
    let mut lines = vec![
        title.to_owned(),
        format!("Generated: {}", Local::now().format("%Y-%m-%d %H:%M:%S")),
        String::new(),
    ];
    for row in rows {
        if include_bad {
            let bad = row
                .bad_sectors
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_owned());
            lines.push(format!(
                "{} | {} | bad/non-clean sectors: {} | {}",
                row.floppy, row.image, bad, row.reason
            ));
        } else {
            lines.push(format!("{} | {} | {}", row.floppy, row.image, row.reason));
        }
    }
    if rows.is_empty() {
        lines.push("None.".to_owned());
    }
    let text = format!("\u{feff}{}\r\n", lines.join("\r\n"));
    fs::write(path, text)
        .map_err(|error| format!("Nem sikerült kiírni {}: {error}", path.display()))
}

fn csv(value: &str) -> String {
    value.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt() -> AttemptSummary {
        AttemptSummary {
            attempt_number: 1,
            status: "OK".to_owned(),
            timestamp_unix_ms: 0,
            image_file: "001.img".to_owned(),
            metadata_path: PathBuf::new(),
            log_file: "001.log".to_owned(),
            parsed_log: Some(crate::legacy_logs::ParsedArchiverLog {
                status: ArchiverLogStatus::Ok,
                disk_number: Some(1),
                attempt_number: Some(1),
                source: None,
                geometry: Default::default(),
                bad_sectors: Vec::new(),
                retry_failures: 0,
                retry_recovered: 0,
                bytes_written: None,
                sha256: None,
                begin_seen: true,
                end_seen: true,
            }),
            parsed_dmde_log: None,
            legacy_image: true,
            attention_required: false,
            sha256: String::new(),
            total_sectors: 2880,
            retry_recovered_sectors: 0,
            bad_sectors: Vec::new(),
        }
    }

    #[test]
    fn manual_recovery_always_wins() {
        let mut value = attempt();
        value.attention_required = true;
        let presence = ExtractionPresence::ManualRecovery {
            output_directory: PathBuf::from("manual"),
            file_count: 1,
            total_bytes: 1,
        };
        assert_eq!(
            classify_attempt(&value, &presence),
            BatchDisposition::ManualRecovered
        );
    }

    #[test]
    fn clean_completed_attempt_is_extractable() {
        let value = attempt();
        let presence = ExtractionPresence::Missing {
            expected_directory: PathBuf::from("expected"),
        };
        assert_eq!(
            classify_attempt(&value, &presence),
            BatchDisposition::Extract
        );
    }

    #[test]
    fn incomplete_log_is_not_treated_as_broken() {
        let mut value = attempt();
        value.parsed_log.as_mut().unwrap().status = ArchiverLogStatus::InProgress;
        value.attention_required = true;
        let presence = ExtractionPresence::Missing {
            expected_directory: PathBuf::from("expected"),
        };
        assert_eq!(
            classify_attempt(&value, &presence),
            BatchDisposition::InProgress
        );
    }

    #[test]
    fn missing_log_routes_to_recovery() {
        let mut value = attempt();
        value.parsed_log = None;
        value.log_file.clear();
        value.attention_required = true;
        let presence = ExtractionPresence::Missing {
            expected_directory: PathBuf::from("expected"),
        };
        assert_eq!(
            classify_attempt(&value, &presence),
            BatchDisposition::Recovery
        );
    }

    #[test]
    #[ignore = "requires FLUXVAULT_BATCH_TEST_PROJECT_ROOT and FLUXVAULT_TEST_7Z"]
    fn processes_a_real_project_fixture_without_physical_media() {
        let root = PathBuf::from(
            std::env::var("FLUXVAULT_BATCH_TEST_PROJECT_ROOT")
                .expect("FLUXVAULT_BATCH_TEST_PROJECT_ROOT is required"),
        );
        let seven_zip_executable = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_7Z").expect("FLUXVAULT_TEST_7Z is required"),
        );
        let result = run_batch_extraction(
            &BatchExtractionRequest {
                seven_zip_executable,
                images_directory: root.join("Images"),
                logs_directory: root.join("Logs"),
                extracted_root: root.join("Extracted"),
                recovery_root: root.join("Recovery"),
                reports_directory: root.join("Reports"),
                command_audit_path: root.join("Logs").join("external-tools.jsonl"),
            },
            &|_| {},
            &|_, _| {},
        )
        .unwrap();

        assert!(result.total_disks > 0);
        assert!(result.summary_path.is_file());
        assert!(result.broken_path.is_file());
        assert!(result.manual_path.is_file());
        assert!(result.in_progress_path.is_file());
        assert!(result.manifest.path.is_file());
    }
}
