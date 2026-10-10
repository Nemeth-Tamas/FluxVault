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
    io::{Read, Write},
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
    #[serde(default)]
    pub time_limit_scope: TimeLimitScope,
    pub no_improvement_limit: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeLimitScope {
    #[default]
    PerStage,
    WholeJob,
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
            time_limit_scope: TimeLimitScope::PerStage,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stage {
    capture_attempt: u32,
    decode_attempt: Option<u32>,
    settings: CaptureSettings,
    /// Cumulative physical capture time, including failed/reseat attempts.
    #[serde(default)]
    capture_elapsed_ms: u64,
    /// Durable in-flight marker: a process restart cannot renew this budget.
    #[serde(default)]
    capture_started_unix_ms: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    schema_version: u32,
    disk: u32,
    profile: String,
    #[serde(default)]
    automatic_format: bool,
    drive: char,
    started_unix_ms: u64,
    #[serde(default)]
    empty_capture_budget_restarts: Vec<u64>,
    /// Missing in older whole-job journals; migrate unfinished jobs once only.
    #[serde(default)]
    stage_budget_version: u32,
    policy: RecoveryPolicy,
    stages: Vec<Stage>,
    result: Option<RecoveryResult>,
    /// Operator-rejected identity evidence stays on disk but is never a donor.
    #[serde(default)]
    rejected_capture_attempts: BTreeSet<u32>,
    #[serde(default)]
    read_conflicts: Option<crate::read_conflicts::Confirmation>,
}

fn capture_remaining_ms(job: &Journal, index: usize, now: u64) -> u64 {
    let spent = match job.policy.time_limit_scope {
        TimeLimitScope::WholeJob => now.saturating_sub(job.started_unix_ms),
        TimeLimitScope::PerStage => job.stages.get(index).map_or(0, |s| {
            s.capture_elapsed_ms.saturating_add(
                s.capture_started_unix_ms
                    .map_or(0, |start| now.saturating_sub(start)),
            )
        }),
    };
    (job.policy.max_seconds * 1000).saturating_sub(spent)
}

fn finish_capture_clock(stage: &mut Stage, now: u64) {
    if let Some(started) = stage.capture_started_unix_ms.take() {
        stage.capture_elapsed_ms = stage
            .capture_elapsed_ms
            .saturating_add(now.saturating_sub(started));
    }
}

fn stage_capture_slot(
    flux: &Path,
    disk: u32,
    stages: &[Stage],
    replacing: Option<usize>,
) -> Result<u32, String> {
    let reserved_next = stages
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != replacing)
        .map(|(_, stage)| stage.capture_attempt)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("Capture attempt limit reached")?;
    flux_capture::next_capture_attempt_from(flux, disk, reserved_next)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryResult {
    pub disk: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_profile: Option<String>,
    pub status: String,
    pub stop_reason: String,
    pub capture_attempts: Vec<u32>,
    #[serde(default, skip_serializing_if = "empty_path")]
    pub image: PathBuf,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image_sha256: String,
    pub provenance: PathBuf,
    pub provenance_sha256: String,
    pub missing_lbas: Vec<u64>,
    pub conflicting_lbas: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_conflict_lbas: Vec<u64>,
    pub corroborated_sectors: usize,
    pub single_capture_sectors: usize,
    pub physical_reads_this_run: usize,
    pub resumed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_exception: Option<FormatException>,
}

/// No supported geometry was established. Preserve raw evidence, never invent
/// a sector image or claim a known readable/missing-sector count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatException {
    pub capture_attempt: u32,
    pub source_sha256: String,
    pub reason: String,
}

fn empty_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Deserialize)]
struct CatalogProvenance {
    schema_version: u32,
    disk: u32,
    profile: String,
    policy: RecoveryPolicy,
    stages: Vec<Stage>,
    image_sha256: String,
    sectors: Vec<SectorProvenance>,
    #[serde(default)]
    read_conflicts: Option<crate::read_conflicts::Confirmation>,
}

/// Replay the immutable publication's own stages, not the latest mutable job.
/// This verifies lineage/agreement only: vendor-reported good is NOT native CRC proof.
pub(crate) fn verify_catalog_metadata(
    images: &Path,
    value: &serde_json::Value,
) -> Result<(), String> {
    if value["source_backend"] != "greaseweazle-derived" {
        if value.get("flux_provenance").is_some() || value.get("flux_provenance_sha256").is_some() {
            return Err("Flux lineage was relabelled as another acquisition backend".into());
        }
        return Ok(());
    }
    crate::cancellation::check()?;
    let images = images.canonicalize().map_err(|e| e.to_string())?;
    let root = images.parent().ok_or("Missing catalog project root")?;
    let directory = root.join("Flux").join("Recovery");
    if directory.canonicalize().map_err(|e| e.to_string())? != directory {
        return Err("Flux lineage directory is redirected".into());
    }
    let provenance = crate::recovery_plan::resolve_image_path(
        &directory,
        value["flux_provenance"]
            .as_str()
            .ok_or("Missing flux lineage path")?,
    )?;
    let read = |path: &Path, limit: u64| -> Result<Vec<u8>, String> {
        let info = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if info.file_attributes() & 0x400 != 0 {
                return Err("Flux lineage contains a reparse point".into());
            }
        }
        if !info.file_type().is_file() || info.len() > limit {
            return Err("Flux lineage artifact exceeds its regular-file bound".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("Flux lineage artifact grew beyond its bound".into());
        }
        Ok(bytes)
    };
    let proof_bytes = read(&provenance, 4 * 1024 * 1024)?;
    let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    if value["flux_provenance_sha256"] != hash(&proof_bytes) {
        return Err("Catalog flux lineage hash changed".into());
    }
    let proof: CatalogProvenance = serde_json::from_slice(&proof_bytes)
        .map_err(|e| format!("Invalid catalog flux lineage: {e}"))?;
    proof.policy.validate()?;
    if proof.schema_version != 1
        || proof.disk == 0
        || value["disk_number"] != proof.disk
        || proof.stages.is_empty()
        || proof.stages.len() > proof.policy.passes.len()
        || proof
            .stages
            .iter()
            .map(|s| s.capture_attempt)
            .collect::<BTreeSet<_>>()
            .len()
            != proof.stages.len()
    {
        return Err("Invalid catalog flux lineage identity/stage inventory".into());
    }
    for stage in &proof.stages {
        stage.settings.validate()?;
        if stage.capture_attempt == 0 || stage.decode_attempt == Some(0) {
            return Err("Invalid catalog flux capture/decode identity".into());
        }
    }
    let profile = GreaseweazleProfile::parse(&proof.profile)?;
    let project = ProjectState::open_without_session(root.to_owned())?;
    let replay = aggregate(&project, proof.disk, profile, &proof.stages)?;
    let image = crate::recovery_plan::resolve_image_path(
        &images,
        value["image_file"]
            .as_str()
            .ok_or("Missing catalog image")?,
    )?;
    let image_bytes = read(&image, profile.expected_sector_image_bytes())?;
    let bad = value["bad_sectors"]
        .as_array()
        .ok_or("Missing catalog flux map")?
        .iter()
        .map(|b| b["lba"].as_u64().ok_or("Invalid catalog flux LBA"))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_bad = replay
        .missing
        .iter()
        .chain(&replay.conflicts)
        .copied()
        .collect::<BTreeSet<_>>();
    let sectors = replay.bytes.len() / 512;
    if image_bytes != replay.bytes
        || proof.sectors != replay.sectors
        || hash(&image_bytes) != proof.image_sha256
        || value["sha256"] != proof.image_sha256
        || bad.len() != expected_bad.len()
        || bad.iter().copied().collect::<BTreeSet<_>>() != expected_bad
        || value["total_sectors"] != sectors
        || value["bytes_written"] != image_bytes.len()
        || value["bad_sector_count"] != bad.len()
        || value["status"] != if bad.is_empty() { "OK" } else { "PARTIAL" }
        || value["geometry"]["cylinders"] != 80
        || value["geometry"]["heads"] != 2
        || value["geometry"]["sectors_per_track"] != sectors / 160
        || value["geometry"]["bytes_per_sector"] != 512
        || value["geometry"]["total_bytes"] != image_bytes.len()
    {
        return Err("Catalog flux image/map/origins disagree with saved capture replay".into());
    }
    let recorded: Option<crate::read_conflicts::Confirmation> = serde_json::from_value(
        value
            .get("read_conflicts")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|e| e.to_string())?;
    if recorded != proof.read_conflicts {
        return Err(
            "Catalog read-conflict confirmation disagrees with immutable publication".into(),
        );
    }
    if let Some(approval) = &proof.read_conflicts {
        if approval.disk != proof.disk {
            return Err("Wrong confirmed disk identity".into());
        }
        crate::read_conflicts::verify(&project, approval, &image_bytes, &bad)?;
    }
    crate::cancellation::check()?;
    if read(&provenance, 4 * 1024 * 1024)? != proof_bytes {
        return Err("Catalog flux lineage changed during replay".into());
    }
    Ok(())
}

fn aggregate(
    project: &ProjectState,
    disk: u32,
    profile: GreaseweazleProfile,
    stages: &[Stage],
) -> Result<Evidence, String> {
    let count = profile.expected_sector_image_bytes() as usize / 512;
    let status = flux_capture::inspect_saved_decodes(project, disk)?;
    let mut sources = Vec::new();
    for stage in stages {
        if let Some(number) = stage.decode_attempt {
            let d = status
                .decodes
                .iter()
                .find(|d| {
                    d.capture_attempt == stage.capture_attempt
                        && d.decode_attempt == number
                        && d.profile == profile.argument()
                })
                .ok_or("Saved recovery decode is missing")?;
            if d.profile != profile.argument() {
                return Err("Recovery profile changed".to_owned());
            }
            if d.capture_settings
                .as_ref()
                .is_some_and(|settings| settings != &stage.settings)
            {
                return Err("Recovery stage settings disagree with saved capture/decode".into());
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
    recover_impl(
        project,
        disk,
        profile,
        false,
        drive,
        policy,
        backend,
        progress,
        &|_, _| Ok(()),
    )
}

/// Capture once, select a supported format offline, then target its missing sectors.
pub fn recover_auto(
    project: &ProjectState,
    disk: u32,
    drive: char,
    policy: RecoveryPolicy,
    backend: &mut impl GreaseweazleBackend,
    progress: &impl Fn(&str),
) -> Result<RecoveryResult, String> {
    recover_impl(
        project,
        disk,
        GreaseweazleProfile::Ibm1440,
        true,
        drive,
        policy,
        backend,
        progress,
        &|_, _| Ok(()),
    )
}

/// Dual-station acceptance runs before publication, including reused results.
pub type AcceptanceCheck<'a> = dyn Fn(&[u8], &[u64]) -> Result<(), String> + 'a;

pub fn recover_auto_checked(
    project: &ProjectState,
    disk: u32,
    drive: char,
    policy: RecoveryPolicy,
    backend: &mut impl GreaseweazleBackend,
    progress: &impl Fn(&str),
    accept: &AcceptanceCheck<'_>,
) -> Result<RecoveryResult, String> {
    recover_impl(
        project,
        disk,
        GreaseweazleProfile::Ibm1440,
        true,
        drive,
        policy,
        backend,
        progress,
        accept,
    )
}

// Keep the public capture variants' explicit safety/policy inputs visible at
// their shared implementation boundary; do not collapse them into optional flags.
#[allow(clippy::too_many_arguments)]
fn recover_impl(
    project: &ProjectState,
    disk: u32,
    mut profile: GreaseweazleProfile,
    automatic_format: bool,
    drive: char,
    policy: RecoveryPolicy,
    backend: &mut impl GreaseweazleBackend,
    progress: &impl Fn(&str),
    accept: &AcceptanceCheck<'_>,
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
            automatic_format,
            drive,
            started_unix_ms: external_tools::current_unix_ms(),
            empty_capture_budget_restarts: Vec::new(),
            stage_budget_version: 1,
            policy: policy.clone(),
            stages: Vec::new(),
            result: None,
            rejected_capture_attempts: BTreeSet::new(),
            read_conflicts: None,
        }
    };
    if j.schema_version != 1
        || j.disk != disk
        || j.automatic_format != automatic_format
        || (!automatic_format && j.profile != profile.argument())
        || j.drive != drive
        || j.policy != policy
    {
        return Err("Saved job has different disk/settings/policy; new reads refused".to_owned());
    }
    profile = GreaseweazleProfile::parse(&j.profile)?;
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
    let rejected = rejected_capture_attempts(project, disk)?;
    if j.stages
        .iter()
        .any(|s| rejected.contains(&s.capture_attempt))
    {
        return Err("Current recovery references identity-rejected captures; no read or publication authorized".into());
    }
    if j.stage_budget_version > 1 {
        return Err("Unsupported recovery stage budget version".to_owned());
    }
    if !resumed && automatic_format && flux_capture::latest_capture_attempt(project, disk).is_ok() {
        let status = flux_capture::inspect_disk(project, disk)?;
        if let Some(capture) = status.captures.iter().rev().find(|c| {
            c.status == "complete"
                && c.hash_matches
                && c.capture_settings
                    .as_ref()
                    .is_none_or(|s| s.cylinders.is_none())
        }) {
            progress(&format!(
                "Using saved whole-disk capture #{} for automatic format identification",
                capture.attempt
            ));
            j.stages.push(Stage {
                capture_attempt: capture.attempt,
                decode_attempt: None,
                settings: capture.capture_settings.clone().unwrap_or(CaptureSettings {
                    cylinders: None,
                    retries: 3,
                }),
                capture_elapsed_ms: 0,
                capture_started_unix_ms: None,
            });
        }
    }
    if !resumed && !automatic_format && flux_capture::latest_capture_attempt(project, disk).is_ok()
    {
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
                capture_elapsed_ms: 0,
                capture_started_unix_ms: None,
            });
        }
    }
    if let Some(mut result) = j.result.clone() {
        verify_completed_result(project, &result)?;
        if result.format_exception.is_none() {
            let bytes = fs::read(&result.image).map_err(|e| e.to_string())?;
            let bad = result
                .missing_lbas
                .iter()
                .chain(&result.conflicting_lbas)
                .copied()
                .collect::<Vec<_>>();
            accept(&bytes, &bad)?;
        }
        result.resumed = true;
        result.physical_reads_this_run = 0;
        return Ok(result);
    }
    if let Some(approval) = &j.read_conflicts {
        // Confirmation authorizes this exact finished saved candidate, not
        // another physical pass or a different source edition.
        let evidence = aggregate(project, disk, profile, &j.stages)?;
        let bad: Vec<_> = evidence
            .missing
            .iter()
            .chain(&evidence.conflicts)
            .copied()
            .collect();
        crate::read_conflicts::verify(project, approval, &evidence.bytes, &bad)?;
        accept(&evidence.bytes, &bad)?;
        let mut result = publish(
            project,
            &dir,
            &j,
            &evidence,
            "operator_confirmed_minor_cross_reader_disagreement",
        )?;
        result.resumed = true;
        result.physical_reads_this_run = 0;
        j.result = Some(result.clone());
        save_journal(&state, &j)?;
        return Ok(result);
    }
    if j.policy.time_limit_scope == TimeLimitScope::PerStage {
        if j.stage_budget_version == 0 {
            j.stage_budget_version = 1;
            progress(
                "Upgrading unfinished recovery to per-stage capture budgets; saved passes and failed attempts retained.",
            );
        }
        for stage in &mut j.stages {
            finish_capture_clock(stage, external_tools::current_unix_ms());
        }
    }
    // An operator-confirmed retry after an empty first capture gets a fresh bounded
    // window. Never reset a job that has any full/partial raw evidence or a decode.
    if resumed
        && j.policy.time_limit_scope == TimeLimitScope::WholeJob
        && j.stages.len() <= 1
        && j.stages.iter().all(|s| s.decode_attempt.is_none())
        && external_tools::current_unix_ms().saturating_sub(j.started_unix_ms) / 1000
            >= policy.max_seconds
        && !fs::read_dir(&flux)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with(&format!("{disk:03}_attempt_"))
                    && (name.ends_with(".scp")
                        || (name.ends_with(".json") && !name.ends_with(".partial.json")))
            })
    {
        j.empty_capture_budget_restarts.push(j.started_unix_ms);
        j.started_unix_ms = external_tools::current_unix_ms();
        progress(
            "Restarting the bounded first-pass budget after a capture produced no raw file; prior attempts retained.",
        );
    }
    save_journal(&state, &j)?;
    let mut reads = 0;
    let mut no_improvement = 0;
    let mut previous = profile.expected_sector_image_bytes() as usize / 512;
    let mut reason = "pass_limit";
    for (index, pass) in policy.passes.iter().enumerate() {
        crate::cancellation::check()?;
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
        let pending_raw = j.stages.get(index).is_some_and(|stage| {
            flux.join(format!(
                "{disk:03}_attempt_{:03}.json",
                stage.capture_attempt
            ))
            .exists()
        });
        if capture_remaining_ms(&j, index, external_tools::current_unix_ms()) == 0 && !pending_raw {
            reason = if policy.time_limit_scope == TimeLimitScope::PerStage {
                "stage_time_limit"
            } else {
                "time_limit"
            };
            if policy.time_limit_scope == TimeLimitScope::PerStage
                && j.stages.iter().any(|s| s.decode_attempt.is_some())
            {
                progress(&format!(
                    "{} stage capture budget exhausted; using verified earlier passes and advancing within the saved recovery policy.",
                    pass.name
                ));
                continue;
            }
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
                capture_attempt: stage_capture_slot(&flux, disk, &j.stages, None)?,
                decode_attempt: None,
                settings: CaptureSettings {
                    cylinders,
                    retries: pass.retries,
                },
                capture_elapsed_ms: 0,
                capture_started_unix_ms: None,
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
            j.stages[index].capture_attempt =
                stage_capture_slot(&flux, disk, &j.stages, Some(index))?;
            save_journal(&state, &j)?;
            progress(&format!(
                "{} pass: {} [capture budget: {:.1}s remaining; {}]",
                pass.name,
                stage
                    .settings
                    .cylinders
                    .as_ref()
                    .map(|v| format!("rereading cylinders {v:?}"))
                    .unwrap_or_else(|| "reading the whole floppy".to_owned()),
                capture_remaining_ms(&j, index, external_tools::current_unix_ms()) as f64 / 1000.0,
                if policy.time_limit_scope == TimeLimitScope::PerStage {
                    "per stage"
                } else {
                    "whole job"
                },
            ));
            let remaining = capture_remaining_ms(&j, index, external_tools::current_unix_ms());
            if remaining == 0 {
                reason = if policy.time_limit_scope == TimeLimitScope::PerStage {
                    "stage_time_limit"
                } else {
                    "time_limit"
                };
                if policy.time_limit_scope == TimeLimitScope::PerStage {
                    continue;
                }
                break;
            }
            let operation_ms = if policy.time_limit_scope == TimeLimitScope::PerStage {
                remaining
            } else {
                remaining.min(300_000)
            };
            backend.set_operation_timeout(Duration::from_millis(operation_ms));
            if policy.time_limit_scope == TimeLimitScope::PerStage {
                j.stages[index].capture_started_unix_ms = Some(external_tools::current_unix_ms());
                save_journal(&state, &j)?;
            }
            let capture_outcome = flux_capture::capture_with_settings_at_attempt(
                project,
                CaptureRequest {
                    disk_number: disk,
                    profile,
                    drive,
                    revolutions: pass.revolutions,
                },
                &stage.settings,
                backend,
                j.stages[index].capture_attempt,
            );
            if policy.time_limit_scope == TimeLimitScope::PerStage {
                finish_capture_clock(&mut j.stages[index], external_tools::current_unix_ms());
                save_journal(&state, &j)?;
            }
            let captured = match capture_outcome {
                Ok(captured) => captured,
                Err(error)
                    if error.starts_with(flux_capture::CAPTURE_TIMEOUT_ERROR_PREFIX)
                        && operation_ms == remaining
                        && capture_remaining_ms(&j, index, external_tools::current_unix_ms())
                            == 0
                        && j.stages.iter().any(|s| s.decode_attempt.is_some()) =>
                {
                    // A bounded policy deadline is a normal recovery stop,
                    // not loss of the earlier independently verified passes.
                    // Never decode/promote an unfinished SCP or reset budget.
                    reads += 1;
                    reason = if policy.time_limit_scope == TimeLimitScope::PerStage {
                        "stage_time_limit"
                    } else {
                        "time_limit"
                    };
                    progress(&format!(
                        "Capture time limit reached during {} pass. Interrupted capture retained separately; verified earlier passes preserved.",
                        pass.name
                    ));
                    if policy.time_limit_scope == TimeLimitScope::PerStage {
                        continue;
                    }
                    break;
                }
                Err(error) => return Err(error),
            };
            if captured.attempt_number != j.stages[index].capture_attempt {
                return Err("Capture slot changed during recovery".to_owned());
            }
            reads += 1;
        }
        let capture = j.stages[index].capture_attempt;
        if automatic_format && index == 0 && j.stages[index].decode_attempt.is_none() {
            let (decision, report) =
                crate::flux_format::identify(project, disk, capture, backend, progress)?;
            let selected = match decision.selected_profile.as_deref() {
                Some(selected) => selected,
                None if decision.candidates.len() == 2
                    && decision.candidates.iter().all(|c| c.error.is_none()) =>
                {
                    let result = RecoveryResult {
                        disk, selected_profile:None, status:"raw_format_exception".into(),
                        stop_reason:"supported_formats_ambiguous_or_insufficient; raw evidence preserved; no geometry assumed".into(),
                        capture_attempts:vec![capture], image:PathBuf::new(), image_sha256:String::new(),
                        provenance:report.clone(),provenance_sha256:hash_path(&report)?,missing_lbas:vec![],conflicting_lbas:vec![],read_conflict_lbas:vec![],
                        corroborated_sectors:0,single_capture_sectors:0,physical_reads_this_run:reads,resumed,
                        format_exception:Some(FormatException {capture_attempt:capture,source_sha256:decision.source_sha256.clone(),reason:decision.reason.clone()}),
                    };
                    j.result = Some(result.clone());
                    save_journal(&state, &j)?;
                    verify_completed_result(project, &result)?;
                    progress(&format!(
                        "RAW-ONLY FORMAT EXCEPTION {disk:03}: raw capture and format report verified; no sector image claimed. Report: {}",
                        report.display()
                    ));
                    return Ok(result);
                }
                None => {
                    return Err(format!(
                        "Cannot safely identify disk {disk:03}: {}. A decoder trial failed; raw preserved, number not advanced. Decision: {}",
                        decision.reason,
                        report.display()
                    ));
                }
            };
            profile = GreaseweazleProfile::parse(selected)?;
            j.profile = selected.to_owned();
            let chosen = decision
                .candidates
                .iter()
                .find(|c| c.profile == selected)
                .ok_or("Selected format has no candidate")?;
            j.stages[index].decode_attempt = chosen.decode_attempt;
            save_journal(&state, &j)?;
            progress(&format!(
                "Detected {selected}: {}. Evidence: {}",
                decision.reason,
                report.display()
            ));
        }
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
                // Finishing a saved raw capture offline never adds physical media stress.
                backend.set_operation_timeout(Duration::from_secs(60));
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
    let bad = e
        .missing
        .iter()
        .chain(&e.conflicts)
        .copied()
        .collect::<Vec<_>>();
    accept(&e.bytes, &bad)?;
    if let Some(approval) = &j.read_conflicts {
        crate::read_conflicts::verify(project, approval, &e.bytes, &bad)?;
    }
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

fn identity_job(
    project: &ProjectState,
    disk: u32,
) -> Result<Option<(std::path::PathBuf, Vec<u8>, Journal)>, String> {
    let flux = flux_capture::project_flux_dir(project)?;
    let dir = flux.join("Recovery");
    if !dir.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    if dir.parent() != Some(flux.as_path()) {
        return Err("Recovery directory escapes Flux".into());
    }
    let path = dir.join(format!("{disk:03}_job.json"));
    let info = match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(info) => info,
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if info.file_attributes() & 0x400 != 0 {
            return Err("Unsafe recovery journal reparse point".into());
        }
    }
    if !info.is_file() || info.len() > 1048576 {
        return Err("Unsafe or oversized recovery journal".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(1048577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1048576 {
        return Err("Oversized recovery journal".into());
    }
    let job: Journal = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if job.schema_version != 1
        || job.disk != disk
        || job.rejected_capture_attempts.len() > 4096
        || job.rejected_capture_attempts.contains(&0)
    {
        return Err("Invalid identity recovery journal".into());
    }
    Ok(Some((path, bytes, job)))
}

pub(crate) fn confirmed_read_conflicts(
    project: &ProjectState,
    disk: u32,
) -> Result<Option<crate::read_conflicts::Confirmation>, String> {
    Ok(identity_job(project, disk)?.and_then(|(_, _, job)| job.read_conflicts))
}

pub(crate) fn confirm_same_disk(
    project: &ProjectState,
    disk: u32,
    usb_attempt: u32,
    generation: u64,
) -> Result<crate::read_conflicts::Confirmation, String> {
    let (path, bytes, mut job) =
        identity_job(project, disk)?.ok_or("No saved GW candidate to confirm")?;
    if job.result.is_some()
        || job.stages.is_empty()
        || job.stages.iter().any(|s| s.decode_attempt.is_none())
    {
        return Err(
            "Confirmation requires an unpublished, fully decoded candidate; no active read".into(),
        );
    }
    job.policy.validate()?;
    if job.stages.len() > job.policy.passes.len() {
        return Err("Invalid candidate stages".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.parent().unwrap().join(format!("{disk:03}.lock")))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "Recovery is active; same-disk confirmation refused".to_string())?;
    let evidence = aggregate(
        project,
        disk,
        GreaseweazleProfile::parse(&job.profile)?,
        &job.stages,
    )?;
    let bad: Vec<_> = evidence
        .missing
        .iter()
        .chain(&evidence.conflicts)
        .copied()
        .collect();
    let approval = crate::read_conflicts::build(
        project,
        disk,
        usb_attempt,
        generation,
        external_tools::current_unix_ms(),
        &evidence.bytes,
        &bad,
    )?;
    if fs::read(&path).map_err(|e| e.to_string())? != bytes {
        return Err("Candidate job changed during confirmation".into());
    }
    job.read_conflicts = Some(approval.clone());
    save_journal(&path, &job)?;
    Ok(approval)
}

pub(crate) fn rejected_capture_attempts(
    project: &ProjectState,
    disk: u32,
) -> Result<BTreeSet<u32>, String> {
    // Immutable rejection archives, not the mutable current job, own this
    // exclusion. Older catalog replay must not depend on a later job's schema.
    let flux = flux_capture::project_flux_dir(project)?;
    let dir = flux.join("Recovery");
    if !dir.try_exists().map_err(|e| e.to_string())? {
        return Ok(BTreeSet::new());
    }
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    if dir.parent() != Some(flux.as_path()) {
        return Err("Rejection directory escapes Flux".into());
    }
    let prefix = format!("{disk:03}_identity_rejected_");
    let mut rejected = BTreeSet::new();
    let mut count = 0;
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name
            .strip_prefix(&prefix)
            .and_then(|n| n.strip_suffix(".json"))
            .and_then(|n| n.parse::<u64>().ok())
            .is_none()
        {
            continue;
        }
        count += 1;
        if count > 4096 {
            return Err("Too many identity rejection records".into());
        }
        let info = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if info.file_attributes() & 0x400 != 0 {
                return Err("Unsafe identity rejection reparse point".into());
            }
        }
        if !info.is_file() || info.len() > 1048576 {
            return Err("Unsafe identity rejection record".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(entry.path())
            .map_err(|e| e.to_string())?
            .take(1048577)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1048576 {
            return Err("Oversized identity rejection record".into());
        }
        let job: Journal = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if job.disk != disk
            || job.schema_version != 1
            || job.result.is_some()
            || job.stages.len() > 8
        {
            return Err("Invalid identity rejection binding".into());
        }
        rejected.extend(job.stages.iter().map(|s| s.capture_attempt));
        rejected.extend(job.rejected_capture_attempts);
        if rejected.len() > 4096 || rejected.contains(&0) {
            return Err("Invalid rejected capture inventory".into());
        }
    }
    Ok(rejected)
}

/// Under the project owner, archive an UNPUBLISHED mismatched job and restart
/// with fresh attempt slots. Original raw/decoded bytes and IDs remain intact.
pub(crate) fn reject_identity_job(
    project: &ProjectState,
    disk: u32,
    generation: u64,
) -> Result<(), String> {
    let Some((path, bytes, mut job)) = identity_job(project, disk)? else {
        return Ok(());
    };
    if job.result.is_some() {
        return Err("Cannot release a published recovery result as a mistaken identity".into());
    }
    job.policy.validate()?;
    if job.stages.len() > job.policy.passes.len()
        || job.stages.iter().any(|s| s.capture_attempt == 0)
    {
        return Err("Invalid recovery stages; identity release refused".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.parent().unwrap().join(format!("{disk:03}.lock")))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "Recovery is still active; identity release refused".to_string())?;
    let archive = path.with_file_name(format!("{disk:03}_identity_rejected_{generation}.json"));
    if archive.try_exists().map_err(|e| e.to_string())? {
        let info = fs::symlink_metadata(&archive).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if info.file_attributes() & 0x400 != 0 {
                return Err("Unsafe rejection archive".into());
            }
        }
        if !info.is_file() || info.len() > 1048576 {
            return Err("Unsafe rejection archive".into());
        }
        let old = fs::read(&archive).map_err(|e| e.to_string())?;
        let archived: Journal = serde_json::from_slice(&old).map_err(|e| e.to_string())?;
        if archived.disk != disk || archived.schema_version != 1 || archived.result.is_some() {
            return Err("Changed identity rejection archive; release refused".into());
        }
        if job.stages.is_empty()
            && archived
                .stages
                .iter()
                .all(|s| job.rejected_capture_attempts.contains(&s.capture_attempt))
        {
            // Journal reset committed, but production custody commit was interrupted.
            return Ok(());
        }
        if old != bytes {
            return Err("Changed identity rejection archive; release refused".into());
        }
    }
    if fs::read(&path).map_err(|e| e.to_string())? != bytes {
        return Err("Recovery job changed during identity release".into());
    }
    job.rejected_capture_attempts
        .extend(job.stages.iter().map(|s| s.capture_attempt));
    if job.rejected_capture_attempts.len() > 4096 {
        return Err("Identity rejection history exceeds bound".into());
    }
    if !archive.try_exists().map_err(|e| e.to_string())? {
        write_new(&archive, &bytes)?;
    }
    job.stages.clear();
    job.read_conflicts = None;
    job.started_unix_ms = external_tools::current_unix_ms();
    job.empty_capture_budget_restarts.clear();
    job.stage_budget_version = 1;
    save_journal(&path, &job)
}

/// Read a completed job's selected format; this hint never replaces hash checks.
pub(crate) fn completed_profile(
    project: &ProjectState,
    disk: u32,
) -> Result<Option<String>, String> {
    let path = flux_capture::project_flux_dir(project)?
        .join("Recovery")
        .join(format!("{disk:03}_job.json"));
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(info) if !info.file_type().is_file() || info.len() > 1024 * 1024 => {
            return Err("Unsafe recovery journal".to_owned());
        }
        _ => {}
    }
    let job: Journal = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid recovery journal: {e}"))?;
    if job.schema_version != 1 || job.disk != disk {
        return Err("Recovery journal identity mismatch".to_owned());
    }
    GreaseweazleProfile::parse(&job.profile)?;
    Ok(job
        .result
        .as_ref()
        .filter(|r| r.format_exception.is_none())
        .map(|_| job.profile))
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
        || saved.selected_profile != result.selected_profile
        || result
            .selected_profile
            .as_ref()
            .is_some_and(|p| p != &job.profile)
        || saved.image != result.image
        || saved.image_sha256 != result.image_sha256
        || saved.provenance != result.provenance
        || saved.provenance_sha256 != result.provenance_sha256
        || saved.status != result.status
        || saved.missing_lbas != result.missing_lbas
        || saved.conflicting_lbas != result.conflicting_lbas
        || saved.read_conflict_lbas != result.read_conflict_lbas
        || saved.capture_attempts != result.capture_attempts
        || saved.format_exception != result.format_exception
    {
        return Err("Batch result disagrees with committed recovery evidence".to_owned());
    }
    job.policy.validate()?;
    if let Some(exception) = &result.format_exception {
        return verify_format_exception(project, &flux, &job, result, exception);
    }
    if result.status == "raw_format_exception" {
        return Err("Raw exception lacks its source binding".into());
    }
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
    if let Some(approval) = &job.read_conflicts {
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(result.image.with_extension("json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let recorded: Option<crate::read_conflicts::Confirmation> = serde_json::from_value(
            value
                .get("read_conflicts")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|e| e.to_string())?;
        if recorded.as_ref() != Some(approval) {
            return Err("Completed conflict confirmation differs from its job".into());
        }
        crate::read_conflicts::verify_metadata_binding(&project.images_dir(), &value)?;
        verify_catalog_metadata(&project.images_dir(), &value)?;
    }
    Ok(())
}

/// Diagnostic replay binds the exact journal and compares the published sector
/// provenance/image with a fresh aggregation, rather than trusting labels alone.
pub(crate) fn diagnostic_binding(
    project: &ProjectState,
    disk: u32,
) -> Result<Option<serde_json::Value>, String> {
    let flux = flux_capture::project_flux_dir(project)?;
    let path = flux.join("Recovery").join(format!("{disk:03}_job.json"));
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(info) if !info.file_type().is_file() || info.len() > 1024 * 1024 => {
            return Err("Unsafe diagnostic recovery journal".into());
        }
        _ => {}
    }
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if canonical.parent().and_then(|p| p.parent()) != Some(flux.as_path()) {
        return Err("Diagnostic recovery journal escapes Flux".into());
    }
    let bytes = fs::read(&canonical).map_err(|e| e.to_string())?;
    let journal_hash = format!("{:x}", Sha256::digest(&bytes));
    let job: Journal = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if job.schema_version != 1 || job.disk != disk {
        return Err("Diagnostic journal identity mismatch".into());
    }
    job.policy.validate()?;
    if job.stages.len() > job.policy.passes.len()
        || job
            .stages
            .iter()
            .map(|s| s.capture_attempt)
            .collect::<BTreeSet<_>>()
            .len()
            != job.stages.len()
    {
        return Err("Invalid/bounded diagnostic stage identities".into());
    }
    for stage in &job.stages {
        stage.settings.validate()?;
        if stage.capture_attempt == 0 || stage.decode_attempt == Some(0) {
            return Err("Invalid diagnostic stage number".into());
        }
    }
    let mut sectors = serde_json::Value::Null;
    if let Some(result) = &job.result {
        if result.disk != disk {
            return Err("Diagnostic result identity mismatch".into());
        }
        for (path, parent) in [
            (
                &result.provenance,
                flux.join(if result.format_exception.is_some() {
                    "Formats"
                } else {
                    "Recovery"
                }),
            ),
            (&result.image, project.images_dir()),
        ] {
            if path.as_os_str().is_empty() {
                continue;
            }
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            if canonical.parent()
                != Some(parent.canonicalize().map_err(|e| e.to_string())?.as_path())
            {
                return Err("Diagnostic result points outside its managed directory".into());
            }
        }
        if result.format_exception.is_none() {
            let profile = GreaseweazleProfile::parse(&job.profile)?;
            if fs::metadata(&result.image)
                .map_err(|e| e.to_string())?
                .len()
                != profile.expected_sector_image_bytes()
                || fs::metadata(&result.provenance)
                    .map_err(|e| e.to_string())?
                    .len()
                    > 4 * 1024 * 1024
            {
                return Err("Diagnostic final artifact exceeds expected bounds".into());
            }
        }
        verify_completed_result(project, result)?;
        if result.format_exception.is_none() {
            let evidence = aggregate(
                project,
                disk,
                GreaseweazleProfile::parse(&job.profile)?,
                &job.stages,
            )?;
            if evidence.missing != result.missing_lbas
                || evidence.conflicts != result.conflicting_lbas
                || format!("{:x}", Sha256::digest(&evidence.bytes)) != result.image_sha256
            {
                return Err("Published recovery image disagrees with replayed sectors".into());
            }
            let info = fs::symlink_metadata(&result.provenance).map_err(|e| e.to_string())?;
            if !info.file_type().is_file() || info.len() > 4 * 1024 * 1024 {
                return Err("Unsafe diagnostic provenance".into());
            }
            let published: serde_json::Value =
                serde_json::from_slice(&fs::read(&result.provenance).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            sectors = serde_json::to_value(&evidence.sectors).map_err(|e| e.to_string())?;
            if published["disk"] != disk
                || published["profile"] != job.profile
                || published["image_sha256"] != result.image_sha256
                || published["sectors"] != sectors
                || serde_json::from_value::<Vec<Stage>>(published["stages"].clone())
                    .map_err(|e| e.to_string())?
                    != job.stages
            {
                return Err("Published recovery provenance disagrees with replay".into());
            }
        }
    }
    if hash_path(&canonical)? != journal_hash {
        return Err("Recovery journal changed during diagnostics".into());
    }
    Ok(Some(
        json!({"journal":canonical,"journal_sha256":journal_hash,"profile":job.profile,"stages":job.stages,"result":job.result,"sectors":sectors}),
    ))
}
fn verify_format_exception(
    project: &ProjectState,
    flux: &Path,
    job: &Journal,
    result: &RecoveryResult,
    exception: &FormatException,
) -> Result<(), String> {
    if result.status != "raw_format_exception"
        || result.selected_profile.is_some()
        || !result.image.as_os_str().is_empty()
        || !result.image_sha256.is_empty()
        || !result.missing_lbas.is_empty()
        || !result.conflicting_lbas.is_empty()
        || result.corroborated_sectors != 0
        || result.single_capture_sectors != 0
        || result.capture_attempts != [exception.capture_attempt]
        || job.stages.len() != 1
        || job.stages[0].capture_attempt != exception.capture_attempt
        || job.stages[0].decode_attempt.is_some()
        || !job.automatic_format
    {
        return Err("Invalid raw-only format exception; numbering not advanced".into());
    }
    let formats = flux
        .join("Formats")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let report = result
        .provenance
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if formats.parent() != Some(flux)
        || report.parent() != Some(formats.as_path())
        || !fs::symlink_metadata(&report)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        || fs::metadata(&report).map_err(|e| e.to_string())?.len() > 131072
        || hash_path(&report)? != result.provenance_sha256
    {
        return Err("Raw-only format decision path/hash changed".into());
    }
    let decision: crate::flux_format::FormatDecision =
        serde_json::from_slice(&fs::read(&report).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if decision.schema_version != 1
        || decision.disk != result.disk
        || decision.capture_attempt != exception.capture_attempt
        || decision.source_sha256 != exception.source_sha256
        || decision.reason != exception.reason
        || decision.selected_profile.is_some()
        || decision.physical_media_access
        || decision.candidates.len() != 2
    {
        return Err("Raw-only format decision binding disagrees".into());
    }
    let inspected = flux_capture::inspect_disk(project, result.disk)?;
    let capture = inspected
        .captures
        .iter()
        .find(|c| c.attempt == exception.capture_attempt)
        .ok_or("Raw exception capture missing")?;
    if capture.status != "complete"
        || !capture.hash_matches
        || capture.sha256.as_deref() != Some(exception.source_sha256.as_str())
        || capture
            .capture_settings
            .as_ref()
            .is_none_or(|s| s.cylinders.is_some())
    {
        return Err("Raw exception source binding changed".into());
    }
    let mut profiles = BTreeSet::new();
    for candidate in &decision.candidates {
        let profile = GreaseweazleProfile::parse(&candidate.profile)?;
        if !profiles.insert(candidate.profile.clone()) || candidate.error.is_some() {
            return Err("Raw exception has failed/duplicate format trials".into());
        }
        let decode = inspected
            .decodes
            .iter()
            .find(|d| {
                d.capture_attempt == exception.capture_attempt
                    && d.profile == candidate.profile
                    && Some(d.decode_attempt) == candidate.decode_attempt
            })
            .ok_or("Raw exception trial missing")?;
        let count = profile.expected_sector_image_bytes() as usize / 512;
        let (_, bad) = flux_capture::verified_decode(project, decode, count)?;
        if candidate.total_sectors != count
            || candidate.good_sectors != count - bad.len()
            || candidate.image_sha256.as_deref() != Some(decode.output_sha256.as_str())
        {
            return Err("Raw exception candidate evidence changed".into());
        }
    }
    Ok(())
}

/// Raw-only jobs belong to project attention even though Images/ has no image.
pub(crate) fn format_exceptions(project: &ProjectState) -> Result<Vec<RecoveryResult>, String> {
    let flux = project
        .root()
        .join("Flux")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if flux.parent()
        != Some(
            project
                .root()
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Flux escapes project".into());
    }
    let directory = flux.join("Recovery");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if directory.parent() != Some(flux.as_path()) {
        return Err("Recovery journal directory escapes Flux".into());
    }
    let mut results = Vec::new();
    for item in fs::read_dir(&directory).map_err(|e| e.to_string())? {
        let path = item.map_err(|e| e.to_string())?.path();
        let Some(disk) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix("_job.json"))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        {
            return Err("Unsafe recovery journal".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(1048577)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1048576 {
            return Err("Oversized recovery journal".into());
        }
        let job: Journal =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid recovery journal: {e}"))?;
        if job.disk != disk || job.schema_version != 1 {
            return Err("Invalid recovery journal identity".into());
        }
        if let Some(result) = job.result.filter(|r| r.format_exception.is_some()) {
            verify_completed_result(project, &result)?;
            results.push(result);
        }
        if results.len() > 4096 {
            return Err("Too many raw-only exceptions".into());
        }
    }
    results.sort_by_key(|r| r.disk);
    Ok(results)
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
pub(crate) fn publish_image_no_replace(source: &Path, destination: &Path) -> Result<(), String> {
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
    let _snapshot = crate::project_work::snapshot(project.root())?;
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
    let partial_metadata = images.join(format!("{stem}.partial.json"));
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
        "stop_reason": reason,
        "incomplete_capture_attempts": j.stages.iter().filter(|s| s.decode_attempt.is_none()).map(|s| s.capture_attempt).collect::<Vec<_>>(),
        "image_sha256": sha256,
        "sectors": e.sectors,
        "read_conflicts": j.read_conflicts,
        "note": "Single-capture sectors have lower confidence. Within-GW unreadable/conflicting sectors are zero-filled. Optional read_conflicts records separately preserved USB/GW disagreements; the observed GW edition is retained, not guessed or certified. No customer-delivery certification."
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
        "flux_provenance_sha256": prov_hash,
        "read_conflicts": j.read_conflicts
    }))
    .map_err(|e| e.to_string())?;
    publish_image_no_replace(&partial, &image)?;
    // Metadata is the completed-image commit record. Readers never see a partly
    // written JSON document; interruption leaves only an ignored partial record.
    write_new(&partial_metadata, &meta)?;
    publish_image_no_replace(&partial_metadata, &metadata)?;
    Ok(RecoveryResult {
        disk: j.disk,
        selected_profile: Some(j.profile.clone()),
        status: if unresolved.is_empty() && j.read_conflicts.is_none() {
            "acquired"
        } else if unresolved.len() == e.sectors.len() {
            "unrecoverable_within_policy"
        } else {
            "partial"
        }
        .to_owned(),
        stop_reason: reason.to_owned(),
        capture_attempts: j
            .stages
            .iter()
            .filter(|s| s.decode_attempt.is_some())
            .map(|s| s.capture_attempt)
            .collect(),
        image,
        image_sha256: sha256,
        provenance,
        provenance_sha256: prov_hash,
        missing_lbas: e.missing.clone(),
        conflicting_lbas: e.conflicts.clone(),
        read_conflict_lbas: j
            .read_conflicts
            .as_ref()
            .map_or(Vec::new(), |c| c.conflicts.iter().map(|s| s.lba).collect()),
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
        format_exception: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires FV_READ_CONFLICT_SOURCE; read-only saved 133 replay, never hardware"]
    fn saved_133_minor_reader_conflicts_are_hash_bound_without_modifying_source() {
        let root = std::env::var_os("FV_READ_CONFLICT_SOURCE").unwrap();
        let p = ProjectState::open_without_session(root.into()).unwrap();
        let (_, before, job) = identity_job(&p, 133).unwrap().unwrap();
        assert!(job.result.is_none());
        let evidence = aggregate(
            &p,
            133,
            GreaseweazleProfile::parse(&job.profile).unwrap(),
            &job.stages,
        )
        .unwrap();
        let bad: Vec<_> = evidence
            .missing
            .iter()
            .chain(&evidence.conflicts)
            .copied()
            .collect();
        let approval =
            crate::read_conflicts::build(&p, 133, 1, 146, 1, &evidence.bytes, &bad).unwrap();
        assert_eq!(approval.agreeing_sectors, 2584);
        assert_eq!(
            approval.conflicts.iter().map(|c| c.lba).collect::<Vec<_>>(),
            vec![17, 773]
        );
        assert!(bad.is_empty());
        crate::read_conflicts::verify(&p, &approval, &evidence.bytes, &bad).unwrap();
        assert_eq!(identity_job(&p, 133).unwrap().unwrap().1, before);
    }

    #[test]
    fn identity_rejection_preserves_bytes_excludes_old_capture_and_keeps_slots() {
        let p = ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-identity-release-{}-{}",
            std::process::id(),
            external_tools::current_unix_ms()
        )))
        .unwrap();
        let flux = flux_capture::project_flux_dir(&p).unwrap();
        fs::create_dir_all(flux.join("Recovery")).unwrap();
        let path = flux.join("Recovery/023_job.json");
        let job = clock_job();
        let bytes = serde_json::to_vec(&job).unwrap();
        fs::write(&path, &bytes).unwrap();
        fs::write(
            flux.join("023_attempt_001.scp"),
            b"wrong disk flux retained",
        )
        .unwrap();
        fs::write(flux.join("023_attempt_001.json"), b"{}").unwrap();
        reject_identity_job(&p, 23, 8).unwrap();
        reject_identity_job(&p, 23, 8).unwrap(); // Restart between the two commits.
        assert_eq!(
            fs::read(flux.join("Recovery/023_identity_rejected_8.json")).unwrap(),
            bytes
        );
        assert_eq!(
            fs::read(flux.join("023_attempt_001.scp")).unwrap(),
            b"wrong disk flux retained"
        );
        let restarted: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(restarted.stages.is_empty() && restarted.result.is_none());
        assert_eq!(
            rejected_capture_attempts(&p, 23).unwrap(),
            BTreeSet::from([1])
        );
        assert!(flux_capture::latest_capture_attempt(&p, 23).is_err());
        assert!(flux_capture::inspect_disk(&p, 23).is_err());
        fs::write(flux.join("023_attempt_002.json"), b"{}").unwrap();
        assert_eq!(flux_capture::latest_capture_attempt(&p, 23).unwrap(), 2);
        let status = flux_capture::inspect_disk(&p, 23).unwrap();
        assert_eq!(status.captures.len(), 1);
        assert_eq!(status.captures[0].attempt, 2);
        // A later mutable job cannot erase the exclusion ledger.
        fs::write(&path, b"{}").unwrap();
        assert_eq!(
            rejected_capture_attempts(&p, 23).unwrap(),
            BTreeSet::from([1])
        );
        assert!(reject_identity_job(&p, 23, 9).is_err());
        fs::remove_dir_all(p.root()).unwrap();
    }

    fn clock_job() -> Journal {
        Journal {
            schema_version: 1,
            disk: 23,
            profile: "ibm.1440".into(),
            automatic_format: false,
            drive: 'B',
            started_unix_ms: 0,
            empty_capture_budget_restarts: vec![],
            stage_budget_version: 1,
            policy: RecoveryPolicy::default(),
            result: None,
            rejected_capture_attempts: BTreeSet::new(),
            read_conflicts: None,
            stages: vec![Stage {
                capture_attempt: 1,
                decode_attempt: None,
                settings: CaptureSettings::default(),
                capture_elapsed_ms: 100_000,
                capture_started_unix_ms: Some(1_000_000),
            }],
        }
    }

    #[test]
    fn stage_clocks_are_independent_and_restart_cannot_renew_in_flight_time() {
        let mut job = clock_job();
        assert_eq!(capture_remaining_ms(&job, 0, 1_040_000), 460_000);
        assert_eq!(capture_remaining_ms(&job, 1, 1_040_000), 600_000);
        finish_capture_clock(&mut job.stages[0], 1_040_000);
        finish_capture_clock(&mut job.stages[0], 1_080_000);
        assert_eq!(job.stages[0].capture_elapsed_ms, 140_000);
        assert_eq!(capture_remaining_ms(&job, 0, 99_000_000), 460_000);
        job.stages[0].capture_started_unix_ms = Some(99_000_000);
        finish_capture_clock(&mut job.stages[0], 99_010_000);
        assert_eq!(job.stages[0].capture_elapsed_ms, 150_000);
        assert_eq!(capture_remaining_ms(&job, 0, 99_020_000), 450_000);
    }

    #[test]
    fn old_policy_defaults_to_per_stage_and_whole_job_is_an_explicit_expert_option() {
        let mut policy = serde_json::to_value(RecoveryPolicy::default()).unwrap();
        policy.as_object_mut().unwrap().remove("time_limit_scope");
        assert_eq!(
            serde_json::from_value::<RecoveryPolicy>(policy.clone())
                .unwrap()
                .time_limit_scope,
            TimeLimitScope::PerStage
        );
        policy["time_limit_scope"] = json!("whole_job");
        assert_eq!(
            serde_json::from_value::<RecoveryPolicy>(policy.clone())
                .unwrap()
                .time_limit_scope,
            TimeLimitScope::WholeJob
        );
        policy["time_limit_scope"] = json!("forever");
        assert!(serde_json::from_value::<RecoveryPolicy>(policy).is_err());
    }

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
