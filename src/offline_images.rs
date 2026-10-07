//! Replayable offline sector recovery, published as explicitly DERIVED attempts.
//! The caller owns the project writer and short publication snapshot gate.
use crate::{
    composite::{CompositeReplacement, CompositeResult},
    imaging::AttemptSummary,
    project::ProjectState,
    sector_recovery::{self, ReconstructionRecord, ReconstructionResult},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const BACKEND: &str = "offline-recovery-derived";
const SOURCES: usize = 16;
const CONTROL: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seal {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    attempt: u32,
    status: String,
    image: Seal,
    metadata: Option<Seal>,
    log: Seal,
    log_kind: String,
    total_sectors: usize,
    bad: Vec<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", deny_unknown_fields)]
enum Step {
    Composite {
        image: Seal,
        provenance: Seal,
        copies: Vec<CompositeReplacement>,
        bad: Vec<u64>,
    },
    MirroredFat {
        image: Seal,
        provenance: Seal,
        copies: Vec<ReconstructionRecord>,
        bad: Vec<u64>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: u32,
    disk: u32,
    base: u32,
    sources: Vec<Source>,
    steps: Vec<Step>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    schema: u32,
    key: String,
    disk: u32,
    attempt: u32,
    created_unix_ms: u128,
    geometry: Value,
    image_sha256: String,
    unresolved: Vec<u64>,
    recipe: Recipe,
    warning: String,
}
pub(crate) struct Published {
    pub attempt: u32,
    pub image: PathBuf,
    pub report: PathBuf,
    pub reused: bool,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn regular(path: &Path) -> Result<u64, String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Offline evidence is a reparse point".into());
        }
    }
    if !m.file_type().is_file() {
        return Err("Offline evidence is not a regular file".into());
    }
    Ok(m.len())
}
fn resolve(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty()
        || relative.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.chars()
                    .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
                || p.ends_with([' ', '.'])
        })
    {
        return Err("Unsafe offline evidence path".into());
    }
    let path = relative
        .split('/')
        .fold(root.to_owned(), |p, part| p.join(part));
    let actual = path.canonicalize().map_err(|e| e.to_string())?;
    if actual != path {
        return Err("Offline evidence path is redirected".into());
    }
    regular(&path)?;
    Ok(path)
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    if regular(path)? > limit {
        return Err("Offline evidence exceeds its size bound".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Offline evidence grew beyond its size bound".into());
    }
    Ok(bytes)
}
fn seal(root: &Path, path: &Path, limit: u64) -> Result<Seal, String> {
    regular(path)?;
    let actual = path.canonicalize().map_err(|e| e.to_string())?;
    let relative = actual
        .strip_prefix(root)
        .map_err(|_| "Offline evidence escapes project")?
        .to_str()
        .ok_or("Non-UTF8 offline evidence path")?
        .replace('\\', "/");
    let actual = resolve(root, &relative)?;
    let bytes = read(&actual, limit)?;
    Ok(Seal {
        path: relative,
        bytes: bytes.len() as u64,
        sha256: hash(&bytes),
    })
}
fn sealed(root: &Path, value: &Seal, limit: u64) -> Result<Vec<u8>, String> {
    let bytes = read(&resolve(root, &value.path)?, limit)?;
    if bytes.len() as u64 != value.bytes || hash(&bytes) != value.sha256 {
        return Err(format!("Offline recovery evidence changed: {}", value.path));
    }
    Ok(bytes)
}
fn map(bad: &[u64], sectors: usize) -> Result<BTreeSet<u64>, String> {
    if sectors == 0 || sectors > crate::fat12::MAX_IMAGE_BYTES / 512 || bad.len() > sectors {
        return Err("Offline sector map exceeds its bound".into());
    }
    let values = bad.iter().copied().collect::<BTreeSet<_>>();
    if values.len() != bad.len() || values.iter().any(|l| *l >= sectors as u64) {
        return Err("Invalid offline recovery sector map".into());
    }
    Ok(values)
}
fn copy(source: &[u8], destination: &mut [u8], from: u64, to: u64) {
    destination[to as usize * 512..(to as usize + 1) * 512]
        .copy_from_slice(&source[from as usize * 512..(from as usize + 1) * 512]);
}

/// Verify every source binding, re-read its authoritative sector log, and replay
/// every sector operation. A derived image hash alone is never enough.
fn replay(root: &Path, recipe: &Recipe) -> Result<(Vec<u8>, Vec<u64>), String> {
    if recipe.schema != 1
        || recipe.disk == 0
        || recipe.sources.is_empty()
        || recipe.sources.len() > SOURCES
        || recipe.steps.is_empty()
        || recipe.steps.len() > 2
    {
        return Err("Invalid/bounded offline recovery recipe".into());
    }
    let mut sources = BTreeMap::new();
    for s in &recipe.sources {
        if !s.image.path.starts_with("Images/")
            || s.image.path.matches('/').count() != 1
            || !s.log.path.starts_with("Logs/")
            || s.log.path.matches('/').count() != 1
            || !matches!(s.status.as_str(), "OK" | "PARTIAL")
        {
            return Err("Offline recipe source is not an original completed acquisition".into());
        }
        let image = sealed(root, &s.image, crate::fat12::MAX_IMAGE_BYTES as u64)?;
        if image.is_empty()
            || image.len()
                != s.total_sectors
                    .checked_mul(512)
                    .ok_or("Sector size overflow")?
        {
            return Err("Offline source is not a complete 512-byte sector image".into());
        }
        let bad = map(&s.bad, s.total_sectors)?;
        let log = sealed(root, &s.log, CONTROL)?;
        let text = std::str::from_utf8(&log).map_err(|e| e.to_string())?;
        let archiver = if s.log_kind == "archiver" {
            Some(crate::legacy_logs::parse_archiver_log(text)?)
        } else {
            None
        };
        if archiver.as_ref().is_some_and(|l| {
            l.disk_number != Some(recipe.disk)
                || (s.attempt > 0 && l.attempt_number != Some(s.attempt))
        }) {
            return Err("Offline acquisition log belongs to another disk/attempt".into());
        }
        let dmde = if s.log_kind == "dmde" {
            Some(crate::dmde_logs::parse_dmde_log(text)?)
        } else {
            None
        };
        let attempt = AttemptSummary {
            attempt_number: s.attempt,
            status: s.status.clone(),
            timestamp_unix_ms: 0,
            image_file: s.image.path.clone(),
            metadata_path: PathBuf::new(),
            log_file: s.log.path.clone(),
            parsed_log: archiver,
            parsed_dmde_log: dmde,
            legacy_image: s.metadata.is_none(),
            attention_required: !bad.is_empty(),
            sha256: s.image.sha256.clone(),
            total_sectors: s.total_sectors,
            retry_recovered_sectors: 0,
            bad_sectors: s.bad.clone(),
        };
        crate::fat12_recovery::validate_sector_evidence(
            &attempt,
            s.total_sectors,
            &s.image.sha256,
        )?;
        if let Some(meta) = &s.metadata {
            if !meta.path.starts_with("Images/") || meta.path.matches('/').count() != 1 {
                return Err("Offline source metadata escapes Images".into());
            }
            let value: Value =
                serde_json::from_slice(&sealed(root, meta, CONTROL)?).map_err(|e| e.to_string())?;
            let metadata_bad = value["bad_sectors"]
                .as_array()
                .ok_or("Missing original sector map")?
                .iter()
                .map(|v| v["lba"].as_u64().ok_or("Invalid original LBA"))
                .collect::<Result<Vec<_>, _>>()?;
            let image_path = crate::recovery_plan::resolve_image_path(
                &root.join("Images"),
                value["image_file"]
                    .as_str()
                    .ok_or("Missing original image path")?,
            )?
            .canonicalize()
            .map_err(|e| e.to_string())?;
            if value["disk_number"] != recipe.disk
                || value["attempt_number"] != s.attempt
                || value["status"] != s.status
                || value["sha256"] != s.image.sha256
                || value["total_sectors"] != s.total_sectors
                || map(&metadata_bad, s.total_sectors)? != bad
                || image_path != resolve(root, &s.image.path)?
                || value["source_backend"] == BACKEND
            {
                return Err("Offline source metadata identity/map disagrees".into());
            }
        } else if s.attempt != 0
            || Path::new(&s.image.path)
                .file_stem()
                .and_then(|s| s.to_str())
                != Some(format!("{:03}", recipe.disk).as_str())
        {
            return Err("Offline source lacks correct numbered/legacy metadata identity".into());
        }
        if sources.insert(s.attempt, (s, image, bad)).is_some() {
            return Err("Duplicate offline source attempt".into());
        }
    }
    let base = sources
        .get(&recipe.base)
        .ok_or("Missing offline base attempt")?;
    let sectors = base.0.total_sectors;
    if sources.values().any(|(s, _, _)| s.total_sectors != sectors) {
        return Err("Offline source geometry differs".into());
    }
    let mut image = base.1.clone();
    let mut bad = base.2.iter().copied().collect::<Vec<_>>();
    for (index, step) in recipe.steps.iter().enumerate() {
        let (step_image, provenance, expected_bad) = match step {
            Step::Composite {
                image,
                provenance,
                bad,
                ..
            }
            | Step::MirroredFat {
                image,
                provenance,
                bad,
                ..
            } => (image, provenance, bad),
        };
        let prefix = format!("Recovery/{:03}/", recipe.disk);
        if !step_image.path.starts_with(&prefix)
            || !provenance.path.starts_with(&prefix)
            || step_image.path.matches('/').count() != 2
            || provenance.path.matches('/').count() != 2
        {
            return Err("Offline derived step escapes its recovery disk".into());
        }
        let prov: Value = serde_json::from_slice(&sealed(root, provenance, CONTROL)?)
            .map_err(|e| e.to_string())?;
        match step {
            Step::Composite { copies, .. } => {
                if index != 0 || sources.len() < 2 {
                    return Err("Invalid composite stage order".into());
                }
                let best = sources
                    .values()
                    .min_by_key(|(s, _, b)| (b.len(), std::cmp::Reverse(s.attempt)))
                    .unwrap();
                if best.0.attempt != recipe.base {
                    return Err("Offline composite base ranking changed".into());
                }
                for lba in 0..sectors as u64 {
                    let mut good: Option<&[u8]> = None;
                    for (_, bytes, missing) in sources.values() {
                        if missing.contains(&lba) {
                            continue;
                        }
                        let bytes = &bytes[lba as usize * 512..(lba as usize + 1) * 512];
                        if good.is_some_and(|g| g != bytes) {
                            return Err("Readable offline source sectors disagree".into());
                        }
                        good = Some(bytes);
                    }
                }
                let mut expected = Vec::new();
                let mut unresolved = Vec::new();
                for lba in &bad {
                    let donor = sources
                        .values()
                        .filter(|(s, _, b)| s.attempt != recipe.base && !b.contains(lba))
                        .min_by_key(|(s, _, b)| (b.len(), std::cmp::Reverse(s.attempt)));
                    if let Some((s, bytes, _)) = donor {
                        copy(bytes, &mut image, *lba, *lba);
                        expected.push((*lba, s.attempt));
                    } else {
                        unresolved.push(*lba);
                    }
                }
                if copies
                    .iter()
                    .map(|c| (c.target_lba, c.source_attempt))
                    .collect::<Vec<_>>()
                    != expected
                    || copies.is_empty()
                    || &unresolved != expected_bad
                    || prov["disk_number"] != recipe.disk
                    || prov["base_attempt"] != recipe.base
                    || prov["replacements"]
                        != serde_json::to_value(copies).map_err(|e| e.to_string())?
                {
                    return Err("Composite recipe/provenance does not replay".into());
                }
                for c in copies {
                    let source = sources
                        .get(&c.source_attempt)
                        .ok_or("Unknown composite donor")?
                        .0;
                    if c.source_sha256 != source.image.sha256
                        || Path::new(&c.source_image)
                            .canonicalize()
                            .map_err(|e| e.to_string())?
                            != resolve(root, &source.image.path)?
                    {
                        return Err("Composite donor binding changed".into());
                    }
                }
                bad = unresolved;
            }
            Step::MirroredFat { copies, .. } => {
                if sources.len() > 1 && index != 1 {
                    return Err("Mirrored FAT must follow the composite".into());
                }
                let (expected, unresolved) =
                    sector_recovery::reconstruction_evidence(&image, &bad)?;
                if &expected != copies
                    || copies.is_empty()
                    || &unresolved != expected_bad
                    || prov["source_sha256"] != hash(&image)
                    || prov["reconstructed_sectors"]
                        != serde_json::to_value(copies).map_err(|e| e.to_string())?
                {
                    return Err("Mirrored-FAT recipe/provenance does not replay".into());
                }
                let before = image.clone();
                for c in copies {
                    copy(&before, &mut image, c.source_lba, c.target_lba);
                }
                bad = unresolved;
            }
        }
        if prov["schema_version"] != 1
            || prov["derived_sha256"] != hash(&image)
            || prov["unresolved_bad_sectors"] != json!(bad)
            || sealed(root, step_image, crate::fat12::MAX_IMAGE_BYTES as u64)? != image
        {
            return Err("Offline derived step bytes/map changed".into());
        }
    }
    Ok((image, bad))
}

fn directories(project: &ProjectState) -> Result<PathBuf, String> {
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let text = root.to_string_lossy().to_ascii_uppercase();
    if ["A:", "B:", "\\\\?\\A:", "\\\\?\\B:"]
        .iter()
        .any(|p| text.starts_with(p))
    {
        return Err("Offline publication refuses floppy output".into());
    }
    for name in ["Images", "Logs", "Reports", "Recovery"] {
        if root.join(name).canonicalize().map_err(|e| e.to_string())? != root.join(name) {
            return Err("Offline publication directory is redirected".into());
        }
    }
    Ok(root)
}

fn geometry(root: &Path, recipe: &Recipe) -> Result<Value, String> {
    if recipe.sources.is_empty() || recipe.sources.len() > SOURCES {
        return Err("Invalid offline geometry source count".into());
    }
    for source in &recipe.sources {
        if source.total_sectors == 0 || source.total_sectors > crate::fat12::MAX_IMAGE_BYTES / 512 {
            return Err("Offline geometry exceeds the floppy bound".into());
        }
        if source.log_kind != "archiver" {
            continue;
        }
        let data = sealed(root, &source.log, CONTROL)?;
        let log = crate::legacy_logs::parse_archiver_log(
            std::str::from_utf8(&data).map_err(|e| e.to_string())?,
        )?;
        let g = &log.geometry;
        if let (Some(c), Some(h), Some(s)) = (g.cylinders, g.heads, g.sectors_per_track) {
            if c > 0
                && h > 0
                && s > 0
                && g.bytes_per_sector == Some(512)
                && c.checked_mul(h as u64)
                    .and_then(|n| n.checked_mul(s as u64))
                    == Some(source.total_sectors as u64)
            {
                return Ok(
                    json!({"cylinders":c, "heads":h, "sectors_per_track":s, "bytes_per_sector":512,
                    "total_bytes":source.total_sectors * 512, "format_guess":"offline-derived; source geometry, not a new physical measurement"}),
                );
            }
        }
    }
    Err("Offline publication needs complete corroborating acquisition geometry".into())
}

#[cfg(test)]
#[path = "offline_images_tests.rs"]
mod tests;

fn reservation(images: &Path, disk: u32, key: &str) -> Result<Option<Vec<u8>>, String> {
    let mut found = None;
    let prefix = format!("{disk:03}_attempt_");
    let mut count = 0;
    for entry in fs::read_dir(images).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if !name.starts_with(&prefix) || !name.ends_with(".partial.json") {
            continue;
        }
        count += 1;
        if count > 128 {
            return Err("Too many offline/acquisition reservations".into());
        }
        let bytes = read(&path, CONTROL)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        // Other acquisition backends use ordinary metadata in their partial files.
        if value.get("key").is_none() {
            continue;
        }
        let record: Publication = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if record.key != key {
            continue;
        }
        if record.disk != disk
            || record.attempt == 0
            || name != format!("{disk:03}_attempt_{:03}.partial.json", record.attempt)
            || found.is_some()
        {
            return Err("Conflicting offline publication reservation".into());
        }
        found = Some(bytes);
    }
    Ok(found)
}
fn metadata(p: &Publication, report: &str, report_hash: &str) -> Value {
    let stem = format!("{:03}_attempt_{:03}", p.disk, p.attempt);
    let total = p.recipe.sources[0].total_sectors;
    let heads = p.geometry["heads"].as_u64().unwrap();
    let spt = p.geometry["sectors_per_track"].as_u64().unwrap();
    json!({"fluxvault_version":env!("CARGO_PKG_VERSION"), "status":"DERIVED", "disk_number":p.disk,
        "attempt_number":p.attempt, "source_backend":BACKEND, "source_device":"saved images; no physical read",
        "image_file":format!("{stem}.img"), "log_file":format!("Logs/{stem}.log"),
        "timestamp_unix_ms":p.created_unix_ms, "geometry":p.geometry, "sector_retries":0,
        "total_sectors":total, "bytes_written":total * 512, "retry_recovered_sectors":0,
        "bad_sector_count":p.unresolved.len(), "bad_sectors":p.unresolved.iter().map(|l| json!({"lba":l,
            "cylinder":l / (heads * spt), "head":l / spt % heads, "sector":l % spt + 1})).collect::<Vec<_>>(),
        "sha256":p.image_sha256, "offline_provenance":report, "offline_provenance_sha256":report_hash})
}
fn log(p: &Publication) -> String {
    let g = &p.geometry;
    let mut text = format!(
        "BEGIN | disk={} | attempt={} | source={BACKEND}\nGEOMETRY | cylinders={} | heads={} | sectors_per_track={} | bytes_per_sector=512 | total_sectors={} | total_bytes={}\n",
        p.disk,
        p.attempt,
        g["cylinders"],
        g["heads"],
        g["sectors_per_track"],
        p.recipe.sources[0].total_sectors,
        p.recipe.sources[0].total_sectors * 512
    );
    for l in &p.unresolved {
        text.push_str(&format!("BAD_SECTOR | LBA={l}\n"));
    }
    text.push_str(&format!(
        "END | status=DERIVED | bad_sectors={} | retry_recovered=0 | bytes={} | sha256={}\n",
        p.unresolved.len(),
        p.recipe.sources[0].total_sectors * 512,
        p.image_sha256
    ));
    text
}
fn write_once(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            if read(path, bytes.len() as u64)? != bytes {
                return Err(format!(
                    "Existing offline publication differs: {}",
                    path.display()
                ));
            }
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let temporary = path.with_file_name(format!(
        "{}.partial.{}-{}",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    output
        .write_all(bytes)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    drop(output);
    crate::flux_recovery::publish_image_no_replace(&temporary, path)
}

pub(crate) fn publish(
    project: &ProjectState,
    disk: u32,
    attempts: &[AttemptSummary],
    composite: Option<&CompositeResult>,
    reconstruction: Option<&ReconstructionResult>,
) -> Result<Published, String> {
    let root = directories(project)?;
    if attempts.is_empty() || attempts.len() > SOURCES {
        return Err("Offline source count exceeds its bound".into());
    }
    let base = composite
        .map(|c| c.base_attempt)
        .unwrap_or_else(|| attempts[0].attempt_number);
    let mut sources = Vec::new();
    for a in attempts {
        let source = seal(
            &root,
            &crate::recovery_plan::resolve_image_path(&project.images_dir(), &a.image_file)?,
            crate::fat12::MAX_IMAGE_BYTES as u64,
        )?;
        if source.sha256 != a.sha256 {
            return Err("Offline publication source hash changed".into());
        }
        sources.push(Source {
            attempt: a.attempt_number,
            status: a.status.clone(),
            image: source,
            metadata: if a.metadata_path.as_os_str().is_empty() {
                None
            } else {
                Some(seal(&root, &a.metadata_path, CONTROL)?)
            },
            log: seal(&root, Path::new(&a.log_file), CONTROL)?,
            log_kind: if a.parsed_log.is_some() {
                "archiver"
            } else {
                "dmde"
            }
            .into(),
            total_sectors: a.total_sectors,
            bad: a.bad_sectors.clone(),
        });
    }
    sources.sort_by_key(|s| s.attempt);
    let mut steps = Vec::new();
    if let Some(c) = composite {
        steps.push(Step::Composite {
            image: seal(
                &root,
                c.derived_image.as_ref().ok_or("No composite image")?,
                crate::fat12::MAX_IMAGE_BYTES as u64,
            )?,
            provenance: seal(
                &root,
                c.provenance_path
                    .as_ref()
                    .ok_or("No composite provenance")?,
                CONTROL,
            )?,
            copies: c.replacements.clone(),
            bad: c.unresolved_bad_sectors.clone(),
        });
    }
    if let Some(r) = reconstruction {
        steps.push(Step::MirroredFat {
            image: seal(
                &root,
                r.derived_image.as_ref().ok_or("No FAT image")?,
                crate::fat12::MAX_IMAGE_BYTES as u64,
            )?,
            provenance: seal(
                &root,
                r.provenance_path.as_ref().ok_or("No FAT provenance")?,
                CONTROL,
            )?,
            copies: r.reconstructed.clone(),
            bad: r.unresolved_bad_sectors.clone(),
        });
    }
    let recipe = Recipe {
        schema: 1,
        disk,
        base,
        sources,
        steps,
    };
    let (bytes, unresolved) = replay(&root, &recipe)?;
    let key = hash(&serde_json::to_vec(&recipe).map_err(|e| e.to_string())?);
    let name = format!("OfflineDerived-{disk:03}-{key}.json");
    let report = root.join("Reports").join(&name);
    let existing = match fs::symlink_metadata(&report) {
        Ok(_) => Some(read(&report, CONTROL)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            reservation(&project.images_dir(), disk, &key)?
        }
        Err(e) => return Err(e.to_string()),
    };
    let (publication, report_bytes) = if let Some(data) = existing {
        let p: Publication = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        if p.schema != 1
            || p.attempt == 0
            || p.key != key
            || p.disk != disk
            || p.image_sha256 != hash(&bytes)
            || p.unresolved != unresolved
            || p.geometry != geometry(&root, &recipe)?
            || serde_json::to_vec(&p.recipe).map_err(|e| e.to_string())?
                != serde_json::to_vec(&recipe).map_err(|e| e.to_string())?
        {
            return Err("Existing offline publication intent changed".into());
        }
        (p, data)
    } else {
        let p = Publication { schema:1, key, disk, attempt:crate::imaging::next_attempt_number(&project.images_dir(), disk)?,
            created_unix_ms:crate::external_tools::current_unix_ms() as u128, image_sha256:hash(&bytes), unresolved,
            geometry:geometry(&root, &recipe)?, recipe,
            warning:"DERIVED: exact saved-sector copies with replayable provenance, not a physical read. Mirrored FAT copies are reconstructed redundancy, not independent captures. Matching controls cannot prove original custody. Filesystem/customer completeness remains unverified.".into() };
        let data = serde_json::to_vec_pretty(&p).map_err(|e| e.to_string())?;
        (p, data)
    };
    let stem = format!("{disk:03}_attempt_{:03}", publication.attempt);
    let image = root.join("Images").join(format!("{stem}.img"));
    let meta = root.join("Images").join(format!("{stem}.json"));
    let reused = meta.is_file();
    let value = metadata(&publication, &name, &hash(&report_bytes));
    // Reserve the acquisition slot before publication. A killed writer cannot let
    // a later physical acquisition reuse its planned attempt number.
    write_once(&meta.with_extension("partial.json"), &report_bytes)?;
    write_once(&report, &report_bytes)?;
    write_once(&image, &bytes)?;
    write_once(
        &root.join("Logs").join(format!("{stem}.log")),
        log(&publication).as_bytes(),
    )?;
    write_once(
        &meta,
        &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )?;
    verify_metadata(&project.images_dir(), &meta, &value)?;
    Ok(Published {
        attempt: publication.attempt,
        image,
        report,
        reused,
    })
}

pub(crate) fn verify_metadata(
    images: &Path,
    metadata_path: &Path,
    value: &Value,
) -> Result<(), String> {
    if value["source_backend"] != BACKEND {
        let reservation = metadata_path.with_extension("partial.json");
        if reservation.is_file() {
            let data = read(&reservation, CONTROL)?;
            if serde_json::from_slice::<Publication>(&data).is_ok() {
                return Err(
                    "Reserved offline DERIVED attempt was relabelled as an acquisition".into(),
                );
            }
        }
        if value.get("offline_provenance").is_some() || value["status"] == "DERIVED" {
            return Err("Derived acquisition lacks its offline recovery binding".into());
        }
        return Ok(());
    }
    let root = images
        .parent()
        .ok_or("Invalid offline Images parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let name = value["offline_provenance"]
        .as_str()
        .ok_or("Missing offline provenance")?;
    if name.contains(['/', '\\']) {
        return Err("Offline provenance must be a report basename".into());
    }
    let report = resolve(&root, &format!("Reports/{name}"))?;
    let bytes = read(&report, CONTROL)?;
    if value["offline_provenance_sha256"] != hash(&bytes) {
        return Err("Offline provenance hash changed".into());
    }
    let p: Publication = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if p.schema != 1
        || p.attempt == 0
        || p.disk != p.recipe.disk
        || p.geometry["heads"].as_u64().unwrap_or(0) == 0
        || p.geometry["sectors_per_track"].as_u64().unwrap_or(0) == 0
        || p.recipe.sources.is_empty()
        || p.geometry != geometry(&root, &p.recipe)?
        || p.key != hash(&serde_json::to_vec(&p.recipe).map_err(|e| e.to_string())?)
        || name != format!("OfflineDerived-{:03}-{}.json", p.disk, p.key)
        || metadata_path.file_name().and_then(|n| n.to_str())
            != Some(format!("{:03}_attempt_{:03}.json", p.disk, p.attempt).as_str())
        || &metadata(&p, name, &hash(&bytes)) != value
    {
        return Err("Offline publication metadata/identity changed".into());
    }
    let (replayed, bad) = replay(&root, &p.recipe)?;
    if hash(&replayed) != p.image_sha256
        || bad != p.unresolved
        || read(
            &resolve(
                &root,
                &format!("Images/{:03}_attempt_{:03}.img", p.disk, p.attempt),
            )?,
            crate::fat12::MAX_IMAGE_BYTES as u64,
        )? != replayed
        || read(
            &resolve(
                &root,
                &format!("Logs/{:03}_attempt_{:03}.log", p.disk, p.attempt),
            )?,
            CONTROL,
        )? != log(&p).as_bytes()
    {
        return Err("Offline published bytes/map/log do not replay".into());
    }
    Ok(())
}

pub(crate) fn verify_attempt(images: &Path, attempt: &AttemptSummary) -> Result<(), String> {
    if attempt.status != "DERIVED"
        && !attempt
            .parsed_log
            .as_ref()
            .is_some_and(|log| log.status == crate::legacy_logs::ArchiverLogStatus::Derived)
    {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(&read(&attempt.metadata_path, CONTROL)?)
        .map_err(|e| e.to_string())?;
    verify_metadata(images, &attempt.metadata_path, &value)?;
    let bad = value["bad_sectors"]
        .as_array()
        .ok_or("Missing derived sector map")?
        .iter()
        .map(|v| v["lba"].as_u64().ok_or("Invalid derived LBA"))
        .collect::<Result<Vec<_>, _>>()?;
    if value["attempt_number"] != attempt.attempt_number
        || value["sha256"] != attempt.sha256
        || value["total_sectors"] != attempt.total_sectors
        || value["status"] != attempt.status
        || map(&bad, attempt.total_sectors)? != map(&attempt.bad_sectors, attempt.total_sectors)?
        || crate::recovery_plan::resolve_image_path(
            images,
            value["image_file"]
                .as_str()
                .ok_or("Missing derived image")?,
        )?
        .canonicalize()
        .map_err(|e| e.to_string())?
            != crate::recovery_plan::resolve_image_path(images, &attempt.image_file)?
                .canonicalize()
                .map_err(|e| e.to_string())?
    {
        return Err("Derived attempt summary does not match verified publication".into());
    }
    Ok(())
}
