//! One-button workstation-side processing of already acquired images.
//! Physical source media are never opened by this module.

use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};

use serde::Serialize;

use crate::{
    audit::{self, AuditResult},
    batch_extraction::{self, BatchExtractionRequest, BatchExtractionResult},
    composite::{self, CompositeRequest, CompositeSource},
    conversion::ConversionPlanningRequest,
    conversion_run::{self, ConversionRequest, ConversionResult},
    imaging,
    project::ProjectState,
    recovery_plan::{self, RecoveryAction},
    report,
    sector_recovery::{self, ReconstructionRequest},
};

#[derive(Debug, Clone)]
pub struct PipelineRequest {
    pub project: ProjectState,
    pub seven_zip_executable: PathBuf,
    pub libreoffice_executable: PathBuf,
    pub command_audit_path: PathBuf,
    pub conversion_workers: usize,
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Stage(String),
    Finished(Box<Result<PipelineResult, String>>),
}

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub raw_format_exceptions: usize,
    pub composited_disks: usize,
    pub reused_composites: usize,
    pub declined_composites: usize,
    pub published_recovery_images: usize,
    pub reused_recovery_images: usize,
    pub declined_recovery_publications: usize,
    pub recovery_decisions_path: PathBuf,
    pub reconstructed_disks: usize,
    pub reused_reconstructions: usize,
    pub extraction: BatchExtractionResult,
    pub conversion: ConversionResult,
    pub audit: AuditResult,
    pub workbook_path: PathBuf,
}

#[derive(Serialize)]
struct OfflineRecoveryDecision {
    disk_number: u32,
    best_attempt: u32,
    planned_action: RecoveryAction,
    outcome: String,
    detail: String,
    derived_images: Vec<PathBuf>,
    unresolved_bad_sectors: Option<usize>,
    published_attempt: Option<u32>,
    publication_report: Option<PathBuf>,
    publication_error: Option<String>,
}

#[derive(Default)]
struct PublicationCounts {
    published: usize,
    reused: usize,
    declined: usize,
}

fn publish_recovery(
    project: &ProjectState,
    disk: u32,
    attempts: &[imaging::AttemptSummary],
    composite: Option<&composite::CompositeResult>,
    reconstruction: Option<&sector_recovery::ReconstructionResult>,
    counts: &mut PublicationCounts,
    stage: &impl Fn(&str),
) -> (Option<crate::offline_images::Published>, Option<String>) {
    match crate::offline_images::publish(project, disk, attempts, composite, reconstruction) {
        Ok(p) => {
            counts.published += 1;
            counts.reused += usize::from(p.reused);
            stage(&format!(
                "1/5: Disk {disk:03}: {} DERIVED attempt #{:03} for automatic saved-file processing",
                if p.reused {
                    "verified/reused"
                } else {
                    "published"
                },
                p.attempt
            ));
            (Some(p), None)
        }
        Err(e) => {
            counts.declined += 1;
            stage(&format!(
                "1/5: Disk {disk:03}: derived publication declined; original attempts preserved: {e}"
            ));
            (None, Some(e))
        }
    }
}

pub fn spawn_pipeline(request: PipelineRequest) -> Receiver<PipelineEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = run_pipeline(&request, &|stage| {
            let _ = sender.send(PipelineEvent::Stage(stage.to_owned()));
        });
        let _ = sender.send(PipelineEvent::Finished(Box::new(result)));
    });
    receiver
}

pub(crate) fn run_pipeline(
    request: &PipelineRequest,
    stage: &impl Fn(&str),
) -> Result<PipelineResult, String> {
    run_pipeline_mode(request, stage, false)
}

pub(crate) fn run_pipeline_incremental(
    request: &PipelineRequest,
    stage: &impl Fn(&str),
) -> Result<PipelineResult, String> {
    run_pipeline_mode(request, stage, true)
}

fn run_pipeline_mode(
    request: &PipelineRequest,
    stage: &impl Fn(&str),
    incremental: bool,
) -> Result<PipelineResult, String> {
    if !request.seven_zip_executable.is_file() {
        return Err(format!(
            "7-Zip is unavailable: {}",
            request.seven_zip_executable.display()
        ));
    }
    if !request.libreoffice_executable.is_file() {
        return Err(format!(
            "LibreOffice is unavailable: {}",
            request.libreoffice_executable.display()
        ));
    }
    let project = &request.project;
    let recovery_budget = crate::resource_budget::background(
        crate::resource_budget::Kind::Recovery,
        256 * crate::resource_budget::MIB,
        &[(project.root(), 1024 * crate::resource_budget::MIB)],
        stage,
    )?;
    let snapshot = crate::project_work::snapshot(project.root())?;
    let raw_format_exceptions = crate::flux_recovery::format_exceptions(project)?.len();
    stage("1/5: Planning safe offline recovery from saved images...");
    let mut composited_disks = 0;
    let mut reused_composites = 0;
    let mut declined_composites = 0;
    let mut reconstructed_disks = 0;
    let mut reused_reconstructions = 0;
    let mut decisions = Vec::new();
    let mut publications = PublicationCounts::default();
    for plan in recovery_plan::plan_project(&project.images_dir())? {
        if plan.action == RecoveryAction::CompareAndComposite {
            let attempts =
                imaging::load_attempts_for_disk(&project.images_dir(), plan.disk_number)?;
            let best = attempts
                .iter()
                .find(|attempt| attempt.attempt_number == plan.best_attempt)
                .ok_or_else(|| {
                    format!("Disk {:03} selected attempt disappeared", plan.disk_number)
                })?;
            let sources = attempts
                .iter()
                .filter(|attempt| {
                    attempt.total_sectors == best.total_sectors
                        && attempt.status != "DERIVED"
                        && (attempt.parsed_log.is_some() || attempt.parsed_dmde_log.is_some())
                })
                .map(|attempt| CompositeSource {
                    attempt_number: attempt.attempt_number,
                    image_path: project.images_dir().join(&attempt.image_file),
                    expected_sha256: Some(attempt.sha256.clone()),
                    total_sectors: attempt.total_sectors,
                    bad_sectors: attempt.bad_sectors.clone(),
                })
                .collect::<Vec<_>>();
            stage(&format!(
                "1/5: Disk {:03}: comparing saved attempts before compositing...",
                plan.disk_number
            ));
            let result = composite::run_composite(
                &CompositeRequest {
                    images_directory: project.images_dir(),
                    recovery_root: project.recovery_dir(),
                    disk_number: plan.disk_number,
                    sources,
                },
                &|message| stage(&format!("1/5: {message}")),
            );
            match result {
                Ok(result) => {
                    let mut derived_images = Vec::new();
                    let mut reconstruction_for_publication = None;
                    let mut unresolved = result.unresolved_bad_sectors.len();
                    if let Some(image_path) = &result.derived_image {
                        derived_images.push(image_path.clone());
                        composited_disks += 1;
                        reused_composites += usize::from(result.reused);
                        stage(&format!(
                            "1/5: Disk {:03}: {} sectors from other attempts, {} unresolved{}",
                            plan.disk_number,
                            result.replacements.len(),
                            result.unresolved_bad_sectors.len(),
                            if result.reused {
                                " (verified prior result reused)"
                            } else {
                                ""
                            }
                        ));
                        if !result.unresolved_bad_sectors.is_empty() {
                            let image = fs::read(image_path).map_err(|error| {
                                format!(
                                    "Cannot read derived composite {}: {error}",
                                    image_path.display()
                                )
                            })?;
                            if sector_recovery::inspect_mirrored_fat(
                                &image,
                                &result.unresolved_bad_sectors,
                            )
                            .is_ok_and(|(recoverable, _)| recoverable > 0)
                            {
                                let reconstruction = sector_recovery::run_reconstruction(
                                    &ReconstructionRequest {
                                        image_path: image_path.clone(),
                                        expected_sha256: result.derived_sha256.clone(),
                                        recovery_root: project.recovery_dir(),
                                        disk_number: plan.disk_number,
                                        attempt_number: result.base_attempt,
                                        bad_sectors: result.unresolved_bad_sectors.clone(),
                                    },
                                    &|message| stage(&format!("1/5: {message}")),
                                )?;
                                reconstructed_disks +=
                                    usize::from(reconstruction.derived_image.is_some());
                                reused_reconstructions += usize::from(reconstruction.reused);
                                unresolved = reconstruction.unresolved_bad_sectors.len();
                                if reconstruction.derived_image.is_some() {
                                    reconstruction_for_publication = Some(reconstruction.clone());
                                }
                                if let Some(derived) = reconstruction.derived_image {
                                    derived_images.push(derived);
                                }
                            }
                        }
                    }
                    let (publication, publication_error) = if result.derived_image.is_some() {
                        let eligible = attempts
                            .iter()
                            .filter(|a| {
                                a.status != "DERIVED"
                                    && a.total_sectors == best.total_sectors
                                    && (a.parsed_log.is_some() || a.parsed_dmde_log.is_some())
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        let (published, error) = publish_recovery(
                            project,
                            plan.disk_number,
                            &eligible,
                            Some(&result),
                            reconstruction_for_publication.as_ref(),
                            &mut publications,
                            stage,
                        );
                        if let Some(p) = &published {
                            derived_images.push(p.image.clone());
                        }
                        (published, error)
                    } else {
                        (None, None)
                    };
                    decisions.push(OfflineRecoveryDecision {
                        disk_number: plan.disk_number,
                        best_attempt: plan.best_attempt,
                        planned_action: plan.action,
                        outcome: if result.derived_image.is_none() {
                            "no_change"
                        } else if result.reused {
                            "reused"
                        } else {
                            "derived"
                        }.to_owned(),
                        detail: format!(
                            "{} sectors copied from another attempt; {unresolved} remain unresolved",
                            result.replacements.len()
                        ),
                        derived_images,
                        unresolved_bad_sectors: Some(unresolved),
                        published_attempt: publication.as_ref().map(|p| p.attempt),
                        publication_report: publication.map(|p| p.report),
                        publication_error,
                    });
                }
                Err(error) => {
                    declined_composites += 1;
                    stage(&format!(
                        "1/5: Disk {:03}: composite declined; {}",
                        plan.disk_number, error
                    ));
                    decisions.push(OfflineRecoveryDecision {
                        disk_number: plan.disk_number,
                        best_attempt: plan.best_attempt,
                        planned_action: plan.action,
                        outcome: "declined".to_owned(),
                        detail: error,
                        derived_images: Vec::new(),
                        unresolved_bad_sectors: Some(plan.best_bad_sectors),
                        published_attempt: None,
                        publication_report: None,
                        publication_error: None,
                    });
                }
            }
            continue;
        }
        if plan.action != RecoveryAction::ReconstructMirroredFat {
            decisions.push(OfflineRecoveryDecision {
                disk_number: plan.disk_number,
                best_attempt: plan.best_attempt,
                planned_action: plan.action,
                outcome: "not_run".to_owned(),
                detail: plan.reason,
                derived_images: Vec::new(),
                unresolved_bad_sectors: Some(plan.best_bad_sectors),
                published_attempt: None,
                publication_report: None,
                publication_error: None,
            });
            continue;
        }
        let attempts = imaging::load_attempts_for_disk(&project.images_dir(), plan.disk_number)?;
        let attempt = attempts
            .iter()
            .find(|attempt| attempt.attempt_number == plan.best_attempt)
            .ok_or_else(|| format!("Disk {:03} selected attempt disappeared", plan.disk_number))?;
        stage(&format!(
            "1/5: Disk {:03}: deriving mirrored FAT evidence from saved image...",
            plan.disk_number
        ));
        let result = sector_recovery::run_reconstruction(
            &ReconstructionRequest {
                image_path: project.images_dir().join(&attempt.image_file),
                expected_sha256: Some(attempt.sha256.clone()),
                recovery_root: project.recovery_dir(),
                disk_number: plan.disk_number,
                attempt_number: attempt.attempt_number,
                bad_sectors: attempt.bad_sectors.clone(),
            },
            &|message| stage(&format!("1/5: {message}")),
        )?;
        if result.derived_image.is_some() {
            reconstructed_disks += 1;
            reused_reconstructions += usize::from(result.reused);
            stage(&format!(
                "1/5: Disk {:03}: {} FAT sector(s) reconstructed, {} still unresolved{}",
                plan.disk_number,
                result.reconstructed.len(),
                result.unresolved_bad_sectors.len(),
                if result.reused {
                    " (verified prior result reused)"
                } else {
                    ""
                }
            ));
        }
        let (publication, publication_error) = if result.derived_image.is_some() {
            publish_recovery(
                project,
                plan.disk_number,
                std::slice::from_ref(attempt),
                None,
                Some(&result),
                &mut publications,
                stage,
            )
        } else {
            (None, None)
        };
        decisions.push(OfflineRecoveryDecision {
            disk_number: plan.disk_number,
            best_attempt: plan.best_attempt,
            planned_action: plan.action,
            outcome: if result.derived_image.is_none() {
                "no_change"
            } else if result.reused {
                "reused"
            } else {
                "derived"
            }
            .to_owned(),
            detail: format!(
                "{} mirrored FAT sectors reconstructed; {} remain unresolved",
                result.reconstructed.len(),
                result.unresolved_bad_sectors.len()
            ),
            derived_images: result
                .derived_image
                .into_iter()
                .chain(publication.as_ref().map(|p| p.image.clone()))
                .collect(),
            unresolved_bad_sectors: Some(result.unresolved_bad_sectors.len()),
            published_attempt: publication.as_ref().map(|p| p.attempt),
            publication_report: publication.map(|p| p.report),
            publication_error,
        });
    }
    let recovery_decisions_path = project.reports_dir().join("OfflineRecoveryDecisions.json");
    let report = serde_json::json!({
        "schema_version": 1,
        "customer_delivery_certified": false,
        "decisions": decisions,
    });
    fs::write(
        &recovery_decisions_path,
        serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("Cannot serialize offline recovery decisions: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "Cannot write {}: {error}",
            recovery_decisions_path.display()
        )
    })?;
    stage("2/5: Extracting eligible images and refreshing the recovered-file manifest...");
    let extraction = batch_extraction::run_batch_extraction(
        &BatchExtractionRequest {
            seven_zip_executable: request.seven_zip_executable.clone(),
            images_directory: project.images_dir(),
            logs_directory: project.logs_dir(),
            extracted_root: project.extracted_dir(),
            recovery_root: project.recovery_dir(),
            reports_directory: project.reports_dir(),
            command_audit_path: request.command_audit_path.clone(),
        },
        &|message| stage(&format!("2/5: {message}")),
        &|completed, total| stage(&format!("2/5: {completed}/{total} disks processed")),
    )?;
    drop(snapshot); // Long Office work must not block the next image publication.
    drop(recovery_budget); // Never nest a pipeline permit with Office admission.
    stage("3/5: Converting eligible legacy Office files...");
    let conversion = conversion_run::run_conversion_mode(
        &ConversionRequest {
            planning: ConversionPlanningRequest {
                extracted_root: project.extracted_dir(),
                converted_root: project.converted_dir(),
                reports_directory: project.reports_dir(),
            },
            libreoffice_executable: request.libreoffice_executable.clone(),
            command_audit_path: request.command_audit_path.clone(),
            timeout_seconds: 45,
            workers: request.conversion_workers,
            selected_sources: None,
            previous_result: None,
        },
        &|message| stage(&format!("3/5: {message}")),
        &|completed, total| stage(&format!("3/5: {completed}/{total} conversions processed")),
        incremental,
    )?;
    let _audit_budget = crate::resource_budget::background(
        crate::resource_budget::Kind::Recovery,
        256 * crate::resource_budget::MIB,
        &[(project.root(), 64 * crate::resource_budget::MIB)],
        stage,
    )?;
    let _snapshot = crate::project_work::snapshot(project.root())?;
    stage("4/5: Auditing source images and managed extracted files...");
    let audit = audit::run_audit(project, &|message| stage(&format!("4/5: {message}")))?;
    stage("5/5: Creating the Hungarian project workbook...");
    let statistics = imaging::load_project_statistics(&project.images_dir())?;
    let workbook_path = report::export_hungarian_report(
        project.name(),
        &project.reports_dir(),
        &project.images_dir(),
        &statistics,
    )?;
    Ok(PipelineResult {
        raw_format_exceptions,
        composited_disks,
        reused_composites,
        declined_composites,
        published_recovery_images: publications.published,
        reused_recovery_images: publications.reused,
        declined_recovery_publications: publications.declined,
        recovery_decisions_path,
        reconstructed_disks,
        reused_reconstructions,
        extraction,
        conversion,
        audit,
        workbook_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn saved_attempts_are_composited_once_and_reused_on_next_process_run() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-pipeline-composite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let images = project.images_dir();
        let logs = project.logs_dir();
        let mut first = vec![0u8; 3 * 512];
        let mut second = first.clone();
        first[2 * 512..3 * 512].fill(0x22);
        second[512..2 * 512].fill(0x11);

        for (number, bytes, bad_lba) in [(1, first, 1), (2, second, 2)] {
            let image = images.join(format!("001_attempt_{number:03}.img"));
            let log = logs.join(format!("001_attempt_{number:03}.log"));
            let hash = format!("{:x}", Sha256::digest(&bytes));
            fs::write(&image, &bytes).unwrap();
            fs::write(&log, format!(
                "BEGIN | disk=001 | attempt={number:03}\nGEOMETRY | cylinders=1 | heads=1 | sectors_per_track=3 | bytes_per_sector=512 | total_sectors=3 | total_bytes=1536\nBAD_SECTOR | LBA={bad_lba} | zero_filled=true\nEND | status=PARTIAL | bad_sectors=1 | retry_recovered=0 | bytes=1536 | sha256={hash}\n"
            )).unwrap();
            let metadata = serde_json::json!({
                "fluxvault_version": "0.1.0", "status": "PARTIAL",
                "disk_number": 1, "attempt_number": number,
                "source_backend": "synthetic-test", "source_device": "none",
                "image_file": image, "log_file": log,
                "timestamp_unix_ms": number,
                "geometry": {"cylinders": 1, "heads": 1, "sectors_per_track": 3,
                    "bytes_per_sector": 512, "total_bytes": 1536, "format_guess": "fixture"},
                "sector_retries": 0, "total_sectors": 3, "bytes_written": 1536,
                "retry_recovered_sectors": 0, "bad_sector_count": 1,
                "bad_sectors": [{"lba": bad_lba, "cylinder": 0, "head": 0, "sector": bad_lba + 1}],
                "sha256": hash,
            });
            fs::write(
                images.join(format!("001_attempt_{number:03}.json")),
                serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        }
        let fake_tool = root.join("unused-tool.exe");
        fs::write(&fake_tool, []).unwrap();
        let request = PipelineRequest {
            project,
            seven_zip_executable: fake_tool.clone(),
            libreoffice_executable: fake_tool,
            command_audit_path: root.join("Logs").join("tools.jsonl"),
            conversion_workers: conversion_run::DEFAULT_CONVERSION_WORKERS,
        };
        let first = run_pipeline(&request, &|_| {}).unwrap();
        assert_eq!(first.composited_disks, 1);
        assert_eq!(first.reused_composites, 0);
        assert_eq!(first.declined_composites, 0);
        assert_eq!(first.extraction.recovery_disks, 1);
        let first_decisions: serde_json::Value =
            serde_json::from_slice(&fs::read(&first.recovery_decisions_path).unwrap()).unwrap();
        assert_eq!(first_decisions["decisions"][0]["outcome"], "derived");
        let second = run_pipeline(&request, &|_| {}).unwrap();
        assert_eq!(second.composited_disks, 1);
        assert_eq!(second.reused_composites, 1);
        let second_decisions: serde_json::Value =
            serde_json::from_slice(&fs::read(&second.recovery_decisions_path).unwrap()).unwrap();
        assert_eq!(second_decisions["decisions"][0]["outcome"], "reused");
        assert_eq!(
            fs::read_dir(root.join("Recovery").join("001"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "img"))
                .count(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }
}
