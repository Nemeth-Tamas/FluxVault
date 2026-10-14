//! Bind an images-only comparison to actual ZIP inputs, not editable receipts.
use super::*;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ReplayVerification {
    pub import_report_sha256: String,
    pub verified_input_files: usize,
    pub verified_input_bytes: u64,
    pub source_archive_sha256: String,
    pub scope: &'static str,
}

pub(crate) fn verify_replay_inputs(
    project: &ProjectState,
    source: &Path,
    archive_sha256: &str,
) -> Result<Option<ReplayVerification>, String> {
    let receipt = project.root().join(REPORT);
    if !receipt.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    regular(&receipt, false)?;
    let mut bytes = Vec::new();
    File::open(&receipt)
        .map_err(|e| e.to_string())?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("Replay import report exceeds bound".into());
    }
    let report: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    match report["import_scope"].as_str() {
        None | Some("full_archive") => return Ok(None), // Backward-compatible non-replay import.
        Some("images_and_logs_only") => {}
        _ => return Err("Unknown replay import scope".into()),
    }
    if report["schema"] != 1
        || report["selected_member_payloads_verified"] != true
        || report["reference_payloads_imported"] != false
        || report["source_sha256"].as_str() != Some(archive_sha256)
    {
        return Err(
            "Images-only replay does not bind this reference archive or verified inputs".into(),
        );
    }
    let files = report["files"]
        .as_array()
        .ok_or("Replay lacks its copied-input inventory")?;
    if files.is_empty() || files.len() > MAX_ENTRIES {
        return Err("Replay input inventory exceeds bounds".into());
    }
    let mut expected = BTreeMap::new();
    for file in files {
        let path = safe_path(
            file["path"].as_str().ok_or("Replay input path missing")?,
            false,
        )?;
        if !path.starts_with("Images/") && !path.starts_with("Logs/") {
            return Err("Replay inventory contains a reference/recovered payload".into());
        }
        let size = file["bytes"]
            .as_u64()
            .filter(|n| *n <= MAX_FILE)
            .ok_or("Replay input size invalid")?;
        let sha = file["sha256"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("Replay input hash invalid")?;
        if expected
            .insert(path.to_lowercase(), (path, size, sha.to_ascii_lowercase()))
            .is_some()
        {
            return Err("Duplicate replay source binding".into());
        }
    }
    regular(source, false)?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(File::open(source).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("Replay archive member bound exceeded".into());
    }
    let mut count = 0;
    let mut total = 0u64;
    for index in 0..archive.len() {
        crate::cancellation::check()?;
        let mut file = archive.by_index(index).map_err(|e| e.to_string())?;
        let path = safe_path(file.name(), file.is_dir())?;
        if file.is_dir() || (!path.starts_with("Images/") && !path.starts_with("Logs/")) {
            continue;
        }
        if file
            .unix_mode()
            .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000))
        {
            return Err("Replay input is not a regular archive member".into());
        }
        let (relative, size, claimed) = expected
            .remove(&path.to_lowercase())
            .ok_or("Replay omits/duplicates an archived acquisition input")?;
        if file.size() != size {
            return Err("Replay source extent disagrees with ZIP member".into());
        }
        let archived = hash(&mut file, MAX_FILE, None)?; // Read EOF => selected-member CRC checked.
        let copied = project.root().join(relative);
        regular(&copied, false)?;
        if !copied
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&root)
        {
            return Err("Replay input escapes the project".into());
        }
        if archived != (size, claimed) || hash_file(&copied, MAX_FILE)? != archived {
            return Err(format!("Replay input changed from archived bytes: {path}"));
        }
        total = total
            .checked_add(size)
            .filter(|n| *n <= MAX_TOTAL)
            .ok_or("Replay input byte bound exceeded")?;
        count += 1;
    }
    if !expected.is_empty() {
        return Err("Replay inventory names inputs absent from the archive".into());
    }
    Ok(Some(ReplayVerification {
        import_report_sha256: format!("{:x}", Sha256::digest(bytes)),
        verified_input_files: count,
        verified_input_bytes: total,
        source_archive_sha256: archive_sha256.to_owned(),
        scope: "archived_image_and_log_bytes_only_not_output_automation_or_customer_certification",
    }))
}
