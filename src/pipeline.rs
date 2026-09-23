//! One-button workstation-side processing of already acquired images.
//! Physical source media are never opened by this module.

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};

use crate::{
    audit::{self, AuditResult},
    batch_extraction::{self, BatchExtractionRequest, BatchExtractionResult},
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
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Stage(String),
    Finished(Result<PipelineResult, String>),
}

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub reconstructed_disks: usize,
    pub reused_reconstructions: usize,
    pub extraction: BatchExtractionResult,
    pub conversion: ConversionResult,
    pub audit: AuditResult,
    pub workbook_path: PathBuf,
}

pub fn spawn_pipeline(request: PipelineRequest) -> Receiver<PipelineEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = run_pipeline(&request, &|stage| {
            let _ = sender.send(PipelineEvent::Stage(stage.to_owned()));
        });
        let _ = sender.send(PipelineEvent::Finished(result));
    });
    receiver
}

pub(crate) fn run_pipeline(
    request: &PipelineRequest,
    stage: &impl Fn(&str),
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
    stage("1/5: Planning safe offline recovery from saved images...");
    let mut reconstructed_disks = 0;
    let mut reused_reconstructions = 0;
    for plan in recovery_plan::plan_project(&project.images_dir())? {
        if plan.action != RecoveryAction::ReconstructMirroredFat {
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
    }
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
    stage("3/5: Converting eligible legacy Office files...");
    let conversion = conversion_run::run_conversion(
        &ConversionRequest {
            planning: ConversionPlanningRequest {
                extracted_root: project.extracted_dir(),
                converted_root: project.converted_dir(),
                reports_directory: project.reports_dir(),
            },
            libreoffice_executable: request.libreoffice_executable.clone(),
            command_audit_path: request.command_audit_path.clone(),
            timeout_seconds: 45,
        },
        &|message| stage(&format!("3/5: {message}")),
        &|completed, total| stage(&format!("3/5: {completed}/{total} conversions processed")),
    )?;
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
        reconstructed_disks,
        reused_reconstructions,
        extraction,
        conversion,
        audit,
        workbook_path,
    })
}
