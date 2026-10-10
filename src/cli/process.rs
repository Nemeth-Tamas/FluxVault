//! Shared saved-image processing for process and the whole-run finalizer.
use super::CliResponse;
use crate::{
    conversion_run,
    external_tools::{self, ToolKind},
    pipeline::{self, PipelineRequest, PipelineResult},
    project::ProjectState,
};
use serde_json::json;
use std::path::Path;

pub(super) fn request(project: &ProjectState, workers: usize) -> Result<PipelineRequest, String> {
    crate::cancellation::check()?;
    let settings = project.tool_settings()?;
    let command_audit_path = project.logs_dir().join("external-tools.jsonl");
    let seven_zip_executable = external_tools::find_ready_tool(
        ToolKind::SevenZip,
        settings.path(ToolKind::SevenZip),
        &command_audit_path,
    )?;
    let libreoffice_executable = external_tools::find_ready_tool(
        ToolKind::LibreOffice,
        settings.path(ToolKind::LibreOffice),
        &command_audit_path,
    )?;
    crate::cancellation::check()?;
    Ok(PipelineRequest {
        project: project.clone(),
        seven_zip_executable,
        libreoffice_executable,
        command_audit_path,
        conversion_workers: workers,
    })
}

pub(super) fn run(
    project: &ProjectState,
    workers: usize,
    json_output: bool,
) -> Result<CliResponse, String> {
    let _control = crate::run_control::Session::start(project, "process")?;
    let result = pipeline::run_pipeline(&request(project, workers)?, &|message| {
        eprintln!("{message}")
    })?;
    Ok(response(&result, &project.reports_dir(), json_output))
}

pub(super) fn response(
    result: &PipelineResult,
    reports_directory: &Path,
    json_output: bool,
) -> CliResponse {
    let conversion_state = conversion_run::snapshot_path(reports_directory);
    let needs_attention = result.audit.attention_disks > 0
        || result.raw_format_exceptions > 0
        || result.extraction.recovery_disks > 0
        || result.declined_composites > 0
        || result.declined_recovery_publications > 0
        || result.conversion.partial > 0
        || result.conversion.failed > 0;
    let output = if json_output {
        json!({"disks": result.extraction.total_disks,
            "raw_format_exceptions":result.raw_format_exceptions,
            "composited_disks": result.composited_disks,
            "composites_reused": result.reused_composites,
            "composites_declined": result.declined_composites,
            "published_recovery_images": result.published_recovery_images,
            "reused_recovery_images": result.reused_recovery_images,
            "declined_recovery_publications": result.declined_recovery_publications,
            "recovery_decisions": result.recovery_decisions_path,
            "mirrored_fat_derived_disks": result.reconstructed_disks,
            "mirrored_fat_reused": result.reused_reconstructions,
            "extracted": result.extraction.extracted_disks,
            "recovery_queue": result.extraction.recovery_disks,
            "converted_ok": result.conversion.ok,
            "converted_partial": result.conversion.partial,
            "converted_failed": result.conversion.failed,
            "converted_retried_outputs": result.conversion.retried_outputs,
            "conversion_state": conversion_state,
            "evidence_verified": result.audit.verified_disks,
            "evidence_attention": result.audit.attention_disks,
            "workbook": result.workbook_path,
            "customer_delivery_certified": false})
        .to_string()
    } else {
        format!(
            "Project processing complete: {} image disks, {} verified evidence sets, {} image disks need attention.\nRaw-only format exceptions: {} (separate from image counts; decoding/recovery still needed).\nComposites: {} derived disk(s), {} reused, {} declined.\nMirrored FAT: {} derived disk(s), {} reused.\nAutomatic DERIVED image handoff: {} published/verified, {} reused, {} declined.\nConversions: {} OK, {} partial, {} failed, {} outputs retried.\nRecovery decisions: {}\nWorkbook: {}\nConversion state: {}",
            result.extraction.total_disks,
            result.audit.verified_disks,
            result.audit.attention_disks,
            result.raw_format_exceptions,
            result.composited_disks,
            result.reused_composites,
            result.declined_composites,
            result.reconstructed_disks,
            result.reused_reconstructions,
            result.published_recovery_images,
            result.reused_recovery_images,
            result.declined_recovery_publications,
            result.conversion.ok,
            result.conversion.partial,
            result.conversion.failed,
            result.conversion.retried_outputs,
            result.recovery_decisions_path.display(),
            result.workbook_path.display(),
            conversion_state.display()
        )
    };
    CliResponse {
        output,
        exit_code: if needs_attention { 3 } else { 0 },
    }
}
