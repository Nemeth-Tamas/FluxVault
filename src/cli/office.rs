//! CLI routes for delivery planning and Office conversion.

use serde_json::json;
use std::path::Path;

use crate::{
    conversion::{self, ConversionPlanningRequest},
    conversion_run::{self, ConversionRequest},
    external_tools::{self, ToolKind},
    project::ProjectState,
};

use super::CliResponse;

pub(super) fn run(
    command: &str,
    project: &ProjectState,
    json_output: bool,
    workers: usize,
    selected_source: Option<&Path>,
    cwd: &Path,
) -> Result<CliResponse, String> {
    let planning = ConversionPlanningRequest {
        extracted_root: project.extracted_dir(),
        converted_root: project.converted_dir(),
        reports_directory: project.reports_dir(),
    };
    if command == "plan" {
        let result = conversion::build_conversion_plan(&planning, &|stage| eprintln!("{stage}"))?;
        return Ok(CliResponse {
            output: if json_output {
                json!({
                    "project": project.root(), "disks": result.disk_count,
                    "mirrored_files": result.mirrored_files,
                    "reused_files": result.reused_files,
                    "conversion_candidates": result.conversion_candidates,
                    "path_map": result.path_map,
                    "conversion_plan": result.conversion_plan
                })
                .to_string()
            } else {
                format!(
                    "Conversion plan: {} disks, {} source files mirrored ({} reused), {} Office candidates.\nPath map: {}\nPlan: {}",
                    result.disk_count,
                    result.mirrored_files,
                    result.reused_files,
                    result.conversion_candidates,
                    result.path_map.display(),
                    result.conversion_plan.display()
                )
            },
            exit_code: 0,
        });
    }
    if command == "issues" {
        let saved = conversion_run::load_snapshot(&project.reports_dir(), project.root())?;
        return Ok(CliResponse {
            output: if json_output {
                json!({"project": project.root(), "issues": saved.issues}).to_string()
            } else if saved.issues.is_empty() {
                "No saved conversion issues.".to_owned()
            } else {
                saved
                    .issues
                    .iter()
                    .map(|issue| {
                        format!(
                            "{} | {} | {} | Modern: {} | PDF: {}",
                            issue.floppy,
                            issue.source_path.display(),
                            issue.status,
                            issue.modern_detail,
                            issue.pdf_detail
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            exit_code: if saved.issues.is_empty() { 0 } else { 3 },
        });
    }
    let (selected_sources, previous_result) = if command == "retry" {
        let previous = conversion_run::load_snapshot(&project.reports_dir(), project.root())?;
        let sources = if let Some(source) = selected_source {
            let source = if source.is_absolute() {
                source.to_path_buf()
            } else {
                cwd.join(source)
            };
            let source = source
                .canonicalize()
                .map_err(|error| format!("Cannot resolve selected source: {error}"))?;
            let issue = previous.issues.iter().find(|issue| {
                issue.source_path.canonicalize().ok().as_deref() == Some(source.as_path())
            });
            vec![
                issue
                    .ok_or("Selected source is not a saved conversion issue")?
                    .source_path
                    .clone(),
            ]
        } else {
            previous
                .issues
                .iter()
                .map(|issue| issue.source_path.clone())
                .collect::<Vec<_>>()
        };
        if sources.is_empty() {
            return Ok(CliResponse {
                output: if json_output {
                    json!({"project": project.root(), "retried": 0, "issues": []}).to_string()
                } else {
                    "No saved conversion issues to retry.".to_owned()
                },
                exit_code: 0,
            });
        }
        (Some(sources), Some(Box::new(previous)))
    } else {
        (None, None)
    };
    let settings = external_tools::load_settings()?;
    let audit_path = project.logs_dir().join("external-tools.jsonl");
    let executable = external_tools::find_ready_tool(
        ToolKind::LibreOffice,
        settings.path(ToolKind::LibreOffice),
        &audit_path,
    )?;
    let result = conversion_run::run_conversion(
        &ConversionRequest {
            planning,
            libreoffice_executable: executable,
            command_audit_path: audit_path,
            timeout_seconds: 45,
            workers,
            selected_sources,
            previous_result,
        },
        &|stage| eprintln!("{stage}"),
        &|completed, total| eprintln!("Converted {completed}/{total} files"),
    )?;
    let snapshot = conversion_run::save_snapshot(&project.reports_dir(), project.root(), &result)?;
    let needs_attention = result.partial > 0 || result.failed > 0 || result.timed_out > 0;
    Ok(CliResponse {
        output: if json_output {
            json!({
                "project": project.root(), "candidates": result.planning.conversion_candidates,
                "ok": result.ok, "partial": result.partial,
                "failed": result.failed, "timed_out": result.timed_out,
                "reused_outputs": result.reused_outputs,
                "retried_outputs": result.retried_outputs,
                "summary": result.summary_path,
                "failures": result.failures_path,
                "state": snapshot,
                "issues": result.issues.iter().map(|issue| json!({
                    "source": issue.source_path,
                    "disk": issue.floppy,
                    "status": issue.status,
                    "modern_result": issue.modern_result,
                    "modern_detail": issue.modern_detail,
                    "pdf_result": issue.pdf_result,
                    "pdf_detail": issue.pdf_detail
                })).collect::<Vec<_>>()
            })
            .to_string()
        } else {
            format!(
                "Conversion: {} OK, {} partial, {} failed, {} timed out, {} reused outputs.\nSummary: {}\nIssues: {}\nState: {}",
                result.ok,
                result.partial,
                result.failed,
                result.timed_out,
                result.reused_outputs,
                result.summary_path.display(),
                result.failures_path.display(),
                snapshot.display()
            )
        },
        exit_code: if needs_attention { 3 } else { 0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn plan_route_matches_gui_delivery_planning_without_external_tools() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-cli-conversion-plan-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let source = project
            .extracted_dir()
            .join("001")
            .join("$Root")
            .join("note.doc");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"synthetic legacy document").unwrap();
        let result = run("plan", &project, true, 4, None, &root).unwrap();
        let json: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(json["disks"], 1);
        assert_eq!(json["conversion_candidates"], 1);
        assert!(Path::new(json["path_map"].as_str().unwrap()).is_file());
        assert!(
            project
                .converted_dir()
                .join("001")
                .join("note.doc")
                .is_file()
        );
        assert_eq!(fs::read(source).unwrap(), b"synthetic legacy document");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires installed LibreOffice; runs only on a disposable synthetic RTF"]
    fn run_route_converts_disposable_rtf_with_audited_external_tool() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-cli-conversion-run-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let source = project.extracted_dir().join("001").join("test.rtf");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"{\\rtf1\\ansi Disposable FluxVault CLI test}").unwrap();
        let first = run("run", &project, true, 1, None, &root).unwrap();
        let first_json: serde_json::Value = serde_json::from_str(&first.output).unwrap();
        assert_eq!(first.exit_code, 0);
        assert_eq!(first_json["ok"], 1);
        assert!(
            project
                .converted_dir()
                .join("001")
                .join("test [from RTF].docx")
                .is_file()
        );
        assert!(
            project
                .converted_dir()
                .join("001")
                .join("test [from RTF].pdf")
                .is_file()
        );
        assert!(project.logs_dir().join("external-tools.jsonl").is_file());
        let second = run("run", &project, true, 1, None, &root).unwrap();
        let second_json: serde_json::Value = serde_json::from_str(&second.output).unwrap();
        assert_eq!(second_json["reused_outputs"], 2);
        assert!(project.reports_dir().join("ConversionState.json").is_file());
        let pdf = project
            .converted_dir()
            .join("001")
            .join("test [from RTF].pdf");
        fs::write(&pdf, b"invalid disposable output").unwrap();
        let partial = run("run", &project, true, 1, None, &root).unwrap();
        let partial_json: serde_json::Value = serde_json::from_str(&partial.output).unwrap();
        assert_eq!(partial.exit_code, 3);
        assert_eq!(partial_json["issues"].as_array().unwrap().len(), 1);
        let issues = run("issues", &project, true, 1, None, &root).unwrap();
        assert_eq!(issues.exit_code, 3);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&issues.output).unwrap()["issues"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        fs::remove_file(&pdf).unwrap();
        let retry = run("retry", &project, true, 1, Some(&source), &root).unwrap();
        let retry_json: serde_json::Value = serde_json::from_str(&retry.output).unwrap();
        assert_eq!(retry.exit_code, 0);
        assert_eq!(retry_json["ok"], 1);
        assert!(pdf.is_file());
        assert_eq!(
            run("issues", &project, true, 1, None, &root)
                .unwrap()
                .exit_code,
            0
        );
        assert_eq!(
            fs::read(source).unwrap(),
            b"{\\rtf1\\ansi Disposable FluxVault CLI test}"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
