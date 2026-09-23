//! Small, hardware-free CLI foundation. Physical media commands will be added
//! only when they can use the same read-only workflow as the GUI.

use std::{
    env,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    audit,
    external_tools::{self, ToolKind},
    imaging,
    package::{self, PackageRequest},
    pipeline::{self, PipelineRequest},
    project::ProjectState,
};

const HELP: &str = "FluxVault — floppy archiving\n\
Usage:\n\
  fluxvault                         Open the GUI\n\
  fluxvault init [path]             Create a project\n\
  fluxvault status [--project PATH] Show project status\n\
  fluxvault audit [--project PATH]  Verify image/extraction evidence\n\
  fluxvault process [--project PATH]\n\
                                    Extract, convert, audit, and report\n\
  fluxvault package build --destination PATH [--project PATH]\n\
                                    Create and verify an archival ZIP\n\
  fluxvault --help                  Show this help\n\
Options:\n\
  --json                            Output machine-readable JSON\n\
  --project PATH                    Use a specific project instead of searching upward\n\
  --destination PATH                Output folder outside the project";

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
            Ok(output) => {
                println!("{output}");
                0
            }
            Err(message) => {
                eprintln!("FluxVault: {message}");
                2
            }
        },
    )
}

fn run(args: &[String], cwd: &Path) -> Result<String, String> {
    let mut json_output = false;
    let mut project_override: Option<PathBuf> = None;
    let mut destination: Option<PathBuf> = None;
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
            "--help" | "-h" => positional.push("help".to_owned()),
            value if value.starts_with('-') => {
                return Err(format!("Unknown option: {value}\n{HELP}"));
            }
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }

    match positional.first().map(String::as_str) {
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
            let root = match project_override {
                Some(path) if path.is_absolute() => path,
                Some(path) => cwd.join(path),
                None => discover_project(cwd).ok_or("No FluxVault project found in this directory or its parents; use --project PATH")?,
            };
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
        Some("audit") if positional.len() == 1 && destination.is_none() => {
            let root = match project_override {
                Some(path) if path.is_absolute() => path,
                Some(path) => cwd.join(path),
                None => {
                    discover_project(cwd).ok_or("No FluxVault project found; use --project PATH")?
                }
            };
            let project = ProjectState::open_without_session(root)?;
            let result = audit::run_audit(&project, &|stage| eprintln!("{stage}"))?;
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
            let root = match project_override {
                Some(path) if path.is_absolute() => path,
                Some(path) => cwd.join(path),
                None => {
                    discover_project(cwd).ok_or("No FluxVault project found; use --project PATH")?
                }
            };
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
                },
                &|stage| eprintln!("{stage}"),
            )?;
            if json_output {
                Ok(json!({"disks": result.extraction.total_disks,
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
                    "Project processing complete: {} disks, {} verified evidence sets, {} need attention.\nConversions: {} OK, {} partial, {} failed.\nWorkbook: {}",
                    result.extraction.total_disks,
                    result.audit.verified_disks,
                    result.audit.attention_disks,
                    result.conversion.ok,
                    result.conversion.partial,
                    result.conversion.failed,
                    result.workbook_path.display()
                ))
            }
        }
        Some("package") if positional.len() == 2 && positional[1] == "build" => {
            let root = match project_override {
                Some(path) if path.is_absolute() => path,
                Some(path) => cwd.join(path),
                None => {
                    discover_project(cwd).ok_or("No FluxVault project found; use --project PATH")?
                }
            };
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
                    "sha256": result.sha256, "files": result.file_count, "bytes": result.total_bytes}).to_string())
            } else {
                Ok(format!(
                    "Verified package: {}\nFiles: {} | Source bytes: {}\nSHA-256: {}",
                    result.zip_path.display(),
                    result.file_count,
                    result.total_bytes,
                    result.sha256
                ))
            }
        }
        _ => Err(format!("Unsupported command or arguments.\n{HELP}")),
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
        assert!(output.contains("Created project"));
        assert!(root.join("project.json").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
