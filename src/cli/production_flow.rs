//! One explicit production owner/controller from feeding through verified ZIP.
//! Child scan/finish commands retain their shared guards and durable journals.
use super::{CliResponse, finalize, flux_scan};
use crate::project::ProjectState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

const RECORD: &str = ".fluxvault-production-workflow.json";
const LIMIT: u64 = 64 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Feeding,
    WaitingForDisks,
    Finishing,
    CompleteClean,
    CompleteAttention,
    Interrupted,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    project: PathBuf,
    first: u32,
    last: u32,
    dual: bool,
    destination: PathBuf,
    workers: usize,
    scan_args: Vec<String>,
    phase: Phase,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    package: Option<Value>,
    #[serde(default)]
    acquisition_attention: bool,
}
#[derive(Default)]
struct Parsed {
    root: Option<PathBuf>,
    destination: Option<PathBuf>,
    last: Option<u32>,
    workers: Option<usize>,
    persistent: Vec<String>,
    ephemeral: Vec<String>,
    json: bool,
    resume: bool,
    dual: bool,
}
fn parse(args: &[String], cwd: &Path) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--json" => p.json = true,
            "--no-verify" => p.ephemeral.push(a.clone()),
            "--double" => {
                p.dual = true;
                p.persistent.push(a.clone());
            }
            "--write-blocker-verified" => {
                p.ephemeral.push(a.clone());
            }
            "--source-write-protected" => {} // custody prompts still assert the tab on each disk
            "--project"
            | "--destination"
            | "--last-disk"
            | "--conversion-workers"
            | "--gw-drive"
            | "--drive"
            | "--profile"
            | "--policy"
            | "--profile-map"
            | "--color"
            | "--sound" => {
                i += 1;
                let value = args.get(i).ok_or_else(|| format!("{a} requires a value"))?;
                match a.as_str() {
                    "--project" => p.root = Some(cwd.join(value)),
                    "--destination" => p.destination = Some(cwd.join(value)),
                    "--last-disk" => {
                        p.last = Some(
                            value
                                .parse::<u32>()
                                .ok()
                                .filter(|n| *n > 0 && *n < u32::MAX)
                                .ok_or(
                                    "--last-disk requires a positive endpoint below 4294967295",
                                )?,
                        )
                    }
                    "--conversion-workers" => {
                        p.workers = Some(
                            value
                                .parse::<usize>()
                                .ok()
                                .filter(|n| (1..=16).contains(n))
                                .ok_or("--conversion-workers requires 1..16")?,
                        )
                    }
                    "--color" | "--sound" => {
                        p.ephemeral.extend([a.clone(), value.clone()]);
                    }
                    _ => {
                        p.persistent.push(a.clone());
                        p.persistent
                            .push(if matches!(a.as_str(), "--policy" | "--profile-map") {
                                cwd.join(value).display().to_string()
                            } else {
                                value.clone()
                            });
                    }
                }
            }
            a if a.starts_with('-') => {
                return Err(format!(
                    "{a} is not supported by production start; use expert scan for acquisition-only/custom tail work"
                ));
            }
            _ => pos.push(a.as_str()),
        }
        i += 1;
    }
    if pos != ["production", "start"] && pos != ["production", "resume"] {
        return Err("Use production start or production resume".into());
    }
    p.resume = pos[1] == "resume";
    if p.dual && p.ephemeral.iter().any(|a| a == "--no-verify") {
        return Err("Dual production requires exact uN/gN labels, not --no-verify".into());
    }
    Ok(p)
}
fn regular(path: &Path, directory: bool) -> Result<(), String> {
    crate::safety::workstation_path(path)?;
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err("Production paths must be regular workstation directories/files".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Unsafe production reparse point".into());
        }
    }
    crate::safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)
}
fn load(project: &ProjectState) -> Result<Option<Record>, String> {
    let path = project.root().join(RECORD);
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    regular(&path, false)?;
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|e| e.to_string())?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Oversized production workflow receipt".into());
    }
    let r: Record = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    if r.schema != 1
        || r.project != root
        || r.first == 0
        || r.last < r.first
        || r.last == u32::MAX
        || u64::from(r.last) - u64::from(r.first) >= 4096
        || !(1..=16).contains(&r.workers)
        || r.scan_args.len() > 128
    {
        return Err("Invalid/foreign production workflow receipt".into());
    }
    let mut args = vec!["production".into(), "start".into()];
    args.extend(r.scan_args.clone());
    let parsed = parse(&args, &root)?;
    if parsed.dual != r.dual
        || parsed.persistent != r.scan_args
        || parsed.root.is_some()
        || parsed.destination.is_some()
        || !parsed.ephemeral.is_empty()
    {
        return Err("Invalid saved production options".into());
    }
    crate::safety::workstation_path(&r.destination)?;
    if !r.destination.is_absolute() || r.destination.starts_with(&root) {
        return Err("Invalid production archive destination".into());
    }
    if matches!(r.phase, Phase::CompleteClean | Phase::CompleteAttention) {
        let package = r
            .package
            .as_ref()
            .ok_or("Finished production receipt has no archive")?;
        let zip = PathBuf::from(
            package["zip"]
                .as_str()
                .ok_or("Invalid production archive receipt")?,
        );
        crate::safety::workstation_path(&zip)?;
        if !zip.is_absolute()
            || zip.parent() != Some(r.destination.as_path())
            || package["sha256"]
                .as_str()
                .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("Invalid finished production archive binding".into());
        }
    }
    Ok(Some(r))
}
fn save(project: &ProjectState, r: &Record) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(r).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Production receipt exceeds its bound".into());
    }
    let path = project.root().join(RECORD);
    if path.try_exists().map_err(|e| e.to_string())? {
        regular(&path, false)?;
    }
    let tmp = project.root().join(format!(
        ".production-flow-{}-{}.partial.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(&bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    fs::rename(tmp, path).map_err(|e| e.to_string())
}
fn destination(root: &Path, supplied: Option<PathBuf>) -> Result<PathBuf, String> {
    let dest = match supplied {
        Some(dest) => dest,
        None => root
            .parent()
            .ok_or("Project needs a parent directory for its delivery folder")?
            .join(format!(
                "{}-Delivery",
                root.file_name()
                    .ok_or("Project needs a directory name")?
                    .to_string_lossy()
            )),
    };
    crate::safety::workstation_path(&dest)?;
    if dest.starts_with(root) || root.starts_with(&dest) {
        return Err(
            "Use an archive directory outside the project, not the project or its ancestor".into(),
        );
    }
    let parent = dest.parent().ok_or("Archive destination has no parent")?;
    regular(parent, true)?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    let dest = parent.join(
        dest.file_name()
            .ok_or("Archive destination needs a directory name")?,
    );
    if dest.starts_with(root) || root.starts_with(&dest) {
        return Err("Archive destination resolves into the project".into());
    }
    if !dest.try_exists().map_err(|e| e.to_string())? {
        fs::create_dir(&dest).map_err(|e| e.to_string())?;
    }
    regular(&dest, true)?;
    let canonical = dest.canonicalize().map_err(|e| e.to_string())?;
    if canonical.starts_with(root) || root.starts_with(&canonical) {
        return Err("Archive destination escapes its expected workstation scope".into());
    }
    Ok(canonical)
}
fn ready(project: &ProjectState, r: &Record) -> Result<bool, String> {
    if !r.dual {
        return Ok(flux_scan::endpoint_outcome(project, r.first, r.last)?.is_some());
    }
    let state = crate::production::status(project)?;
    if state["initialized"] == false {
        return Ok(false);
    }
    if state["first"] != r.first || state["last"] != r.last {
        return Err("Dual production range differs from saved workflow".into());
    }
    let Some(disks) = state["disks"].as_object() else {
        return Ok(false);
    };
    if state["remaining_unclaimed_fresh_labels"].as_u64() != Some(0)
        || state["pending_initial_reads"].as_u64() != Some(0)
        || state["usb_transfer_pending"]
            .as_array()
            .is_none_or(|q| !q.is_empty())
    {
        return Ok(false);
    }
    Ok((r.first..=r.last).all(|n| {
        disks
            .get(&n.to_string())
            .is_some_and(|d| matches!(d["phase"].as_str(), Some("complete" | "partial" | "saved")))
    }))
}
pub(super) fn status(project: &ProjectState) -> Result<Value, String> {
    let record = load(project)?;
    let control = crate::run_control::status(project)?;
    Ok(
        json!({"record":record,"active":control["active"]==true && control["record"]["operation"]=="production_start",
        "receipt_is_historical_not_fresh_verification":true,"physical_media_access":false,
        "resume":"fv production start","finish_policy":"archive partial results with attention reports; never customer-certify"}),
    )
}
pub(super) fn run(args: &[String], cwd: &Path) -> Result<CliResponse, String> {
    let parsed = parse(args, cwd)?;
    crate::safety::workstation_path(cwd)?;
    if let Some(p) = &parsed.root {
        crate::safety::workstation_path(p)?;
    }
    let root = super::resolve_project_root(cwd, parsed.root.as_deref())?;
    let project = crate::sector_inspection::open_project(root)?;
    crate::processing::validate_workspace(&project)?;
    let owner = crate::project_work::reserve(project.root())?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let old = load(&project)?;
    if parsed.resume && old.is_none() {
        return Err("No production workflow to resume; use production start --last-disk N".into());
    }
    let mut record = if let Some(r) = old {
        if parsed.last.is_some_and(|n| n != r.last)
            || parsed.workers.is_some_and(|n| n != r.workers)
            || (!parsed.persistent.is_empty() && parsed.persistent != r.scan_args)
            || parsed
                .destination
                .as_ref()
                .is_some_and(|p| p.canonicalize().ok().as_ref() != Some(&r.destination))
        {
            return Err(
                "Resume with saved range/stations/options; use a new project for a different batch"
                    .into(),
            );
        }
        r
    } else {
        let first = project.current_disk_number();
        let last = parsed
            .last
            .ok_or("First production start requires --last-disk N; subsequent starts reuse it")?;
        if last < first || u64::from(last) - u64::from(first) >= 4096 {
            return Err(
                "Production range must include the current number and at most 4096 disks".into(),
            );
        }
        Record {
            schema: 1,
            project: root.clone(),
            first,
            last,
            dual: parsed.dual,
            destination: destination(&root, parsed.destination.clone())?,
            workers: parsed
                .workers
                .unwrap_or(crate::conversion_run::DEFAULT_CONVERSION_WORKERS),
            scan_args: parsed.persistent.clone(),
            phase: Phase::Feeding,
            error: None,
            package: None,
            acquisition_attention: false,
        }
    };
    if record.dual && parsed.ephemeral.iter().any(|a| a == "--no-verify") {
        return Err("Dual production requires exact uN/gN labels, not --no-verify".into());
    }
    if !record.dual
        && (record.scan_args.iter().any(|a| a == "--drive")
            || parsed
                .ephemeral
                .iter()
                .any(|a| a == "--write-blocker-verified"))
    {
        return Err("USB drive/blocker options require --double in production; single production uses Greaseweazle".into());
    }
    if matches!(
        record.phase,
        Phase::CompleteClean | Phase::CompleteAttention
    ) {
        return Ok(CliResponse {
            exit_code: if record.phase == Phase::CompleteAttention {
                3
            } else {
                0
            },
            output: if parsed.json {
                json!({"workflow":record,"already_finished":true,"receipt_is_historical_not_fresh_verification":true,"customer_delivery_certified":false}).to_string()
            } else {
                format!(
                    "Production already finished. Saved archive receipt: {}\nThis is historical, not a fresh integrity check. Use finalize for a new checked snapshot or a new project for the next batch.",
                    record
                        .package
                        .as_ref()
                        .and_then(|v| v["zip"].as_str())
                        .unwrap_or("See Reports")
                )
            },
        });
    }
    record.destination = destination(&root, Some(record.destination.clone()))?;
    let control = crate::run_control::Session::start(&project, "production_start")?;
    let started = Instant::now();
    let mut outcome = crate::project_work::with_owner(project.root(), &owner, || {
        control.with_children(|| execute(&project, &mut record, &parsed, started))
    })?;
    if outcome.is_err() && crate::cancellation::requested() {
        outcome = Err(crate::cancellation::MESSAGE.into());
    }
    if let Err(error) = &outcome {
        record.phase = if crate::cancellation::stopped(error) {
            Phase::Interrupted
        } else {
            Phase::Failed
        };
        record.error = Some(error.chars().take(4096).collect());
        if let Err(e) = save(&project, &record) {
            eprintln!(
                "Production failure receipt could not be saved: {e}; earlier evidence retained"
            );
        }
    }
    outcome
}
fn execute(
    project: &ProjectState,
    r: &mut Record,
    p: &Parsed,
    started: Instant,
) -> Result<CliResponse, String> {
    crate::cancellation::check()?;
    r.error = None;
    // Always verify saved acquisitions before deciding whether hardware is
    // needed. Interrupted finishing therefore resumes offline, independent of
    // the saved phase label (which is not an integrity certificate).
    let finishing = ready(project, r)?;
    let mut scan = None;
    if !finishing {
        r.phase = Phase::Feeding;
        save(project, r)?;
        eprintln!(
            "PRODUCTION / READ ONLY / {}..{} / scan, recovery, files and archival ZIP are automatic",
            r.first, r.last
        );
        let mut args = vec!["scan".into()];
        args.extend(r.scan_args.clone());
        args.extend(p.ephemeral.clone());
        if r.dual && !args.iter().any(|s| s == "--write-blocker-verified") {
            return Err("Dual production requires --write-blocker-verified on each invocation; confirmations remain exact uN/gN labels".into());
        }
        args.extend([
            "--last-disk".into(),
            r.last.to_string(),
            "--conversion-workers".into(),
            r.workers.to_string(),
            "--project".into(),
            r.project.display().to_string(),
            "--json".into(),
        ]);
        let response = super::run(&args, project.root())?;
        if response.exit_code == 130 {
            return Err(crate::cancellation::MESSAGE.into());
        }
        if !matches!(response.exit_code, 0 | 3) {
            return Err("Production scan did not complete safely".into());
        }
        scan = Some(serde_json::from_str::<Value>(&response.output).map_err(|e| e.to_string())?);
        crate::cancellation::check()?;
        if !ready(project, r)? {
            r.phase = Phase::WaitingForDisks;
            save(project, r)?;
            return Ok(CliResponse {
                exit_code: 3,
                output: if p.json {
                    json!({"workflow":r,"scan":scan,"package":null,"next_action":"resume production start; feed remaining labels and USB-to-GW transfers","session_elapsed_ms":started.elapsed().as_millis(),"customer_delivery_certified":false}).to_string()
                } else {
                    "PRODUCTION PAUSED / saved work retained. Remaining labels or USB-to-GW transfers prevent automatic packaging. Resume: fv production start (add --write-blocker-verified for dual mode).".into()
                },
            });
        }
    }
    crate::cancellation::check()?;
    r.acquisition_attention = if r.dual {
        let state = crate::production::status(project)?;
        state["disks"].as_object().is_some_and(|disks| {
            disks.values().any(|d| {
                d["phase"] == "partial" || d["gw"]["bad"].as_array().is_some_and(|b| !b.is_empty())
            })
        })
    } else {
        flux_scan::endpoint_outcome(project, r.first, r.last)?
            .ok_or("Production endpoint ceased to be complete")?
    };
    r.phase = Phase::Finishing;
    save(project, r)?;
    eprintln!("PRODUCTION / ALL READS SAVED / REMOVE DISKS / finishing from saved evidence only");
    // Explicit production archival policy allows partial evidence with reports;
    // fatal integrity/tool errors still refuse publication. Ordinary finalize
    // retains its strict, attention-blocking default.
    let result = if !r.dual
        && crate::imaging::load_project_statistics(&project.images_dir())?.disk_count == 0
    {
        finalize::raw_endpoint(project, r.first, r.last, r.destination.clone(), r.workers)?
    } else {
        finalize::run(
            project.root(),
            r.project.clone(),
            r.destination.clone(),
            r.workers,
            true,
            true,
        )?
    };
    if !matches!(result.exit_code, 0 | 3) {
        return Err("Production finalization did not complete safely".into());
    }
    let finished: Value = serde_json::from_str(&result.output).map_err(|e| e.to_string())?;
    if finished["package"].is_null() {
        return Err("Production finishing returned no verified archive".into());
    }
    r.package = Some(finished["package"].clone());
    crate::cancellation::check()?;
    let exit_code = if result.exit_code == 3 || r.acquisition_attention {
        3
    } else {
        0
    };
    r.phase = if exit_code == 3 {
        Phase::CompleteAttention
    } else {
        Phase::CompleteClean
    };
    save(project, r)?;
    Ok(CliResponse {
        exit_code,
        output: if p.json {
            json!({"workflow":r,"scan":scan,"finalization":finished,"session_elapsed_ms":started.elapsed().as_millis(),"customer_delivery_certified":false}).to_string()
        } else {
            format!(
                "PRODUCTION FINISHED{} / REMOVE DISKS\nArchive: {}\nSHA-256: {}\nVerified archival ZIP; attention stays in reports, NOT customer-certified.\nElapsed: {:.1}s",
                if exit_code == 3 {
                    " WITH ATTENTION"
                } else {
                    ""
                },
                finished["package"]["zip"].as_str().unwrap_or(""),
                finished["package"]["sha256"].as_str().unwrap_or(""),
                started.elapsed().as_secs_f64()
            )
        },
    })
}

#[cfg(test)]
#[path = "production_flow_tests.rs"]
mod tests;
