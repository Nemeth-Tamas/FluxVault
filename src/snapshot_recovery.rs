//! Snapshot-only candidates when historical acquisition maps are unavailable.
//! This never manufactures a physical readable-sector mask or managed extraction.
use crate::{carving, fat12, imaging::AttemptSummary, recovery_plan, sector_inspection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const ENGINE: u32 = 1;
const REPORT: &str = "SnapshotCandidates.json";
const MAX_FILES: usize = 512;
const MAX_TOTAL: usize = fat12::MAX_IMAGE_BYTES * 2;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const WARNING: &str = "Snapshot byte candidates only: physical readable-sector evidence is unavailable or contradictory. Source extents and structural/allocation observations bind saved bytes, not successful media reads, original content, ownership or semantic completeness. No missing bytes are filled, no source/acquisition map is rewritten, and these files never enter normal managed extraction or automatic conversion. Readability and customer certification remain unknown; inspect candidates before use.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extent {
    pub offset: usize,
    pub bytes: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub path: String,
    pub observed_name: Option<String>,
    pub bytes: usize,
    pub sha256: String,
    pub method: String,
    pub validation: String,
    pub extents: Vec<Extent>,
    pub metadata_lbas: Vec<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub engine_version: u32,
    pub disk: u32,
    pub attempt: u32,
    pub source_image: String,
    pub source_sha256: String,
    pub source_bytes: usize,
    pub historical_log_sha256_claim: Option<String>,
    pub observed_log_file_sha256: Option<String>,
    pub sector_evidence_refusal: String,
    pub acquisition_mask_known: bool,
    pub physical_readability: String,
    pub bad_sector_count: Option<usize>,
    pub filesystem_error: Option<String>,
    pub carving_limits_reached: bool,
    pub rejected_signatures: usize,
    pub files: Vec<Candidate>,
    pub total_bytes: usize,
    pub warning: String,
    pub customer_delivery_certified: bool,
    pub default_live_extraction_changed: bool,
}
#[derive(Debug, Serialize)]
pub struct ResultSummary {
    pub report: Report,
    pub output_directory: PathBuf,
    pub report_path: PathBuf,
    pub reused: bool,
    pub plan_only: bool,
    pub physical_media_access: bool,
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    sector_inspection::regular(path, false)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Snapshot candidate input exceeds bound".into());
    }
    Ok(bytes)
}
fn directory(path: &Path, parent: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    sector_inspection::regular(path, true)?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if path.parent() != Some(parent) {
        return Err("Snapshot candidate directory escapes its parent".into());
    }
    Ok(path)
}
pub(crate) fn eligible(images: &Path, attempt: &AttemptSummary) -> bool {
    attempt.legacy_image
        && attempt.metadata_path.as_os_str().is_empty()
        && !attempt
            .parsed_log
            .as_ref()
            .is_some_and(|l| l.status == crate::legacy_logs::ArchiverLogStatus::InProgress)
        && !attempt
            .parsed_dmde_log
            .as_ref()
            .is_some_and(|l| l.status == crate::dmde_logs::DmdeLogStatus::InProgress)
        && !attempt.status.to_ascii_uppercase().contains("PROGRESS")
        && observed_sectors(images, attempt).is_some_and(|sectors| {
            crate::fat12_recovery::validate_sector_evidence(attempt, sectors, &attempt.sha256)
                .is_err()
        })
}
fn observed_sectors(images: &Path, attempt: &AttemptSummary) -> Option<usize> {
    // The catalogue may repeat a historical log's geometry even when the saved
    // image is truncated. Check evidence against actual bounded source bytes,
    // never the log's own claim, and never follow an unsafe input path.
    let image = recovery_plan::resolve_image_path(images, &attempt.image_file).ok()?;
    sector_inspection::regular(&image, false).ok()?;
    let bytes = fs::metadata(image).ok()?.len();
    (bytes >= 512 && bytes <= fat12::MAX_IMAGE_BYTES as u64 && bytes.is_multiple_of(512))
        .then_some(bytes as usize / 512)
}
fn analyze(
    images: &Path,
    disk: u32,
    attempt: &AttemptSummary,
) -> Result<(Report, Vec<u8>, PathBuf), String> {
    if !(1..=4096).contains(&disk) || !eligible(images, attempt) {
        return Err("Snapshot candidates require a completed saved legacy snapshot with unavailable/contradictory sector evidence; use ordinary native extraction for a trustworthy map".into());
    }
    sector_inspection::regular(images, true)?;
    let images = images.canonicalize().map_err(|e| e.to_string())?;
    if !crate::imaging::load_attempts_for_disk(&images, disk)?
        .iter()
        .any(|a| {
            a.image_file == attempt.image_file
                && a.attempt_number == attempt.attempt_number
                && a.sha256.eq_ignore_ascii_case(&attempt.sha256)
                && a.log_file.is_empty() == attempt.log_file.is_empty()
                && (a.log_file.is_empty()
                    || Path::new(&a.log_file).canonicalize().ok()
                        == Path::new(&attempt.log_file).canonicalize().ok())
                && eligible(&images, a)
        })
    {
        return Err("Snapshot candidate disk/attempt does not bind the current catalogue".into());
    }
    let image = recovery_plan::resolve_image_path(&images, &attempt.image_file)?;
    let snapshot = read(&image, fat12::MAX_IMAGE_BYTES)?;
    let log_path = Path::new(&attempt.log_file);
    let observed_log_file_sha256 = if attempt.log_file.is_empty() {
        None
    } else {
        let logs = images
            .parent()
            .ok_or("Snapshot images have no project root")?
            .join("Logs")
            .canonicalize()
            .map_err(|e| e.to_string())?;
        sector_inspection::regular(log_path, false)?;
        if log_path.canonicalize().map_err(|e| e.to_string())?.parent() != Some(logs.as_path()) {
            return Err("Snapshot historical log escapes Logs".into());
        }
        Some(hash(&read(log_path, 8 * 1024 * 1024)?))
    };
    if snapshot.len() < 512
        || !snapshot.len().is_multiple_of(512)
        || hash(&snapshot) != attempt.sha256.to_ascii_lowercase()
    {
        return Err(
            "Snapshot extent/hash changed or is unsupported; candidate publication refused".into(),
        );
    }
    let refusal = crate::fat12_recovery::validate_sector_evidence(
        attempt,
        snapshot.len() / 512,
        &attempt.sha256,
    )
    .err()
    .ok_or("Snapshot sector evidence now validates; use ordinary native extraction")?;
    // An empty exclusion vector here describes a local byte-parser experiment,
    // NOT an acquisition/readability map. Never serialize it as physical evidence.
    let analysis = fat12::analyze(&snapshot, &[]);
    let mut files = Vec::new();
    let mut known = Vec::new();
    if let Ok(a) = &analysis {
        for f in &a.recovered_files {
            crate::cancellation::check()?;
            let bytes = fat12::file_bytes(&snapshot, f);
            let mut left = f.bytes;
            let mut extents = Vec::new();
            for lba in &f.data_lbas {
                if left == 0 {
                    break;
                }
                let n = left.min(512);
                extents.push(Extent {
                    offset: *lba as usize * 512,
                    bytes: n,
                });
                left -= n;
            }
            if left != 0 || bytes.len() != f.bytes || hash(&bytes) != f.sha256 {
                return Err("Snapshot FAT extent binding failed".into());
            }
            files.push(Candidate { path: format!("Files/{:04}/{}", files.len()+1, safe_name(&f.path)), observed_name: Some(f.path.clone()), bytes: f.bytes,
                sha256:f.sha256.clone(),method:"observed_snapshot_fat_chain".into(),validation:"Coherent observed FAT/name/chain extent; physical readability and original ownership unknown".into(),extents,metadata_lbas:f.metadata_lbas.clone() });
            known.push(f.sha256.clone());
        }
    }
    let regions = if let Ok(a) = &analysis {
        fat12::orphan_regions(&snapshot, a).0
    } else {
        carving::raw_readable_regions(&snapshot, &[])
    };
    let carved = carving::analyze(&snapshot, &[], &regions, &known)?;
    for f in &carved.files {
        let bytes = carving::file_bytes(&snapshot, f)?;
        if hash(&bytes) != f.sha256 {
            return Err("Snapshot signature extent binding failed".into());
        }
        files.push(Candidate {
            path: format!("Files/{:04}/{}", files.len() + 1, safe_name(&f.path)),
            observed_name: None,
            bytes: f.bytes,
            sha256: f.sha256.clone(),
            method: "structurally_validated_snapshot_signature".into(),
            validation: f.validation.clone(),
            extents: f
                .source_extents
                .iter()
                .map(|e| Extent {
                    offset: e.source_byte_offset,
                    bytes: e.bytes,
                })
                .collect(),
            metadata_lbas: f.metadata_lbas.clone(),
        });
    }
    let total = files
        .iter()
        .try_fold(0usize, |n, f| n.checked_add(f.bytes))
        .ok_or("Snapshot candidate total overflow")?;
    if files.len() > MAX_FILES || total > MAX_TOTAL {
        return Err("Snapshot candidate publication bound exceeded; no truncated success".into());
    }
    let report = Report {
        schema_version: 1,
        engine_version: ENGINE,
        disk,
        attempt: attempt.attempt_number,
        source_image: attempt.image_file.clone(),
        source_sha256: attempt.sha256.to_ascii_lowercase(),
        source_bytes: snapshot.len(),
        historical_log_sha256_claim: attempt.parsed_log.as_ref().and_then(|l| l.sha256.clone()),
        observed_log_file_sha256,
        sector_evidence_refusal: refusal,
        acquisition_mask_known: false,
        physical_readability: "unknown".into(),
        bad_sector_count: None,
        filesystem_error: analysis.err(),
        carving_limits_reached: carved.limits_reached,
        rejected_signatures: carved.rejected.len(),
        files,
        total_bytes: total,
        warning: WARNING.into(),
        customer_delivery_certified: false,
        default_live_extraction_changed: false,
    };
    Ok((report, snapshot, image))
}
fn safe_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or("candidate.bin");
    let name = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect::<String>();
    let name = name.trim_end_matches(['.', ' ']);
    if name.is_empty() {
        "candidate.bin".into()
    } else {
        format!("candidate_{name}")
    }
}
fn payload(snapshot: &[u8], file: &Candidate) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    for e in &file.extents {
        let end = e
            .offset
            .checked_add(e.bytes)
            .ok_or("Snapshot extent overflow")?;
        bytes.extend_from_slice(
            snapshot
                .get(e.offset..end)
                .ok_or("Snapshot extent outside source")?,
        );
        if bytes.len() > fat12::MAX_IMAGE_BYTES {
            return Err("Snapshot payload exceeds bound".into());
        }
    }
    if bytes.len() != file.bytes || hash(&bytes) != file.sha256 {
        return Err("Snapshot candidate extent/hash disagreement".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
fn folder(report: &Report) -> String {
    format!(
        "attempt_{:03}_v{ENGINE}_{}_{}",
        report.attempt,
        &report.source_sha256[..16],
        &hash(&serde_json::to_vec(report).expect("Snapshot report fields serialize"))[..12]
    )
}
pub fn recover(
    images: &Path,
    recovery: &Path,
    disk: u32,
    attempt: &AttemptSummary,
    plan: bool,
    progress: &impl Fn(&str),
) -> Result<ResultSummary, String> {
    let (report, snapshot, image) = analyze(images, disk, attempt)?;
    let planned = recovery
        .join("SnapshotCandidates")
        .join(format!("{disk:03}"))
        .join(folder(&report));
    if plan {
        return Ok(ResultSummary {
            report,
            report_path: planned.join(REPORT),
            output_directory: planned,
            reused: false,
            plan_only: true,
            physical_media_access: false,
        });
    }
    sector_inspection::regular(recovery, true)?;
    let recovery = recovery.canonicalize().map_err(|e| e.to_string())?;
    if recovery.parent() != images.canonicalize().map_err(|e| e.to_string())?.parent() {
        return Err("Snapshot candidate workspace must share the project root".into());
    }
    let parent = directory(&recovery.join("SnapshotCandidates"), &recovery)?;
    let disk_dir = directory(&parent.join(format!("{disk:03}")), &parent)?;
    let lock_path = disk_dir.join(".fluxvault-snapshot.lock");
    if lock_path.try_exists().map_err(|e| e.to_string())? {
        sector_inspection::regular(&lock_path, false)?;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "Snapshot candidate recovery already owns this disk")?;
    let output = disk_dir.join(folder(&report));
    if output.try_exists().map_err(|e| e.to_string())? {
        verify_generation(&output, &report, &snapshot)?;
        if analyze(images, disk, attempt)?.0 != report
            || hash(&read(&image, fat12::MAX_IMAGE_BYTES)?) != report.source_sha256
        {
            return Err("Snapshot changed during reuse".into());
        }
        return Ok(ResultSummary {
            report,
            report_path: output.join(REPORT),
            output_directory: output,
            reused: true,
            plan_only: false,
            physical_media_access: false,
        });
    }
    let staging = disk_dir.join(format!(
        ".tmp-snapshot-{}-{}-{}",
        std::process::id(),
        crate::external_tools::current_unix_ms(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    progress(&format!(
        "Snapshot {disk:03}: saving {} byte candidates separately; physical readability remains UNKNOWN",
        report.files.len()
    ));
    for file in &report.files {
        crate::cancellation::check()?;
        let target = staging.join(&file.path);
        fs::create_dir_all(target.parent().ok_or("Snapshot path has no parent")?)
            .map_err(|e| e.to_string())?;
        write_new(&target, &payload(&snapshot, file)?)?;
    }
    write_new(
        &staging.join(REPORT),
        &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )?;
    verify_generation(&staging, &report, &snapshot)?;
    crate::cancellation::check()?;
    if hash(&read(&image, fat12::MAX_IMAGE_BYTES)?) != report.source_sha256
        || analyze(images, disk, attempt)?.0 != report
    {
        return Err("Snapshot changed before publication; staged candidates retained".into());
    }
    crate::flux_recovery::publish_image_no_replace(&staging, &output)?;
    Ok(ResultSummary {
        report,
        report_path: output.join(REPORT),
        output_directory: output,
        reused: false,
        plan_only: false,
        physical_media_access: false,
    })
}
fn verify_generation(output: &Path, expected: &Report, snapshot: &[u8]) -> Result<(), String> {
    sector_inspection::regular(output, true)?;
    let actual: Report = serde_json::from_slice(&read(&output.join(REPORT), 16 * 1024 * 1024)?)
        .map_err(|e| e.to_string())?;
    if &actual != expected {
        return Err(
            "Snapshot candidate report differs from independently replayed source analysis".into(),
        );
    }
    let mut expected_paths = expected
        .files
        .iter()
        .map(|f| f.path.clone())
        .collect::<BTreeSet<_>>();
    expected_paths.insert(REPORT.into());
    let root = output.canonicalize().map_err(|e| e.to_string())?;
    let mut queue = vec![root.clone()];
    let mut found = BTreeSet::new();
    let mut entries = 0;
    while let Some(directory) = queue.pop() {
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            crate::cancellation::check()?;
            let path = entry.map_err(|e| e.to_string())?.path();
            entries += 1;
            if entries > MAX_FILES * 4 + 4 {
                return Err("Snapshot inventory exceeds bound".into());
            }
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            sector_inspection::regular(&path, metadata.is_dir())?;
            if metadata.is_dir() {
                queue.push(path);
            } else {
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                found.insert(relative);
            }
        }
    }
    if found != expected_paths {
        return Err("Snapshot candidate inventory changed; missing/extra files refused".into());
    }
    for file in &expected.files {
        if read(&output.join(&file.path), fat12::MAX_IMAGE_BYTES)? != payload(snapshot, file)? {
            return Err("Snapshot candidate payload changed; original generation preserved".into());
        }
    }
    Ok(())
}
pub(crate) fn verified_existing(
    images: &Path,
    recovery: &Path,
    disk: u32,
    attempt: &AttemptSummary,
) -> Result<Option<(Report, PathBuf)>, String> {
    if !eligible(images, attempt) {
        return Ok(None);
    }
    sector_inspection::regular(recovery, true)?;
    let recovery = recovery.canonicalize().map_err(|e| e.to_string())?;
    if recovery.parent() != images.canonicalize().map_err(|e| e.to_string())?.parent() {
        return Err("Snapshot verification workspace escapes project".into());
    }
    // Optional candidates must not make ordinary comparison reject a short or
    // unsupported legacy image when no candidate generation was ever saved.
    let candidate_root = recovery.join("SnapshotCandidates");
    if !candidate_root.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    sector_inspection::regular(&candidate_root, true)?;
    let candidate_disk = candidate_root.join(format!("{disk:03}"));
    if !candidate_disk.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    sector_inspection::regular(&candidate_disk, true)?;
    if attempt.sha256.len() != 64 || !attempt.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid snapshot catalogue hash".into());
    }
    let prefix = format!(
        "attempt_{:03}_v{ENGINE}_{}_",
        attempt.attempt_number,
        &attempt.sha256.to_ascii_lowercase()[..16]
    );
    let mut found = false;
    let mut count = 0;
    for entry in fs::read_dir(&candidate_disk).map_err(|e| e.to_string())? {
        count += 1;
        if count > 4096 {
            return Err("Snapshot generation directory exceeds bound".into());
        }
        if entry
            .map_err(|e| e.to_string())?
            .file_name()
            .to_string_lossy()
            .starts_with(&prefix)
        {
            found = true;
        }
    }
    if !found {
        return Ok(None);
    }
    let (report, snapshot, image) = analyze(images, disk, attempt)?;
    let output = recovery
        .join("SnapshotCandidates")
        .join(format!("{disk:03}"))
        .join(folder(&report));
    if !output.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    let parent = recovery.join("SnapshotCandidates");
    sector_inspection::regular(&parent, true)?;
    let disk_directory = parent.join(format!("{disk:03}"));
    sector_inspection::regular(&disk_directory, true)?;
    if parent.canonicalize().map_err(|e| e.to_string())?.parent() != Some(recovery.as_path())
        || disk_directory
            .canonicalize()
            .map_err(|e| e.to_string())?
            .parent()
            != Some(parent.canonicalize().map_err(|e| e.to_string())?.as_path())
    {
        return Err("Snapshot verification directory escapes project".into());
    }
    verify_generation(&output, &report, &snapshot)?;
    if hash(&read(&image, fat12::MAX_IMAGE_BYTES)?) != report.source_sha256 {
        return Err("Snapshot changed during verification".into());
    }
    Ok(Some((report, output)))
}

#[cfg(test)]
#[path = "snapshot_recovery_tests.rs"]
mod tests;

pub(crate) fn trace_candidate(
    project: &crate::project::ProjectState,
    disk: u32,
    relative: &Path,
) -> Result<serde_json::Value, String> {
    let requested = project
        .root()
        .join(relative)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    for attempt in crate::imaging::load_attempts_for_disk(&project.images_dir(), disk)? {
        if let Some((report, output)) = verified_existing(
            &project.images_dir(),
            &project.recovery_dir(),
            disk,
            &attempt,
        )? {
            for file in &report.files {
                if output
                    .join(&file.path)
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    == requested
                {
                    return Ok(
                        serde_json::json!({"kind":"unverified_snapshot_candidate","path":relative.to_string_lossy().replace('\\',"/"),"bytes":file.bytes,"sha256":file.sha256,
                        "confidence":"hash_bound_saved_snapshot_bytes_only; physical_readability_unknown","method":file.method,"structural_validation":file.validation,
                        "source_image":{"image_file":report.source_image,"sha256":report.source_sha256,"attempt":report.attempt,"bytes":report.source_bytes},
                        "source_extents":file.extents,"observed_metadata_lbas":file.metadata_lbas,"observed_name":file.observed_name,"acquisition_mask_known":false,
                        "read_pass_provenance":"unknown; no sector/read/corroboration assertion","report_sha256":hash(&read(&output.join(REPORT),16*1024*1024)?),"customer_delivery_certified":false}),
                    );
                }
            }
        }
    }
    Err("Snapshot file does not bind a replay-verified current generation for this disk".into())
}
