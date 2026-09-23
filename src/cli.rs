//! Small, hardware-free CLI foundation. Physical media commands will be added
//! only when they can use the same read-only workflow as the GUI.

use std::{
    env,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    audit,
    conversion_run::DEFAULT_CONVERSION_WORKERS,
    external_tools::{self, ToolKind},
    imaging,
    package::{self, PackageRequest},
    pipeline::{self, PipelineRequest},
    project::ProjectState,
    recovery_plan::{self, RecoveryAction},
};

const HELP: &str = "FluxVault — floppy archiving\n\
Usage:\n\
  fluxvault                         Open the GUI\n\
  fluxvault init [path]             Create a project\n\
  fluxvault status [--project PATH] Show project status\n\
  fluxvault disk list [--project PATH]\n\
  fluxvault disk show N [--project PATH]\n\
                                    Inspect saved disk attempts\n\
  fluxvault recovery plan [N] [--project PATH]\n\
                                    Inspect evidence-ranked offline next steps\n\
  fluxvault audit [--project PATH]  Verify image/extraction evidence\n\
  fluxvault process [--project PATH] [--conversion-workers N]\n\
                                    Extract, convert, audit, and report\n\
  fluxvault package build --destination PATH [--project PATH]\n\
                                    Create and verify an archival ZIP\n\
  fluxvault --help                  Show this help\n\
Options:\n\
  --json                            Output machine-readable JSON\n\
  --project PATH                    Use a specific project instead of searching upward\n\
  --destination PATH                Output folder outside the project\n\
  --conversion-workers N            Parallel Office files during process (1-16; default 4)\n\
Exit codes: 0 complete, 3 attention/partial, 2 invalid input or operation error";

struct CliResponse {
    output: String,
    exit_code: i32,
}

pub fn run_from_env() -> Option<i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return None;
    }
    Some(
        match run(
            &args,
            &env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ) {
            Ok(response) => {
                println!("{}", response.output);
                response.exit_code
            }
            Err(message) => {
                eprintln!("FluxVault: {message}");
                2
            }
        },
    )
}

fn run(args: &[String], cwd: &Path) -> Result<CliResponse, String> {
    let mut json_output = false;
    let mut project_override: Option<PathBuf> = None;
    let mut destination: Option<PathBuf> = None;
    let mut conversion_workers: Option<usize> = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json_output = true,
            "--project" => {
                index += 1;
                let value = args.get(index).ok_or("--project requires a path")?;
                project_override = Some(PathBuf::from(value));
            }
            "--destination" => {
                index += 1;
                let value = args.get(index).ok_or("--destination requires a path")?;
                destination = Some(PathBuf::from(value));
            }
            "--conversion-workers" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("--conversion-workers requires a number")?;
                let workers = value
                    .parse::<usize>()
                    .map_err(|_| "--conversion-workers must be a number from 1 to 16")?;
                if !(1..=16).contains(&workers) {
                    return Err("--conversion-workers must be a number from 1 to 16".to_owned());
                }
                conversion_workers = Some(workers);
            }
            "--help" | "-h" => positional.push("help".to_owned()),
            value if value.starts_with('-') => {
                return Err(format!("Unknown option: {value}\n{HELP}"));
            }
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }

    if conversion_workers.is_some() && positional.first().map(String::as_str) != Some("process") {
        return Err("--conversion-workers is only valid with process".to_owned());
    }

    let mut needs_attention = false;
    let output = match positional.first().map(String::as_str) {
        Some("help") if positional.len() == 1 => Ok(HELP.to_owned()),
        Some("init") if positional.len() <= 2 && project_override.is_none() => {
            let root = positional
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| cwd.to_path_buf());
            let root = if root.is_absolute() {
                root
            } else {
                cwd.join(root)
            };
            let project = ProjectState::create_without_session(root)?;
            if json_output {
                Ok(
                    json!({"project": project.root(), "name": project.name(), "created": true})
                        .to_string(),
                )
            } else {
                Ok(format!(
                    "Created project {} at {}",
                    project.name(),
                    project.root().display()
                ))
            }
        }
        Some("status") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let stats = imaging::load_project_statistics(&project.images_dir())?;
            if json_output {
                Ok(json!({
                    "project": project.root(),
                    "name": project.name(),
                    "current_disk": project.current_disk_number(),
                    "disks": stats.disk_count,
                    "attempts": stats.total_attempts,
                    "ok_disks": stats.ok_disks,
                    "partial_disks": stats.partial_disks,
                    "best_known_bad_sectors": stats.best_known_bad_sectors,
                })
                .to_string())
            } else {
                Ok(format!(
                    "{} ({})\nCurrent disk: {:03}\nDisks: {} ({} OK, {} partial)\nAttempts: {}\nBest known bad sectors: {}",
                    project.name(),
                    project.root().display(),
                    project.current_disk_number(),
                    stats.disk_count,
                    stats.ok_disks,
                    stats.partial_disks,
                    stats.total_attempts,
                    stats.best_known_bad_sectors
                ))
            }
        }
        Some("disk")
            if destination.is_none() && positional.len() == 2 && positional[1] == "list" =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let stats = imaging::load_project_statistics(&project.images_dir())?;
            if json_output {
                Ok(json!({"project": project.root(), "disks": stats.disks.iter().map(|disk| json!({
                    "number": disk.disk_number, "attempts": disk.attempt_count,
                    "best_attempt": disk.best_attempt_number, "best_bad_sectors": disk.best_bad_sectors,
                    "latest_attempt": disk.latest_attempt_number, "attention_required": disk.attention_required
                })).collect::<Vec<_>>()}).to_string())
            } else if stats.disks.is_empty() {
                Ok("No acquired disks in this project.".to_owned())
            } else {
                Ok(stats
                    .disks
                    .iter()
                    .map(|disk| {
                        format!(
                            "{:03} | {} attempts | best #{:03}: {} bad sectors | {}",
                            disk.disk_number,
                            disk.attempt_count,
                            disk.best_attempt_number,
                            disk.best_bad_sectors,
                            if disk.attention_required {
                                "needs recovery"
                            } else {
                                "OK"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        Some("disk")
            if destination.is_none() && positional.len() == 3 && positional[1] == "show" =>
        {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("disk show requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk_number)?;
            if attempts.is_empty() {
                return Err(format!("No image attempts found for disk {disk_number:03}"));
            }
            if json_output {
                Ok(json!({"project": project.root(), "disk": disk_number, "attempts": attempts.iter().map(|attempt| json!({
                    "number": attempt.attempt_number, "status": attempt.status,
                    "image": attempt.image_file, "sha256": attempt.sha256,
                    "bad_sectors": attempt.bad_sectors, "attention_required": attempt.attention_required
                })).collect::<Vec<_>>()}).to_string())
            } else {
                Ok(format!(
                    "Disk {disk_number:03}\n{}",
                    attempts
                        .iter()
                        .map(|attempt| format!(
                            "  #{:03} | {} | {} bad sectors | {}",
                            attempt.attempt_number,
                            attempt.status,
                            attempt.bad_sectors.len(),
                            attempt.image_file
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
            }
        }
        Some("recovery")
            if destination.is_none()
                && (positional.len() == 2 || positional.len() == 3)
                && positional[1] == "plan" =>
        {
            let requested_disk = positional
                .get(2)
                .map(|text| {
                    text.parse::<u32>()
                        .ok()
                        .filter(|number| *number > 0)
                        .ok_or("recovery plan requires a positive disk number")
                })
                .transpose()?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let plans = recovery_plan::plan_project(&project.images_dir())?;
            let plans = plans
                .into_iter()
                .filter(|plan| requested_disk.is_none_or(|number| plan.disk_number == number))
                .collect::<Vec<_>>();
            if let Some(number) = requested_disk
                && plans.is_empty()
            {
                return Err(format!("No saved disk {number:03} in this project"));
            }
            needs_attention = plans
                .iter()
                .any(|plan| plan.action != RecoveryAction::Complete);
            if json_output {
                Ok(json!({"project": project.root(), "plans": plans}).to_string())
            } else if plans.is_empty() {
                Ok("No saved disks in this project.".to_owned())
            } else {
                Ok(plans
                    .iter()
                    .map(|plan| {
                        format!(
                            "{:03} | {:?} | {} bad sectors | composite candidates: {} | mirrored FAT candidates: {}\n  {}",
                            plan.disk_number,
                            plan.action,
                            plan.best_bad_sectors,
                            plan.composite_candidate_sectors,
                            plan.mirrored_fat_candidate_sectors,
                            plan.reason
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        Some("audit") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let result = audit::run_audit(&project, &|stage| eprintln!("{stage}"))?;
            needs_attention = result.attention_disks > 0;
            if json_output {
                Ok(json!({"json": result.json_path, "csv": result.csv_path, "disks": result.disk_count,
                    "verified": result.verified_disks, "attention": result.attention_disks,
                    "customer_delivery_certified": false}).to_string())
            } else {
                Ok(format!(
                    "Evidence audit: {} of {} disk evidence sets verified; {} need attention.\nReport: {}",
                    result.verified_disks,
                    result.disk_count,
                    result.attention_disks,
                    result.csv_path.display()
                ))
            }
        }
        Some("process") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let settings = external_tools::load_settings()?;
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
            let result = pipeline::run_pipeline(
                &PipelineRequest {
                    project,
                    seven_zip_executable,
                    libreoffice_executable,
                    command_audit_path,
                    conversion_workers: conversion_workers.unwrap_or(DEFAULT_CONVERSION_WORKERS),
                },
                &|stage| eprintln!("{stage}"),
            )?;
            needs_attention = result.audit.attention_disks > 0
                || result.extraction.recovery_disks > 0
                || result.declined_composites > 0
                || result.conversion.partial > 0
                || result.conversion.failed > 0;
            if json_output {
                Ok(json!({"disks": result.extraction.total_disks,
                    "composited_disks": result.composited_disks,
                    "composites_reused": result.reused_composites,
                    "composites_declined": result.declined_composites,
                    "recovery_decisions": result.recovery_decisions_path,
                    "mirrored_fat_derived_disks": result.reconstructed_disks,
                    "mirrored_fat_reused": result.reused_reconstructions,
                    "extracted": result.extraction.extracted_disks,
                    "recovery_queue": result.extraction.recovery_disks,
                    "converted_ok": result.conversion.ok,
                    "converted_partial": result.conversion.partial,
                    "converted_failed": result.conversion.failed,
                    "evidence_verified": result.audit.verified_disks,
                    "evidence_attention": result.audit.attention_disks,
                    "workbook": result.workbook_path,
                    "customer_delivery_certified": false})
                .to_string())
            } else {
                Ok(format!(
                    "Project processing complete: {} disks, {} verified evidence sets, {} need attention.\nComposites: {} derived disk(s), {} reused, {} declined.\nMirrored FAT: {} derived disk(s), {} reused.\nConversions: {} OK, {} partial, {} failed.\nRecovery decisions: {}\nWorkbook: {}",
                    result.extraction.total_disks,
                    result.audit.verified_disks,
                    result.audit.attention_disks,
                    result.composited_disks,
                    result.reused_composites,
                    result.declined_composites,
                    result.reconstructed_disks,
                    result.reused_reconstructions,
                    result.conversion.ok,
                    result.conversion.partial,
                    result.conversion.failed,
                    result.recovery_decisions_path.display(),
                    result.workbook_path.display()
                ))
            }
        }
        Some("package") if positional.len() == 2 && positional[1] == "build" => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let destination = destination.ok_or("package build requires --destination PATH")?;
            let destination = if destination.is_absolute() {
                destination
            } else {
                cwd.join(destination)
            };
            let result = package::build_package(
                &PackageRequest {
                    project_root: project.root().to_path_buf(),
                    destination,
                    project_name: project.name().to_owned(),
                },
                &|stage| eprintln!("{stage}"),
            )?;
            if json_output {
                Ok(json!({"zip": result.zip_path, "sha256_file": result.sha256_path,
                    "sha256": result.sha256, "files": result.file_count, "bytes": result.total_bytes,
                    "customer_delivery_certified": false}).to_string())
            } else {
                Ok(format!(
                    "Verified archival ZIP (not customer-certified): {}\nFiles: {} | Source bytes: {}\nSHA-256: {}",
                    result.zip_path.display(),
                    result.file_count,
                    result.total_bytes,
                    result.sha256
                ))
            }
        }
        _ => Err(format!("Unsupported command or arguments.\n{HELP}")),
    }?;
    Ok(CliResponse {
        output,
        exit_code: if needs_attention { 3 } else { 0 },
    })
}

fn resolve_project_root(cwd: &Path, project_override: Option<&Path>) -> Result<PathBuf, String> {
    match project_override {
        Some(path) if path.is_absolute() => Ok(path.to_path_buf()),
        Some(path) => Ok(cwd.join(path)),
        None => discover_project(cwd).ok_or_else(|| {
            "No FluxVault project found in this directory or its parents; use --project PATH"
                .to_owned()
        }),
    }
}

fn discover_project(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|directory| directory.join("project.json").is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_project_from_nested_directory() {
        let temp = env::temp_dir().join(format!("fluxvault-cli-discovery-{}", std::process::id()));
        // Never delete an existing directory, even if an earlier test run left it behind.
        std::fs::create_dir(&temp).unwrap();
        let nested = temp.join("Extracted").join("007");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(temp.join("project.json"), "{}").unwrap();
        assert_eq!(discover_project(&nested), Some(temp.clone()));
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn rejects_unknown_commands_without_opening_gui() {
        assert!(run(&["acquire".to_owned()], Path::new(".")).is_err());
    }

    #[test]
    fn rejects_invalid_conversion_worker_options_before_project_access() {
        for workers in ["0", "17", "oops"] {
            assert!(
                run(
                    &[
                        "process".to_owned(),
                        "--conversion-workers".to_owned(),
                        workers.to_owned()
                    ],
                    Path::new("."),
                )
                .is_err()
            );
        }
        assert!(
            run(
                &[
                    "status".to_owned(),
                    "--conversion-workers".to_owned(),
                    "2".to_owned()
                ],
                Path::new("."),
            )
            .is_err()
        );
    }

    #[test]
    fn init_creates_a_project_without_gui_session_state() {
        let root = env::temp_dir().join(format!(
            "fluxvault-cli-init-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = run(
            &["init".to_owned(), root.display().to_string()],
            Path::new("."),
        )
        .unwrap();
        assert!(output.output.contains("Created project"));
        assert_eq!(output.exit_code, 0);
        assert!(root.join("project.json").is_file());
        let list = run(
            &[
                "disk".to_owned(),
                "list".to_owned(),
                "--json".to_owned(),
                "--project".to_owned(),
                root.display().to_string(),
            ],
            Path::new("."),
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&list.output).unwrap();
        assert_eq!(json["disks"].as_array().unwrap().len(), 0);
        assert!(
            run(
                &[
                    "disk".to_owned(),
                    "show".to_owned(),
                    "1".to_owned(),
                    "--project".to_owned(),
                    root.display().to_string()
                ],
                Path::new(".")
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
