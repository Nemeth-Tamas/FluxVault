//! CLI routes for saved-image recovery. These never open a physical drive.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::{
    composite::{self, CompositeRequest, CompositeSource},
    imaging,
    manifest::{self, ManifestRequest},
    manual_recovery_import::{self, ManualRecoveryImportRequest},
    project::ProjectState,
    recovery_plan::{self, RecoveryAction},
    sector_recovery::{self, ReconstructionRequest},
};

use super::CliResponse;

pub(super) fn run_advanced(
    positional: &[String],
    project: &ProjectState,
    cwd: &Path,
    json_output: bool,
    import_source: Option<&Path>,
    import_log: Option<&Path>,
) -> Result<CliResponse, String> {
    if positional[1] == "queue" {
        let pending = recovery_plan::plan_project(&project.images_dir())?
            .into_iter()
            .filter(|plan| plan.action != RecoveryAction::Complete)
            .collect::<Vec<_>>();
        return Ok(CliResponse {
            output: if json_output {
                json!({"project": project.root(), "queue": pending}).to_string()
            } else if pending.is_empty() {
                "No disks currently need recovery decisions.".to_owned()
            } else {
                pending
                    .iter()
                    .map(|plan| {
                        format!(
                            "{:03} | {:?} | {} bad sectors | {}",
                            plan.disk_number, plan.action, plan.best_bad_sectors, plan.reason
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            exit_code: if pending.is_empty() { 0 } else { 3 },
        });
    }

    let disk_number = positional[2]
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or("recovery command requires a positive disk number")?;
    match positional[1].as_str() {
        "composite" => {
            let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk_number)?;
            if attempts.len() < 2 {
                return Err("Composite requires at least two saved attempts".to_owned());
            }
            if attempts
                .iter()
                .any(|attempt| attempt.bad_sectors.is_empty())
            {
                return Err("A clean attempt already exists; composite is unnecessary".to_owned());
            }
            let sources = attempts
                .iter()
                .map(|attempt| {
                    Ok(CompositeSource {
                        attempt_number: attempt.attempt_number,
                        image_path: recovery_plan::resolve_image_path(
                            &project.images_dir(),
                            &attempt.image_file,
                        )?,
                        expected_sha256: (!attempt.sha256.is_empty())
                            .then(|| attempt.sha256.clone()),
                        total_sectors: attempt.total_sectors,
                        bad_sectors: attempt.bad_sectors.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let result = composite::run_composite(
                &CompositeRequest {
                    images_directory: project.images_dir(),
                    recovery_root: project.recovery_dir(),
                    disk_number,
                    sources,
                },
                &|stage| eprintln!("{stage}"),
            )?;
            Ok(CliResponse {
                output: if json_output {
                    json!({
                        "disk": disk_number, "base_attempt": result.base_attempt,
                        "derived_image": result.derived_image,
                        "derived_sha256": result.derived_sha256,
                        "provenance": result.provenance_path,
                        "reused": result.reused,
                        "replacements": result.replacements,
                        "unresolved_bad_sectors": result.unresolved_bad_sectors
                    })
                    .to_string()
                } else {
                    format!(
                        "Disk {disk_number:03}: {} composite, {} sectors replaced, {} unresolved.\nImage: {}",
                        if result.reused { "reused" } else { "derived" },
                        result.replacements.len(),
                        result.unresolved_bad_sectors.len(),
                        result
                            .derived_image
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "none".to_owned())
                    )
                },
                exit_code: if result.unresolved_bad_sectors.is_empty() {
                    0
                } else {
                    3
                },
            })
        }
        "fat" => {
            let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk_number)?;
            let attempt = attempts
                .last()
                .ok_or_else(|| format!("No saved disk {disk_number:03} in this project"))?;
            if attempt.bad_sectors.is_empty() {
                return Err("Mirrored FAT reconstruction requires bad sectors".to_owned());
            }
            let result = sector_recovery::run_reconstruction(
                &ReconstructionRequest {
                    image_path: recovery_plan::resolve_image_path(
                        &project.images_dir(),
                        &attempt.image_file,
                    )?,
                    expected_sha256: (!attempt.sha256.is_empty()).then(|| attempt.sha256.clone()),
                    recovery_root: project.recovery_dir(),
                    disk_number,
                    attempt_number: attempt.attempt_number,
                    bad_sectors: attempt.bad_sectors.clone(),
                },
                &|stage| eprintln!("{stage}"),
            )?;
            Ok(CliResponse {
                output: if json_output {
                    json!({
                        "disk": disk_number, "source_image": result.source_image,
                        "source_sha256": result.source_sha256,
                        "derived_image": result.derived_image,
                        "derived_sha256": result.derived_sha256,
                        "provenance": result.provenance_path,
                        "filesystem": result.filesystem,
                        "reused": result.reused,
                        "reconstructed": result.reconstructed,
                        "unresolved_bad_sectors": result.unresolved_bad_sectors
                    })
                    .to_string()
                } else {
                    format!(
                        "Disk {disk_number:03}: {} FAT-derived image, {} sectors reconstructed, {} unresolved.\nImage: {}",
                        if result.reused { "reused" } else { "new" },
                        result.reconstructed.len(),
                        result.unresolved_bad_sectors.len(),
                        result
                            .derived_image
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "none".to_owned())
                    )
                },
                exit_code: if result.unresolved_bad_sectors.is_empty() {
                    0
                } else {
                    3
                },
            })
        }
        "import" => {
            let source = resolve_external(cwd, import_source.ok_or("--source DIR is required")?);
            let log = resolve_external(cwd, import_log.ok_or("--dmde-log FILE is required")?);
            let result = manual_recovery_import::import_manual_recovery(
                &ManualRecoveryImportRequest {
                    source_directory: source,
                    dmde_log_path: log,
                    extracted_root: project.extracted_dir(),
                    recovery_root: project.recovery_dir(),
                    disk_number,
                },
                &|stage| eprintln!("{stage}"),
            )?;
            let manifest = manifest::build_manifest(
                &ManifestRequest {
                    extracted_root: project.extracted_dir(),
                    images_directory: project.images_dir(),
                    reports_directory: project.reports_dir(),
                },
                &|stage| eprintln!("{stage}"),
            )?;
            Ok(CliResponse {
                output: if json_output {
                    json!({
                        "disk": disk_number, "output": result.output_directory,
                        "evidence": result.evidence_directory,
                        "copied_log": result.copied_log_path,
                        "import_manifest": result.manifest_path,
                        "project_manifest": manifest.path,
                        "files": result.file_count, "bytes": result.total_bytes,
                        "customer_delivery_certified": false
                    })
                    .to_string()
                } else {
                    format!(
                        "Disk {disk_number:03}: imported {} files ({} bytes).\nEvidence: {}\nProject manifest: {}",
                        result.file_count,
                        result.total_bytes,
                        result.evidence_directory.display(),
                        manifest.path.display()
                    )
                },
                exit_code: 3,
            })
        }
        _ => Err("Unsupported recovery command".to_owned()),
    }
}

fn resolve_external(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn fixture_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fluxvault-cli-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn composite_route_is_repeatable_and_preserves_saved_attempts() {
        let root = fixture_root("composite");
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let mut first = vec![0u8; 3 * 512];
        let mut second = first.clone();
        first[2 * 512..].fill(0x22);
        second[512..2 * 512].fill(0x11);
        for (number, bytes, bad_lba) in [(1, first.clone(), 1), (2, second.clone(), 2)] {
            let image = project
                .images_dir()
                .join(format!("001_attempt_{number:03}.img"));
            let log = project
                .logs_dir()
                .join(format!("001_attempt_{number:03}.log"));
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
                "bad_sectors": [{"lba": bad_lba, "cylinder": 0, "head": 0,
                    "sector": bad_lba + 1}],
                "sha256": hash,
            });
            fs::write(
                project
                    .images_dir()
                    .join(format!("001_attempt_{number:03}.json")),
                serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        }
        let queue = run_advanced(
            &["recovery".to_owned(), "queue".to_owned()],
            &project,
            &root,
            true,
            None,
            None,
        )
        .unwrap();
        assert_eq!(queue.exit_code, 3);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&queue.output).unwrap()["queue"][0]["disk_number"],
            1
        );
        let args = [
            "recovery".to_owned(),
            "composite".to_owned(),
            "1".to_owned(),
        ];
        let first_run = run_advanced(&args, &project, &root, true, None, None).unwrap();
        let first_json: serde_json::Value = serde_json::from_str(&first_run.output).unwrap();
        assert_eq!(first_run.exit_code, 0);
        assert_eq!(first_json["replacements"].as_array().unwrap().len(), 1);
        assert_eq!(first_json["reused"], false);
        let second_run = run_advanced(&args, &project, &root, true, None, None).unwrap();
        let second_json: serde_json::Value = serde_json::from_str(&second_run.output).unwrap();
        assert_eq!(second_json["reused"], true);
        assert_eq!(first_json["derived_image"], second_json["derived_image"]);
        assert_eq!(
            fs::read(project.images_dir().join("001_attempt_001.img")).unwrap(),
            first
        );
        assert_eq!(
            fs::read(project.images_dir().join("001_attempt_002.img")).unwrap(),
            second
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dmde_import_route_preserves_external_source_and_refuses_overwrite() {
        let root = fixture_root("import");
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        let source = root.join("external-recovery");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("recovered.txt"), b"evidence").unwrap();
        let log = root.join("copy.log");
        fs::write(&log, b"START 2026-09-22 12:00:00.000\nlogsec=512\nC 1 > > 0 : 1\nSTOP 2026-09-22 12:01:00.000\n").unwrap();
        let args = ["recovery".to_owned(), "import".to_owned(), "7".to_owned()];
        let result = run_advanced(&args, &project, &root, true, Some(&source), Some(&log)).unwrap();
        let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(result.exit_code, 3);
        assert_eq!(output["files"], 1);
        assert!(Path::new(output["project_manifest"].as_str().unwrap()).is_file());
        assert!(run_advanced(&args, &project, &root, true, Some(&source), Some(&log)).is_err());
        assert_eq!(fs::read(source.join("recovered.txt")).unwrap(), b"evidence");
        fs::remove_dir_all(root).unwrap();
    }
}
