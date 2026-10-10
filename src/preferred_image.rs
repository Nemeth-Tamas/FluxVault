//! Hash-bound expert selection. A changed/missing pinned image never falls back.
use crate::{
    imaging::{self, AttemptSummary},
    project::ProjectState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[cfg(test)]
#[path = "preferred_image_tests.rs"]
mod tests;
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
struct Binding {
    attempt: u32,
    image_sha256: String,
    metadata_sha256: Option<String>,
    log_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    schema_version: u32,
    disk: u32,
    binding: Option<Binding>,
    updated_unix_ms: u64,
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    crate::cancellation::check()?;
    crate::safety::workstation_path(path)?;
    crate::safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)?;
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.file_type().is_file() || m.len() > limit {
        return Err("Selection/evidence must be a bounded regular file".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Preferred evidence refuses reparse points".into());
        }
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Selection/evidence grew beyond its bound".into());
    }
    crate::cancellation::check()?;
    Ok(bytes)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn directory(images: &Path) -> Result<PathBuf, String> {
    let root = images
        .parent()
        .ok_or("Missing project root")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    crate::safety::workstation_path(&root)?;
    let dir = root.join(".fluxvault-preferred-images");
    if dir.try_exists().map_err(|e| e.to_string())?
        && (!dir.is_dir() || dir.canonicalize().map_err(|e| e.to_string())? != dir)
    {
        return Err("Preferred folder escapes project".into());
    }
    Ok(dir)
}
fn binding(images: &Path, a: &AttemptSummary) -> Result<Binding, String> {
    crate::offline_images::verify_attempt(images, a)?;
    if !matches!(a.status.as_str(), "OK" | "PARTIAL" | "DERIVED") {
        return Err("Only completed evidence can be preferred".into());
    }
    let bytes = read(
        &crate::recovery_plan::resolve_image_path(images, &a.image_file)?,
        crate::fat12::MAX_IMAGE_BYTES as u64,
    )?;
    if hash(&bytes) != a.sha256 {
        return Err("Preferred image hash changed".into());
    }
    crate::fat12_recovery::validate_sector_evidence(a, bytes.len() / 512, &a.sha256)?;
    let root = images.parent().ok_or("Missing project root")?;
    let log = Path::new(&a.log_file);
    crate::safety::workstation_path(log)?;
    if log.canonicalize().map_err(|e| e.to_string())?.parent()
        != Some(
            root.join("Logs")
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Preferred log escapes project Logs".into());
    }
    let log_sha256 = hash(&read(log, 32 * 1024 * 1024)?);
    let metadata_sha256 = if a.metadata_path.as_os_str().is_empty() {
        None
    } else {
        if a.metadata_path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .parent()
            != Some(images.canonicalize().map_err(|e| e.to_string())?.as_path())
        {
            return Err("Preferred metadata escapes Images".into());
        }
        Some(hash(&read(&a.metadata_path, 1024 * 1024)?))
    };
    Ok(Binding {
        attempt: a.attempt_number,
        image_sha256: a.sha256.clone(),
        metadata_sha256,
        log_sha256,
    })
}
fn load(path: &Path, disk: u32) -> Result<Option<Selection>, String> {
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    let s: Selection = serde_json::from_slice(&read(path, 16384)?).map_err(|e| e.to_string())?;
    if s.schema_version != 1 || s.disk != disk {
        return Err("Preferred selection identity/schema mismatch".into());
    }
    Ok(Some(s))
}
pub(crate) fn apply(
    images: &Path,
    disk: u32,
    attempts: &mut [AttemptSummary],
) -> Result<(), String> {
    if !images.exists() {
        return Ok(());
    }
    let Some(s) = load(&directory(images)?.join(format!("{disk:03}.json")), disk)? else {
        return Ok(());
    };
    let Some(expected) = s.binding else {
        return Ok(());
    };
    if attempts
        .iter()
        .filter(|a| a.attempt_number == expected.attempt)
        .count()
        != 1
    {
        return Err("Preferred attempt missing or ambiguous; no silent fallback".into());
    }
    let a = attempts
        .iter_mut()
        .find(|a| a.attempt_number == expected.attempt)
        .ok_or(
            "Preferred attempt missing; use disk prefer N auto explicitly, no silent fallback",
        )?;
    if binding(images, a)? != expected {
        return Err("Preferred binding changed; no silent automatic fallback".into());
    }
    a.preferred = true;
    Ok(())
}

// Keep older diagnostic metadata compatible when no explicit pin exists.
pub(crate) fn preferred_number(images: &Path, disk: u32) -> Result<Option<u32>, String> {
    let Some(s) = load(&directory(images)?.join(format!("{disk:03}.json")), disk)? else {
        return Ok(None);
    };
    let Some(expected) = s.binding else {
        return Ok(None);
    };
    let attempts = imaging::load_attempts_for_disk(images, disk)?;
    let number = attempts
        .iter()
        .find(|a| a.preferred)
        .map(|a| a.attempt_number);
    if number != Some(expected.attempt) {
        return Err("Preference changed during inspection; retry".into());
    }
    Ok(number)
}
pub(crate) fn set(
    project: &ProjectState,
    disk: u32,
    attempt: Option<u32>,
) -> Result<serde_json::Value, String> {
    if disk == 0 || disk == u32::MAX {
        return Err("Preferred disk label must be positive and bounded".into());
    }
    crate::processing::validate_workspace(project)?;
    let _owner = crate::project_work::reserve(project.root())?;
    let _snapshot = crate::project_work::snapshot(project.root())?;
    let images = project.images_dir();
    let attempts = imaging::load_unselected_attempts(&images, disk)?;
    let selected = attempt
        .map(|n| {
            if attempts.iter().filter(|a| a.attempt_number == n).count() != 1 {
                return Err("Requested attempt missing or ambiguous".into());
            }
            binding(
                &images,
                attempts
                    .iter()
                    .find(|a| a.attempt_number == n)
                    .ok_or("Requested attempt does not exist")?,
            )
        })
        .transpose()?;
    let dir = directory(&images)?;
    if !dir.exists() {
        fs::create_dir(&dir).map_err(|e| e.to_string())?;
    }
    directory(&images)?;
    let path = dir.join(format!("{disk:03}.json"));
    let previous = if path.try_exists().map_err(|e| e.to_string())? {
        Some(read(&path, 16384)?)
    } else {
        None
    };
    load(&path, disk)?;
    let s = Selection {
        schema_version: 1,
        disk,
        binding: selected,
        updated_unix_ms: crate::external_tools::current_unix_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&s).map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let write_new = |p: &Path, b: &[u8]| -> Result<(), String> {
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(p)
            .map_err(|e| e.to_string())?;
        f.write_all(b)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())
    };
    if let Some(old) = &previous {
        write_new(&dir.join(format!("{disk:03}-{nonce}.history.json")), old)?;
    }
    let temp = dir.join(format!("{disk:03}-{nonce}.partial.json"));
    write_new(&temp, &bytes)?;
    if previous.is_none() && path.try_exists().map_err(|e| e.to_string())? {
        return Err("Preferred selection appeared externally; partial retained".into());
    }
    if previous.as_ref().map(|_| read(&path, 16384)).transpose()? != previous {
        return Err("Preferred selection externally changed; partial/history retained".into());
    }
    if let Some(expected) = &s.binding {
        let fresh = imaging::load_unselected_attempts(&images, disk)?;
        if fresh
            .iter()
            .filter(|a| a.attempt_number == expected.attempt)
            .count()
            != 1
        {
            return Err("Selected evidence became ambiguous; selection not promoted".into());
        }
        if binding(
            &images,
            fresh
                .iter()
                .find(|a| a.attempt_number == expected.attempt)
                .ok_or("Selected attempt disappeared")?,
        )? != *expected
        {
            return Err("Selected evidence changed; selection not promoted".into());
        }
    }
    crate::cancellation::check()?;
    fs::rename(temp, &path).map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"schema_version":1,"disk":disk,"attempt":attempt,"mode":if attempt.is_some(){"operator"}else{"automatic"},"selection":path,"physical_media_access":false,"old_attempts_preserved":true,"processing_refresh_required":true}),
    )
}

/// Refresh native extraction/inventory/audit after an explicit preference.
/// Caller holds project ownership; no external tools or media are opened.
pub(crate) fn refresh(project: &ProjectState, disk: u32) -> Result<serde_json::Value, String> {
    let _snapshot = crate::project_work::snapshot(project.root())?;
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk)?;
    let selected =
        imaging::best_attempt(&attempts).ok_or("Preferred disk has no completed attempts")?;
    let extraction = crate::fat12_recovery::recover_attempt(
        &project.images_dir(),
        &project.extracted_dir(),
        &project.recovery_dir(),
        disk,
        selected,
        &|s| eprintln!("{s}"),
    );
    let (native, native_error) = match extraction {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(e)),
    };
    let manifest = crate::manifest::build_manifest(
        &crate::manifest::ManifestRequest {
            extracted_root: project.extracted_dir(),
            images_directory: project.images_dir(),
            reports_directory: project.reports_dir(),
        },
        &|s| eprintln!("{s}"),
    )?;
    let mut audit = crate::audit::run_audit(project, &|s| eprintln!("{s}"))?;
    let report =
        crate::final_report::export(project, &audit, crate::final_report::Language::Hungarian)?;
    crate::final_report::reconcile_audit(&mut audit, &report);
    Ok(
        serde_json::json!({"native_extraction":native,"native_error":native_error,"manifest":manifest.path,"audit":audit.json_path,"workbook":report.workbook,"attention_required":native_error.is_some()||audit.attention_disks>0,"physical_media_access":false,"office_conversion_performed":false,"customer_delivery_certified":false}),
    )
}
