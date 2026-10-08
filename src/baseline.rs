//! Compare recovered source payloads, never Office derivatives or raw-media yield.
//! The reference ZIP is streamed in place; nothing is unpacked or overwritten.
use crate::{
    extraction::{self, ExtractionPresence},
    imaging,
    project::ProjectState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

#[derive(Clone, Debug, Serialize)]
pub struct Payload {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub category: &'static str,
    pub origin: &'static str,
    pub reference_state: &'static str,
    #[serde(skip)]
    key: String,
}

#[derive(Debug, Serialize)]
pub struct FileComparison {
    pub status: &'static str,
    pub reference: Option<Payload>,
    pub current: Option<Payload>,
}

#[derive(Debug, Serialize)]
pub struct DiskComparison {
    pub disk: u32,
    pub reference_image_sha256: Option<String>,
    pub current_image_sha256: Option<String>,
    pub same_image_bytes: Option<bool>,
    pub extraction_binding_verified: bool,
    pub reference_payloads: usize,
    pub matched_payloads: usize,
    pub changed_payloads: usize,
    pub missing_payloads: usize,
    pub current_only_payloads: usize,
    pub files: Vec<FileComparison>,
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub schema_version: u32,
    pub scope: &'static str,
    pub include_deleted: bool,
    pub reference_archive: PathBuf,
    pub reference_archive_sha256: String,
    pub selected_disks: Vec<u32>,
    pub unscanned_reference_disks: Vec<u32>,
    pub reference_payloads: usize,
    pub matched_payloads: usize,
    pub changed_payloads: usize,
    pub missing_payloads: usize,
    pub current_only_payloads: usize,
    pub excluded_reference_files: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
    pub disks: Vec<DiskComparison>,
    pub physical_media_access: bool,
    pub customer_delivery_certified: bool,
}

fn workstation(path: &Path) -> Result<(), String> {
    let name = path.to_string_lossy().to_ascii_uppercase();
    if [
        "A:",
        "B:",
        "\\\\?\\A:",
        "\\\\?\\B:",
        "\\\\.\\A:",
        "\\\\.\\B:",
    ]
    .iter()
    .any(|p| name.starts_with(p))
    {
        return Err("Baseline comparison uses workstation files only, never floppy drives".into());
    }
    Ok(())
}

fn hash_reader(reader: &mut impl Read, bound: u64) -> Result<(u64, String), String> {
    let mut bytes = 0u64;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        bytes = bytes
            .checked_add(n as u64)
            .ok_or("Comparison size overflow")?;
        if bytes > bound {
            return Err("Comparison file exceeds bounded size".into());
        }
        digest.update(&buffer[..n]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}

fn hash_file(path: &Path, bound: u64) -> Result<(u64, String), String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Comparison source must be a regular workstation file".into());
    }
    hash_reader(&mut File::open(path).map_err(|e| e.to_string())?, bound)
}

fn safe_relative(value: &str) -> Result<String, String> {
    let value = value.replace('\\', "/");
    if value.len() > 4096
        || value.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.contains(':')
                || part.chars().any(char::is_control)
        })
    {
        return Err("Unsafe comparison relative path".into());
    }
    Ok(value)
}

fn category(path: &str, bytes: u64) -> &'static str {
    let lower = path.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or("");
    if lower.split('/').any(|p| {
        matches!(
            p,
            "system volume information" | "$recycle.bin" | "recycled" | "recycler"
        )
    }) {
        "os_metadata"
    } else if name.starts_with(".fluxvault-") || name.starts_with("__") {
        "internal_metadata"
    } else if !lower.contains('/')
        && matches!(name, "filelist.htm" | "filelist.html" | "filelist.txt")
    {
        "recovery_tool_report"
    } else if bytes == 0 {
        "empty"
    } else if name.starts_with('~') || name.ends_with(".tmp") {
        "temporary"
    } else {
        "payload"
    }
}

fn payload(
    path: &str,
    bytes: u64,
    sha256: String,
    origin: &'static str,
) -> Result<Payload, String> {
    let path = safe_relative(path)?;
    let key = crate::conversion::comparison_path(Path::new(&path))?
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    Ok(Payload {
        category: category(&path, bytes),
        path,
        bytes,
        sha256,
        origin,
        reference_state: "unknown",
        key,
    })
}

/// DMDE's exported table has six columns before the path, which can contain
/// spaces. Only a unique path with an equal byte count classifies a ZIP member.
fn dmde_filelist(bytes: &[u8]) -> Result<BTreeMap<String, (u64, &'static str)>, String> {
    let text = if bytes.starts_with(&[0xff, 0xfe])
        || (bytes.len() > 8 && bytes[1] == 0 && bytes[3] == 0)
    {
        let bytes = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
        if !bytes.len().is_multiple_of(2) {
            return Err("Invalid UTF-16 DMDE file list".into());
        }
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?
    };
    let mut result = BTreeMap::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut columns = Vec::new();
        for _ in 0..6 {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            columns.push(&rest[..end]);
            rest = rest[end..].trim_start();
        }
        let Ok(size) = columns[2].parse::<u64>() else {
            continue;
        };
        let state = match columns[5] {
            "x" | "xf" => "deleted",
            "." | "f" => "live",
            kind if kind.starts_with("+.") => "signature_carved",
            _ => "unknown",
        };
        if rest.is_empty() {
            continue;
        }
        let key = safe_relative(rest)?.to_lowercase();
        // Deleted versions can share a live filename. Ambiguous paths must
        // stay unknown (in scope), never be silently excluded as deleted.
        if result.contains_key(&key) {
            result.insert(key, (0, "ambiguous"));
        } else {
            result.insert(key, (size, state));
        }
    }
    Ok(result)
}

fn matches(reference: Vec<Payload>, current: Vec<Payload>) -> Vec<FileComparison> {
    let mut reference: Vec<_> = reference.into_iter().map(Some).collect();
    let mut current: Vec<_> = current.into_iter().map(Some).collect();
    let mut rows = Vec::new();
    // Globally reserve exact matches first, then renamed identical bytes, then
    // changed same-name files. Each candidate is consumed only once, per disk.
    for pass in 0..6 {
        let mode = pass % 3;
        for old in &mut reference {
            let Some(baseline) = old.as_ref() else {
                continue;
            };
            if (baseline.category == "payload") != (pass < 3) {
                continue;
            }
            let found = current.iter().position(|new| {
                new.as_ref().is_some_and(|new| match mode {
                    0 => baseline.key == new.key && baseline.sha256 == new.sha256,
                    1 => baseline.sha256 == new.sha256 && baseline.bytes == new.bytes,
                    _ => baseline.key == new.key,
                })
            });
            if let Some(index) = found {
                rows.push(FileComparison {
                    status: match mode {
                        0 => "same_path_and_hash",
                        1 => "same_hash_different_path",
                        _ => "same_path_changed_bytes",
                    },
                    reference: old.take(),
                    current: current[index].take(),
                });
            }
        }
    }
    rows.extend(reference.into_iter().flatten().map(|old| FileComparison {
        status: "missing_from_current",
        reference: Some(old),
        current: None,
    }));
    rows.extend(current.into_iter().flatten().map(|new| FileComparison {
        status: "current_only",
        reference: None,
        current: Some(new),
    }));
    rows.sort_by(|a, b| {
        a.reference
            .as_ref()
            .or(a.current.as_ref())
            .unwrap()
            .path
            .cmp(&b.reference.as_ref().or(b.current.as_ref()).unwrap().path)
    });
    rows
}

fn current_files(root: &Path, origin: &'static str) -> Result<Vec<Payload>, String> {
    let resolved = root.canonicalize().map_err(|e| e.to_string())?;
    let mut pending = vec![resolved.clone()];
    let mut result = Vec::new();
    let mut total = 0;
    let mut entries = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err("Too many current comparison entries".into());
            }
            let path = entry.path();
            let relative = safe_relative(
                &path
                    .strip_prefix(&resolved)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy(),
            )?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink()
                || !path
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&resolved)
            {
                return Err("Comparison file escapes extracted content".into());
            }
            if kind.is_dir() {
                // Ignore other managed attempts when operator recovery overrides them.
                if path.join(".fluxvault-extraction.json").is_file() {
                    continue;
                }
                if category(&relative, 1) != "os_metadata" {
                    pending.push(path);
                }
            } else if kind.is_file() {
                if matches!(
                    category(&relative, 1),
                    "internal_metadata" | "os_metadata" | "recovery_tool_report"
                ) {
                    continue;
                }
                let (bytes, sha) = hash_file(&path, MAX_FILE)?;
                total += bytes;
                if total > MAX_TOTAL {
                    return Err("Current payloads exceed comparison bound".into());
                }
                let origin = if relative
                    .split('/')
                    .any(|p| p.eq_ignore_ascii_case("SignatureRecovery"))
                {
                    "signature_carved"
                } else if relative
                    .split('/')
                    .any(|p| p.eq_ignore_ascii_case("DirectoryRecovery"))
                {
                    "reconstructed_directory"
                } else if relative
                    .split('/')
                    .any(|p| p.eq_ignore_ascii_case("FragmentRecovery"))
                {
                    "fragment_chain_hypothesis"
                } else {
                    origin
                };
                result.push(payload(&relative, bytes, sha, origin)?);
            } else {
                return Err("Unsupported comparison file type".into());
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// The same ownership reservation covers inventory and report publication.
pub fn run(
    project: &ProjectState,
    baseline: &Path,
    include_deleted: bool,
    progress: &impl Fn(&str),
) -> Result<(Comparison, PathBuf, PathBuf), String> {
    workstation(project.root())?;
    workstation(baseline)?;
    let _owner = crate::project_work::reserve(project.root())?;
    let result = compare(project, baseline, include_deleted, progress)?;
    let (json, csv) = export(project, &result)?;
    Ok((result, json, csv))
}

fn compare(
    project: &ProjectState,
    baseline: &Path,
    include_deleted: bool,
    progress: &impl Fn(&str),
) -> Result<Comparison, String> {
    workstation(project.root())?;
    workstation(baseline)?;
    crate::processing::validate_workspace(project)?;
    let baseline = baseline.canonicalize().map_err(|e| e.to_string())?;
    workstation(&baseline)?;
    progress("Hashing the reference archive; no files will be unpacked...");
    let (_, archive_hash) = hash_file(&baseline, 4 * 1024 * 1024 * 1024)?;
    let mut selected = BTreeSet::new();
    let mut images = BTreeMap::new();
    let mut current = BTreeMap::new();
    let mut bound = BTreeMap::new();
    let mut warnings = vec!["Content matches are within each numbered disk, with one-to-one multiplicity. Changed source-image bytes prevent a same-evidence yield claim. Renamed identical content does not validate original names. Empty/temporary files and tool/OS reports are separate from payload totals; matching source payloads is not document-render or customer-delivery certification.".into()];
    for disk in imaging::load_project_statistics(&project.images_dir())?.disks {
        selected.insert(disk.disk_number);
        let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk.disk_number)?;
        let attempt = attempts
            .iter()
            .min_by(|a, b| {
                a.attention_required
                    .cmp(&b.attention_required)
                    .then(a.bad_sectors.len().cmp(&b.bad_sectors.len()))
                    .then(b.attempt_number.cmp(&a.attempt_number))
            })
            .ok_or("Missing comparison image attempt")?;
        let image =
            crate::recovery_plan::resolve_image_path(&project.images_dir(), &attempt.image_file)?;
        let (_, hash) = hash_file(&image, crate::fat12::MAX_IMAGE_BYTES as u64)?;
        if !attempt.sha256.is_empty() && attempt.sha256.to_lowercase() != hash {
            return Err("Current comparison image changed from acquisition binding".into());
        }
        images.insert(disk.disk_number, hash.clone());
        let presence = extraction::inspect_extraction_presence(
            &project.extracted_dir(),
            disk.disk_number,
            attempt.attempt_number,
        )?;
        let (root, verified, origin) = match presence {
            ExtractionPresence::Automatic {
                output_directory,
                recovery_attention,
                ..
            } => {
                extraction::verify_managed_extraction(&output_directory, &hash)?;
                (
                    Some(output_directory),
                    true,
                    if recovery_attention {
                        "native_readable_chains"
                    } else {
                        "filesystem"
                    },
                )
            }
            ExtractionPresence::ManualRecovery {
                output_directory, ..
            } => (Some(output_directory), false, "operator_recovery"),
            ExtractionPresence::Missing { .. } => {
                // Compare preserved older engine generations too. A newer
                // engine must not turn existing intact output into "missing".
                let disk_root = project
                    .extracted_dir()
                    .join(format!("{:03}", disk.disk_number));
                let mut candidates = Vec::new();
                if disk_root.is_dir() {
                    for (index, entry) in fs::read_dir(&disk_root)
                        .map_err(|e| e.to_string())?
                        .enumerate()
                    {
                        if index >= MAX_ENTRIES {
                            return Err("Too many managed extraction entries".into());
                        }
                        let entry = entry.map_err(|e| e.to_string())?;
                        let path = entry.path();
                        if path.join(".fluxvault-extraction.json").is_file()
                            && extraction::managed_directory_order(&path).0
                                == attempt.attempt_number
                        {
                            candidates.push(path);
                        }
                    }
                }
                candidates.sort_by_key(|p| extraction::managed_directory_order(p));
                if let Some(root) = candidates.pop() {
                    if !root.canonicalize().map_err(|e| e.to_string())?.starts_with(
                        project
                            .extracted_dir()
                            .canonicalize()
                            .map_err(|e| e.to_string())?,
                    ) {
                        return Err("Comparison extraction escapes project".into());
                    }
                    extraction::verify_managed_extraction(&root, &hash)?;
                    (Some(root), true, "preserved_managed_generation")
                } else {
                    (None, false, "missing")
                }
            }
            ExtractionPresence::InvalidAutomatic { detail, .. } => {
                return Err(format!("Invalid extraction for comparison: {detail}"));
            }
        };
        if let Some(root) = &root {
            let resolved = root.canonicalize().map_err(|e| e.to_string())?;
            if !resolved.starts_with(
                project
                    .extracted_dir()
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
            ) {
                return Err("Comparison extraction escapes project".into());
            }
        }
        current.insert(
            disk.disk_number,
            root.map(|r| current_files(&r, origin))
                .transpose()?
                .unwrap_or_default(),
        );
        bound.insert(disk.disk_number, verified);
    }
    for result in crate::flux_recovery::format_exceptions(project)? {
        selected.insert(result.disk);
    }
    if selected.is_empty() {
        return Err(
            "No acquired disks to compare; unscanned disks cannot be scored as missing".into(),
        );
    }
    let mut archive = zip::ZipArchive::new(File::open(&baseline).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("Reference archive exceeds entry bound".into());
    }
    let mut reference: BTreeMap<u32, Vec<Payload>> = BTreeMap::new();
    let mut reference_images = BTreeMap::new();
    let mut all_disks = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut excluded: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0;
    let mut filelists = BTreeMap::new();
    progress("Comparing recovered payload hashes against reference ZIP entries...");
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|e| e.to_string())?;
        if file.is_dir() {
            continue;
        }
        let name = safe_relative(file.name())?;
        if !seen.insert(name.to_lowercase()) {
            return Err("Duplicate reference ZIP member identity".into());
        }
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Symlink in reference archive".into());
        }
        let parts: Vec<_> = name.split('/').collect();
        if parts.first() == Some(&"Images") && parts.len() == 2 {
            let filename = parts[1];
            let number = filename
                .strip_suffix(".bin")
                .or_else(|| filename.strip_suffix(".img"))
                .and_then(|s| s.parse::<u32>().ok());
            if let Some(disk) = number.filter(|n| *n > 0) {
                all_disks.insert(disk);
                if selected.contains(&disk) {
                    let expected = file.size();
                    let (bytes, hash) =
                        hash_reader(&mut file, crate::fat12::MAX_IMAGE_BYTES as u64)?;
                    if bytes != expected || reference_images.insert(disk, hash).is_some() {
                        return Err("Invalid/duplicate reference image".into());
                    }
                }
            }
            continue;
        }
        if parts.first() != Some(&"Extracted") || parts.len() < 3 {
            continue;
        }
        let disk = parts[1]
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("Invalid reference disk number")?;
        all_disks.insert(disk);
        if !selected.contains(&disk) {
            continue;
        }
        let relative = parts[2..].join("/");
        let kind = category(&relative, file.size());
        if relative.eq_ignore_ascii_case("filelist.txt") {
            let mut bytes = Vec::new();
            if file.size() > 1024 * 1024 {
                return Err("DMDE classification report exceeds size bound".into());
            }
            file.by_ref()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 != file.size() {
                return Err("DMDE classification report size mismatch".into());
            }
            filelists.insert(disk, dmde_filelist(&bytes)?);
        }
        if matches!(
            kind,
            "os_metadata" | "internal_metadata" | "recovery_tool_report"
        ) {
            *excluded.entry(kind.into()).or_default() += 1;
            continue;
        }
        let expected = file.size();
        let (bytes, hash) = hash_reader(&mut file, MAX_FILE)?;
        if bytes != expected {
            return Err("Reference member size differs from ZIP metadata".into());
        }
        total += bytes;
        if total > MAX_TOTAL {
            return Err("Reference payloads exceed comparison bound".into());
        }
        let origin =
            if relative.contains("[$Raw Files by Signatures]") || relative.contains("/$Raw/") {
                "signature_carved"
            } else if relative.contains("$Noname") || relative.contains("$Root") {
                "dmde_filesystem"
            } else {
                "filesystem"
            };
        reference
            .entry(disk)
            .or_default()
            .push(payload(&relative, bytes, hash, origin)?);
    }
    drop(archive);
    if hash_file(&baseline, 4 * 1024 * 1024 * 1024)?.1 != archive_hash {
        return Err("Reference archive changed during comparison; report refused".into());
    }
    for (disk, files) in &mut reference {
        for file in files {
            if let Some((bytes, state)) = filelists
                .get(disk)
                .and_then(|list| list.get(&file.path.to_lowercase()))
                && *bytes == file.bytes
            {
                file.reference_state = state;
            }
            if file.reference_state == "deleted" && !include_deleted && file.category == "payload" {
                file.category = "deleted_out_of_scope";
                *excluded.entry("deleted_out_of_scope".into()).or_default() += 1;
            }
        }
    }
    warnings.push(if include_deleted {
        "Confirmed deleted reference payloads are included by explicit comparison-only opt-in; this flag does not enable deleted-file recovery.".into()
    } else {
        "Confirmed deleted reference payloads are excluded by default. Unknown, ambiguous and signature-carved entries remain in scope; DMDE collision-renamed versions cannot be safely classified by guessing filenames.".into()
    });
    let mut result = Comparison {
        schema_version: 1,
        scope: "selected_acquired_disks_recovered_source_payloads_against_script_archive",
        include_deleted,
        reference_archive: baseline,
        reference_archive_sha256: archive_hash,
        selected_disks: selected.iter().copied().collect(),
        unscanned_reference_disks: all_disks.difference(&selected).copied().collect(),
        reference_payloads: 0,
        matched_payloads: 0,
        changed_payloads: 0,
        missing_payloads: 0,
        current_only_payloads: 0,
        excluded_reference_files: excluded,
        warnings: vec![],
        disks: vec![],
        physical_media_access: false,
        customer_delivery_certified: false,
    };
    for disk in selected {
        let mut old = reference.remove(&disk).unwrap_or_default();
        // In-scope reference content must win identical-byte matches over
        // excluded deleted/temporary content, regardless of filename order.
        old.sort_by(|a, b| {
            (a.category != "payload", &a.path).cmp(&(b.category != "payload", &b.path))
        });
        let files = matches(old, current.remove(&disk).unwrap_or_default());
        let mut record = DiskComparison {
            disk,
            reference_image_sha256: reference_images.remove(&disk),
            current_image_sha256: images.remove(&disk),
            same_image_bytes: None,
            extraction_binding_verified: bound.remove(&disk).unwrap_or(false),
            reference_payloads: 0,
            matched_payloads: 0,
            changed_payloads: 0,
            missing_payloads: 0,
            current_only_payloads: 0,
            files,
        };
        record.same_image_bytes = record
            .reference_image_sha256
            .as_ref()
            .zip(record.current_image_sha256.as_ref())
            .map(|(a, b)| a == b);
        if !all_disks.contains(&disk) {
            warnings.push(format!("Disk {disk:03} is absent from the reference archive; no baseline equivalence can be asserted."));
        }
        for row in &record.files {
            if row
                .reference
                .as_ref()
                .is_some_and(|p| p.category == "payload")
            {
                record.reference_payloads += 1;
                match row.status {
                    "same_path_and_hash" | "same_hash_different_path" => {
                        record.matched_payloads += 1
                    }
                    "same_path_changed_bytes" => record.changed_payloads += 1,
                    _ => record.missing_payloads += 1,
                }
            }
            if row.status == "current_only"
                && row
                    .current
                    .as_ref()
                    .is_some_and(|p| p.category == "payload")
            {
                record.current_only_payloads += 1;
            }
        }
        result.reference_payloads += record.reference_payloads;
        result.matched_payloads += record.matched_payloads;
        result.changed_payloads += record.changed_payloads;
        result.missing_payloads += record.missing_payloads;
        result.current_only_payloads += record.current_only_payloads;
        result.disks.push(record);
    }
    result.warnings = warnings;
    Ok(result)
}

fn export(project: &ProjectState, result: &Comparison) -> Result<(PathBuf, PathBuf), String> {
    let reports = project
        .reports_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if reports.parent()
        != Some(
            project
                .root()
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Unsafe comparison report directory".into());
    }
    let nonce = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
        std::process::id()
    );
    let json = reports.join(format!("BaselineComparison-{nonce}.json"));
    let csv = reports.join(format!("BaselineComparison-{nonce}.csv"));
    let field = |value: &str| {
        let prefix = if value.starts_with(['=', '+', '-', '@']) {
            "'"
        } else {
            ""
        };
        format!("\"{prefix}{}\"", value.replace('"', "\"\""))
    };
    let mut text = String::from(
        "\u{feff}Disk,Status,Category,Origin,ReferenceState,ReferencePath,CurrentPath,ReferenceSHA256,CurrentSHA256,SameImageBytes\r\n",
    );
    for disk in &result.disks {
        for row in &disk.files {
            let source = row.reference.as_ref().or(row.current.as_ref()).unwrap();
            text.push_str(
                &[
                    disk.disk.to_string(),
                    row.status.into(),
                    source.category.into(),
                    source.origin.into(),
                    source.reference_state.into(),
                    row.reference
                        .as_ref()
                        .map(|p| p.path.clone())
                        .unwrap_or_default(),
                    row.current
                        .as_ref()
                        .map(|p| p.path.clone())
                        .unwrap_or_default(),
                    row.reference
                        .as_ref()
                        .map(|p| p.sha256.clone())
                        .unwrap_or_default(),
                    row.current
                        .as_ref()
                        .map(|p| p.sha256.clone())
                        .unwrap_or_default(),
                    disk.same_image_bytes
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                ]
                .iter()
                .map(|v| field(v))
                .collect::<Vec<_>>()
                .join(","),
            );
            text.push_str("\r\n");
        }
    }
    // Unique filenames preserve every prior result; never rewrite customer files.
    for (path, bytes) in [
        (
            &json,
            serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?,
        ),
        (&csv, text.into_bytes()),
    ] {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok((json, csv))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_signature_candidates_keep_their_origin_in_comparisons() {
        let root = std::env::temp_dir().join(format!(
            "fv-baseline-carves-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("SignatureRecovery")).unwrap();
        fs::write(root.join("SignatureRecovery/carved.doc"), b"candidate").unwrap();
        fs::write(root.join("normal.doc"), b"reachable").unwrap();
        for folder in ["DirectoryRecovery", "FragmentRecovery"] {
            fs::create_dir_all(root.join(folder)).unwrap();
            fs::write(root.join(folder).join("candidate.doc"), b"hypothesis").unwrap();
        }
        let rows = current_files(&root, "native_readable_chains").unwrap();
        assert_eq!(
            rows.iter()
                .find(|f| f.path.starts_with("SignatureRecovery"))
                .unwrap()
                .origin,
            "signature_carved"
        );
        assert_eq!(
            rows.iter().find(|f| f.path == "normal.doc").unwrap().origin,
            "native_readable_chains"
        );
        for (prefix, origin) in [
            ("DirectoryRecovery", "reconstructed_directory"),
            ("FragmentRecovery", "fragment_chain_hypothesis"),
        ] {
            assert_eq!(
                rows.iter()
                    .find(|f| f.path.starts_with(prefix))
                    .unwrap()
                    .origin,
                origin
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn comparison_refuses_competing_project_owner_before_inventory() {
        let root = std::env::temp_dir().join(format!(
            "fv-baseline-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let owner = crate::project_work::reserve(&root).unwrap();
        assert!(
            run(&project, &root.join("absent.zip"), false, &|_| {})
                .unwrap_err()
                .contains("processing owner")
        );
        assert_eq!(fs::read_dir(project.reports_dir()).unwrap().count(), 0);
        drop(owner);
        fs::remove_dir_all(root).unwrap();
    }
    fn item(path: &str, content: &[u8]) -> Payload {
        payload(
            path,
            content.len() as u64,
            format!("{:x}", Sha256::digest(content)),
            "filesystem",
        )
        .unwrap()
    }
    #[test]
    fn exact_matches_are_reserved_before_renames_and_multiplicity_is_not_inflated() {
        let rows = matches(
            vec![
                item("$Noname 01/$Root/a.doc", b"same"),
                item("b.doc", b"same"),
                item("c.doc", b"old"),
                item("lost.jpg", b"missing"),
            ],
            vec![
                item("b.doc", b"same"),
                item("renamed.doc", b"same"),
                item("c.doc", b"new"),
                item("extra.doc", b"extra"),
            ],
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.status == "same_path_and_hash")
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.status == "same_hash_different_path")
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.status == "same_path_changed_bytes")
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.status == "missing_from_current")
                .count(),
            1
        );
        assert_eq!(
            rows.iter().filter(|r| r.status == "current_only").count(),
            1
        );
        let rows = matches(
            vec![item("a", b"one"), item("b", b"one")],
            vec![item("a", b"one")],
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.status == "missing_from_current")
                .count(),
            1
        );
    }
    #[test]
    fn paths_categories_and_bounds_are_explicit() {
        for value in ["../bad", "/root", "x/../bad", "x:ads", "x//y", "x\n.csv"] {
            assert!(safe_relative(value).is_err());
        }
        assert_eq!(category("filelist.htm", 400), "recovery_tool_report");
        assert_eq!(category("mydocs/filelist.htm", 400), "payload");
        assert_eq!(
            category("System Volume Information/file", 400),
            "os_metadata"
        );
        assert_eq!(category("~WRD.tmp", 400), "temporary");
        assert_eq!(category("empty", 0), "empty");
        assert!(hash_reader(&mut &b"1234"[..], 3).is_err());
        assert!(workstation(Path::new("A:\\reference.zip")).is_err());
    }

    #[test]
    fn dmde_utf16_deleted_live_and_carved_are_not_conflated() {
        let text = "2000-07-20 02:56:18.000 4 ----- ---A . $Noname 01\\$Root\\live file.doc\r\n2000-01-27 13:59:08.000 5 ----- ---A xf lost.jpg\r\n-- -- <DIR> ----- ---- x directory\r\n-- -- 6 ----- ---- +.+ [$Raw Files by Signatures]\\f1.doc\r\n";
        let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let parsed = dmde_filelist(&encoded).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed["$noname 01/$root/live file.doc"], (4, "live"));
        assert_eq!(parsed["lost.jpg"], (5, "deleted"));
        assert_eq!(
            parsed["[$raw files by signatures]/f1.doc"],
            (6, "signature_carved")
        );
        assert_eq!(
            dmde_filelist(format!("{text}{text}").as_bytes()).unwrap()["lost.jpg"],
            (0, "ambiguous")
        );
        assert!(dmde_filelist(b"-- -- 5 ----- ---- x ../bad").is_err());
    }

    #[test]
    fn excluded_exact_match_cannot_steal_an_in_scope_renamed_file() {
        let mut deleted = item("deleted.doc", b"same");
        deleted.category = "deleted_out_of_scope";
        let rows = matches(
            vec![deleted, item("live.doc", b"same")],
            vec![item("deleted.doc", b"same")],
        );
        assert!(rows.iter().any(|r| r.status == "same_hash_different_path"
            && r.reference.as_ref().unwrap().path == "live.doc"));
        assert!(rows.iter().any(|r| r.status == "missing_from_current"
            && r.reference.as_ref().unwrap().category == "deleted_out_of_scope"));
    }
}
