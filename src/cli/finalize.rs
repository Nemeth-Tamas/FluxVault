//! One owned, stoppable saved-image finishing operation; no media access.
#[path = "finalization_state.rs"]
mod state;

use super::{CliResponse, process};
use crate::{
    imaging,
    package::{self, PackageRequest},
    pipeline,
    project::ProjectState,
};
use serde_json::{Value, json};
use state::{Options, Phase, Record};
use std::path::{Path, PathBuf};

pub(super) fn run(
    cwd: &Path,
    project_root: PathBuf,
    destination: PathBuf,
    workers: usize,
    allow_attention: bool,
    json_output: bool,
) -> Result<CliResponse, String> {
    state::workstation(&project_root)?;
    let project = ProjectState::open_without_session(project_root)?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let destination = state::destination(cwd, &root, destination)?;
    execute(
        &project,
        Options {
            destination,
            workers,
            allow_attention,
        },
        json_output,
    )
}

pub(super) fn resume(
    project: &ProjectState,
    allow_attention: bool,
    json_output: bool,
) -> Result<CliResponse, String> {
    // Hold ownership before reading the options, not just before their use.
    let owner = crate::project_work::reserve(project.root())?;
    let record = state::load(project)?.ok_or("No saved finalization to resume")?;
    if record.phase.complete() {
        return Err("Finalization already completed. Its receipt is historical, not a fresh verification; use finalize --destination PATH for a new checked archive".into());
    }
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let mut options = record.options;
    options.destination = state::destination(project.root(), &root, options.destination)?;
    options.allow_attention |= allow_attention;
    execute_owned(project, options, json_output, &owner)
}

pub(super) fn status(project: &ProjectState, json_output: bool) -> Result<CliResponse, String> {
    let record = state::load(project)?;
    let control = crate::run_control::status(project)?;
    let active = control["active"].as_bool() == Some(true)
        && control["record"]["operation"].as_str() == Some("finalize");
    let value = json!({"record":record, "active":active,
        "project_owner_active":crate::project_work::active(project.root())?,
        "physical_media_access":false, "receipt_is_historical_not_current_verification":true,
        "resume_rechecks_inputs_and_outputs":true, "customer_delivery_certified":false});
    Ok(CliResponse {
        exit_code: 0,
        output: if json_output {
            value.to_string()
        } else {
            match record {
                Some(record) => format!(
                    "Finalization: {:?} ({})\nDestination: {}\nSaved receipt only; files/package have not been rechecked by status.\n{}",
                    record.phase,
                    if active { "active" } else { "inactive" },
                    record.options.destination.display(),
                    if record.phase.complete() {
                        "Finished; use finalize --destination PATH for a new checked archive."
                    } else {
                        "Continue offline with: fv finalize resume"
                    }
                ),
                None => "No saved finalization. Use fv finalize --destination PATH.".into(),
            }
        },
    })
}

fn execute(
    project: &ProjectState,
    options: Options,
    json_output: bool,
) -> Result<CliResponse, String> {
    let owner = crate::project_work::reserve(project.root())?;
    execute_owned(project, options, json_output, &owner)
}

fn execute_owned(
    project: &ProjectState,
    options: Options,
    json_output: bool,
    _owner: &std::fs::File,
) -> Result<CliResponse, String> {
    crate::processing::validate_workspace(project)?;
    if imaging::load_project_statistics(&project.images_dir())?.disk_count == 0 {
        return Err("No saved disk images to finalize; no package was created".into());
    }
    let _control = crate::run_control::Session::start(project, "finalize")?;
    let mut record = Record::new(project, options)?;
    record.save(project)?;
    let outcome = finish_owned(project, &mut record, json_output);
    if let Err(error) = &outcome {
        record.phase = if crate::cancellation::stopped(error) {
            Phase::Interrupted
        } else {
            Phase::Failed
        };
        record.error = Some(error.chars().take(4096).collect());
        if let Err(receipt_error) = record.save(project) {
            eprintln!(
                "Could not save finalization failure receipt: {receipt_error}. Earlier receipt/evidence retained."
            );
        }
    }
    outcome
}

fn finish_owned(
    project: &ProjectState,
    record: &mut Record,
    json_output: bool,
) -> Result<CliResponse, String> {
    let workers = record.options.workers;
    let destination = record.options.destination.clone();
    finish_with(
        project,
        record,
        json_output,
        || {
            eprintln!("FINALIZE / processing saved images; no floppy needed");
            let result =
                pipeline::run_pipeline(&process::request(project, workers)?, &|message| {
                    eprintln!("{message}")
                })?;
            Ok(process::response(&result, &project.reports_dir(), true))
        },
        || {
            let result = package::build_package(
                &PackageRequest {
                    project_root: project.root().to_path_buf(),
                    destination,
                    project_name: project.name().to_owned(),
                },
                &|message| eprintln!("{message}"),
            )?;
            Ok(
                json!({"zip":result.zip_path,"sha256_file":result.sha256_path,"sha256":result.sha256,
            "files":result.file_count,"bytes":result.total_bytes,"customer_delivery_certified":false}),
            )
        },
    )
}

fn finish_with(
    project: &ProjectState,
    record: &mut Record,
    json_output: bool,
    process: impl FnOnce() -> Result<CliResponse, String>,
    package: impl FnOnce() -> Result<Value, String>,
) -> Result<CliResponse, String> {
    crate::cancellation::check()?;
    let process = process()?;
    if !matches!(process.exit_code, 0 | 3) {
        return Err("Processing did not complete; no package was created".into());
    }
    let process_json: Value = serde_json::from_str(&process.output).map_err(|e| e.to_string())?;
    let attention = process.exit_code == 3;
    record.processing = Some(process_json.clone());
    if attention && !record.options.allow_attention {
        record.phase = Phase::BlockedByAttention;
        record.save(project)?;
        return Ok(response(process_json, None, attention, false, json_output));
    }
    crate::cancellation::check()?;
    if attention {
        eprintln!(
            "FINALIZE / ATTENTION: explicitly archiving partial results with reports; NOT customer-certified"
        );
    }
    record.phase = Phase::Packaging;
    record.save(project)?;
    // Snapshot stays held during inventory, streaming and verification. The
    // processing owner is held for the entire command, including tool checks.
    let _snapshot = crate::project_work::snapshot(project.root())?;
    let package_json = package()?;
    record.package = Some(package_json.clone());
    record.phase = if attention {
        Phase::CompleteAttention
    } else {
        Phase::CompleteClean
    };
    record.save(project)?;
    Ok(response(
        process_json,
        Some(package_json),
        attention,
        record.options.allow_attention,
        json_output,
    ))
}

fn response(
    process: Value,
    package: Option<Value>,
    attention: bool,
    allow_attention: bool,
    json_output: bool,
) -> CliResponse {
    let status = if package.is_some() {
        if attention {
            "verified_archival_zip_with_attention"
        } else {
            "verified_archival_zip"
        }
    } else {
        "blocked_by_attention"
    };
    let output = if json_output {
        json!({"processing":process,"package":package,"package_status":status,
            "allow_attention":allow_attention,"physical_media_access":false,
            "customer_delivery_certified":false})
        .to_string()
    } else if let Some(package) = package {
        format!(
            "Finalization complete{}; verified archival ZIP, NOT customer-certified.\n{}\nSHA-256: {}\nReport: {}",
            if attention { " WITH ATTENTION" } else { "" },
            package["zip"].as_str().unwrap_or(""),
            package["sha256"].as_str().unwrap_or(""),
            process["workbook"].as_str().unwrap_or("")
        )
    } else {
        format!(
            "Processing finished with attention; no package was created.\nReport: {}\nTo explicitly archive partial results and their warnings: fv finalize resume --allow-attention",
            process["workbook"].as_str().unwrap_or("")
        )
    };
    CliResponse {
        output,
        exit_code: if attention { 3 } else { 0 },
    }
}

#[cfg(test)]
#[path = "finalize_tests.rs"]
mod tests;
