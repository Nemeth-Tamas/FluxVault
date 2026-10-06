//! Greaseweazle-only recovery: bounded passes, durable stages, and sector provenance.
use crate::{
    external_tools,
    flux_capture::{self, CaptureRequest},
    greaseweazle::{
        CaptureSettings, GreaseweazleBackend, GreaseweazleCommand, GreaseweazleDeviceStatus,
        GreaseweazleProfile, classify_info_output,
    },
    imaging,
    project::ProjectState,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPass {
    pub name: String,
    pub revolutions: u32,
    pub retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPolicy {
    pub passes: Vec<RecoveryPass>,
    pub max_seconds: u64,
    pub no_improvement_limit: usize,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            passes: [
                ("Fast", 2, 0),
                ("Normal", 3, 2),
                ("Recovery", 5, 3),
                ("Detective", 8, 5),
            ]
            .into_iter()
            .map(|(name, revolutions, retries)| RecoveryPass {
                name: name.to_owned(),
                revolutions,
                retries,
            })
            .collect(),
            max_seconds: 600,
            no_improvement_limit: 2,
        }
    }
}
impl RecoveryPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.passes.is_empty()
            || self.passes.len() > 8
            || !(30..=1800).contains(&self.max_seconds)
            || !(1..=3).contains(&self.no_improvement_limit)
        {
            return Err(
                "Policy needs 1-8 passes, 30-1800 seconds, and 1-3 non-improving passes".to_owned(),
            );
        }
        for p in &self.passes {
            if p.name.is_empty() || !(1..=10).contains(&p.revolutions) || p.retries > 10 {
                return Err("Each pass needs a name, 1-10 revolutions, and 0-10 retries".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stage {
    capture_attempt: u32,
    decode_attempt: Option<u32>,
    settings: CaptureSettings,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    schema_version: u32,
    disk: u32,
    profile: String,
    drive: char,
    started_unix_ms: u64,
    policy: RecoveryPolicy,
    stages: Vec<Stage>,
    result: Option<RecoveryResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryResult {
    pub disk: u32,
    pub status: String,
    pub stop_reason: String,
    pub capture_attempts: Vec<u32>,
    pub image: PathBuf,
    pub image_sha256: String,
    pub provenance: PathBuf,
    pub provenance_sha256: String,
    pub missing_lbas: Vec<u64>,
    pub conflicting_lbas: Vec<u64>,
    pub corroborated_sectors: usize,
    pub single_capture_sectors: usize,
    pub physical_reads_this_run: usize,
    pub resumed: bool,
}
#[derive(Debug, Clone, Serialize)]
struct SectorProvenance {
    lba: u64,
    capture_attempts: Vec<u32>,
    decode_attempts: Vec<u32>,
    confidence: String,
}
struct Evidence {
    bytes: Vec<u8>,
    sectors: Vec<SectorProvenance>,
    missing: Vec<u64>,
    conflicts: Vec<u64>,
    controls: Vec<u32>,
}
fn unfinished(e: &Evidence) -> usize {
    e.missing.len() + e.conflicts.len()
}

fn aggregate(
    project: &ProjectState,
    disk: u32,
    profile: GreaseweazleProfile,
    stages: &[Stage],
) -> Result<Evidence, String> {
    let count = profile.expected_sector_image_bytes() as usize / 512;
    let status = flux_capture::inspect_disk(project, disk)?;
    let mut sources = Vec::new();
    for stage in stages {
        if let Some(number) = stage.decode_attempt {
            let d = status
                .decodes
                .iter()
                .find(|d| d.capture_attempt == stage.capture_attempt && d.decode_attempt == number)
                .ok_or("Saved recovery decode is missing")?;
            if d.profile != profile.argument() {
                return Err("Recovery profile changed".to_owned());
            }
            sources.push((d, flux_capture::verified_decode(project, d, count)?));
        }
    }
    if sources.is_empty() {
        return Err("No completed recovery decode".to_owned());
    }
    let mut e = Evidence {
        bytes: vec![0; count * 512],
        sectors: Vec::with_capacity(count),
        missing: Vec::new(),
        conflicts: Vec::new(),
        controls: Vec::new(),
    };
    let spt = count / 160;
    // Re-read two clean control cylinders to detect common custody mistakes.
    // Matching controls support identity, but cannot prove it.
    for c in 0..80 {
        if (c * 2 * spt..(c + 1) * 2 * spt).all(|lba| !sources[0].1.1.contains(&(lba as u64))) {
            e.controls.push(c as u32);
            if e.controls.len() == 2 {
                break;
            }
        }
    }
    for (_, (bytes, bad)) in sources.iter().skip(1) {
        let matching_controls = e
            .controls
            .iter()
            .flat_map(|c| {
                let start = *c as usize * 2 * spt;
                start..start + 2 * spt
            })
            .filter(|lba| {
                !bad.contains(&(*lba as u64))
                    && bytes[*lba * 512..(*lba + 1) * 512]
                        == sources[0].1.0[*lba * 512..(*lba + 1) * 512]
            })
            .count();
        if matching_controls < 16 {
            return Err(
                "Not enough matching control sectors to accept another capture; evidence retained"
                    .to_owned(),
            );
        }
    }
    for lba in 0..count {
        let range = lba * 512..(lba + 1) * 512;
        let good = sources
            .iter()
            .filter(|(_, (_, bad))| !bad.contains(&(lba as u64)))
            .collect::<Vec<_>>();
        let conflict = good.first().is_some_and(|first| {
            good.iter()
                .skip(1)
                .any(|s| s.1.0[range.clone()] != first.1.0[range.clone()])
        });
        if conflict {
            e.conflicts.push(lba as u64);
        } else if let Some(first) = good.first() {
            e.bytes[range.clone()].copy_from_slice(&first.1.0[range]);
        } else {
            e.missing.push(lba as u64);
        }
        e.sectors.push(SectorProvenance {
            lba: lba as u64,
            capture_attempts: good.iter().map(|(d, _)| d.capture_attempt).collect(),
            decode_attempts: good.iter().map(|(d, _)| d.decode_attempt).collect(),
            confidence: if conflict {
                "conflict"
            } else if good.is_empty() {
                "unreadable"
            } else if good.len() > 1 {
                "corroborated"
            } else {
                "single_capture_gw_reported_good"
            }
            .to_owned(),
        });
    }
    if e.conflicts
        .iter()
        .any(|lba| e.controls.contains(&((*lba as usize / (spt * 2)) as u32)))
    {
        return Err(
            "Control sectors disagree; disk identity may have changed. No image published"
                .to_owned(),
        );
    }
    Ok(e)
}

pub fn recover(
    project: &ProjectState,
    disk: u32,
    profile: GreaseweazleProfile,
    drive: char,
    policy: RecoveryPolicy,
    backend: &mut impl GreaseweazleBackend,
    progress: &impl Fn(&str),
) -> Result<RecoveryResult, String> {
    policy.validate()?;
    if disk == 0 || !matches!(drive, 'A' | 'B') {
        return Err("Recovery requires a positive disk number and drive A or B".to_owned());
    }
    let flux = flux_capture::project_flux_dir(project)?;
    let dir = flux.join("Recovery");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    if dir.parent() != Some(flux.as_path()) {
        return Err("Recovery directory escapes Flux".to_owned());
    }
    let lock_path = dir.join(format!("{disk:03}.lock"));
    if lock_path.exists()
        && !fs::symlink_metadata(&lock_path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
    {
        return Err("Unsafe recovery lock".to_owned());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "This disk already has a running recovery job".to_owned())?;
    let state = dir.join(format!("{disk:03}_job.json"));
    let resumed = state.exists();
    let mut j: Journal = if resumed {
        if !fs::symlink_metadata(&state)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        {
            return Err("Unsafe recovery journal".to_owned());
        }
        serde_json::from_slice(&fs::read(&state).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid recovery journal: {e}"))?
    } else {
        Journal {
            schema_version: 1,
            disk,
            profile: profile.argument().to_owned(),
            drive,
            started_unix_ms: external_tools::current_unix_ms(),
            policy: policy.clone(),
            stages: Vec::new(),
            result: None,
        }
    };
    if j.schema_version != 1
        || j.disk != disk
        || j.profile != profile.argument()
        || j.drive != drive
        || j.policy != policy
    {
        return Err("Saved job has different disk/settings/policy; new reads refused".to_owned());
    }
    if j.stages.len() > policy.passes.len()
        || j.stages
            .iter()
            .map(|s| s.capture_attempt)
            .collect::<BTreeSet<_>>()
            .len()
            != j.stages.len()
    {
        return Err("Recovery journal has duplicated captures or too many stages".to_owned());
    }
    for stage in &j.stages {
        stage.settings.validate()?;
    }
    if !resumed && flux_capture::latest_capture_attempt(project, disk).is_ok() {
        let status = flux_capture::inspect_disk(project, disk)?;
        if let Some(d) = status.decodes.iter().rev().find(|d| {
            d.profile == profile.argument()
                && d.capture_settings
                    .as_ref()
                    .is_none_or(|s| s.cylinders.is_none())
        }) {
            flux_capture::verified_decode(
                project,
                d,
                profile.expected_sector_image_bytes() as usize / 512,
            )?;
            progress(&format!(
                "Using saved whole-disk capture #{} as the first pass",
                d.capture_attempt
            ));
            j.stages.push(Stage {
                capture_attempt: d.capture_attempt,
                decode_attempt: Some(d.decode_attempt),
                settings: d.capture_settings.clone().unwrap_or(CaptureSettings {
                    cylinders: None,
                    retries: 3,
                }),
            });
        }
    }
    if let Some(mut result) = j.result.clone() {
        aggregate(project, disk, profile, &j.stages)?;
        if hash_path(&result.image)? != result.image_sha256
            || hash_path(&result.provenance)? != result.provenance_sha256
        {
            return Err("Completed recovery output changed".to_owned());
        }
        result.resumed = true;
        result.physical_reads_this_run = 0;
        return Ok(result);
    }
    save_journal(&state, &j)?;
    let mut reads = 0;
    let mut no_improvement = 0;
    let mut previous = profile.expected_sector_image_bytes() as usize / 512;
    let mut reason = "pass_limit";
    for (index, pass) in policy.passes.iter().enumerate() {
        if index < j.stages.len() && j.stages[index].decode_attempt.is_some() {
            let remaining = unfinished(&aggregate(project, disk, profile, &j.stages[..=index])?);
            no_improvement = if index > 0 && remaining >= previous {
                no_improvement + 1
            } else {
                0
            };
            previous = remaining;
            if remaining == 0 {
                reason = "complete_sector_map";
                break;
            }
            if no_improvement >= policy.no_improvement_limit {
                reason = "no_improvement";
                break;
            }
            continue;
        }
        let elapsed = external_tools::current_unix_ms().saturating_sub(j.started_unix_ms) / 1000;
        let pending_raw = j.stages.get(index).is_some_and(|stage| {
            flux.join(format!(
                "{disk:03}_attempt_{:03}.json",
                stage.capture_attempt
            ))
            .exists()
        });
        if elapsed >= policy.max_seconds && !pending_raw {
            reason = "time_limit";
            break;
        }
        if index >= j.stages.len() {
            let cylinders = if index == 0 {
                None
            } else {
                let e = aggregate(project, disk, profile, &j.stages)?;
                let spt = profile.expected_sector_image_bytes() as usize / 512 / 160;
                let mut cs = e
                    .missing
                    .iter()
                    .chain(&e.conflicts)
                    .map(|lba| (*lba as usize / (spt * 2)) as u32)
                    .collect::<BTreeSet<_>>();
                cs.extend(e.controls);
                Some(cs.into_iter().collect())
            };
            j.stages.push(Stage {
                capture_attempt: flux_capture::next_capture_attempt(&flux, disk)?,
                decode_attempt: None,
                settings: CaptureSettings {
                    cylinders,
                    retries: pass.retries,
                },
            });
            save_journal(&state, &j)?;
        }
        let stage = j.stages[index].clone();
        if !flux
            .join(format!(
                "{disk:03}_attempt_{:03}.json",
                stage.capture_attempt
            ))
            .exists()
        {
            backend.set_operation_timeout(Duration::from_secs(15));
            let info = backend.execute(&GreaseweazleCommand::info())?;
            if !info.success
                || classify_info_output(&info.output_text()) != GreaseweazleDeviceStatus::Connected
            {
                return Err(
                    "Greaseweazle board is absent or unverified; no physical capture started"
                        .to_owned(),
                );
            }
            j.stages[index].capture_attempt = flux_capture::next_capture_attempt(&flux, disk)?;
            save_journal(&state, &j)?;
            progress(&format!(
                "{} pass: {}",
                pass.name,
                stage
                    .settings
                    .cylinders
                    .as_ref()
                    .map(|v| format!("rereading cylinders {v:?}"))
                    .unwrap_or_else(|| "reading the whole floppy".to_owned())
            ));
            let remaining = policy.max_seconds.saturating_sub(
                external_tools::current_unix_ms().saturating_sub(j.started_unix_ms) / 1000,
            );
            if remaining == 0 {
                return Err(
                    "Capture time ceiling reached before starting the physical read".to_owned(),
                );
            }
            backend.set_operation_timeout(Duration::from_secs(remaining.min(300)));
            let captured = flux_capture::capture_with_settings(
                project,
                CaptureRequest {
                    disk_number: disk,
                    profile,
                    drive,
                    revolutions: pass.revolutions,
                },
                &stage.settings,
                backend,
            )?;
            if captured.attempt_number != j.stages[index].capture_attempt {
                return Err("Capture slot changed during recovery".to_owned());
            }
            reads += 1;
        }
        let capture = j.stages[index].capture_attempt;
        let existing = flux_capture::inspect_disk(project, disk)?
            .decodes
            .into_iter()
            .rfind(|d| {
                d.capture_attempt == capture
                    && d.profile == profile.argument()
                    && d.output_hash_matches
                    && d.source_hash_matches
            });
        let number = match existing {
            Some(d) => {
                flux_capture::verified_decode(
                    project,
                    &d,
                    profile.expected_sector_image_bytes() as usize / 512,
                )?;
                d.decode_attempt
            }
            None => {
                let remaining = policy.max_seconds.saturating_sub(
                    external_tools::current_unix_ms().saturating_sub(j.started_unix_ms) / 1000,
                );
                // Finishing a saved raw capture offline never adds physical media stress.
                backend.set_operation_timeout(Duration::from_secs(if remaining == 0 {
                    60
                } else {
                    remaining.min(60)
                }));
                flux_capture::decode(project, disk, capture, Some(profile), backend)?.decode_attempt
            }
        };
        j.stages[index].decode_attempt = Some(number);
        save_journal(&state, &j)?;
        let e = aggregate(project, disk, profile, &j.stages)?;
        let remaining = unfinished(&e);
        progress(&format!(
            "{} pass: {} sectors available, {} missing, {} conflicting",
            pass.name,
            e.sectors.len() - remaining,
            e.missing.len(),
            e.conflicts.len()
        ));
        no_improvement = if index > 0 && remaining >= previous {
            no_improvement + 1
        } else {
            0
        };
        previous = remaining;
        if remaining == 0 {
            reason = "complete_sector_map";
            break;
        }
        if no_improvement >= policy.no_improvement_limit {
            reason = "no_improvement";
            break;
        }
    }
    let e = aggregate(project, disk, profile, &j.stages)?;
    let mut result = publish(project, &dir, &j, &e, reason)?;
    result.physical_reads_this_run = reads;
    result.resumed = resumed;
    j.result = Some(result.clone());
    save_journal(&state, &j)?;
    Ok(result)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("Cannot reserve {}: {e}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

/// Recheck a published batch result without invoking a host tool or reading media.
pub(crate) fn verify_completed_result(
    project: &ProjectState,
    result: &RecoveryResult,
) -> Result<(), String> {
    let flux = flux_capture::project_flux_dir(project)?;
    let dir = flux
        .join("Recovery")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if dir.parent() != Some(flux.as_path()) {
        return Err("Recovery directory escapes Flux".to_owned());
    }
    let state = dir.join(format!("{:03}_job.json", result.disk));
    if !fs::symlink_metadata(&state)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Unsafe recovery journal".to_owned());
    }
    let job: Journal = serde_json::from_slice(&fs::read(state).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid recovery journal: {e}"))?;
    let saved = job
        .result
        .as_ref()
        .ok_or("Recovery result was not committed")?;
    if job.schema_version != 1
        || job.disk != result.disk
        || saved.disk != result.disk
        || saved.image != result.image
        || saved.image_sha256 != result.image_sha256
        || saved.provenance != result.provenance
        || saved.provenance_sha256 != result.provenance_sha256
        || saved.status != result.status
        || saved.missing_lbas != result.missing_lbas
        || saved.conflicting_lbas != result.conflicting_lbas
    {
        return Err("Batch result disagrees with committed recovery evidence".to_owned());
    }
    job.policy.validate()?;
    aggregate(
        project,
        result.disk,
        GreaseweazleProfile::parse(&job.profile)?,
        &job.stages,
    )?;
    if hash_path(&result.image)? != result.image_sha256
        || hash_path(&result.provenance)? != result.provenance_sha256
    {
        return Err("Completed recovery output changed; disk numbering not advanced".to_owned());
    }
    Ok(())
}
fn save_journal(path: &Path, j: &Journal) -> Result<(), String> {
    let tmp = path.with_extension(format!(
        "{}-{}.partial.json",
        std::process::id(),
        external_tools::current_unix_ms()
    ));
    write_new(
        &tmp,
        &serde_json::to_vec_pretty(j).map_err(|e| e.to_string())?,
    )?;
    fs::rename(tmp, path).map_err(|e| format!("Cannot commit recovery journal: {e}"))
}
fn hash_path(path: &Path) -> Result<String, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Expected regular recovery artifact".to_owned());
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(fs::read(path).map_err(|e| e.to_string())?)
    ))
}

/// Commit a newly derived image without replacing evidence that appeared
/// after slot selection. std::fs::rename can replace an existing destination.
fn publish_image_no_replace(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{Win32::Storage::FileSystem::MoveFileW, core::PCWSTR};
        let source = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // Both buffers are live and null-terminated. MoveFileW deliberately
        // fails if the destination exists (unlike MoveFileExW with replacement).
        unsafe { MoveFileW(PCWSTR(source.as_ptr()), PCWSTR(destination.as_ptr())) }
            .map_err(|e| format!("Cannot publish image without overwriting evidence: {e}"))
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(source, destination)
            .map_err(|e| format!("Cannot publish image without overwriting evidence: {e}"))?;
        fs::remove_file(source).map_err(|e| e.to_string())
    }
}

fn publish(
    project: &ProjectState,
    dir: &Path,
    j: &Journal,
    e: &Evidence,
    reason: &str,
) -> Result<RecoveryResult, String> {
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let images = project
        .images_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let logs = project
        .logs_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if images.parent() != Some(root.as_path()) || logs.parent() != Some(root.as_path()) {
        return Err("Image/log directory escapes project".to_owned());
    }
    let attempt = imaging::next_attempt_number(&images, j.disk)?;
    let stem = format!("{:03}_attempt_{attempt:03}", j.disk);
    let image = images.join(format!("{stem}.img"));
    let metadata = images.join(format!("{stem}.json"));
    let partial = images.join(format!("{stem}.partial.img"));
    let log = logs.join(format!("{stem}.log"));
    let provenance = dir.join(format!("{stem}_provenance.json"));
    let sha256 = format!("{:x}", Sha256::digest(&e.bytes));
    let unresolved = e
        .missing
        .iter()
        .chain(&e.conflicts)
        .copied()
        .collect::<BTreeSet<_>>();
    let status = if unresolved.is_empty() {
        "OK"
    } else {
        "PARTIAL"
    };
    let spt = e.sectors.len() / 160;
    let prov = serde_json::to_vec_pretty(&json!({
        "schema_version": 1,
        "disk": j.disk,
        "profile": j.profile,
        "policy": j.policy,
        "stages": j.stages,
        "image_sha256": sha256,
        "sectors": e.sectors,
        "note": "Single-capture sectors have lower confidence. Unreadable/conflicting sectors are explicitly zero-filled. No customer-delivery certification."
    })).map_err(|e| e.to_string())?;
    write_new(&provenance, &prov)?;
    write_new(&partial, &e.bytes)?;
    let mut text = format!(
        "BEGIN | disk={} | attempt={attempt} | source=greaseweazle-derived\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track={spt} | bytes_per_sector=512 | total_sectors={} | total_bytes={}\n",
        j.disk,
        e.sectors.len(),
        e.bytes.len()
    );
    for lba in &unresolved {
        text.push_str(&format!("BAD_SECTOR | lba={lba}\n"));
    }
    text.push_str(&format!(
        "END | status={status} | bad_sectors={} | retry_recovered=0 | bytes={} | sha256={sha256}\n",
        unresolved.len(),
        e.bytes.len()
    ));
    write_new(&log, text.as_bytes())?;
    let prov_hash = format!("{:x}", Sha256::digest(&prov));
    let bad_sectors = unresolved
        .iter()
        .map(|lba| {
            json!({
                "lba": lba,
                "cylinder": lba / (spt as u64 * 2),
                "head": lba / spt as u64 % 2,
                "sector": lba % spt as u64 + 1
            })
        })
        .collect::<Vec<_>>();
    let meta = serde_json::to_vec_pretty(&json!({
        "fluxvault_version": env!("CARGO_PKG_VERSION"),
        "status": status,
        "disk_number": j.disk,
        "attempt_number": attempt,
        "source_backend": "greaseweazle-derived",
        "source_device": format!("Greaseweazle drive {}", j.drive),
        "image_file": image,
        "log_file": log,
        "timestamp_unix_ms": external_tools::current_unix_ms(),
        "geometry": {
            "cylinders": 80, "heads": 2, "sectors_per_track": spt,
            "bytes_per_sector": 512, "total_bytes": e.bytes.len(), "format_guess": j.profile
        },
        "sector_retries": 0,
        "total_sectors": e.sectors.len(),
        "bytes_written": e.bytes.len(),
        "retry_recovered_sectors": 0,
        "bad_sector_count": unresolved.len(),
        "bad_sectors": bad_sectors,
        "sha256": sha256,
        "flux_provenance": provenance,
        "flux_provenance_sha256": prov_hash
    }))
    .map_err(|e| e.to_string())?;
    publish_image_no_replace(&partial, &image)?;
    write_new(&metadata, &meta)?;
    Ok(RecoveryResult {
        disk: j.disk,
        status: if unresolved.is_empty() {
            "acquired"
        } else if unresolved.len() == e.sectors.len() {
            "unrecoverable_within_policy"
        } else {
            "partial"
        }
        .to_owned(),
        stop_reason: reason.to_owned(),
        capture_attempts: j.stages.iter().map(|s| s.capture_attempt).collect(),
        image,
        image_sha256: sha256,
        provenance,
        provenance_sha256: prov_hash,
        missing_lbas: e.missing.clone(),
        conflicting_lbas: e.conflicts.clone(),
        corroborated_sectors: e
            .sectors
            .iter()
            .filter(|s| s.confidence == "corroborated")
            .count(),
        single_capture_sectors: e
            .sectors
            .iter()
            .filter(|s| s.confidence == "single_capture_gw_reported_good")
            .count(),
        physical_reads_this_run: 0,
        resumed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishing_never_replaces_an_existing_image() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-publish-{}-{}",
            std::process::id(),
            external_tools::current_unix_ms()
        ));
        fs::create_dir(&root).unwrap();
        let partial = root.join("new.partial.img");
        let image = root.join("existing.img");
        write_new(&partial, b"new evidence").unwrap();
        write_new(&image, b"original evidence").unwrap();
        assert!(publish_image_no_replace(&partial, &image).is_err());
        assert_eq!(fs::read(&image).unwrap(), b"original evidence");
        assert_eq!(fs::read(&partial).unwrap(), b"new evidence");
        let unused = root.join("unused.img");
        publish_image_no_replace(&partial, &unused).unwrap();
        assert_eq!(fs::read(&unused).unwrap(), b"new evidence");
        assert!(!partial.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
