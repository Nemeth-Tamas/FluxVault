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
    report,
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
    stage("1/4: Extracting eligible images and refreshing the recovered-file manifest...");
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
        &|message| stage(&format!("1/4: {message}")),
        &|completed, total| stage(&format!("1/4: {completed}/{total} disks processed")),
    )?;
    stage("2/4: Converting eligible legacy Office files...");
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
        &|message| stage(&format!("2/4: {message}")),
        &|completed, total| stage(&format!("2/4: {completed}/{total} conversions processed")),
    )?;
    stage("3/4: Auditing source images and managed extracted files...");
    let audit = audit::run_audit(project, &|message| stage(&format!("3/4: {message}")))?;
    stage("4/4: Creating the Hungarian project workbook...");
    let statistics = imaging::load_project_statistics(&project.images_dir())?;
    let workbook_path = report::export_hungarian_report(
        project.name(),
        &project.reports_dir(),
        &project.images_dir(),
        &statistics,
    )?;
    Ok(PipelineResult {
        extraction,
        conversion,
        audit,
        workbook_path,
    })
}
