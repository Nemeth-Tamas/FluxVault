//! Durable slot reservation and immutable replay for a GW image publication.
//! This module never talks to hardware or replaces a customer/earlier artifact.
use super::{Journal, publish_image_no_replace, write_new};
use crate::{imaging, project::ProjectState, sector_inspection};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reservation {
    schema_version: u32,
    disk: u32,
    job_started_unix_ms: u64,
    pub attempt: u32,
    profile: String,
    image_sha256: String,
    provenance_sha256: String,
    stop_reason: String,
    pub created_unix_ms: u64,
}
pub(super) fn reserve(
    project: &ProjectState,
    directory: &Path,
    job: &Journal,
    image_sha256: &str,
    provenance_sha256: &str,
    reason: &str,
) -> Result<Reservation, String> {
    sector_inspection::regular(directory, true)?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    sector_inspection::regular(&root.join("Flux"), true)?;
    if directory.canonicalize().map_err(|e| e.to_string())? != root.join("Flux/Recovery") {
        return Err("GW publication reservation escapes Flux/Recovery".into());
    }
    let path = directory.join(format!(
        ".fluxvault-gw-publication-{:03}-{}.json",
        job.disk, job.started_unix_ms
    ));
    let r = if path.try_exists().map_err(|e| e.to_string())? {
        sector_inspection::regular(&path, false)?;
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 16 * 1024 {
            return Err("GW publication reservation exceeds bound".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 16 * 1024 {
            return Err("GW reservation grew beyond bound".into());
        }
        serde_json::from_slice::<Reservation>(&bytes)
            .map_err(|e| format!("Invalid GW publication reservation: {e}"))?
    } else {
        let r = Reservation {
            schema_version: 1,
            disk: job.disk,
            job_started_unix_ms: job.started_unix_ms,
            attempt: imaging::next_attempt_number(&project.images_dir(), job.disk)?,
            profile: job.profile.clone(),
            image_sha256: image_sha256.into(),
            provenance_sha256: provenance_sha256.into(),
            stop_reason: reason.into(),
            created_unix_ms: crate::external_tools::current_unix_ms(),
        };
        let temporary = path.with_extension(format!(
            "partial-{}-{}-{}.json",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        write_new(
            &temporary,
            &serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?,
        )?;
        checkpoint("reservation_partial")?;
        publish_image_no_replace(&temporary, &path)?;
        r
    };
    if r.schema_version != 1
        || r.disk != job.disk
        || r.job_started_unix_ms != job.started_unix_ms
        || r.attempt == 0
        || r.profile != job.profile
        || r.image_sha256 != image_sha256
        || r.provenance_sha256 != provenance_sha256
        || r.stop_reason != reason
        || r.created_unix_ms == 0
    {
        return Err(
            "GW publication reservation identity/evidence changed; artifacts preserved".into(),
        );
    }
    checkpoint("reservation")?;
    Ok(r)
}

/// An accepted publication is a frozen finishing decision. Resume it before
/// reconsidering budgets/stages, even if only its reservation was committed.
pub(super) fn pending_reason(directory: &Path, job: &Journal) -> Result<Option<String>, String> {
    sector_inspection::regular(directory, true)?;
    let path = directory.join(format!(
        ".fluxvault-gw-publication-{:03}-{}.json",
        job.disk, job.started_unix_ms
    ));
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    sector_inspection::regular(&path, false)?;
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 {
        return Err("GW reservation exceeds bound".into());
    }
    let r: Reservation = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Invalid GW publication reservation: {e}"))?;
    if r.schema_version != 1
        || r.disk != job.disk
        || r.job_started_unix_ms != job.started_unix_ms
        || r.profile != job.profile
        || r.attempt == 0
        || r.created_unix_ms == 0
        || r.stop_reason.is_empty()
        || r.stop_reason.len() > 128
    {
        return Err("Invalid pending GW publication identity; no new read authorized".into());
    }
    Ok(Some(r.stop_reason))
}

pub(super) fn write_or_verify(path: &Path, expected: &[u8]) -> Result<(), String> {
    if path.try_exists().map_err(|e| e.to_string())? {
        sector_inspection::regular(path, false)?;
        let mut actual = Vec::new();
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(expected.len() as u64 + 1)
            .read_to_end(&mut actual)
            .map_err(|e| e.to_string())?;
        if actual != expected {
            return Err(format!(
                "Existing GW publication bytes differ; preserved: {}",
                path.display()
            ));
        }
        Ok(())
    } else {
        write_new(path, expected)
    }
}
pub(super) fn promote(partial: &Path, complete: &Path, expected: &[u8]) -> Result<(), String> {
    if complete.try_exists().map_err(|e| e.to_string())? {
        write_or_verify(complete, expected)?;
        if partial.try_exists().map_err(|e| e.to_string())? {
            write_or_verify(partial, expected)?;
        }
        Ok(())
    } else {
        write_or_verify(partial, expected)?;
        publish_image_no_replace(partial, complete)
    }
}
pub(super) fn paths(images: &Path, disk: u32, attempt: u32) -> (PathBuf, PathBuf) {
    let stem = format!("{disk:03}_attempt_{attempt:03}");
    (
        images.join(format!("{stem}.img")),
        images.join(format!("{stem}.partial.img")),
    )
}
pub(super) fn checkpoint(_name: &'static str) -> Result<(), String> {
    #[cfg(test)]
    if FAILURE.with(|v| v.get()) == Some(_name) {
        return Err(format!("Injected GW publication cutpoint: {_name}"));
    }
    Ok(())
}
#[cfg(test)]
thread_local! { static FAILURE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(super) fn fail_at(name: Option<&'static str>) {
    FAILURE.with(|v| v.set(name));
}
