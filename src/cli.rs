//! Command-line entry points over the guarded workflow services.

mod acquire;
mod dual_scan;
mod finalize;
mod flux;
mod flux_scan;
mod media_reservation;
mod office;
mod read_progress;
mod recovery;
mod scan;
mod terminal;

use std::{
    env,
    io::IsTerminal,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    audit,
    batch_extraction::{self, BatchExtractionRequest},
    conversion_run::{self, DEFAULT_CONVERSION_WORKERS},
    external_tools::{self, ToolHealth, ToolKind},
    floppy::{self, FloppyDrive, WriteProtectionStatus},
    greaseweazle::{
        GreaseweazleBackend, GreaseweazleCommand, GreaseweazleDeviceStatus, GreaseweazleProfile,
        ProcessGreaseweazleBackend, parse_info_output,
    },
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
  fluxvault                         Show command help
  fluxvault init [path]             Create a project
  fluxvault status [--project PATH] Show project status
  fluxvault benchmark report [--project PATH]
                                    Export recorded pilot timings and recovery outcomes offline
  fluxvault benchmark compare --baseline ZIP [--include-deleted] [--project PATH]
                                    Compare recovered source payloads with a script archive offline
  fluxvault storage benchmark N [--project PATH]
                                    Measure verified lossless capture compression; no evidence changed
  fluxvault storage pack N [--capture-attempt N] [--retire-raw]
                                    Pack a saved SCP; retain raw unless retirement explicitly requested
  fluxvault storage resume          Finish durable scan packing tasks without hardware
  fluxvault project show [--project PATH]
                                    Show saved project metadata
  fluxvault disk list [--project PATH]
  fluxvault disk show N [--details] [--project PATH]
                                    Inspect saved disk attempts and evidence paths
  fluxvault disk select N [--project PATH]
  fluxvault disk next [--project PATH]
                                    Select the current/next disk number (no drive access)
  fluxvault drive list              List removable drives without reading media
  fluxvault drive probe --drive A:  Read-only 512-byte media and protection probe
  fluxvault acquire --drive A: --disk N [--retries N] --write-blocker-verified
                                    Read-only image; requires independently verified hardware
  fluxvault scan [--last-disk N] [--no-verify] [--conversion-workers N]
                                    Guided Greaseweazle scan; reuses saved project settings
  fluxvault start [scan options]    Same as scan; resumes the same project
  fluxvault stop [--project PATH]   Request safe stop from a second console; wait for STOPPED
  fluxvault run status [--project PATH]
                                    Inspect active controllable work without reading a drive
  fluxvault scan --usb [--drive A:] --write-blocker-verified
                                    USB-only shortcut; numbered labels or legacy READ
  fluxvault scan --double --write-blocker-verified [--last-disk N]
                                    Dual pilot: u1 / g2; p pause / r resume / s status / q drain
  fluxvault scan --double --plan [--last-disk N]
                                    Offline dual-station preview; no drives opened
  fluxvault production status       Inspect saved coordinator state offline
  fluxvault production queue        Rank saved USB partials for GW; no physical read
  fluxvault production benchmark    Inspect durable dual-session timings offline
  fluxvault scan --drive A: [--count N] [--retries N] --write-blocker-verified
                                    Guided read-only USB loop; confirm each numbered label
  fluxvault tools check [--project PATH]
                                    Check external tool versions and record audit
  fluxvault tools show              Show configured tool paths
  fluxvault tools set NAME PATH     Configure sevenzip/libreoffice/greaseweazle
  fluxvault tools clear NAME        Return a tool to auto-discovery
  fluxvault greaseweazle preview    Show safe raw-capture and decode command examples
  fluxvault greaseweazle info [--project PATH]
                                    Query device/firmware with audited read-only gw info
  fluxvault greaseweazle capture N [--gw-drive A|B] [--profile ibm.1440|ibm.720]
      [--revs 1..10] --source-write-protected [--project PATH]
                                    Preserve immutable raw SCP flux; never write the floppy
  fluxvault greaseweazle decode N [--capture-attempt N] [--profile ibm.1440|ibm.720]
                                    Decode saved SCP offline; result remains unverified
  fluxvault greaseweazle identify N [--capture-attempt N]
                                    Identify supported IBM format from saved whole-disk flux offline
  fluxvault greaseweazle status N   Verify saved flux/decode evidence without hardware
  fluxvault diagnose N             Export saved track/revolution/pass/sector diagnostics
  fluxvault greaseweazle compare N  Compare saved USB and flux sectors offline, read-only
  fluxvault greaseweazle consensus N
                                    Cross-check decodes from two raw captures offline
  fluxvault greaseweazle plan N     Rank USB/dual-flux donor candidates offline
  fluxvault greaseweazle recover N [--gw-drive A|B] [--profile auto|ibm.1440|ibm.720]
      --source-write-protected [--policy FILE] [--acquisition-only]
                                    Automatic bounded recovery without a USB reader
  fluxvault greaseweazle scan [--count N] [--last-disk N] [--gw-drive A|B]
      [--profile auto|ibm.1440|ibm.720] [--profile-map FILE] [--policy FILE] [--acquisition-only]
      [--capture-storage packed|raw]
                                    Guided disk swaps, durable numbering, automatic processing
  fluxvault extract all [--project PATH]
                                    Process saved images with the extraction service
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
  fluxvault recovery extract N [--include-deleted] [--project PATH]
                                    Recover intact/signature files offline; deleted opt-in stays forensic-only
  fluxvault recovery documents N [--project PATH]
                                    Native recovery plus forensic Word text salvage
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
  fluxvault processing status      Show durable background work without tools or hardware
  fluxvault processing resume      Drain interrupted saved-image work offline
  fluxvault finalize --destination PATH [--project PATH]
                                    Process saved images, then package only if clean
  fluxvault package build --destination PATH [--project PATH]
                                    Create and verify an archival ZIP
  fluxvault --help                  Show this help
Options:
  --usb                             scan: existing USB-only loop, default Windows A:
  --double                         scan: opt-in simultaneous USB/GW pilot; exact labels
  --plan                           scan --double: offline preview only
  --json                            Output machine-readable JSON
  --project PATH                    Use a specific project instead of searching upward
  --destination PATH                Output folder outside the project
  --drive LETTER:                   Enumerated removable drive for read-only probe
  --disk N                          Disk number for acquisition
  --retries N                       Bad-sector retry passes for acquisition (0-10; default 2)
  --count N                         Stop guided scan after N disks (default: until QUIT)
  --last-disk N                     Stop GW scan after this numbered disk, across restarts
  --write-blocker-verified          Operator asserts separate hardware protection test
  --source DIR                      External recovered-files folder for DMDE import
  --baseline ZIP                    Script archive for recovered-payload comparison
  --include-deleted                  Opt-in forensic deleted recovery (recovery extract), or baseline comparison scope
  --dmde-log FILE                   Matching DMDE log for recovery import
  --conversion-workers N            Parallel Office jobs (1-16; default 4); scan saves this setting
  --details                        Include bad-sector LBAs and evidence paths in disk show
  --gw-drive A|B                   GW selector (capture/recover default A; scan saved/B; not Windows A:)
  --profile NAME                   auto, ibm.1440 or ibm.720 (new scans default auto)
  --capture-storage packed|raw     Scan retention (new scans pack verified complete captures)
  --processing-mode background|tail
                                    Scan processing (new scans default background)
  --retire-raw                     storage pack only: retire raw copy AFTER verified publication
  --profile-map FILE               Known per-disk formats for mixed-format GW scans
  --revs N                         Raw-flux revolutions per track (1-10; default 3)
  --capture-attempt N              Capture to decode/identify/pack (default latest complete)
  --source-write-protected         Confirm the source floppy's physical tab is protected
  --policy FILE                    JSON recovery policy (default: 10 minutes PER STAGE)
  --acquisition-only               Skip downstream processing after recovery
  --no-verify                      GW scan: Enter confirms displayed disk; skips label typing ONLY
  --color auto|always|never         GW scan cues (default auto; respects NO_COLOR)
During scanning: QUIT drains; STOP cancels active work. Windows Ctrl+C requests safe stop.
Keep disks seated until STOPPED and drive activity has stopped. Resume the same command/project.
Exit codes: 0 complete, 3 attention/partial, 2 invalid input or operation error, 130 operator stop"#;

#[derive(Debug)]
pub(crate) struct CliResponse {
    pub(crate) output: String,
    pub(crate) exit_code: i32,
}

pub fn run_from_env() -> i32 {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        println!("{HELP}");
        return 0;
    }
    let json_output = args.iter().any(|argument| argument == "--json");
    let stop_token = crate::cancellation::current();
    if let Err(error) = crate::cancellation::install_console(stop_token.clone()) {
        eprintln!("FluxVault: {error}");
        return 2;
    }
    let _stop_scope = crate::cancellation::enter(stop_token.clone());
    match run(
        &args,
        &env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    ) {
        Ok(response) => {
            println!("{}", response.output);
            if stop_token.requested() {
                eprintln!(
                    "STOPPED / host workers have returned. Wait for drive activity to stop before removing disks; resume the same project and reconfirm pending labels."
                );
            }
            if stop_token.requested() {
                130
            } else {
                response.exit_code
            }
        }
        Err(message) => {
            let message = if stop_token.requested() {
                crate::cancellation::MESSAGE.to_owned()
            } else {
                message
            };
            if stop_token.requested() {
                eprintln!(
                    "STOPPED / host workers have returned. Wait for drive activity to stop before removing disks; resume the same project and reconfirm pending labels."
                );
            }
            if json_output {
                println!("{}", json_error(&message));
            } else {
                eprintln!("FluxVault: {message}");
            }
            if crate::cancellation::stopped(&message) {
                130
            } else {
                2
            }
        }
    }
}

fn json_error(message: &str) -> String {
    json!({"error": {"code": if crate::cancellation::stopped(message) {"operation_cancelled"} else {"operation_error"}, "message": message}}).to_string()
}

pub(crate) fn run(args: &[String], cwd: &Path) -> Result<CliResponse, String> {
    // Help must never become an init path (or trigger tools/media access).
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return Ok(CliResponse {
            output: HELP.to_owned(),
            exit_code: 0,
        });
    }
    let mut json_output = false;
    let mut project_override: Option<PathBuf> = None;
    let mut destination: Option<PathBuf> = None;
    let mut drive_override: Option<String> = None;
    let mut acquisition_disk: Option<u32> = None;
    let mut acquisition_retries: Option<usize> = None;
    let mut scan_count: Option<usize> = None;
    let mut last_disk: Option<u32> = None;
    let mut write_blocker_verified = false;
    let mut import_source: Option<PathBuf> = None;
    let mut import_log: Option<PathBuf> = None;
    let mut baseline_zip: Option<PathBuf> = None;
    let mut include_deleted = false;
    let mut conversion_workers: Option<usize> = None;
    let mut details = false;
    let mut gw_drive: Option<char> = None;
    let mut gw_profile: Option<GreaseweazleProfile> = None;
    let mut automatic_format: Option<bool> = None;
    let mut packed_captures: Option<bool> = None;
    let mut background_processing: Option<bool> = None;
    let mut retire_raw = false;
    let mut gw_revolutions: Option<u32> = None;
    let mut gw_capture_attempt: Option<u32> = None;
    let mut source_write_protected = false;
    let mut recovery_policy: Option<PathBuf> = None;
    let mut profile_map_path: Option<PathBuf> = None;
    let mut acquisition_only = false;
    let mut no_verify = false;
    let mut usb_only = false;
    let mut double = false;
    let mut dual_plan = false;
    let mut color_mode = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json_output = true,
            "--include-deleted" => include_deleted = true,
            "--baseline" => {
                index += 1;
                baseline_zip = Some(PathBuf::from(
                    args.get(index).ok_or("--baseline requires a ZIP file")?,
                ));
            }
            "--details" => details = true,
            "--no-verify" => no_verify = true,
            "--usb" => usb_only = true,
            "--double" => double = true,
            "--plan" => dual_plan = true,
            "--retire-raw" => retire_raw = true,
            "--processing-mode" => {
                index += 1;
                background_processing = Some(match args.get(index).map(String::as_str) {
                    Some("background") => true,
                    Some("tail") => false,
                    _ => return Err("--processing-mode requires background or tail".into()),
                });
            }
            "--capture-storage" => {
                index += 1;
                packed_captures = Some(match args.get(index).map(String::as_str) {
                    Some("packed") => true,
                    Some("raw") => false,
                    _ => return Err("--capture-storage requires packed or raw".to_owned()),
                });
            }
            "--color" => {
                index += 1;
                color_mode = Some(terminal::ColorMode::parse(
                    args.get(index)
                        .ok_or("--color requires auto, always, or never")?,
                )?);
            }
            "--gw-drive" => {
                index += 1;
                let value = args.get(index).ok_or("--gw-drive requires A or B")?;
                gw_drive = match value.to_ascii_uppercase().as_str() {
                    "A" => Some('A'),
                    "B" => Some('B'),
                    _ => return Err("--gw-drive requires A or B (not A: or B:)".to_owned()),
                };
            }
            "--profile" => {
                index += 1;
                let name = args.get(index).ok_or("--profile requires a value")?;
                automatic_format = Some(name == "auto");
                gw_profile = if name == "auto" {
                    None
                } else {
                    Some(GreaseweazleProfile::parse(name)?)
                };
            }
            "--revs" => {
                index += 1;
                gw_revolutions = Some(
                    args.get(index)
                        .ok_or("--revs requires a number")?
                        .parse::<u32>()
                        .ok()
                        .filter(|value| (1..=10).contains(value))
                        .ok_or("--revs must be from 1 to 10")?,
                );
            }
            "--capture-attempt" => {
                index += 1;
                gw_capture_attempt = Some(
                    args.get(index)
                        .ok_or("--capture-attempt requires a number")?
                        .parse::<u32>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or("--capture-attempt requires a positive number")?,
                );
            }
            "--source-write-protected" => source_write_protected = true,
            "--acquisition-only" => acquisition_only = true,
            "--policy" => {
                index += 1;
                recovery_policy = Some(PathBuf::from(
                    args.get(index).ok_or("--policy requires a file")?,
                ));
            }
            "--profile-map" => {
                index += 1;
                if profile_map_path.is_some() {
                    return Err("Only one --profile-map may be supplied".to_owned());
                }
                profile_map_path = Some(PathBuf::from(
                    args.get(index).ok_or("--profile-map requires a file")?,
                ));
            }
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
            "--count" => {
                index += 1;
                scan_count = Some(
                    args.get(index)
                        .ok_or("--count requires a number")?
                        .parse::<usize>()
                        .ok()
                        .filter(|number| *number > 0)
                        .ok_or("--count requires a positive number")?,
                );
            }
            "--last-disk" => {
                index += 1;
                last_disk = Some(
                    args.get(index)
                        .ok_or("--last-disk requires a number")?
                        .parse::<u32>()
                        .ok()
                        .filter(|n| *n > 0 && *n < u32::MAX)
                        .ok_or("--last-disk requires a positive number below 4294967295")?,
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
            value if value.starts_with('-') => {
                return Err(format!("Unknown option: {value}. Run fluxvault --help"));
            }
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }

    if details && !(positional.len() == 3 && positional[0] == "disk" && positional[1] == "show") {
        return Err("--details is only valid with disk show N".to_owned());
    }
    if positional.first().is_some_and(|s| s == "start") {
        positional[0] = "scan".into();
    }
    if (usb_only || double || dual_plan) && positional != ["scan"] {
        return Err("--usb, --double and --plan are only valid with plain scan".into());
    }
    if usb_only && double {
        return Err("Choose --usb or --double, not both".into());
    }
    if dual_plan && !double {
        return Err("--plan currently requires scan --double".into());
    }
    if double {
        if no_verify {
            return Err("Dual mode requires exact label verification; --no-verify cannot select earlier queued USB disks".into());
        }
        if !dual_plan {
            if destination.is_some()
                || acquisition_disk.is_some()
                || acquisition_retries.is_some()
                || scan_count.is_some()
                || import_source.is_some()
                || import_log.is_some()
                || baseline_zip.is_some()
                || include_deleted
                || gw_profile.is_some()
                || automatic_format.is_some()
                || packed_captures.is_some()
                || background_processing.is_some()
                || retire_raw
                || gw_revolutions.is_some()
                || gw_capture_attempt.is_some()
                || recovery_policy.is_some()
                || profile_map_path.is_some()
            {
                return Err("Dual pilot accepts --project, --drive, --gw-drive, --last-disk, --write-blocker-verified, --conversion-workers, --acquisition-only, --color and --json; recovery/format/storage defaults are automatic".into());
            }
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            return dual_scan::run(
                ProjectState::open_without_session(root)?,
                dual_scan::Options {
                    usb: drive_override,
                    gw: gw_drive,
                    last: last_disk,
                    workers: conversion_workers.unwrap_or(DEFAULT_CONVERSION_WORKERS),
                    verified: write_blocker_verified,
                    acquisition_only,
                    json: json_output,
                    color: color_mode.unwrap_or_default().enabled(
                        std::io::stderr().is_terminal(),
                        env::var_os("NO_COLOR").is_some(),
                        env::var("TERM").is_ok_and(|s| s == "dumb"),
                    ),
                },
            );
        }
        if destination.is_some()
            || acquisition_disk.is_some()
            || acquisition_retries.is_some()
            || scan_count.is_some()
            || write_blocker_verified
            || import_source.is_some()
            || import_log.is_some()
            || baseline_zip.is_some()
            || include_deleted
            || conversion_workers.is_some()
            || gw_profile.is_some()
            || automatic_format.is_some()
            || packed_captures.is_some()
            || background_processing.is_some()
            || retire_raw
            || gw_revolutions.is_some()
            || gw_capture_attempt.is_some()
            || source_write_protected
            || recovery_policy.is_some()
            || profile_map_path.is_some()
            || acquisition_only
            || color_mode.is_some()
        {
            return Err("Dual preview accepts only --project, --last-disk, --drive, --gw-drive and --json; no read is started".into());
        }
        let root = resolve_project_root(cwd, project_override.as_deref())?;
        let project = ProjectState::open_without_session(root)?;
        let mut value = crate::production::preview(&project, last_disk)?;
        value["usb_drive"] = json!(drive_override.as_deref().unwrap_or("A:"));
        value["gw_drive"] = json!(gw_drive.unwrap_or('B').to_string());
        return Ok(CliResponse {
            output: if json_output {
                value.to_string()
            } else {
                format!(
                    "DUAL-STATION PREVIEW ONLY - no drives opened, no settings saved.\nUSB {}: fresh first-pass images; partials set aside for GW.\nGW {}: fresh automatic scan/recovery, or type an earlier queued USB label.\nNext available fresh label: {}.\nExact labels required; --no-verify is unavailable in dual mode.\nLive pilot: scan --double --write-blocker-verified; commands uN / gN / STATUS / QUIT.",
                    value["usb_drive"].as_str().unwrap(),
                    value["gw_drive"].as_str().unwrap(),
                    value["next_fresh_disk"]
                        .as_u64()
                        .map(|n| format!("{n:03}"))
                        .unwrap_or("range finished".into())
                )
            },
            exit_code: 0,
        });
    }
    if usb_only && drive_override.is_none() {
        drive_override = Some("A:".into());
    }
    // Keep explicitly selected USB scans intact; the ordinary folder command uses GW.
    if positional == ["scan"]
        && drive_override.is_none()
        && acquisition_retries.is_none()
        && !write_blocker_verified
    {
        positional = vec!["greaseweazle".to_owned(), "scan".to_owned()];
    }
    if positional.len() == 2 && positional[0] == "diagnose" {
        positional = vec![
            "greaseweazle".into(),
            "diagnose".into(),
            positional[1].clone(),
        ];
    }
    let gw_capture =
        positional.len() == 3 && positional[0] == "greaseweazle" && positional[1] == "capture";
    let gw_decode =
        positional.len() == 3 && positional[0] == "greaseweazle" && positional[1] == "decode";
    let gw_identify =
        positional.len() == 3 && positional[0] == "greaseweazle" && positional[1] == "identify";
    let gw_recover =
        positional.len() == 3 && positional[0] == "greaseweazle" && positional[1] == "recover";
    let gw_scan =
        positional.len() == 2 && positional[0] == "greaseweazle" && positional[1] == "scan";
    let storage_pack =
        positional.len() == 3 && positional[0] == "storage" && positional[1] == "pack";
    if packed_captures.is_some() && !gw_scan {
        return Err("--capture-storage is only valid with Greaseweazle scan".to_owned());
    }
    if retire_raw && !storage_pack {
        return Err("--retire-raw is only valid with storage pack".to_owned());
    }
    if (no_verify || color_mode.is_some()) && !gw_scan {
        return Err("--no-verify and --color are only valid with Greaseweazle scan".to_owned());
    }
    if conversion_workers.is_some()
        && !gw_scan
        && positional.first().map(String::as_str) != Some("process")
        && !(positional.len() == 2 && positional[0] == "processing" && positional[1] == "resume")
        && positional.first().map(String::as_str) != Some("finalize")
        && !(positional.len() >= 2
            && positional[0] == "conversion"
            && matches!(positional[1].as_str(), "run" | "retry"))
    {
        return Err("--conversion-workers is only valid with scan, process, processing resume, finalize or conversion run/retry".to_owned());
    }
    if profile_map_path.is_some() && !gw_scan {
        return Err("--profile-map is only valid with greaseweazle scan".to_owned());
    }
    if background_processing.is_some() && !gw_scan {
        return Err("--processing-mode is only valid with Greaseweazle scan".into());
    }
    if (recovery_policy.is_some() || acquisition_only) && !(gw_recover || gw_scan) {
        return Err(
            "--policy and --acquisition-only are only valid with greaseweazle recover/scan"
                .to_owned(),
        );
    }
    if gw_revolutions.is_some() && !gw_capture {
        return Err("--revs is only valid with greaseweazle capture".to_owned());
    }
    if (gw_drive.is_some() || source_write_protected) && !(gw_capture || gw_recover || gw_scan) {
        return Err("--gw-drive and --source-write-protected are only valid with greaseweazle capture/recover/scan".to_owned());
    }
    if gw_capture_attempt.is_some() && !(gw_decode || gw_identify || storage_pack) {
        return Err("--capture-attempt is only valid with greaseweazle decode/identify".to_owned());
    }
    if automatic_format == Some(true) && !(gw_recover || gw_scan) {
        return Err("--profile auto is only valid with greaseweazle recover/scan".to_owned());
    }
    if gw_profile.is_some() && !(gw_capture || gw_decode || gw_recover || gw_scan) {
        return Err(
            "--profile is only valid with greaseweazle capture, decode, recover, or scan"
                .to_owned(),
        );
    }
    if drive_override.is_some()
        && !(positional.len() == 2 && positional[0] == "drive" && positional[1] == "probe")
        && !(positional.len() == 1 && positional[0] == "acquire")
        && !(positional.len() == 1 && positional[0] == "scan")
    {
        return Err("--drive is only valid with drive probe, acquire, or scan".to_owned());
    }
    if acquisition_disk.is_some() && !(positional.len() == 1 && positional[0] == "acquire") {
        return Err("--disk is only valid with acquire".to_owned());
    }
    if (acquisition_retries.is_some() || write_blocker_verified)
        && !(positional.len() == 1 && matches!(positional[0].as_str(), "acquire" | "scan"))
    {
        return Err(
            "--retries and --write-blocker-verified are only valid with acquire or scan".to_owned(),
        );
    }
    if scan_count.is_some() && !(gw_scan || (positional.len() == 1 && positional[0] == "scan")) {
        return Err("--count is only valid with scan or greaseweazle scan".to_owned());
    }
    if last_disk.is_some() && !gw_scan {
        return Err("--last-disk is only valid with greaseweazle scan".to_owned());
    }
    if (import_source.is_some() || import_log.is_some())
        && !(positional.len() == 3 && positional[0] == "recovery" && positional[1] == "import")
    {
        return Err("--source and --dmde-log are only valid with recovery import".to_owned());
    }

    let mut needs_attention = false;
    if baseline_zip.is_some()
        && !(positional.len() == 2 && positional[0] == "benchmark" && positional[1] == "compare")
    {
        return Err("--baseline is only valid with benchmark compare".into());
    }
    if include_deleted
        && !((positional.len() == 2 && positional[0] == "benchmark" && positional[1] == "compare")
            || (positional.len() == 3 && positional[0] == "recovery" && positional[1] == "extract"))
    {
        return Err("--include-deleted is only valid with recovery extract or benchmark compare; default processing excludes known deleted entries".into());
    }
    let workstation_write = matches!(
        positional.first().map(String::as_str),
        Some("process" | "extract" | "files" | "audit" | "report" | "package" | "acquire")
    ) || (positional.first().is_some_and(|s| s == "conversion")
        && positional.get(1).is_some_and(|s| s != "issues"))
        || (positional.first().is_some_and(|s| s == "scan") && drive_override.is_some())
        || (positional.first().is_some_and(|s| s == "recovery")
            && positional
                .get(1)
                .is_some_and(|s| !matches!(s.as_str(), "plan" | "queue" | "compare")))
        || (positional.first().is_some_and(|s| s == "disk")
            && positional
                .get(1)
                .is_some_and(|s| matches!(s.as_str(), "next" | "select")));
    let _workstation_owner = if workstation_write {
        Some(crate::project_work::reserve(&resolve_project_root(
            cwd,
            project_override.as_deref(),
        )?)?)
    } else {
        None
    };
    let output = match positional.first().map(String::as_str) {
        Some("production")
            if positional == ["production", "benchmark"] && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let result = crate::dual_benchmark::report(&project)?;
            let (summary, csv) = crate::dual_benchmark::export(&project, &result)?;
            if json_output {
                Ok(json!({"benchmark":result,"summary":summary,"receipts_csv":csv,"physical_media_access":false}).to_string())
            } else {
                Ok(format!(
                    "Dual benchmark (saved evidence only): {} verified saved labels, {} labels with recorded timings.\n{} finished / {} incomplete invocations; {} numbered read confirmations; {} reader failures.\nRecorded simultaneous reader time: {:.1}s. Finished invocation wall time: {:.1}s (includes swaps/pauses/tail, excludes gaps between invocations).\nWarnings: {}\nSummary: {}\nPer-receipt CSV: {}\nUse --json for session/receipt details. No physical media accessed.",
                    result["verified_saved_unique_labels"],
                    result["timed_saved_unique_labels"],
                    result["finished_sessions"],
                    result["incomplete_sessions"],
                    result["numbered_read_confirmations"],
                    result["reader_failures"],
                    result["recorded_both_readers_overlap_ms"]
                        .as_u64()
                        .unwrap_or(0) as f64
                        / 1000.0,
                    result["finished_session_elapsed_ms"].as_u64().unwrap_or(0) as f64 / 1000.0,
                    result["warnings"],
                    summary.display(),
                    csv.display()
                ))
            }
        }
        Some("production")
            if (positional == ["production", "status"]
                || positional == ["production", "queue"])
                && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let state = crate::production::status(&project)?;
            if json_output {
                Ok(state.to_string())
            } else if positional[1] == "queue" {
                Ok(dual_scan::saved_queue(&state))
            } else {
                Ok(dual_scan::saved_status(&state))
            }
        }
        Some("processing")
            if destination.is_none() && positional.len() == 2 && positional[1] == "status" =>
        {
            let project = ProjectState::open_without_session(resolve_project_root(
                cwd,
                project_override.as_deref(),
            )?)?;
            let state = crate::processing::status(&project)?;
            if json_output {
                Ok(state.to_string())
            } else {
                let budget = &state["worker"]["resource_budget"];
                let budget_text = if budget.is_object() {
                    format!(
                        "{} foreground / {} background / {} waiting; {} background CPU slots",
                        budget["foreground_jobs"],
                        budget["background_jobs"],
                        budget["waiting_jobs"],
                        budget["background_cpu_slots"]
                    )
                } else {
                    "unavailable (older/no saved snapshot)".to_owned()
                };
                Ok(format!(
                    "Background processing: {}\nOwner active: {} | stale recorded stage: {}\nPending/failed jobs: {} | attention jobs: {}\nRecorded resource budget: {}\nDetails: {}\nNo physical media accessed.",
                    state["worker"]["stage"].as_str().unwrap_or("unknown"),
                    state["owner_active"],
                    state["stale_worker_status"],
                    state["pending"],
                    state["attention"],
                    budget_text,
                    project
                        .reports_dir()
                        .join("ProcessingStatus.json")
                        .display()
                ))
            }
        }
        Some("processing")
            if destination.is_none() && positional.len() == 2 && positional[1] == "resume" =>
        {
            let project = ProjectState::open_without_session(resolve_project_root(
                cwd,
                project_override.as_deref(),
            )?)?;
            let _control = crate::run_control::Session::start(&project, "processing_resume")?;
            let outcome = crate::processing::resume(
                &project,
                conversion_workers.unwrap_or(DEFAULT_CONVERSION_WORKERS),
            )?;
            needs_attention = !outcome.errors.is_empty()
                || outcome
                    .last_result
                    .as_ref()
                    .is_some_and(|v| v["exit_code"] != 0);
            if json_output {
                Ok(serde_json::to_string(&outcome).map_err(|e| e.to_string())?)
            } else {
                Ok(format!(
                    "Offline processing: {} coalesced runs, {} jobs completed, {} errors.\n{}\nNo physical media accessed; durable failed jobs remain resumable.",
                    outcome.runs,
                    outcome.processed_jobs,
                    outcome.errors.len(),
                    outcome.errors.join("\n")
                ))
            }
        }
        Some("help") if positional.len() == 1 => Ok(HELP.to_owned()),
        Some("storage")
            if positional.len() == 2 && positional[1] == "resume" && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let _control = crate::run_control::Session::start(&project, "storage_resume")?;
            eprintln!("Resuming durable lossless storage tasks; no board or floppy access...");
            let errors = crate::flux_archive::Queue::start(&project)?.finish();
            let stopped = crate::cancellation::requested();
            return Ok(CliResponse {
                exit_code: if stopped {
                    130
                } else if errors.is_empty() {
                    0
                } else {
                    3
                },
                output: if json_output {
                    json!({"errors":errors,"stopped":stopped,"physical_media_access":false})
                        .to_string()
                } else if stopped {
                    "Capture storage stopped; unfinished tasks and evidence retained. Resume with fv storage resume.".to_owned()
                } else if errors.is_empty() {
                    "Capture storage queue complete; byte-identical originals preserved in verified containers.".to_owned()
                } else {
                    format!(
                        "Capture storage needs attention; tasks and evidence retained:\n{}",
                        errors.join("\n")
                    )
                },
            });
        }
        Some("storage") if storage_pack && destination.is_none() => {
            let disk = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("storage pack requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let attempt = gw_capture_attempt.map_or_else(
                || crate::flux_capture::latest_capture_attempt(&project, disk),
                Ok,
            )?;
            eprintln!(
                "Packing saved disk {disk:03} capture {attempt:03}; verifying full byte-identical decompression..."
            );
            let packed = crate::flux_archive::pack(&project, disk, attempt, retire_raw)?;
            let raw_present = project.root().join("Flux").join(&packed.raw_file).is_file();
            if json_output {
                Ok(json!({"capture":packed,"raw_retirement_requested":retire_raw,"raw_working_copy_present":raw_present,"physical_media_access":false}).to_string())
            } else {
                Ok(format!(
                    "Disk {disk:03} capture {attempt:03}: {:.2} MiB -> {:.2} MiB. SHA-256 roundtrip MATCH.\nOriginal SCP working copy: {}. Packed capture remains available to decode/status/recovery/export.\nNo physical media access.",
                    packed.raw_bytes as f64 / 1048576.0,
                    packed.packed_bytes as f64 / 1048576.0,
                    if raw_present {
                        "retained (use --retire-raw explicitly to retire)"
                    } else {
                        "packed-only; original bytes/hash preserved"
                    }
                ))
            }
        }
        Some("storage")
            if positional.len() == 3 && positional[1] == "benchmark" && destination.is_none() =>
        {
            let disk = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("storage benchmark requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let result =
                crate::flux_storage::benchmark(&project, disk, |stage| eprintln!("{stage}"))?;
            if json_output {
                Ok(serde_json::to_string(&result).map_err(|e| e.to_string())?)
            } else {
                Ok(format!("Lossless capture benchmark: disk {disk:03}, attempt {:03} (largest verified capture; not a whole-project estimate).\n{}\nSource unchanged. No files compressed in place; no physical media access.", result.capture_attempt,
                    result.measurements.iter().map(|m| format!("ZIP/Deflate {}: {:.2} MiB -> {:.2} MiB ({:.1}% saved); {:.2}s compression, {:.2}s roundtrip verification; SHA-256 MATCH", m.level, m.source_bytes as f64 / 1048576.0, m.compressed_bytes as f64 / 1048576.0, m.saved_percent, m.compression_seconds, m.verification_seconds)).collect::<Vec<_>>().join("\n")))
            }
        }
        Some("benchmark")
            if positional.len() == 2 && positional[1] == "compare" && destination.is_none() =>
        {
            let baseline = baseline_zip.ok_or("benchmark compare requires --baseline ZIP")?;
            let baseline = if baseline.is_absolute() {
                baseline
            } else {
                cwd.join(baseline)
            };
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let (result, summary, files_csv) =
                crate::baseline::run(&project, &baseline, include_deleted, &|s| eprintln!("{s}"))?;
            needs_attention = result.missing_payloads > 0
                || result.changed_payloads > 0
                || result.reference_payloads == 0;
            if json_output {
                Ok(json!({"comparison":result,"summary":summary,"files_csv":files_csv,"physical_media_access":false}).to_string())
            } else {
                Ok(format!(
                    "Baseline payload comparison: {} acquired disks; {} reference payloads.\nIdentical bytes: {}; changed: {}; missing: {}; current-only: {}.\nConfirmed deleted reference files: {}. Unknown/carved/ambiguous content remains in scope. This flag controls comparison, not recovery.\nUnscanned reference disks are excluded. Image differences, temporary/empty files and recovery-tool reports are recorded separately; this is not delivery certification.\nSummary: {}\nFile comparison: {}",
                    result.selected_disks.len(),
                    result.reference_payloads,
                    result.matched_payloads,
                    result.changed_payloads,
                    result.missing_payloads,
                    result.current_only_payloads,
                    if result.include_deleted {
                        "included by explicit opt-in"
                    } else {
                        "excluded by default"
                    },
                    summary.display(),
                    files_csv.display()
                ))
            }
        }
        Some("benchmark")
            if positional.len() == 2 && positional[1] == "report" && destination.is_none() =>
        {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let result = crate::benchmark::report(&project)?;
            let (summary, disks_csv) = crate::benchmark::export(&project, &result)?;
            if json_output {
                Ok(
                    json!({"benchmark":result,"summary":summary,"disks_csv":disks_csv,
                    "physical_media_access":false})
                    .to_string(),
                )
            } else {
                Ok(format!(
                    "Pilot benchmark: {} unique disk(s), {} timed physical job(s), {} recovery error(s).\nDownstream: {} operation error(s), {} partial/attention run(s).\nFinished sessions: {}; incomplete/active sessions: {}.\nSummary: {}\nPer-disk CSV: {}\nFeed-only 136-disk projection: {} (observed sample, not a production guarantee).",
                    result.unique_committed_disks,
                    result.timed_physical_jobs,
                    result.recovery_errors,
                    result.downstream_errors,
                    result.downstream_attention,
                    result.finished_sessions,
                    result.incomplete_sessions,
                    summary.display(),
                    disks_csv.display(),
                    result
                        .projected_136_feed_hours
                        .map(|v| format!("{v:.2} hours"))
                        .unwrap_or_else(|| "Not enough physical-read data".to_owned())
                ))
            }
        }
        Some("greaseweazle")
            if positional.len() == 2
                && positional[1] == "preview"
                && project_override.is_none()
                && destination.is_none() =>
        {
            greaseweazle_preview(json_output)
        }
        Some("greaseweazle")
            if positional.len() == 2 && positional[1] == "info" && destination.is_none() =>
        {
            let _reservation = media_reservation::GreaseweazleReservation::acquire()?;
            let settings = external_tools::load_settings()?;
            let audit_path = if project_override.is_some() || discover_project(cwd).is_some() {
                let root = resolve_project_root(cwd, project_override.as_deref())?;
                ProjectState::open_without_session(root)?
                    .logs_dir()
                    .join("external-tools.jsonl")
            } else {
                external_tools::default_audit_path()
            };
            let executable = external_tools::find_ready_tool(
                ToolKind::Greaseweazle,
                settings.path(ToolKind::Greaseweazle),
                &audit_path,
            )?;
            let mut backend = ProcessGreaseweazleBackend::new(executable, audit_path)?;
            let execution = backend.execute(&GreaseweazleCommand::info())?;
            let info = parse_info_output(&execution.output_text());
            needs_attention =
                !execution.success || info.status != GreaseweazleDeviceStatus::Connected;
            if json_output {
                Ok(json!({
                    "success": execution.success,
                    "ready": !needs_attention,
                    "device_status": info.status.as_str(),
                    "host_tools_version": info.host_tools_version,
                    "port": info.port,
                    "model": info.model,
                    "firmware": info.firmware,
                    "exit_code": execution.exit_code,
                    "stdout": execution.stdout,
                    "stderr": execution.stderr,
                    "source_media_access": "read_only"
                })
                .to_string())
            } else {
                let status_label = if execution.success {
                    if needs_attention {
                        match info.status {
                            GreaseweazleDeviceStatus::NotFound => "device not found",
                            GreaseweazleDeviceStatus::Bootloader => "device in bootloader mode",
                            _ => "device not found/unverified",
                        }
                    } else {
                        "ready"
                    }
                } else {
                    "attention required"
                };
                let summary_detail = match (
                    info.model.as_deref(),
                    info.firmware.as_deref(),
                    info.port.as_deref(),
                ) {
                    (Some(m), Some(fw), Some(p)) => format!(" ({m}, fw {fw}, port {p})"),
                    (Some(m), Some(fw), None) => format!(" ({m}, fw {fw})"),
                    _ => String::new(),
                };
                Ok(format!(
                    "Greaseweazle info: {status_label}{summary_detail} (exit {:?})\n{}{}",
                    execution.exit_code, execution.stdout, execution.stderr
                ))
            }
        }
        Some("greaseweazle") if (gw_recover || gw_scan) && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let saved = if gw_scan {
                flux_scan::saved_defaults(&project)?
            } else {
                None
            };
            let policy = match recovery_policy {
                Some(path) => {
                    let path = if path.is_absolute() {
                        path
                    } else {
                        cwd.join(path)
                    };
                    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("Invalid recovery policy: {e}"))?
                }
                None => saved.as_ref().map(|s| s.policy.clone()).unwrap_or_default(),
            };
            if gw_scan {
                return flux_scan::run(
                    project,
                    flux_scan::ScanOptions {
                        background_processing: background_processing.unwrap_or_else(|| {
                            saved.as_ref().is_none_or(|s| s.background_processing)
                        }),
                        packed_captures: packed_captures
                            .unwrap_or_else(|| saved.as_ref().is_none_or(|s| s.packed_captures)),
                        automatic_format: automatic_format
                            .unwrap_or_else(|| saved.as_ref().is_none_or(|s| s.automatic_format)),
                        profile: gw_profile.unwrap_or_else(|| {
                            saved
                                .as_ref()
                                .map_or(GreaseweazleProfile::Ibm1440, |s| s.profile)
                        }),
                        profile_map: match profile_map_path {
                            Some(path) => flux_scan::load_profile_map(&cwd.join(path))?,
                            None => saved
                                .as_ref()
                                .map(|s| s.profile_map.clone())
                                .unwrap_or_default(),
                        },
                        drive: gw_drive.unwrap_or_else(|| saved.as_ref().map_or('B', |s| s.drive)),
                        // The numbered prompt asserts identity AND an open protection tab per disk.
                        protected: true,
                        policy,
                        count: scan_count,
                        last_disk: last_disk.or_else(|| saved.as_ref().and_then(|s| s.last_disk)),
                        acquisition_only,
                        json_output,
                        no_verify,
                        color: color_mode.unwrap_or_default().enabled(
                            std::io::stderr().is_terminal(),
                            env::var_os("NO_COLOR").is_some(),
                            env::var("TERM").is_ok_and(|term| term == "dumb"),
                        ),
                        conversion_workers: conversion_workers.unwrap_or_else(|| {
                            saved
                                .as_ref()
                                .map_or(DEFAULT_CONVERSION_WORKERS, |s| s.conversion_workers)
                        }),
                    },
                );
            }
            let disk = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("recover requires a positive disk number")?;
            return flux::recover(
                &project,
                flux::RecoveryOptions {
                    disk,
                    automatic_format: automatic_format.unwrap_or(false),
                    profile: gw_profile.unwrap_or(GreaseweazleProfile::Ibm1440),
                    drive: gw_drive.unwrap_or('A'),
                    protected: source_write_protected,
                    policy,
                    acquisition_only,
                    json_output,
                },
            );
        }
        Some("greaseweazle") if gw_identify && destination.is_none() => {
            let disk = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("identify requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let attempt = gw_capture_attempt.map_or_else(
                || crate::flux_capture::latest_capture_attempt(&project, disk),
                Ok,
            )?;
            let settings = crate::external_tools::load_settings()?;
            let executable = crate::external_tools::find_offline_greaseweazle(
                settings.path(ToolKind::Greaseweazle),
            )?;
            let mut backend = crate::greaseweazle::ProcessGreaseweazleBackend::new_offline(
                executable,
                project.logs_dir().join("external-tools.jsonl"),
            )?
            .with_stream_to_stderr(false);
            let (decision, report) =
                crate::flux_format::identify(&project, disk, attempt, &mut backend, &|s| {
                    eprintln!("{s}")
                })?;
            return Ok(CliResponse {
                exit_code: if decision.selected_profile.is_some() {
                    0
                } else {
                    3
                },
                output: if json_output {
                    json!({"decision":decision,"report":report,"physical_media_access":false})
                        .to_string()
                } else {
                    format!(
                        "Disk {disk:03} format: {} ({})\nDecision report: {}\nSaved captures only; no physical read.",
                        decision.selected_profile.as_deref().unwrap_or("UNRESOLVED"),
                        decision.reason,
                        report.display()
                    )
                },
            });
        }
        Some("greaseweazle") if gw_capture && destination.is_none() => {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("greaseweazle capture requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return flux::capture(
                &project,
                disk_number,
                gw_profile,
                gw_drive.unwrap_or('A'),
                gw_revolutions.unwrap_or(3),
                source_write_protected,
                json_output,
            );
        }
        Some("greaseweazle") if gw_decode && destination.is_none() => {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("greaseweazle decode requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return flux::decode(
                &project,
                disk_number,
                gw_capture_attempt,
                gw_profile,
                json_output,
            );
        }
        Some("greaseweazle")
            if positional.len() == 3 && positional[1] == "diagnose" && destination.is_none() =>
        {
            let disk = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("diagnose requires a positive disk number")?;
            let project = ProjectState::open_without_session(resolve_project_root(
                cwd,
                project_override.as_deref(),
            )?)?;
            let result = crate::flux_diagnostics::diagnose(&project, disk)?;
            return Ok(CliResponse {
                output: if json_output {
                    serde_json::to_string(&result).map_err(|e| e.to_string())?
                } else {
                    format!(
                        "Disk {disk:03}: {} captures / {} decodes inspected.\nReport: {}\nTrack/revolution CSV: {}\nCommitted-sector CSV: {}\nRecovery note: {}\nSaved evidence only; no hardware/tool calls or recovery/delivery changes.{}",
                        result.captures,
                        result.decodes,
                        result.report.display(),
                        result.tracks_csv.display(),
                        result.sectors_csv.display(),
                        result.note.display(),
                        if result.attention {
                            " Diagnostic attention/partial; see note."
                        } else {
                            ""
                        }
                    )
                },
                exit_code: if result.attention { 3 } else { 0 },
            });
        }
        Some("greaseweazle")
            if positional.len() == 3 && positional[1] == "status" && destination.is_none() =>
        {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("greaseweazle status requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return flux::status(&project, disk_number, json_output);
        }
        Some("greaseweazle")
            if positional.len() == 3 && positional[1] == "compare" && destination.is_none() =>
        {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("greaseweazle compare requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return flux::compare(&project, disk_number, json_output);
        }
        Some("greaseweazle")
            if positional.len() == 3 && positional[1] == "consensus" && destination.is_none() =>
        {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("greaseweazle consensus requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return flux::consensus(&project, disk_number, json_output);
        }
        Some("greaseweazle")
            if positional.len() == 3 && positional[1] == "plan" && destination.is_none() =>
        {
            let disk_number = positional[2]
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("greaseweazle plan requires a positive disk number")?;
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            return flux::plan(&project, disk_number, json_output);
        }
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
        Some("scan") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let mut project = ProjectState::open_without_session(root)?;
            return scan::run(
                &mut project,
                json_output,
                drive_override.as_deref(),
                acquisition_retries.unwrap_or(2),
                scan_count,
                write_blocker_verified,
            );
        }
        Some("stop") if positional.len() == 1 && destination.is_none() => {
            let project = ProjectState::open_without_session(resolve_project_root(
                cwd,
                project_override.as_deref(),
            )?)?;
            let result = crate::run_control::stop(&project)?;
            return Ok(CliResponse {
                exit_code: 0,
                output: if json_output {
                    result.to_string()
                } else {
                    "STOP requested. Wait for the original console to confirm stopped before moving any disk. Resume with the same scan command; pending labels still need confirmation.".into()
                },
            });
        }
        Some("run") if positional == ["run", "status"] && destination.is_none() => {
            let project = ProjectState::open_without_session(resolve_project_root(
                cwd,
                project_override.as_deref(),
            )?)?;
            let result = crate::run_control::status(&project)?;
            return Ok(CliResponse {
                exit_code: 0,
                output: if json_output {
                    result.to_string()
                } else {
                    format!(
                        "Controllable operation active: {}. Saved record: {}. No hardware accessed.",
                        result["active"], result["record"]
                    )
                },
            });
        }
        Some("status") if positional.len() == 1 && destination.is_none() => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let stats = imaging::load_project_statistics(&project.images_dir())?;
            let processing = crate::processing::status(&project)?;
            let raw_exceptions = crate::flux_recovery::format_exceptions(&project)?;
            let mut next_actions = if processing["owner_active"] == true {
                vec![
                    "Saved-file processing is active. Follow the scan's swap prompt; use `fv processing status` for details.",
                ]
            } else {
                status_next_actions(stats.disk_count, stats.partial_disks)
            };
            if !raw_exceptions.is_empty() {
                next_actions.push("Raw-only format exceptions are preserved without supported sector images; inspect `recovery queue` and Flux/Formats reports.");
            }
            needs_attention = stats.partial_disks > 0 || !raw_exceptions.is_empty();
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
                    "background_processing":processing,
                    "raw_format_exceptions":raw_exceptions,
                })
                .to_string())
            } else {
                Ok(format!(
                    "{} ({})\nCurrent disk: {:03}\nImage disks: {} ({} OK, {} partial) | raw-only exceptions: {}\nAttempts: {}\nBest known bad sectors: {}\nBackground: {} | owner active: {} | pending/failed: {}\nNext actions:\n{}",
                    project.name(),
                    project.root().display(),
                    project.current_disk_number(),
                    stats.disk_count,
                    stats.ok_disks,
                    stats.partial_disks,
                    raw_exceptions.len(),
                    stats.total_attempts,
                    stats.best_known_bad_sectors,
                    processing["worker"]["stage"].as_str().unwrap_or("unknown"),
                    processing["owner_active"],
                    processing["pending"],
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
                    "metadata": attempt.metadata_path, "log": attempt.log_file,
                    "timestamp_unix_ms": attempt.timestamp_unix_ms,
                    "total_sectors": attempt.total_sectors,
                    "retry_recovered_sectors": attempt.retry_recovered_sectors,
                    "bad_sectors": attempt.bad_sectors, "attention_required": attempt.attention_required
                })).collect::<Vec<_>>()}).to_string())
            } else {
                Ok(format!(
                    "Disk {disk_number:03}\n{}",
                    attempts
                        .iter()
                        .map(|attempt| {
                            let summary = format!(
                                "  #{:03} | {} | {} bad sectors | {}",
                                attempt.attempt_number,
                                attempt.status,
                                attempt.bad_sectors.len(),
                                attempt.image_file
                            );
                            if details {
                                format!(
                                    "{summary}\n    SHA-256: {}\n    Sectors: {} | retry-recovered: {}\n    Bad LBAs: {:?}\n    Metadata: {}\n    Log: {}",
                                    attempt.sha256,
                                    attempt.total_sectors,
                                    attempt.retry_recovered_sectors,
                                    attempt.bad_sectors,
                                    attempt.metadata_path.display(),
                                    attempt.log_file,
                                )
                            } else {
                                summary
                            }
                        })
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
                        && matches!(
                            positional[1].as_str(),
                            "composite" | "fat" | "import" | "extract" | "documents"
                        ))) =>
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
                include_deleted,
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
            let raw_exceptions = crate::flux_recovery::format_exceptions(&project)?.len();
            needs_attention = result.attention_disks > 0 || raw_exceptions > 0;
            if json_output {
                Ok(json!({"json": result.json_path, "csv": result.csv_path, "disks": result.disk_count,
                    "verified": result.verified_disks, "attention": result.attention_disks,
                    "raw_format_exceptions":raw_exceptions,
                    "customer_delivery_certified": false}).to_string())
            } else {
                Ok(format!(
                    "Evidence audit: {} of {} image evidence sets verified; {} need attention. Raw-only format exceptions: {}.\nReport: {}",
                    result.verified_disks,
                    result.disk_count,
                    result.attention_disks,
                    raw_exceptions,
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
            needs_attention = matches!(
                result.status,
                "recovery" | "in_progress" | "partial_recovered"
            );
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
            let _control = crate::run_control::Session::start(&project, "process")?;
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
            let conversion_state = conversion_run::snapshot_path(&reports_directory);
            needs_attention = result.audit.attention_disks > 0
                || result.raw_format_exceptions > 0
                || result.extraction.recovery_disks > 0
                || result.declined_composites > 0
                || result.declined_recovery_publications > 0
                || result.conversion.partial > 0
                || result.conversion.failed > 0;
            if json_output {
                Ok(json!({"disks": result.extraction.total_disks,
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
                .to_string())
            } else {
                Ok(format!(
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
                ))
            }
        }
        Some("finalize") if positional.len() == 1 => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let destination = destination.ok_or("finalize requires --destination PATH")?;
            return finalize::run(
                cwd,
                root,
                destination,
                conversion_workers.unwrap_or(DEFAULT_CONVERSION_WORKERS),
                json_output,
            );
        }
        Some("package") if positional.len() == 2 && positional[1] == "build" => {
            let root = resolve_project_root(cwd, project_override.as_deref())?;
            let project = ProjectState::open_without_session(root)?;
            let _control = crate::run_control::Session::start(&project, "package")?;
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

fn greaseweazle_preview(json_output: bool) -> Result<String, String> {
    let profiles = [GreaseweazleProfile::Ibm1440, GreaseweazleProfile::Ibm720];
    let examples = profiles
        .into_iter()
        .map(|profile| {
            let raw = GreaseweazleCommand::raw_flux_read(
                profile,
                'A',
                3,
                Path::new("Flux/NNN_attempt_001.scp"),
            )?;
            let decode = GreaseweazleCommand::convert_flux_to_sector_image(
                profile,
                Path::new("Flux/NNN_attempt_001.scp"),
                Path::new("Images/NNN_flux_decode_001.img"),
            )?;
            Ok(json!({
                "profile": profile.argument(),
                "raw_capture": raw.arguments(),
                "sector_decode": decode.arguments(),
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if json_output {
        Ok(
            json!({"examples": examples, "executed": false, "source_media_access": "read_only"})
                .to_string(),
        )
    } else {
        Ok(format!(
            "Greaseweazle command preview (nothing executed):\n{}\nOnly info, raw read, and file-to-file convert are allowed; source-media write/erase commands are forbidden.",
            examples
                .iter()
                .map(|example| format!(
                    "{}:\n  gw {}\n  gw {}",
                    example["profile"].as_str().unwrap_or("unknown"),
                    example["raw_capture"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    example["sector_decode"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                ))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
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
    fn rejects_unknown_commands_without_starting_another_interface() {
        assert!(run(&["acquire".to_owned()], Path::new(".")).is_err());
    }

    #[test]
    fn command_help_never_creates_a_project_or_starts_work() {
        let error: serde_json::Value = serde_json::from_str(&json_error(
            "Host reported an error in customer file [FV_STOPPED].doc",
        ))
        .unwrap();
        assert_eq!(error["error"]["code"], "operation_error");
        let error: serde_json::Value =
            serde_json::from_str(&json_error(crate::cancellation::MESSAGE)).unwrap();
        assert_eq!(error["error"]["code"], "operation_cancelled");
        let root = env::temp_dir().join(format!(
            "fluxvault-help-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms()
        ));
        std::fs::create_dir(&root).unwrap();
        for args in [
            vec!["init", "--help"],
            vec!["init", "unexpected-project", "-h"],
            vec!["scan", "--double", "--help"],
            vec!["start", "-h"],
            vec!["stop", "--help"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            let result = run(&args, &root).unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.output, HELP);
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disk_details_expose_saved_evidence_without_physical_media() {
        let root = env::temp_dir().join(format!(
            "fluxvault-cli-details-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        std::fs::write(project.images_dir().join("001.img"), [0x33; 512]).unwrap();
        let detail = run(
            &[
                "disk".to_owned(),
                "show".to_owned(),
                "1".to_owned(),
                "--details".to_owned(),
            ],
            &root,
        )
        .unwrap();
        assert!(detail.output.contains("Bad LBAs:"));
        assert!(detail.output.contains("Metadata:"));
        let json_response = run(
            &[
                "disk".to_owned(),
                "show".to_owned(),
                "1".to_owned(),
                "--json".to_owned(),
            ],
            &root,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&json_response.output).unwrap();
        assert!(json["attempts"][0]["total_sectors"].is_number());
        assert!(run(&["status".to_owned(), "--details".to_owned()], &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
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
    fn scan_only_custody_and_color_options_cannot_silently_modify_other_commands() {
        for command in ["process", "acquire", "status", "help"] {
            for flag in ["--no-verify", "--color"] {
                let mut args = vec![command.to_owned(), flag.to_owned()];
                if flag == "--color" {
                    args.push("always".to_owned());
                }
                assert!(
                    run(&args, Path::new("."))
                        .unwrap_err()
                        .contains("only valid with Greaseweazle scan")
                );
            }
        }
        assert!(
            run(
                &["scan".to_owned(), "--color".to_owned(), "oops".to_owned()],
                Path::new(".")
            )
            .is_err()
        );
        assert!(
            run(
                &[
                    "scan".to_owned(),
                    "--drive".to_owned(),
                    "A:".to_owned(),
                    "--no-verify".to_owned()
                ],
                Path::new(".")
            )
            .is_err()
        );
    }

    #[test]
    fn init_creates_a_project_without_application_wide_session_state() {
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
