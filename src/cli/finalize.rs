//! Image-only finishing workflow: process evidence, then package only a clean run.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{imaging, project::ProjectState};

use super::{CliResponse, run as run_cli};

pub(super) fn run(
    cwd: &Path,
    project_root: PathBuf,
    destination: PathBuf,
    workers: usize,
    json_output: bool,
) -> Result<CliResponse, String> {
    let project = ProjectState::open_without_session(project_root)?;
    let root = project
        .root()
        .canonicalize()
        .map_err(|error| format!("Cannot resolve project directory: {error}"))?;
    let destination = if destination.is_absolute() {
        destination
    } else {
        cwd.join(destination)
    };
    let destination = destination.canonicalize().map_err(|error| {
        format!(
            "Package destination must be an existing directory: {} ({error})",
            destination.display()
        )
    })?;
    if !destination.is_dir() || destination.starts_with(&root) {
        return Err(
            "Package destination must be an existing folder outside the project".to_owned(),
        );
    }
    if imaging::load_project_statistics(&project.images_dir())?.disk_count == 0 {
        return Err("No saved disk images to finalize; no package was created".to_owned());
    }

    let process = run_cli(
        &[
            "process".to_owned(),
            "--project".to_owned(),
            root.display().to_string(),
            "--conversion-workers".to_owned(),
            workers.to_string(),
            "--json".to_owned(),
        ],
        cwd,
    )?;
    finish(process, json_output, || {
        run_cli(
            &[
                "package".to_owned(),
                "build".to_owned(),
                "--project".to_owned(),
                root.display().to_string(),
                "--destination".to_owned(),
                destination.display().to_string(),
                "--json".to_owned(),
            ],
            cwd,
        )
    })
}

fn finish(
    process: CliResponse,
    json_output: bool,
    build_package: impl FnOnce() -> Result<CliResponse, String>,
) -> Result<CliResponse, String> {
    let process_json: Value = serde_json::from_str(&process.output)
        .map_err(|error| format!("Invalid internal process result: {error}"))?;
    if process.exit_code == 3 {
        return Ok(CliResponse {
            output: if json_output {
                json!({
                    "processing": process_json,
                    "package": null,
                    "package_status": "blocked_by_attention",
                    "customer_delivery_certified": false
                })
                .to_string()
            } else {
                format!(
                    "Processing finished with unresolved attention; no package was created.\n{}",
                    process.output
                )
            },
            exit_code: 3,
        });
    }
    if process.exit_code != 0 {
        return Err("Processing did not complete; no package was created".to_owned());
    }
    let package = build_package()?;
    let package_json: Value = serde_json::from_str(&package.output)
        .map_err(|error| format!("Invalid internal package result: {error}"))?;
    Ok(CliResponse {
        output: if json_output {
            json!({
                "processing": process_json,
                "package": package_json,
                "package_status": "verified_archival_zip",
                "customer_delivery_certified": false
            })
            .to_string()
        } else {
            format!("{}\n{}", process.output, package.output)
        },
        exit_code: package.exit_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn finalize_refuses_empty_project_and_project_internal_destination_before_tools() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-finalize-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        assert!(
            run(&root, project.root().to_path_buf(), outside, 1, true)
                .unwrap_err()
                .contains("No saved disk images")
        );
        assert!(
            run(
                &root,
                project.root().to_path_buf(),
                project.reports_dir(),
                1,
                true
            )
            .unwrap_err()
            .contains("outside the project")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unresolved_processing_never_starts_package_building() {
        let result = finish(
            CliResponse {
                output: json!({"evidence_attention": 1}).to_string(),
                exit_code: 3,
            },
            true,
            || panic!("partial evidence must never be packaged by finalize"),
        )
        .unwrap();
        assert_eq!(result.exit_code, 3);
        let output: Value = serde_json::from_str(&result.output).unwrap();
        assert!(output["package"].is_null());
        assert_eq!(output["package_status"], "blocked_by_attention");
    }

    #[test]
    fn clean_processing_returns_verified_package_result() {
        let result = finish(
            CliResponse {
                output: json!({"evidence_attention": 0}).to_string(),
                exit_code: 0,
            },
            true,
            || {
                Ok(CliResponse {
                    output: json!({"zip": "disposable.zip", "files": 2}).to_string(),
                    exit_code: 0,
                })
            },
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        let output: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(output["package"]["files"], 2);
        assert_eq!(output["package_status"], "verified_archival_zip");
        assert_eq!(output["customer_delivery_certified"], false);
    }
}
