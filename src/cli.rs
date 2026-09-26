//! Command-line entry points over the same guarded workflow services as the GUI.

mod acquire;
mod office;
mod recovery;

use std::{
    env,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    audit,
    batch_extraction::{self, BatchExtractionRequest},
    conversion_run::{self, DEFAULT_CONVERSION_WORKERS},
    external_tools::{self, ToolHealth, ToolKind},
    floppy::{self, FloppyDrive, WriteProtectionStatus},
    imaging,
    manifest::{self, ManifestRequest},
    package::{self, PackageRequest},
    pipeline::{self, PipelineRequest},
    project::ProjectState,
    recovery_backup::{self, RecoveryBackupRequest},
    recovery_plan::{self, RecoveryAction},
    report,
    safety::MediaSafetyPolicy,
};

const HELP: &str = r#"FluxVault — floppy archiving
Usage:
  fluxvault                         Open the GUI
  fluxvault init [path]             Create a project
  fluxvault status [--project PATH] Show project status
  fluxvault project show [--project PATH]
                                    Show saved project metadata
  fluxvault disk list [--project PATH]
  fluxvault disk show N [--project PATH]
                                    Inspect saved disk attempts
  fluxvault disk select N [--project PATH]
  fluxvault disk next [--project PATH]
                                    Select the current/next disk number (no drive access)
  fluxvault drive list              List removable drives without reading media
  fluxvault drive probe --drive A:  Read-only 512-byte media and protection probe
  fluxvault acquire --drive A: --disk N [--retries N] --write-blocker-verified
                                    Read-only image; requires independently verified hardware
  fluxvault tools check [--project PATH]
                                    Check external tool versions and record audit
  fluxvault tools show              Show configured tool paths
  fluxvault tools set NAME PATH     Configure sevenzip/libreoffice/greaseweazle
  fluxvault tools clear NAME        Return a tool to auto-discovery
  fluxvault extract all [--project PATH]
                                    Process saved images with the GUI's extraction service
  fluxvault extract disk N [--project PATH]
                                    Extract one saved disk or preserve recovery evidence
  fluxvault recovery plan [N] [--project PATH]
                                    Inspect evidence-ranked offline next steps
  fluxvault recovery compare N [--project PATH]
                                    Compare the two latest saved attempts
  fluxvault recovery backup N [--project PATH]
                                    Create/reuse immutable pass-1 evidence backup
  fluxvault recovery queue [--project PATH]
                                    Show disks needing recovery decisions
  fluxvault recovery composite N [--project PATH]
                                    Build/reuse an evidence-checked image composite
  fluxvault recovery fat N [--project PATH]
                                    Reconstruct only provable mirrored FAT sectors
  fluxvault recovery import N --source DIR --dmde-log FILE
                                    Import external DMDE recovery without overwriting it
  fluxvault conversion plan [--project PATH]
                                    Plan delivery paths and Office conversions
  fluxvault conversion run [--project PATH] [--conversion-workers N]
                                    Convert saved recovered files with LibreOffice
  fluxvault conversion issues [--project PATH]
                                    Show saved conversion exceptions
  fluxvault conversion retry [SOURCE] [--project PATH]
                                    Retry saved issues after restart; optionally one source
  fluxvault files manifest [--project PATH]
                                    Refresh recovered-file inventory
  fluxvault audit [--project PATH]  Verify image/extraction evidence
  fluxvault report export [--project PATH]
                                    Export the Hungarian XLSX workbook
  fluxvault process [--project PATH] [--conversion-workers N]
                                    Extract, convert, audit, and report
  fluxvault package build --destination PATH [--project PATH]
                                    Create and verify an archival ZIP
  fluxvault --help                  Show this help
Options:
  --json                            Output machine-readable JSON
  --project PATH                    Use a specific project instead of searching upward
  --destination PATH                Output folder outside the project
  --drive LETTER:                   Enumerated removable drive for read-only probe
  --disk N                          Disk number for acquisition
  --retries N                       Bad-sector retry passes for acquisition (0-10; default 2)
  --write-blocker-verified          Operator asserts separate hardware protection test
  --source DIR                      External recovered-files folder for DMDE import
  --dmde-log FILE                   Matching DMDE log for recovery import
  --conversion-workers N            Parallel Office files during process (1-16; default 4)
Exit codes: 0 complete, 3 attention/partial, 2 invalid input or operation error"#;

#[derive(Debug)]
struct CliResponse {
    output: String,
    exit_code: i32,
}

pub fn run_from_env() -> Option<i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return None;
    }
    let json_output = args.iter().any(|argument| argument == "--json");
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
                if json_output {
                    println!("{}", json_error(&message));
                } else {
                    eprintln!("FluxVault: {message}");
                }
                2
            }
        },
    )
}

fn json_error(message: &str) -> String {
    json!({"error": {"code": "operation_error", "message": message}}).to_string()
}

fn run(args: &[String], cwd: &Path) -> Result<CliResponse, String> {
    let mut json_output = false;
    let mut project_override: Option<PathBuf> = None;
    let mut destination: Option<PathBuf> = None;
    let mut drive_override: Option<String> = None;
    let mut acquisition_disk: Option<u32> = None;
    let mut acquisition_retries: Option<usize> = None;
    let mut write_blocker_verified = false;
    let mut import_source: Option<PathBuf> = None;
    let mut import_log: Option<PathBuf> = None;
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
            "--drive" => {
                index += 1;
                let value = args.get(index).ok_or("--drive requires a drive letter")?;
                drive_override = Some(value.to_owned());
            }
            "--disk" => {
                index += 1;
                acquisition_disk = Some(
                    args.get(index)
                        .ok_or("--disk requires a number")?
                        .parse::<u32>()
                        .ok()
                        .filter(|number| *number > 0)
                        .ok_or("--disk requires a positive number")?,
                );
            }
            "--retries" => {
                index += 1;
                acquisition_retries = Some(
                    args.get(index)
                        .ok_or("--retries requires a number")?
                        .parse::<usize>()
                        .ok()
                        .filter(|number| *number <= 10)
                        .ok_or("--retries must be from 0 to 10")?,
                );
            }
            "--write-blocker-verified" => write_blocker_verified = true,
            "--source" => {
                index += 1;
                import_source = Some(PathBuf::from(
                    args.get(index).ok_or("--source requires a folder")?,
                ));
            }
            "--dmde-log" => {
                index += 1;
                import_log = Some(PathBuf::from(
                    args.get(index).ok_or("--dmde-log requires a file")?,
                ));
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
                return Err(format!("Unknown option: {value}. Run fluxvault --help"));
            }
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }

    if conversion_workers.is_some()
        && positional.first().map(String::as_str) != Some("process")
        && !(positional.len() >= 2
            && positional[0] == "conversion"
            && matches!(positional[1].as_str(), "run" | "retry"))
    {
        return Err(
            "--conversion-workers is only valid with process or conversion run/retry".to_owned(),
        );
    }
    if drive_override.is_some()
        && !(positional.len() == 2 && positional[0] == "drive" && positional[1] == "probe")
        && !(positional.len() == 1 && positional[0] == "acquire")
    {
        return Err("--drive is only valid with drive probe or acquire".to_owned());
    }
    if (acquisition_disk.is_some() || acquisition_retries.is_some() || write_blocker_verified)
        && !(positional.len() == 1 && positional[0] == "acquire")
    {
        return Err(
            "--disk, --retries and --write-blocker-verified are only valid with acquire".to_owned(),
        );
    }
    if (import_source.is_some() || import_log.is_some())
        && !(positional.len() == 3 && positional[0] == "recovery" && positional[1] == "import")
    {
        return Err("--source and --dmde-log are only valid with recovery import".to_owned());
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
        Some("acquire") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return acquire::run(
                &project,
                json_output,
                drive_override.as_deref(),
                acquisition_disk,
                acquisition_retries.unwrap_or(2),
                write_blocker_verified,
            );
        }
        Some("status") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let stats = imaging::load_project_statistics(&project.images_dir())?;
            let next_actions = status_next_actions(stats.disk_count, stats.partial_disks);
            needs_attention = stats.partial_disks > 0;
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
                    "next_actions": next_actions,
                })
                .to_string())
            } else {
                Ok(format!(
                    "{} ({})\nCurrent disk: {:03}\nDisks: {} ({} OK, {} partial)\nAttempts: {}\nBest known bad sectors: {}\nNext actions:\n{}",
                    project.name(),
                    project.root().display(),
                    project.current_disk_number(),
                    stats.disk_count,
                    stats.ok_disks,
                    stats.partial_disks,
                    stats.total_attempts,
                    stats.best_known_bad_sectors,
                    next_actions
                        .iter()
                        .map(|action| format!("  - {action}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
            }
        }
        Some("project")
            if destination.is_none() && positional.len() == 2 && positional[1] == "show" =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            if json_output {
                Ok(json!({"project": project.root(), "name": project.name(),
                    "current_disk": project.current_disk_number(),
                    "images": project.images_dir(), "logs": project.logs_dir(),
                    "reports": project.reports_dir()})
                .to_string())
            } else {
                Ok(format!(
                    "{} ({})\nCurrent disk: {:03}\nImages: {}\nLogs: {}\nReports: {}",
                    project.name(),
                    project.root().display(),
                    project.current_disk_number(),
                    project.images_dir().display(),
                    project.logs_dir().display(),
                    project.reports_dir().display()
                ))
            }
        }
        Some("disk")
            if destination.is_none()
                && ((positional.len() == 3 && positional[1] == "select")
                    || (positional.len() == 2 && positional[1] == "next")) =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let mut project = ProjectState::open_without_session(root)?;
            let number = if positional[1] == "select" {
                positional[2]
                    .parse::<u32>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or("disk select requires a positive disk number")?
            } else {
                project
                    .current_disk_number()
                    .checked_add(1)
                    .ok_or("Cannot advance beyond the maximum disk number")?
            };
            project.set_current_disk_number_without_session(number)?;
            if json_output {
                Ok(json!({"project": project.root(), "current_disk": number}).to_string())
            } else {
                Ok(format!(
                    "Selected disk {number:03} (no physical drive accessed)."
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
        Some("drive")
            if positional.len() == 2
                && positional[1] == "list"
                && drive_override.is_none()
                && project_override.is_none()
                && destination.is_none() =>
        {
            let drives = floppy::enumerate_removable_drives()?;
            if json_output {
                Ok(json!({"drives": drives.iter().map(|drive| json!({
                    "root": drive.root, "device": drive.device_path
                })).collect::<Vec<_>>(), "media_read": false})
                .to_string())
            } else if drives.is_empty() {
                Ok("No removable drives found; no media was read.".to_owned())
            } else {
                Ok(drives
                    .iter()
                    .map(FloppyDrive::display_name)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        Some("drive")
            if positional.len() == 2
                && positional[1] == "probe"
                && project_override.is_none()
                && destination.is_none() =>
        {
            MediaSafetyPolicy::assert_invariants();
            let requested = drive_override
                .as_deref()
                .ok_or("drive probe requires --drive A:")?;
            let drives = floppy::enumerate_removable_drives()?;
            let drive = select_removable_drive(&drives, requested)?;
            let probe = floppy::probe_read_only(&drive)?;
            let protection = match &probe.write_protection {
                WriteProtectionStatus::Protected => "protected",
                WriteProtectionStatus::Writable => "writable",
                WriteProtectionStatus::Unknown(_) => "unknown",
            };
            let safe_to_image = probe
                .geometry
                .is_some_and(|geometry| geometry.looks_like_floppy())
                && probe.write_protection == WriteProtectionStatus::Protected;
            needs_attention = !safe_to_image;
            if json_output {
                Ok(json!({
                    "drive": drive.root, "device": drive.device_path,
                    "read_only": true, "bytes_read": probe.bytes_read,
                    "first_bytes_hex": probe.first_bytes_hex(),
                    "boot_signature_hex": probe.boot_signature_hex(),
                    "write_protection": protection,
                    "write_protection_detail": match &probe.write_protection {
                        WriteProtectionStatus::Unknown(detail) => Some(detail.as_str()),
                        _ => None,
                    },
                    "geometry": probe.geometry.map(|geometry| json!({
                        "cylinders": geometry.cylinders, "heads": geometry.heads,
                        "sectors_per_track": geometry.sectors_per_track,
                        "bytes_per_sector": geometry.bytes_per_sector,
                        "total_bytes": geometry.total_bytes(),
                        "format": geometry.format_guess(),
                        "looks_like_floppy": geometry.looks_like_floppy()
                    })),
                    "geometry_error": probe.geometry_error,
                    "acquisition_permitted_by_software_checks": safe_to_image,
                    "hardware_write_protection_independently_verified": false
                })
                .to_string())
            } else {
                Ok(format!(
                    "{}: read-only probe read {} bytes; write protection: {}; geometry: {}; acquisition {} by software checks.\nFirst bytes: {}\nBoot signature: {}\nVerify this adapter's physical write protection separately before customer media.",
                    drive.root,
                    probe.bytes_read,
                    protection,
                    probe
                        .geometry
                        .map(|geometry| format!(
                            "{} ({} bytes)",
                            geometry.format_guess(),
                            geometry.total_bytes()
                        ))
                        .or(probe.geometry_error.clone())
                        .unwrap_or_else(|| "unknown".to_owned()),
                    if safe_to_image {
                        "permitted by checks"
                    } else {
                        "blocked"
                    },
                    probe.first_bytes_hex(),
                    probe.boot_signature_hex()
                ))
            }
        }
        Some("tools")
            if positional.len() == 2 && positional[1] == "check" && destination.is_none() =>
        {
            let settings = external_tools::load_settings()?;
            let audit_path = if project_override.is_some() || discover_project(cwd).is_some() {
                let root = resolve_project_root(cwd, project_override.as_deref())?;
                ProjectState::open_without_session(root)?
                    .logs_dir()
                    .join("external-tools.jsonl")
            } else {
                external_tools::default_audit_path()
            };
            let statuses = ToolKind::ALL
                .map(|kind| external_tools::check_tool(kind, settings.path(kind), &audit_path));
            for status in &statuses {
                if let Some(error) = &status.audit_error {
                    return Err(format!(
                        "{} health-check audit could not be saved: {error}",
                        status.kind.display_name()
                    ));
                }
            }
            needs_attention = statuses
                .iter()
                .any(|status| status.health != ToolHealth::Ready);
            if json_output {
                Ok(
                    json!({"audit_log": audit_path, "tools": statuses.iter().map(|status| json!({
                    "name": status.kind.display_name(),
                    "health": format!("{:?}", status.health).to_ascii_lowercase(),
                    "executable": status.executable, "version": status.version,
                    "detail": status.detail
                })).collect::<Vec<_>>()})
                    .to_string(),
                )
            } else {
                Ok(statuses
                    .iter()
                    .map(|status| {
                        format!(
                            "{}: {:?} | {} | {}",
                            status.kind.display_name(),
                            status.health,
                            status
                                .executable
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "not found".to_owned()),
                            status.version.as_deref().unwrap_or(&status.detail)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        Some("tools")
            if destination.is_none()
                && project_override.is_none()
                && ((positional.len() == 2 && positional[1] == "show")
                    || (positional.len() == 4 && positional[1] == "set")
                    || (positional.len() == 3 && positional[1] == "clear")) =>
        {
            let mut settings = external_tools::load_settings()?;
            if positional[1] != "show" {
                let kind = parse_tool_kind(&positional[2])?;
                if positional[1] == "set" {
                    let configured = PathBuf::from(&positional[3]);
                    let configured = if configured.is_absolute() {
                        configured
                    } else {
                        cwd.join(configured)
                    };
                    if !configured.is_file() {
                        return Err(format!(
                            "Tool executable not found: {}",
                            configured.display()
                        ));
                    }
                    settings.set_path(kind, Some(configured));
                } else {
                    settings.set_path(kind, None);
                }
                external_tools::save_settings(&settings)?;
            }
            if json_output {
                Ok(json!({
                    "tools": ToolKind::ALL.iter().map(|kind| json!({
                        "name": kind.display_name(),
                        "configured_path": settings.path(*kind),
                        "auto_discovery": settings.path(*kind).is_none()
                    })).collect::<Vec<_>>()
                })
                .to_string())
            } else {
                Ok(ToolKind::ALL
                    .iter()
                    .map(|kind| {
                        format!(
                            "{}: {}",
                            kind.display_name(),
                            settings
                                .path(*kind)
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "auto-discovery".to_owned())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        Some("recovery")
            if destination.is_none()
                && ((positional.len() == 2 && positional[1] == "queue")
                    || (positional.len() == 3
                        && matches!(positional[1].as_str(), "composite" | "fat" | "import"))) =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return recovery::run_advanced(
                &positional,
                &project,
                cwd,
                json_output,
                import_source.as_deref(),
                import_log.as_deref(),
            );
        }
        Some("recovery")
            if destination.is_none()
                && positional.len() == 3
                && (positional[1] == "compare" || positional[1] == "backup") =>
        {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("recovery compare/backup requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk_number)?;
            if positional[1] == "compare" {
                let comparison = imaging::compare_latest_attempts(&attempts)
                    .ok_or("Two compatible saved attempts are required for comparison")?;
                needs_attention = !comparison.still_bad_sectors.is_empty();
                if json_output {
                    Ok(json!({
                        "disk": disk_number,
                        "older_attempt": comparison.older_attempt,
                        "newer_attempt": comparison.newer_attempt,
                        "older_bad": comparison.older_bad_count,
                        "newer_bad": comparison.newer_bad_count,
                        "recovered_sectors": comparison.recovered_sectors,
                        "newly_bad_sectors": comparison.newly_bad_sectors,
                        "still_bad_sectors": comparison.still_bad_sectors
                    })
                    .to_string())
                } else {
                    Ok(format!(
                        "Disk {disk_number:03}: attempts #{:03} -> #{:03}\nEarlier bad: {} | newer bad: {}\nRecovered: {:?}\nNewly bad: {:?}\nStill bad: {:?}",
                        comparison.older_attempt,
                        comparison.newer_attempt,
                        comparison.older_bad_count,
                        comparison.newer_bad_count,
                        comparison.recovered_sectors,
                        comparison.newly_bad_sectors,
                        comparison.still_bad_sectors
                    ))
                }
            } else {
                let attempt = attempts
                    .last()
                    .ok_or_else(|| format!("No saved disk {disk_number:03} in this project"))?;
                if !attempt.attention_required {
                    return Err(
                        "A clean acquisition does not need a pass-1 recovery backup".to_owned()
                    );
                }
                let result = recovery_backup::ensure_first_backup(
                    &RecoveryBackupRequest {
                        recovery_root: project.recovery_dir(),
                        disk_number,
                        attempt_number: attempt.attempt_number,
                        image_path: recovery_plan::resolve_image_path(
                            &project.images_dir(),
                            &attempt.image_file,
                        )?,
                        log_path: (!attempt.log_file.is_empty())
                            .then(|| PathBuf::from(&attempt.log_file)),
                        reason: format!(
                            "{}; {} bad sectors",
                            attempt.status,
                            attempt.bad_sectors.len()
                        ),
                    },
                    &|stage| eprintln!("{stage}"),
                )?;
                needs_attention = true;
                if json_output {
                    Ok(json!({
                        "disk": disk_number, "attempt": attempt.attempt_number,
                        "backup": result.directory, "created": result.created,
                        "image": result.image_backup, "log": result.log_backup,
                        "manifest": result.manifest_path
                    })
                    .to_string())
                } else {
                    Ok(format!(
                        "Disk {disk_number:03} pass-1 backup {}: {}",
                        if result.created { "created" } else { "reused" },
                        result.directory.display()
                    ))
                }
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
        Some("conversion")
            if ((positional.len() == 2
                && matches!(positional[1].as_str(), "plan" | "run" | "issues" | "retry"))
                || (positional.len() == 3 && positional[1] == "retry"))
                && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return office::run(
                &positional[1],
                &project,
                json_output,
                conversion_workers.unwrap_or(DEFAULT_CONVERSION_WORKERS),
                positional.get(2).map(Path::new),
                cwd,
            );
        }
        Some("files")
            if positional.len() == 2 && positional[1] == "manifest" && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let result = manifest::build_manifest(
                &ManifestRequest {
                    extracted_root: project.extracted_dir(),
                    images_directory: project.images_dir(),
                    reports_directory: project.reports_dir(),
                },
                &|stage| eprintln!("{stage}"),
            )?;
            if json_output {
                Ok(json!({
                    "project": project.root(), "manifest": result.path,
                    "disks": result.disk_count, "files": result.file_count,
                    "bytes": result.total_bytes
                })
                .to_string())
            } else {
                Ok(format!(
                    "Recovered-file manifest: {} disks, {} files, {} bytes.\n{}",
                    result.disk_count,
                    result.file_count,
                    result.total_bytes,
                    result.path.display()
                ))
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
        Some("extract")
            if positional.len() == 2 && positional[1] == "all" && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let settings = external_tools::load_settings()?;
            let command_audit_path = project.logs_dir().join("external-tools.jsonl");
            let seven_zip_executable = external_tools::find_ready_tool(
                ToolKind::SevenZip,
                settings.path(ToolKind::SevenZip),
                &command_audit_path,
            )?;
            let result = batch_extraction::run_batch_extraction(
                &BatchExtractionRequest {
                    seven_zip_executable,
                    images_directory: project.images_dir(),
                    logs_directory: project.logs_dir(),
                    extracted_root: project.extracted_dir(),
                    recovery_root: project.recovery_dir(),
                    reports_directory: project.reports_dir(),
                    command_audit_path,
                },
                &|stage| eprintln!("{stage}"),
                &|completed, total| eprintln!("Extracted {completed}/{total} disks"),
            )?;
            needs_attention = result.recovery_disks > 0 || result.in_progress_disks > 0;
            if json_output {
                Ok(json!({
                    "disks": result.total_disks,
                    "extracted": result.extracted_disks,
                    "reused": result.reused_disks,
                    "manual": result.manual_disks,
                    "recovery_queue": result.recovery_disks,
                    "in_progress": result.in_progress_disks,
                    "zero_file": result.zero_file_disks,
                    "summary": result.summary_path,
                    "manifest": result.manifest.path
                })
                .to_string())
            } else {
                Ok(format!(
                    "Extraction: {} disks, {} extracted ({} reused), {} manual, {} need recovery, {} in progress.\nSummary: {}\nManifest: {}",
                    result.total_disks,
                    result.extracted_disks,
                    result.reused_disks,
                    result.manual_disks,
                    result.recovery_disks,
                    result.in_progress_disks,
                    result.summary_path.display(),
                    result.manifest.path.display()
                ))
            }
        }
        Some("extract")
            if positional.len() == 3 && positional[1] == "disk" && destination.is_none() =>
        {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("extract disk requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let settings = external_tools::load_settings()?;
            let command_audit_path = project.logs_dir().join("external-tools.jsonl");
            let seven_zip_executable = external_tools::find_ready_tool(
                ToolKind::SevenZip,
                settings.path(ToolKind::SevenZip),
                &command_audit_path,
            )?;
            let result = batch_extraction::run_single_disk_extraction(
                &BatchExtractionRequest {
                    seven_zip_executable,
                    images_directory: project.images_dir(),
                    logs_directory: project.logs_dir(),
                    extracted_root: project.extracted_dir(),
                    recovery_root: project.recovery_dir(),
                    reports_directory: project.reports_dir(),
                    command_audit_path,
                },
                disk_number,
                &|stage| eprintln!("{stage}"),
            )?;
            needs_attention = matches!(result.status, "recovery" | "in_progress");
            if json_output {
                Ok(json!({
                    "disk": result.disk_number,
                    "status": result.status,
                    "reason": result.reason,
                    "files": result.file_count,
                    "reused": result.reused,
                    "manifest": result.manifest_path
                })
                .to_string())
            } else {
                Ok(format!(
                    "Disk {:03}: {} | {} | {} files\nManifest: {}",
                    result.disk_number,
                    result.status,
                    result.reason,
                    result.file_count,
                    result.manifest_path.display()
                ))
            }
        }
        Some("report")
            if positional.len() == 2 && positional[1] == "export" && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let statistics = imaging::load_project_statistics(&project.images_dir())?;
            let workbook = report::export_hungarian_report(
                project.name(),
                &project.reports_dir(),
                &project.images_dir(),
                &statistics,
            )?;
            if json_output {
                Ok(json!({"project": project.root(), "workbook": workbook,
                    "disks": statistics.disk_count})
                .to_string())
            } else {
                Ok(format!("Workbook exported: {}", workbook.display()))
            }
        }
        Some("process") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let project_root = project.root().to_path_buf();
            let reports_directory = project.reports_dir();
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
            let conversion_state = conversion_run::save_snapshot(
                &reports_directory,
                &project_root,
                &result.conversion,
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
                    "converted_retried_outputs": result.conversion.retried_outputs,
                    "conversion_state": conversion_state,
                    "evidence_verified": result.audit.verified_disks,
                    "evidence_attention": result.audit.attention_disks,
                    "workbook": result.workbook_path,
                    "customer_delivery_certified": false})
                .to_string())
            } else {
                Ok(format!(
                    "Project processing complete: {} disks, {} verified evidence sets, {} need attention.\nComposites: {} derived disk(s), {} reused, {} declined.\nMirrored FAT: {} derived disk(s), {} reused.\nConversions: {} OK, {} partial, {} failed, {} outputs retried.\nRecovery decisions: {}\nWorkbook: {}\nConversion state: {}",
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
                    result.conversion.retried_outputs,
                    result.recovery_decisions_path.display(),
                    result.workbook_path.display(),
                    conversion_state.display()
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
        _ => Err("Unsupported command or arguments. Run fluxvault --help".to_owned()),
    }?;
    Ok(CliResponse {
        output,
        exit_code: if needs_attention { 3 } else { 0 },
    })
}

fn status_next_actions(disk_count: usize, partial_disks: usize) -> Vec<&'static str> {
    if disk_count == 0 {
        return vec!["No saved images. Verify physical write protection before any acquisition."];
    }
    let mut actions = Vec::new();
    if partial_disks > 0 {
        actions.push("Review incomplete disks with `fluxvault recovery plan`.");
    }
    actions.push("Run `fluxvault process` to refresh extraction, conversion, audit, and reports.");
    actions
}

fn select_removable_drive(drives: &[FloppyDrive], requested: &str) -> Result<FloppyDrive, String> {
    let bytes = requested.as_bytes();
    if !matches!(bytes, [letter, b':'] | [letter, b':', b'\\'] if letter.is_ascii_alphabetic()) {
        return Err("--drive requires a single drive letter such as A:".to_owned());
    }
    let letter = bytes[0].to_ascii_uppercase() as char;
    let root = format!("{letter}:\\");
    drives
        .iter()
        .find(|drive| drive.root.eq_ignore_ascii_case(&root))
        .cloned()
        .ok_or_else(|| format!("{root} is not a currently enumerated removable drive"))
}

fn parse_tool_kind(value: &str) -> Result<ToolKind, String> {
    match value.to_ascii_lowercase().as_str() {
        "sevenzip" | "7zip" | "7z" => Ok(ToolKind::SevenZip),
        "libreoffice" | "office" => Ok(ToolKind::LibreOffice),
        "greaseweazle" | "gw" => Ok(ToolKind::Greaseweazle),
        _ => Err(format!(
            "Unknown tool {value}; choose sevenzip, libreoffice, or greaseweazle"
        )),
    }
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
    fn tool_names_are_exact_and_safe() {
        assert_eq!(parse_tool_kind("7z").unwrap(), ToolKind::SevenZip);
        assert_eq!(
            parse_tool_kind("LibreOffice").unwrap(),
            ToolKind::LibreOffice
        );
        assert_eq!(parse_tool_kind("GW").unwrap(), ToolKind::Greaseweazle);
        assert!(parse_tool_kind("write").is_err());
        assert!(parse_tool_kind("7z.exe --delete").is_err());
    }

    #[test]
    fn json_error_is_machine_readable_even_with_quoted_message() {
        let error: serde_json::Value = serde_json::from_str(&json_error("bad \"disk\"")).unwrap();
        assert_eq!(error["error"]["code"], "operation_error");
        assert_eq!(error["error"]["message"], "bad \"disk\"");
    }

    #[test]
    fn drive_probe_rejects_arbitrary_paths_and_unlisted_drives() {
        let drives = vec![FloppyDrive {
            root: "A:\\".to_owned(),
            device_path: r"\\.\A:".to_owned(),
        }];
        assert_eq!(select_removable_drive(&drives, "a:").unwrap(), drives[0]);
        assert_eq!(select_removable_drive(&drives, "A:\\").unwrap(), drives[0]);
        for unsafe_path in [r"\\.\PhysicalDrive0", "C:/", "A:/", "A:foo", "AA:"] {
            assert!(select_removable_drive(&drives, unsafe_path).is_err());
        }
        assert!(select_removable_drive(&drives, "B:").is_err());
        assert!(run(&["drive".to_owned(), "probe".to_owned()], Path::new(".")).is_err());
        assert!(
            run(
                &["status".to_owned(), "--drive".to_owned(), "A:".to_owned()],
                Path::new(".")
            )
            .is_err()
        );
        assert!(
            run(
                &[
                    "drive".to_owned(),
                    "list".to_owned(),
                    "--write-blocker-verified".to_owned()
                ],
                Path::new(".")
            )
            .is_err()
        );
    }

    #[test]
    fn report_export_uses_project_without_hardware_or_external_tools() {
        let root = env::temp_dir().join(format!(
            "fluxvault-cli-report-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        ProjectState::create_without_session(root.clone()).unwrap();
        let response = run(
            &[
                "report".to_owned(),
                "export".to_owned(),
                "--json".to_owned(),
            ],
            &root,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&response.output).unwrap();
        assert_eq!(response.exit_code, 0);
        assert_eq!(json["disks"], 0);
        assert!(Path::new(json["workbook"].as_str().unwrap()).is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn files_manifest_route_is_machine_readable_without_hardware() {
        let root = env::temp_dir().join(format!(
            "fluxvault-cli-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let recovered = project.extracted_dir().join("001").join("document.txt");
        std::fs::create_dir_all(recovered.parent().unwrap()).unwrap();
        std::fs::write(&recovered, b"synthetic fixture").unwrap();
        let response = run(
            &[
                "files".to_owned(),
                "manifest".to_owned(),
                "--json".to_owned(),
            ],
            &root,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&response.output).unwrap();
        assert_eq!(json["files"], 1);
        assert!(Path::new(json["manifest"].as_str().unwrap()).is_file());
        assert_eq!(std::fs::read(recovered).unwrap(), b"synthetic fixture");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_backup_cli_is_idempotent_for_unlogged_legacy_image() {
        let root = env::temp_dir().join(format!(
            "fluxvault-cli-backup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        std::fs::write(project.images_dir().join("001.img"), [0x33; 512]).unwrap();
        let args = vec![
            "recovery".to_owned(),
            "backup".to_owned(),
            "1".to_owned(),
            "--json".to_owned(),
        ];
        let first = run(&args, &root).unwrap();
        let first_json: serde_json::Value = serde_json::from_str(&first.output).unwrap();
        assert_eq!(first.exit_code, 3);
        assert_eq!(first_json["created"], true);
        let second = run(&args, &root).unwrap();
        let second_json: serde_json::Value = serde_json::from_str(&second.output).unwrap();
        assert_eq!(second_json["created"], false);
        assert_eq!(second_json["backup"], first_json["backup"]);
        assert!(
            run(
                &["recovery".to_owned(), "compare".to_owned(), "1".to_owned()],
                &root
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

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
        let status = run(
            &[
                "status".to_owned(),
                "--json".to_owned(),
                "--project".to_owned(),
                root.display().to_string(),
            ],
            Path::new("."),
        )
        .unwrap();
        let status_json: serde_json::Value = serde_json::from_str(&status.output).unwrap();
        assert_eq!(status.exit_code, 0);
        assert_eq!(status_json["disks"], 0);
        assert_eq!(status_json["current_disk"], 1);
        assert!(
            status_json["next_actions"][0]
                .as_str()
                .unwrap()
                .contains("write protection")
        );
        let show = run(
            &["project".to_owned(), "show".to_owned(), "--json".to_owned()],
            &root,
        )
        .unwrap();
        let show_json: serde_json::Value = serde_json::from_str(&show.output).unwrap();
        assert_eq!(
            show_json["name"],
            root.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(show_json["current_disk"], 1);
        assert!(
            run(
                &["disk".to_owned(), "select".to_owned(), "0".to_owned()],
                &root
            )
            .is_err()
        );
        let selected = run(
            &[
                "disk".to_owned(),
                "select".to_owned(),
                "7".to_owned(),
                "--json".to_owned(),
            ],
            &root,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&selected.output).unwrap()["current_disk"],
            7
        );
        let advanced = run(&["disk".to_owned(), "next".to_owned()], &root).unwrap();
        assert!(advanced.output.contains("008"));
        assert_eq!(
            ProjectState::open_without_session(root.clone())
                .unwrap()
                .current_disk_number(),
            8
        );
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

    #[test]
    fn status_recommends_recovery_before_processing_partial_disks() {
        let actions = status_next_actions(2, 1);
        assert_eq!(actions.len(), 2);
        assert!(actions[0].contains("recovery plan"));
        assert!(actions[1].contains("process"));
    }

    #[test]
    fn status_json_marks_unlogged_fixture_image_as_attention() {
        let root = env::temp_dir().join(format!(
            "fluxvault-cli-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        std::fs::write(project.images_dir().join("007.img"), [0_u8; 512]).unwrap();
        let response = run(&["status".to_owned(), "--json".to_owned()], &root).unwrap();
        let json: serde_json::Value = serde_json::from_str(&response.output).unwrap();
        assert_eq!(response.exit_code, 3);
        assert_eq!(json["disks"], 1);
        assert_eq!(json["partial_disks"], 1);
        assert!(
            json["next_actions"][0]
                .as_str()
                .unwrap()
                .contains("recovery plan")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
