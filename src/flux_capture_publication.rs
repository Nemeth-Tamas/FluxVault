//! Finish only host-completed, hash-sealed workstation publications. Never
//! interpret a started/failed receipt as success or invoke a physical backend.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_RECORD: u64 = 64 * 1024;

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    crate::sector_inspection::regular(path, false)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err("Prepared GW record exceeds bound".into());
    }
    Ok(bytes)
}

fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&read_bytes(path)?)
        .map_err(|e| format!("Invalid prepared GW record: {e}"))
}

pub(super) fn save_record(path: &Path, record: &impl Serialize) -> Result<(), String> {
    crate::sector_inspection::regular(path, false)?;
    let temporary = path.with_extension(format!(
        "update-{}-{}-{}.partial.json",
        std::process::id(),
        crate::external_tools::current_unix_ms(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    output
        .write_all(&serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    drop(output);
    fs::rename(&temporary, path).map_err(|e| format!("Cannot commit GW record: {e}"))
}

pub(super) fn sync_payload(path: &Path) -> Result<(), String> {
    crate::sector_inspection::regular(path, false)?;
    // Workstation output only; this module cannot resolve or open media paths.
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}

fn verify_payload(path: &Path, size: u64, sha: &str) -> Result<(), String> {
    crate::sector_inspection::regular(path, false)?;
    if size == 0
        || sha.len() != 64
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
        || fs::metadata(path).map_err(|e| e.to_string())?.len() != size
        || !hash_file(path)?.eq_ignore_ascii_case(sha)
    {
        return Err("Prepared GW payload changed; evidence preserved, publication refused".into());
    }
    Ok(())
}

fn promote_payload(partial: &Path, final_path: &Path, size: u64, sha: &str) -> Result<(), String> {
    if path_occupied(final_path)? {
        verify_payload(final_path, size, sha)?;
        if path_occupied(partial)? {
            verify_payload(partial, size, sha)?;
        }
    } else {
        verify_payload(partial, size, sha)?;
        sync_payload(partial)?;
        crate::flux_recovery::publish_image_no_replace(partial, final_path)?;
    }
    Ok(())
}

fn promote_record(partial: &Path, final_path: &Path) -> Result<(), String> {
    if path_occupied(final_path)? {
        crate::sector_inspection::regular(final_path, false)?;
        let _: serde_json::Value = read_record(final_path)?;
        if read_bytes(partial)? != read_bytes(final_path)? {
            return Err("Existing GW metadata differs; no replacement permitted".into());
        }
        // Keep the duplicate prepared receipt rather than delete evidence.
        Ok(())
    } else {
        crate::flux_recovery::publish_image_no_replace(partial, final_path)
    }
}

pub(crate) fn resume_capture_publication(
    project: &ProjectState,
    request: CaptureRequest,
    settings: &CaptureSettings,
    attempt: u32,
) -> Result<bool, String> {
    let flux = project_flux_dir(project)?;
    let stem = format!("{:03}_attempt_{attempt:03}", request.disk_number);
    let prepared = flux.join(format!("{stem}.partial.json"));
    if !path_occupied(&prepared)? {
        return Ok(false);
    }
    let record: CaptureRecord = read_record(&prepared)?;
    if record.status != "complete" {
        return Ok(false); // Failed/unsealed host work is not replayable success.
    }
    let partial = flux.join(format!("{stem}.partial.scp"));
    let final_path = flux.join(format!("{stem}.scp"));
    let expected = GreaseweazleCommand::raw_flux_read_with_settings(
        request.profile,
        request.drive,
        request.revolutions,
        &partial,
        settings,
    )?;
    if record.schema_version != SCHEMA_VERSION
        || record.disk_number != request.disk_number
        || record.attempt_number != attempt
        || attempt == 0
        || record.profile != request.profile.argument()
        || record.drive != request.drive.to_ascii_uppercase()
        || record.revolutions != request.revolutions
        || record.capture_settings.as_ref() != Some(settings)
        || record.flux_file.as_deref() != Some(format!("{stem}.scp").as_str())
        || record.command != expected.arguments()
    {
        return Err("Prepared capture settings/identity changed; no new read authorized".into());
    }
    let size = record.bytes.ok_or("Prepared capture size absent")?;
    if size > u64::from(request.revolutions) * 64 * crate::resource_budget::MIB {
        return Err("Prepared capture size exceeds bound".into());
    }
    let hash = record
        .sha256
        .as_deref()
        .ok_or("Prepared capture hash absent")?;
    promote_payload(&partial, &final_path, size, hash)?;
    promote_record(&prepared, &flux.join(format!("{stem}.json")))?;
    Ok(true)
}

pub(crate) fn resume_decode_publication(
    project: &ProjectState,
    disk: u32,
    capture: u32,
    profile: GreaseweazleProfile,
) -> Result<Option<DecodeResult>, String> {
    let flux = project_flux_dir(project)?;
    let directory = resolve_derived_dir(&flux, true)?;
    let prefix = format!(
        "{disk:03}_flux_{capture:03}_{}_decode_",
        profile.argument().replace('.', "_")
    );
    let mut candidates = Vec::new();
    let mut entries = 0;
    for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
        crate::cancellation::check()?;
        entries += 1;
        if entries > 100_000 {
            return Err("Decode publication directory exceeds bound".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) || !name.ends_with(".partial.json") {
            continue;
        }
        let record: DecodeRecord = read_record(&entry.path())?;
        if record.status != "unverified_sector_quality" {
            continue;
        }
        let stem = format!("{prefix}{:03}", record.decode_attempt);
        if record.schema_version != SCHEMA_VERSION
            || record.disk_number != disk
            || record.capture_attempt != capture
            || record.decode_attempt == 0
            || record.profile != profile.argument()
            || name != format!("{stem}.partial.json")
            || record.output_file != format!("{stem}.img")
            || record.bytes != profile.expected_sector_image_bytes()
        {
            return Err("Prepared decode identity/geometry changed; publication refused".into());
        }
        candidates.push((entry.path(), stem, record));
    }
    if candidates.len() > 1 {
        return Err(
            "Competing prepared decodes; preserve evidence and inspect before resuming".into(),
        );
    }
    let Some((prepared, stem, record)) = candidates.pop() else {
        return Ok(None);
    };
    let source_stem = format!("{disk:03}_attempt_{capture:03}");
    let source: CaptureRecord = read_record(&flux.join(format!("{source_stem}.json")))?;
    if source.schema_version != SCHEMA_VERSION
        || source.disk_number != disk
        || source.attempt_number != capture
        || source.status != "complete"
        || record.source_flux_file != format!("{source_stem}.scp")
        || source.flux_file.as_deref() != Some(record.source_flux_file.as_str())
        || source.sha256.as_deref() != Some(record.source_sha256.as_str())
        || source.capture_settings != record.capture_settings
    {
        return Err("Prepared decode source binding changed; publication refused".into());
    }
    let _source = crate::flux_archive::open_source(
        &flux.join(&record.source_flux_file),
        source.bytes.ok_or("Prepared decode source size absent")?,
        &record.source_sha256,
    )?;
    let image = directory.join(format!("{stem}.img"));
    let metadata = directory.join(format!("{stem}.json"));
    promote_payload(
        &directory.join(format!("{stem}.partial.img")),
        &image,
        record.bytes,
        &record.output_sha256,
    )?;
    promote_record(&prepared, &metadata)?;
    Ok(Some(DecodeResult {
        disk_number: disk,
        capture_attempt: capture,
        decode_attempt: record.decode_attempt,
        image_path: image,
        metadata_path: metadata,
        bytes: record.bytes,
        sha256: record.output_sha256,
        reported_sectors: record
            .reported_found_sectors
            .zip(record.reported_total_sectors),
        gw_bad_lbas: record.gw_bad_lbas,
        host_version: record.host_version,
    }))
}

pub(super) fn checkpoint(_name: &'static str) -> Result<(), String> {
    #[cfg(test)]
    if FAILURE.with(|v| v.get()) == Some(_name) {
        return Err(format!("Injected raw GW publication cutpoint: {_name}"));
    }
    Ok(())
}
#[cfg(test)]
thread_local! { static FAILURE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(super) fn fail_at(name: Option<&'static str>) {
    FAILURE.with(|v| v.set(name));
}
