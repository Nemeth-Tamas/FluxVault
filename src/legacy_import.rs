//! Copy a script ZIP into a fresh, atomically published workstation project.
//! Legacy bytes/logs stay legacy evidence; never synthesize acquisition maps.
use crate::{project::ProjectState, safety};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_ARCHIVE: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 4 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const REPORT: &str = "Reports/LegacyImport.json";
const INVENTORY: &str = "Reports/LegacyImportFiles.csv";
const SUMMARY: &str = "Reports/LegacyImport.txt";

#[derive(Clone, Debug, Serialize)]
struct Member {
    index: usize,
    archive_path: String,
    path: String,
    bytes: u64,
    directory: bool,
}
#[derive(Serialize)]
struct ImportedFile {
    path: String,
    bytes: u64,
    sha256: String,
}
struct Prepared {
    source: PathBuf,
    destination: PathBuf,
    archive_sha256: String,
    members: Vec<Member>,
    images: BTreeMap<u32, String>,
    index_rows: Vec<BTreeMap<String, String>>,
    bytes: u64,
}

fn regular(path: &Path, directory: bool) -> Result<(), String> {
    safety::workstation_path(path)?;
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if m.file_type().is_symlink() || if directory { !m.is_dir() } else { !m.is_file() } {
        return Err("Import paths must be regular workstation files/directories".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Linked import path refused".into());
        }
    }
    safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)
}

fn safe_path(name: &str, directory: bool) -> Result<String, String> {
    let normalized = name.replace('\\', "/");
    let normalized = if directory {
        normalized.strip_suffix('/').unwrap_or(&normalized)
    } else {
        &normalized
    };
    let parts: Vec<_> = normalized.split('/').collect();
    if normalized.len() > 1024
        || parts.len() > 20
        || parts.iter().any(|part| {
            let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.len() > 255
                || part.ends_with(['.', ' '])
                || part
                    .chars()
                    .any(|c| c.is_control() || "<>:\"|?*".contains(c))
                || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
                || base
                    .strip_prefix("COM")
                    .or_else(|| base.strip_prefix("LPT"))
                    .is_some_and(|n| {
                        matches!(
                            n,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                || part.to_ascii_lowercase().starts_with(".fluxvault-")
                || part.starts_with("__")
        })
    {
        return Err(format!("Unsafe/reserved Windows archive path: {name}"));
    }
    let first = parts[0];
    let top = [
        "Images",
        "Logs",
        "Extracted",
        "Converted",
        "Recovery",
        "Reports",
    ]
    .into_iter()
    .find(|p| p.eq_ignore_ascii_case(first));
    let path = if let Some(top) = top {
        format!("{top}{}", &normalized[first.len()..])
    } else if !directory && parts.len() == 1 && first.eq_ignore_ascii_case("README.txt") {
        "README.txt".into()
    } else {
        return Err(format!("Not a supported script archive root: {name}"));
    };
    if [REPORT, INVENTORY, SUMMARY]
        .iter()
        .any(|reserved| path.eq_ignore_ascii_case(reserved))
    {
        return Err("Archive contains reserved import reports".into());
    }
    if top.is_some() && parts.len() == 1 && !directory {
        return Err("Archive folder name is a file".into());
    }
    Ok(path)
}

fn hash(
    reader: &mut impl Read,
    bound: u64,
    mut output: Option<&mut File>,
) -> Result<(u64, String), String> {
    let mut bytes = 0u64;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        crate::cancellation::check()?;
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        bytes = bytes
            .checked_add(n as u64)
            .ok_or("Import byte count overflow")?;
        if bytes > bound {
            return Err("Import exceeds bounded member/archive size".into());
        }
        digest.update(&buffer[..n]);
        if let Some(file) = &mut output {
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        }
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}
fn hash_file(path: &Path, bound: u64) -> Result<(u64, String), String> {
    regular(path, false)?;
    hash(
        &mut File::open(path).map_err(|e| e.to_string())?,
        bound,
        None,
    )
}

fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
        Ok(_) => Err("Import destination already exists; nothing will be overwritten".into()),
    }
}

fn destination_owner(target: &Path) -> Result<File, String> {
    let identity = target.to_string_lossy().to_lowercase();
    let name = format!(
        ".fluxvault-import-target-{:x}.lock",
        Sha256::digest(identity.as_bytes())
    );
    let path = target
        .parent()
        .ok_or("Missing destination parent")?
        .join(name);
    match fs::symlink_metadata(&path) {
        Ok(_) => regular(&path, false)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    regular(&path, false)?;
    file.try_lock()
        .map_err(|_| "Another import owns this destination; nothing overwritten".to_owned())?;
    absent(target)?;
    Ok(file)
}

fn destination(cwd: &Path, requested: &Path) -> Result<PathBuf, String> {
    safety::workstation_path(requested)?;
    let requested = cwd.join(requested);
    safety::workstation_path(&requested)?;
    let parent = requested
        .parent()
        .ok_or("Import destination needs an existing parent")?;
    regular(parent, true)?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    let name = requested
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Import destination needs a folder name")?;
    // Apply the same filename rules without requiring an archive root.
    safe_path(&format!("Recovery/{name}"), true)?;
    if name.starts_with('.') {
        return Err("Choose a visible fresh project folder name".into());
    }
    for ancestor in parent.ancestors() {
        if ancestor
            .join("project.json")
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err("Import into a fresh folder outside existing FluxVault projects".into());
        }
    }
    let target = parent.join(name);
    absent(&target)?;
    Ok(target)
}

fn prepare(cwd: &Path, source: &Path, target: &Path) -> Result<Prepared, String> {
    safety::workstation_path(cwd)?;
    safety::workstation_path(source)?;
    let source = cwd.join(source);
    regular(&source, false)?;
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let destination = destination(cwd, target)?;
    let (_, archive_sha256) = hash_file(&source, MAX_ARCHIVE)?;
    let mut archive = zip::ZipArchive::new(File::open(&source).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("Archive has too many members".into());
    }
    let mut members = Vec::new();
    let mut names = BTreeMap::new();
    let mut images = BTreeMap::new();
    let mut bytes = 0u64;
    let mut index_rows = Vec::new();
    for i in 0..archive.len() {
        crate::cancellation::check()?;
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        if file
            .unix_mode()
            .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err("Archive symlink/device entry refused".into());
        }
        let directory = file.is_dir();
        if directory && file.size() != 0 {
            return Err("Archive directory contains unexpected payload".into());
        }
        let path = safe_path(file.name(), directory)?;
        if names.insert(path.to_lowercase(), directory).is_some() {
            return Err("Duplicate/case-colliding archive member".into());
        }
        if file.size() > MAX_FILE {
            return Err(format!("Oversized archive member: {path}"));
        }
        if path.starts_with("Logs/") && file.size() > 8 * 1024 * 1024 {
            return Err("Legacy log exceeds 8 MiB inspection bound".into());
        }
        if !directory {
            bytes = bytes
                .checked_add(file.size())
                .ok_or("Import size overflow")?;
            if bytes > MAX_TOTAL {
                return Err("Unpacked archive exceeds 4 GiB bound".into());
            }
            if let Some(name) = path.strip_prefix("Images/") {
                if name.contains('/') {
                    return Err("Script images must be flat numbered files".into());
                }
                let p = Path::new(name);
                if !p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    matches!(e.to_ascii_lowercase().as_str(), "bin" | "img" | "ima")
                }) {
                    return Err("Script import accepts bare .bin/.img/.ima images, not native metadata or executables".into());
                }
                let stem = p.file_stem().and_then(|e| e.to_str()).unwrap_or("");
                if !stem.contains(".partial") {
                    let n = stem
                        .parse::<u32>()
                        .ok()
                        .filter(|n| *n > 0 && *n < 1_000_000)
                        .ok_or("Invalid legacy image label")?;
                    if stem != format!("{n:03}")
                        || file.size() == 0
                        || file.size() > crate::fat12::MAX_IMAGE_BYTES as u64
                        || images.insert(n, path.clone()).is_some()
                    {
                        return Err("Duplicate/noncanonical/oversized legacy image".into());
                    }
                }
            }
            if path == "Reports/ArchiveIndex.csv" {
                if file.size() > 1024 * 1024 {
                    return Err("Oversized legacy archive index".into());
                }
                let mut content = String::new();
                file.by_ref()
                    .take(1024 * 1024 + 1)
                    .read_to_string(&mut content)
                    .map_err(|e| e.to_string())?;
                let rows = crate::audit::parse_csv(content.trim_start_matches('\u{feff}'))?;
                if rows.len() > 16385 {
                    return Err("Too many legacy archive-index rows".into());
                }
                let header = rows.first().ok_or("Empty legacy archive index")?;
                if header.len() > 64
                    || !header.iter().any(|h| h == "FloppyNumber")
                    || header.iter().collect::<BTreeSet<_>>().len() != header.len()
                {
                    return Err("Invalid legacy archive index columns".into());
                }
                for row in rows
                    .iter()
                    .skip(1)
                    .filter(|row| row.iter().any(|v| !v.is_empty()))
                {
                    if row.len() != header.len() {
                        return Err("Invalid legacy archive index row width".into());
                    }
                    index_rows.push(header.iter().cloned().zip(row.iter().cloned()).collect());
                }
            }
        }
        members.push(Member {
            index: i,
            archive_path: file.name().into(),
            path,
            bytes: file.size(),
            directory,
        });
    }
    for name in names.keys() {
        let mut parent = Path::new(name).parent();
        while let Some(p) = parent {
            if names
                .get(&p.to_string_lossy().replace('\\', "/"))
                .is_some_and(|dir| !*dir)
            {
                return Err("Archive file/directory prefix collision".into());
            }
            parent = p.parent();
        }
    }
    if images.is_empty() || images.len() > 4096 {
        return Err("Script import requires 1..4096 numbered complete images".into());
    }
    if hash_file(&source, MAX_ARCHIVE)?.1 != archive_sha256 {
        return Err("Source archive changed during planning".into());
    }
    Ok(Prepared {
        source,
        destination,
        archive_sha256,
        members,
        images,
        index_rows,
        bytes,
    })
}

fn planned(p: &Prepared) -> Value {
    json!({"schema":1,"source_archive":p.source,"source_sha256":p.archive_sha256,
    "destination":p.destination,"files":p.members.iter().filter(|m|!m.directory).count(),"source_bytes":p.bytes,
    "disk_labels":p.images.keys().collect::<Vec<_>>(),"images":p.images.len(),"archive_index_rows":p.index_rows.len(),
    "next_disk":p.images.last_key_value().map(|(n,_)|n+1),"physical_media_access":false,
    "customer_delivery_certified":false,"legacy_files_are_not_managed_recovery":true})
}

pub(crate) fn run(
    cwd: &Path,
    source: &Path,
    target: &Path,
    plan: bool,
    progress: &impl Fn(&str),
) -> Result<Value, String> {
    progress("IMPORT / inspecting a saved script ZIP; no drives or external tools are used");
    let p = prepare(cwd, source, target)?;
    if plan {
        let mut value = planned(&p);
        value["plan_only"] = json!(true);
        value["member_payloads_verified"] = json!(false);
        return Ok(value);
    }
    crate::cancellation::check()?;
    let _destination_owner = destination_owner(&p.destination)?;
    let parent = p.destination.parent().ok_or("Missing destination parent")?;
    let stage = parent.join(format!(
        ".fluxvault-import-{}-{}.partial",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    regular(&stage, true)?;
    let inner = stage.join(p.destination.file_name().ok_or("Missing project name")?);
    let mut project = ProjectState::create_without_session(inner.clone())?;
    let owner = crate::project_work::reserve(project.root())?;
    let control = crate::run_control::Session::start(&project, "legacy_import")?;
    progress(&format!(
        "IMPORT / staging: {} / publish only after every member and archive hash pass",
        stage.display()
    ));
    let result = copy(&p, &mut project, progress);
    drop(control);
    drop(owner);
    let result = result.and_then(|result| {
        crate::cancellation::check()?;
        absent(&p.destination)?;
        regular(&inner, true)?;
        // This generated control record belongs to the staging identity, not
        // the published project. Keep it outside the project as import history.
        fs::rename(
            inner.join(".fluxvault-run-control.json"),
            stage.join("IMPORT-RUN-CONTROL.json"),
        )
        .map_err(|e| e.to_string())?;
        // Windows directory rename refuses an existing destination. Never merge.
        fs::rename(&inner, &p.destination).map_err(|e| {
            format!(
                "Cannot publish import; staging retained at {}: {e}",
                stage.display()
            )
        })?;
        Ok(result)
    });
    if let Err(error) = &result {
        let receipt = json!({"schema":1,"destination":p.destination,"source_archive":p.source,"source_sha256":p.archive_sha256,
            "phase":if crate::cancellation::requested(){"interrupted"}else{"failed"},"error":error.chars().take(4096).collect::<String>(),
            "published":false,"physical_media_access":false,"customer_delivery_certified":false});
        if let Err(e) = write_new(
            &stage.join("IMPORT-INCOMPLETE.json"),
            &serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
        ) {
            eprintln!("Could not save incomplete import receipt: {e}");
        }
        // Partial evidence is retained, never recursively deleted by import.
    }
    let mut result = result?;
    result["published"] = json!(true);
    result["project"] = json!(p.destination);
    result["report"] = json!(p.destination.join(REPORT));
    result["inventory"] = json!(p.destination.join(INVENTORY));
    result["summary"] = json!(p.destination.join(SUMMARY));
    result["staging_container"] = json!(stage);
    Ok(result)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
fn copy(
    p: &Prepared,
    project: &mut ProjectState,
    progress: &impl Fn(&str),
) -> Result<Value, String> {
    let mut archive = zip::ZipArchive::new(File::open(&p.source).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for (number, m) in p.members.iter().enumerate() {
        crate::cancellation::check()?;
        let path = project.root().join(&m.path);
        if m.directory {
            fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            regular(&path, true)?;
            continue;
        }
        fs::create_dir_all(path.parent().ok_or("Import member lacks parent")?)
            .map_err(|e| e.to_string())?;
        regular(path.parent().unwrap(), true)?;
        if !path.starts_with(project.root()) {
            return Err("Import member escapes staging".into());
        }
        if number % 50 == 0 {
            progress(&format!(
                "IMPORT / member {} of {}",
                number + 1,
                p.members.len()
            ));
        }
        let mut file = archive.by_index(m.index).map_err(|e| e.to_string())?;
        if file.name() != m.archive_path || file.size() != m.bytes || file.is_dir() {
            return Err("Source ZIP metadata changed".into());
        }
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let (bytes, sha256) = hash(&mut file, m.bytes, Some(&mut output))?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        if bytes != m.bytes || hash_file(&path, MAX_FILE)? != (bytes, sha256.clone()) {
            return Err("Imported member byte/hash verification failed".into());
        }
        files.push(ImportedFile {
            path: m.path.clone(),
            bytes,
            sha256,
        });
    }
    drop(archive);
    progress("IMPORT / reconciling legacy logs, recovered folders and archive-index claims");
    let statistics = crate::imaging::load_project_statistics(&project.images_dir())?;
    if statistics.disk_count != p.images.len() {
        return Err("Imported legacy disk discovery differs from archive labels".into());
    }
    let hashes: BTreeMap<_, _> = files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut disks = Vec::new();
    let mut discrepancies = 0usize;
    for (disk, image) in &p.images {
        let attempt = crate::imaging::load_attempts_for_disk(&project.images_dir(), *disk)?
            .into_iter()
            .find(|a| a.legacy_image)
            .ok_or("Imported legacy attempt missing")?;
        let f = hashes[image.as_str()];
        let log_hash_matches = attempt
            .parsed_log
            .as_ref()
            .and_then(|l| l.sha256.as_ref())
            .map(|h| h.eq_ignore_ascii_case(&f.sha256));
        let mut claims = Vec::new();
        for row in p
            .index_rows
            .iter()
            .filter(|r| r.get("FloppyNumber").and_then(|v| v.parse::<u32>().ok()) == Some(*disk))
        {
            let reported = row.get("SHA256").filter(|s| !s.is_empty());
            let matches = reported.map(|h| h.eq_ignore_ascii_case(&f.sha256));
            if matches == Some(false) {
                discrepancies += 1;
            }
            claims.push(json!({"row":row,"image_sha256_matches":matches,"historical_claim_not_new_measurement":true}));
        }
        if log_hash_matches == Some(false) {
            discrepancies += 1;
        }
        let files_for = |top: &str| {
            let prefix = format!("{top}/{disk:03}/");
            files.iter().filter(|f| f.path.starts_with(&prefix)).count()
        };
        let sector_size = attempt
            .parsed_log
            .as_ref()
            .and_then(|l| l.geometry.bytes_per_sector)
            .or_else(|| attempt.parsed_dmde_log.as_ref().and_then(|l| l.sector_size));
        let extent =
            sector_size.and_then(|size| (size as u64).checked_mul(attempt.total_sectors as u64));
        let extent_matches = sector_size.map(|_| extent == Some(f.bytes));
        if extent_matches == Some(false) {
            discrepancies += 1;
        }
        disks.push(json!({"disk":disk,"image":image,"image_bytes":f.bytes,"image_sha256":f.sha256,
            "legacy_status":attempt.status,"attention_required":attempt.attention_required||log_hash_matches==Some(false)||extent_matches==Some(false)||claims.iter().any(|c|c["image_sha256_matches"]==false),"reported_total_sectors":attempt.total_sectors,
            "reported_bad_sectors":attempt.bad_sectors,"reported_extent_matches_saved_image":extent_matches,"recorded_log_sha256_matches":log_hash_matches,
            "log":Path::new(&attempt.log_file).strip_prefix(project.root()).ok().map(|p|p.to_string_lossy().replace('\\',"/")),
            "log_kind":if attempt.parsed_log.is_some(){"floppy_archiver"}else if attempt.parsed_dmde_log.is_some(){"dmde"}else{"unknown_or_missing"},
            "dmde_passes":attempt.parsed_dmde_log.as_ref().map(|l|json!({"total":l.pass_count,"forward":l.forward_passes,"reverse":l.reverse_passes})),
            "archive_index_claims":claims,"legacy_extracted_files":files_for("Extracted"),"legacy_converted_files":files_for("Converted"),
            "legacy_recovery_files":files_for("Recovery"),"legacy_payload_integrity":"copied bytes verified against ZIP; recovery/Office lineage not newly certified"}));
    }
    let index_only: Vec<_> = p
        .index_rows
        .iter()
        .filter(|r| {
            r.get("FloppyNumber")
                .and_then(|v| v.parse::<u32>().ok())
                .is_none_or(|n| !p.images.contains_key(&n))
        })
        .collect();
    project.set_current_disk_number_without_session(
        p.images
            .last_key_value()
            .ok_or("Missing imported labels")?
            .0
            + 1,
    )?;
    let mut result = planned(p);
    result["plan_only"] = json!(false);
    result["member_payloads_verified"] = json!(true);
    result["legacy_status_counts"] = json!(disks.iter().fold(
        BTreeMap::<String, usize>::new(),
        |mut a, d| {
            *a.entry(d["legacy_status"].as_str().unwrap_or("UNKNOWN").into())
                .or_default() += 1;
            a
        }
    ));
    result["historical_claim_discrepancies"] = json!(discrepancies);
    result["archive_index_status_counts"] = json!(p.index_rows.iter().fold(
        BTreeMap::<String, usize>::new(),
        |mut counts, row| {
            *counts
                .entry(
                    row.get("Status")
                        .cloned()
                        .unwrap_or_else(|| "UNKNOWN".into()),
                )
                .or_default() += 1;
            counts
        }
    ));
    result["attention"] = json!(true);
    let mut summary = format!(
        "FluxVault - script archive import\r\nSource: {}\r\nSource SHA-256: {}\r\n{} images; {} original files; {} original bytes. Next label: {}.\r\n\r\nLEGACY EVIDENCE: copied bytes verified, old recovery/conversion claims not newly certified.\r\nFolder counts include auxiliary files; they are not measured recovered-file yield.\r\nOriginal CSV/logs remain unchanged. Unknown fields are not inferred.\r\n\r\nDisk | Old status | Image bytes | Reported bad | Extracted folder files | Converted folder files | Claim attention\r\n",
        p.source.display(),
        p.archive_sha256,
        p.images.len(),
        files.len(),
        p.bytes,
        result["next_disk"]
    );
    for disk in &disks {
        summary.push_str(&format!(
            "{:03} | {} | {} | {} | {} | {} | {}\r\n",
            disk["disk"].as_u64().unwrap_or(0),
            disk["legacy_status"].as_str().unwrap_or("UNKNOWN"),
            disk["image_bytes"],
            disk["reported_bad_sectors"].as_array().map_or(0, Vec::len),
            disk["legacy_extracted_files"],
            disk["legacy_converted_files"],
            if disk["attention_required"] == true {
                "YES"
            } else {
                "no imaging-claim issue; still legacy"
            }
        ));
    }
    summary.push_str(&format!("\r\nHistorical hash/extent discrepancies: {discrepancies}.\r\nDetails: LegacyImport.json. Per-file hashes: LegacyImportFiles.csv.\r\nNo physical floppy, external tool or newly managed recovery was used.\r\n"));
    write_new(&project.root().join(SUMMARY), summary.as_bytes())?;
    let mut report = result.clone();
    report["disks"] = json!(disks);
    report["files"] = json!(files);
    report["archive_index_rows_without_images"] = json!(index_only);
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("Import report exceeds bound".into());
    }
    write_new(&project.root().join(REPORT), &bytes)?;
    let escape = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut csv = String::from("Path,Bytes,SHA256\r\n");
    for f in &files {
        let displayed = if f.path.starts_with(['=', '+', '-', '@']) {
            format!("'{}", f.path)
        } else {
            f.path.clone()
        };
        csv.push_str(&format!(
            "{},{},{}\r\n",
            escape(&displayed),
            f.bytes,
            f.sha256
        ));
    }
    write_new(&project.root().join(INVENTORY), csv.as_bytes())?;
    progress("IMPORT / independently rechecking every copied member before publication");
    for f in &files {
        if hash_file(&project.root().join(&f.path), MAX_FILE)? != (f.bytes, f.sha256.clone()) {
            return Err("Staged member changed; project publication refused".into());
        }
    }
    progress("IMPORT / final source-archive hash check before project publication");
    if hash_file(&p.source, MAX_ARCHIVE)?.1 != p.archive_sha256 {
        return Err("Source ZIP changed; project publication refused, staging retained".into());
    }
    Ok(result)
}

#[cfg(test)]
#[path = "legacy_import_tests.rs"]
mod tests;
