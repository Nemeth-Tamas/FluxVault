//! Read-only, evidence-ranked next steps for saved acquisition attempts.
//! A plan is not a recovery result: candidate actions still have to validate
//! source agreement before any derived image can be trusted.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    imaging::{self, AttemptSummary, DiskSummary},
    sector_recovery,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    Complete,
    WaitForAcquisition,
    CheckEvidence,
    CompareAndComposite,
    ReconstructMirroredFat,
    RereadOrFlux,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskRecoveryPlan {
    pub disk_number: u32,
    pub best_attempt: u32,
    pub best_bad_sectors: usize,
    pub composite_candidate_sectors: usize,
    pub mirrored_fat_candidate_sectors: usize,
    pub action: RecoveryAction,
    pub reason: String,
}

pub fn plan_project(images_directory: &Path) -> Result<Vec<DiskRecoveryPlan>, String> {
    let statistics = imaging::load_project_statistics(images_directory)?;
    statistics
        .disks
        .iter()
        .map(|disk| {
            let attempts = imaging::load_attempts_for_disk(images_directory, disk.disk_number)?;
            plan_disk(images_directory, disk, &attempts)
        })
        .collect()
}

fn plan_disk(
    images_directory: &Path,
    disk: &DiskSummary,
    attempts: &[AttemptSummary],
) -> Result<DiskRecoveryPlan, String> {
    let best = attempts
        .iter()
        .find(|attempt| attempt.attempt_number == disk.best_attempt_number)
        .ok_or_else(|| format!("Disk {:03}: best attempt is missing", disk.disk_number))?;
    let mut plan = DiskRecoveryPlan {
        disk_number: disk.disk_number,
        best_attempt: best.attempt_number,
        best_bad_sectors: best.bad_sectors.len(),
        composite_candidate_sectors: 0,
        mirrored_fat_candidate_sectors: 0,
        action: RecoveryAction::CheckEvidence,
        reason: String::new(),
    };

    // Before recommending a cross-attempt operation, re-hash each source and
    // reject path indirection. A mismatch is an attention state, not a reason
    // to silently use whatever bytes happen to be on disk now.
    let mut images = BTreeMap::new();
    for attempt in attempts {
        let path = match resolve_image_path(images_directory, &attempt.image_file) {
            Ok(path) => path,
            Err(_) => {
                plan.reason = format!(
                    "Attempt #{:03} has an unsafe image path",
                    attempt.attempt_number
                );
                return Ok(plan);
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            _ => {
                plan.reason = format!(
                    "Attempt #{:03} image is missing or not a regular file",
                    attempt.attempt_number
                );
                return Ok(plan);
            }
        };
        if metadata.len() > 64 * 1024 * 1024 {
            plan.reason = format!(
                "Attempt #{:03} image exceeds the offline floppy size limit",
                attempt.attempt_number
            );
            return Ok(plan);
        }
        let bytes =
            fs::read(&path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        if attempt.sha256.is_empty() || !hash.eq_ignore_ascii_case(&attempt.sha256) {
            plan.reason = format!(
                "Attempt #{:03} image does not match recorded SHA-256",
                attempt.attempt_number
            );
            return Ok(plan);
        }
        if images.insert(attempt.attempt_number, bytes).is_some() {
            plan.reason = format!("Duplicate attempt number #{:03}", attempt.attempt_number);
            return Ok(plan);
        }
    }

    if !best.attention_required && best.bad_sectors.is_empty() {
        plan.action = RecoveryAction::Complete;
        plan.reason = "A verified clean acquisition already exists".to_owned();
        return Ok(plan);
    }

    if best.status.eq_ignore_ascii_case("IN PROGRESS") {
        plan.action = RecoveryAction::WaitForAcquisition;
        plan.reason = "The selected acquisition has not finished".to_owned();
        return Ok(plan);
    }
    if best.parsed_log.is_none() && best.parsed_dmde_log.is_none() {
        plan.reason = "No recognized acquisition log establishes readable sectors".to_owned();
        return Ok(plan);
    }
    if best.bad_sectors.is_empty() {
        plan.reason = format!(
            "Acquisition status is {} despite no listed bad sectors; inspect the log before recommending another read",
            best.status
        );
        return Ok(plan);
    }

    let best_bytes = &images[&best.attempt_number];
    let best_bad = best.bad_sectors.iter().copied().collect::<BTreeSet<_>>();
    let donor_bad = attempts
        .iter()
        .filter(|attempt| {
            attempt.attempt_number != best.attempt_number
                && attempt.total_sectors == best.total_sectors
                && images[&attempt.attempt_number].len() == best_bytes.len()
                && (attempt.parsed_log.is_some() || attempt.parsed_dmde_log.is_some())
        })
        .map(|attempt| attempt.bad_sectors.iter().copied().collect::<BTreeSet<_>>())
        .collect::<Vec<_>>();
    plan.composite_candidate_sectors = best_bad
        .iter()
        .filter(|lba| donor_bad.iter().any(|bad| !bad.contains(lba)))
        .count();

    if let Ok((recoverable, _)) =
        sector_recovery::inspect_mirrored_fat(best_bytes, &best.bad_sectors)
    {
        plan.mirrored_fat_candidate_sectors = recoverable;
    }

    if plan.composite_candidate_sectors > 0 {
        plan.action = RecoveryAction::CompareAndComposite;
        plan.reason = "Other attempts report readable copies of bad sectors; verify sector agreement before deriving a composite".to_owned();
    } else if plan.mirrored_fat_candidate_sectors > 0 {
        plan.action = RecoveryAction::ReconstructMirroredFat;
        plan.reason = "A readable mirrored FAT copy can restore filesystem metadata without guessing file data".to_owned();
    } else {
        plan.action = RecoveryAction::RereadOrFlux;
        plan.reason = "No additional sector bytes are provable from the current saved images; queue a bounded reread or flux capture".to_owned();
    }
    Ok(plan)
}

fn resolve_image_path(images_directory: &Path, recorded: &str) -> Result<PathBuf, String> {
    let source = Path::new(recorded);
    let path = if source.is_absolute() {
        source.to_path_buf()
    } else if source.components().count() == 1
        && matches!(source.components().next(), Some(Component::Normal(_)))
    {
        images_directory.join(source)
    } else {
        return Err("Image path is not a simple filename".to_owned());
    };
    let images_root = fs::canonicalize(images_directory)
        .map_err(|error| format!("Cannot resolve Images directory: {error}"))?;
    let resolved = fs::canonicalize(&path)
        .map_err(|error| format!("Cannot resolve recorded image: {error}"))?;
    if resolved.parent() != Some(images_root.as_path()) {
        return Err("Image is outside the project's Images directory".to_owned());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_logs::{ArchiverLogStatus, ParsedArchiverLog};

    fn attempt(number: u32, image_file: &str, sha256: String, bad: Vec<u64>) -> AttemptSummary {
        AttemptSummary {
            attempt_number: number,
            status: "PARTIAL".to_owned(),
            timestamp_unix_ms: 0,
            image_file: image_file.to_owned(),
            metadata_path: Default::default(),
            log_file: String::new(),
            parsed_log: Some(ParsedArchiverLog {
                status: ArchiverLogStatus::Partial,
                disk_number: Some(1),
                attempt_number: Some(number),
                source: None,
                geometry: Default::default(),
                bad_sectors: bad.clone(),
                retry_failures: 0,
                retry_recovered: 0,
                bytes_written: None,
                sha256: None,
                begin_seen: true,
                end_seen: true,
            }),
            parsed_dmde_log: None,
            legacy_image: false,
            attention_required: true,
            sha256,
            total_sectors: 3,
            retry_recovered_sectors: 0,
            bad_sectors: bad,
        }
    }

    #[test]
    fn plans_offline_composite_without_modifying_images() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-plan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let first = vec![0u8; 3 * 512];
        let second = first.clone();
        fs::write(root.join("001_attempt_001.img"), &first).unwrap();
        fs::write(root.join("001_attempt_002.img"), &second).unwrap();
        let attempts = vec![
            attempt(
                1,
                "001_attempt_001.img",
                format!("{:x}", Sha256::digest(&first)),
                vec![1],
            ),
            attempt(
                2,
                "001_attempt_002.img",
                format!("{:x}", Sha256::digest(&second)),
                vec![2],
            ),
        ];
        let disk = DiskSummary {
            disk_number: 1,
            attempt_count: 2,
            latest_attempt_number: 2,
            latest_status: "PARTIAL".to_owned(),
            latest_bad_sectors: 1,
            latest_timestamp_unix_ms: 0,
            best_attempt_number: 1,
            best_bad_sectors: 1,
            attention_required: true,
            total_sectors: 3,
        };
        let plan = plan_disk(&root, &disk, &attempts).unwrap();
        assert_eq!(plan.action, RecoveryAction::CompareAndComposite);
        assert_eq!(plan.composite_candidate_sectors, 1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_legacy_absolute_image_only_inside_images_directory() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-plan-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let images = root.join("Images");
        fs::create_dir_all(&images).unwrap();
        let inside = images.join("001.img");
        let outside = root.join("outside.img");
        fs::write(&inside, [0u8]).unwrap();
        fs::write(&outside, [0u8]).unwrap();

        assert_eq!(
            resolve_image_path(&images, &inside.display().to_string()).unwrap(),
            inside
        );
        assert!(resolve_image_path(&images, &outside.display().to_string()).is_err());
        assert!(resolve_image_path(&images, "../outside.img").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
