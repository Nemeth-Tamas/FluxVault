//! Immutable, verified customer archive creation from workstation artifacts.
//! This module never opens a physical drive or writes inside the project.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use chrono::{DateTime, Local, Utc};
use sha2::{Digest, Sha256};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const INCLUDED_DIRECTORIES: &[&str] = &[
    "Images",
    "Logs",
    "Extracted",
    "Converted",
    "Recovery",
    "Reports",
    "Flux",
];

#[derive(Debug, Clone)]
pub struct PackageRequest {
    pub project_root: PathBuf,
    pub destination: PathBuf,
    pub project_name: String,
}

#[derive(Debug, Clone)]
pub enum PackageEvent {
    Stage(String),
    Finished(Result<PackageResult, String>),
}

#[derive(Debug, Clone)]
pub struct PackageResult {
    pub zip_path: PathBuf,
    pub sha256_path: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
    pub sha256: String,
}

#[derive(Debug)]
struct PackageFile {
    source: PathBuf,
    archive_path: String,
}

#[derive(Debug)]
struct ManifestRow {
    archive_path: String,
    bytes: u64,
    modified_utc: String,
    sha256: String,
}

pub fn spawn_package(request: PackageRequest) -> Receiver<PackageEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = build_package(&request, &|stage| {
            let _ = sender.send(PackageEvent::Stage(stage.to_owned()));
        });
        let _ = sender.send(PackageEvent::Finished(result));
    });
    receiver
}

pub(crate) fn build_package(
    request: &PackageRequest,
    stage: &impl Fn(&str),
) -> Result<PackageResult, String> {
    let project = request.project_root.canonicalize().map_err(|error| {
        format!(
            "Project directory cannot be resolved {}: {error}",
            request.project_root.display()
        )
    })?;
    if !project.join("project.json").is_file() {
        return Err("The selected source is not a FluxVault project.".to_owned());
    }
    // Replays catalogued offline derivations before exporting their products.
    crate::imaging::load_project_statistics(&project.join("Images"))?;
    // Packed flux is evidence, not an unrelated nested ZIP. Verify its logical
    // original identity before including the container and binding sidecar.
    let flux = project.join("Flux");
    if flux.is_dir() {
        for entry in fs::read_dir(&flux).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".scp.packed.json") {
                let state = crate::project::ProjectState::open_without_session(project.clone())?;
                let (disk, attempt) = stem
                    .split_once("_attempt_")
                    .ok_or("Invalid packed capture filename")?;
                crate::flux_archive::verify_packed(
                    &state,
                    disk.parse().map_err(|_| "Invalid packed disk")?,
                    attempt.parse().map_err(|_| "Invalid packed attempt")?,
                )?;
            }
        }
    }
    if !request.destination.is_dir() {
        return Err(format!(
            "Destination directory does not exist: {}",
            request.destination.display()
        ));
    }
    let destination = request.destination.canonicalize().map_err(|error| {
        format!(
            "Destination directory cannot be resolved {}: {error}",
            request.destination.display()
        )
    })?;
    if destination.starts_with(&project) {
        return Err("Package destination must be outside the project tree.".to_owned());
    }

    stage("Archival files are being inventoried...");
    let files = collect_project_files(&project)?;
    if files.is_empty() {
        return Err("No archival files are available to package.".to_owned());
    }
    let safe_name = safe_file_name(&request.project_name);
    let stem = format!("{safe_name}_{}", Local::now().format("%Y%m%d_%H%M%S_%3f"));
    let zip_path = destination.join(format!("{stem}.zip"));
    let partial_path = destination.join(format!("{stem}.partial.zip"));
    let sha256_path = destination.join(format!("{stem}.zip.sha256"));
    if zip_path.exists() || sha256_path.exists() {
        return Err(format!(
            "A package with this timestamp already exists in {}",
            destination.display()
        ));
    }

    let partial_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial_path)
        .map_err(|error| format!("Cannot create package {}: {error}", partial_path.display()))?;
    let result = write_and_verify(
        partial_file,
        &partial_path,
        &files,
        &request.project_name,
        stage,
    );
    let (file_count, total_bytes) = match result {
        Ok(value) => value,
        Err(error) => {
            // Keep the uniquely named partial for diagnosis rather than silently deleting evidence.
            return Err(format!(
                "{error} Partial package retained at {}",
                partial_path.display()
            ));
        }
    };

    let sha256 = hash_file(&partial_path)?;
    fs::rename(&partial_path, &zip_path).map_err(|error| {
        format!(
            "Verified ZIP could not be promoted to {}: {error}",
            zip_path.display()
        )
    })?;
    let hash_line = format!(
        "{sha256}  {}\n",
        zip_path.file_name().unwrap().to_string_lossy()
    );
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sha256_path)
        .and_then(|mut file| file.write_all(hash_line.as_bytes()))
        .map_err(|error| {
            format!("ZIP is complete, but its SHA-256 sidecar could not be written: {error}")
        })?;
    Ok(PackageResult {
        zip_path,
        sha256_path,
        file_count,
        total_bytes,
        sha256,
    })
}

fn collect_project_files(project: &Path) -> Result<Vec<PackageFile>, String> {
    let mut files = Vec::new();
    let latest_workbook = latest_workbook_name(&project.join("Reports"))?;
    for directory in INCLUDED_DIRECTORIES {
        let root = project.join(directory);
        if !root.exists() {
            continue;
        }
        if is_reparse_point(&root)? {
            return Err(format!(
                "Package refuses a linked project folder: {}",
                root.display()
            ));
        }
        let mut pending = vec![root];
        while let Some(current) = pending.pop() {
            for entry in fs::read_dir(&current)
                .map_err(|error| format!("Cannot read {}: {error}", current.display()))?
            {
                let entry = entry.map_err(|error| format!("Invalid directory entry: {error}"))?;
                let path = entry.path();
                let kind = entry
                    .file_type()
                    .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
                if kind.is_symlink() || is_reparse_point(&path)? {
                    return Err(format!(
                        "Package refuses symbolic links/reparse points: {}",
                        path.display()
                    ));
                }
                if should_exclude(&path)
                    || is_delivery_quarantine(
                        path.strip_prefix(project).map_err(|e| e.to_string())?,
                    )
                {
                    continue;
                }
                if *directory == "Reports" && kind.is_dir() {
                    continue;
                }
                if *directory == "Reports" && kind.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !is_customer_report(&name, latest_workbook.as_deref()) {
                        continue;
                    }
                }
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() {
                    let relative = path
                        .strip_prefix(project)
                        .map_err(|error| error.to_string())?;
                    let archive_path = relative
                        .components()
                        .map(|component| match component {
                            Component::Normal(name) => Ok(name.to_string_lossy().to_string()),
                            _ => Err(format!("Unsafe package path: {}", relative.display())),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .join("/");
                    files.push(PackageFile {
                        source: path,
                        archive_path,
                    });
                }
            }
        }
    }
    files.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));
    Ok(files)
}

fn latest_workbook_name(reports: &Path) -> Result<Option<String>, String> {
    if !reports.is_dir() {
        return Ok(None);
    }
    let mut names = fs::read_dir(reports)
        .map_err(|error| format!("Cannot read Reports folder {}: {error}", reports.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (entry.path().is_file()
                && name.starts_with("FluxVault_Jelentes_")
                && name.to_ascii_lowercase().ends_with(".xlsx"))
            .then_some(name)
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names.pop())
}

fn is_customer_report(name: &str, latest_workbook: Option<&str>) -> bool {
    let normalized = name.to_ascii_lowercase();
    const EXACT: &[&str] = &[
        "archiveindex.csv",
        "extractionsummary.csv",
        "masterfilelist.csv",
        "conversionsummary.csv",
        "conversionfailures.txt",
        "deliverypathmap.csv",
        "deliverymanifest.csv",
        "deliverymanifest.sha256",
        "floppyfinalaudit.csv",
        "floppyfinalaudit.txt",
        "integrityvalidation.csv",
        "evidenceaudit.csv",
        "evidenceaudit.json",
        "offlinerecoverydecisions.json",
        "recoveryexceptions.txt",
    ];
    EXACT.contains(&normalized.as_str())
        || (normalized.starts_with("recoveryselection-") && normalized.ends_with(".json"))
        || (normalized.starts_with("deliverycleanup-") && normalized.ends_with(".json"))
        || (normalized.starts_with("offlinederived-") && normalized.ends_with(".json"))
        || (normalized.starts_with("finalaudit")
            && (normalized.ends_with(".csv") || normalized.ends_with(".txt")))
        || latest_workbook.is_some_and(|latest| latest == name)
}

fn is_delivery_quarantine(relative: &Path) -> bool {
    let mut parts = relative.iter();
    parts
        .next()
        .is_some_and(|n| n.eq_ignore_ascii_case("Recovery"))
        && parts
            .next()
            .is_some_and(|n| n.eq_ignore_ascii_case("DeliveryQuarantine"))
}

fn should_exclude(path: &Path) -> bool {
    if path.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            name.eq_ignore_ascii_case("System Volume Information")
                || name.eq_ignore_ascii_case("$RECYCLE.BIN")
                || name.starts_with(".tmp-fragments-")
                || name.starts_with(".tmp-native-")
        }
        _ => false,
    }) {
        return true;
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    // Recovery control state is internal; keep the sector-provenance evidence.
    let in_flux_recovery = path.parent().is_some_and(|parent| {
        parent
            .file_name()
            .is_some_and(|part| part.eq_ignore_ascii_case("Recovery"))
            && parent.parent().is_some_and(|flux| {
                flux.file_name()
                    .is_some_and(|part| part.eq_ignore_ascii_case("Flux"))
            })
    });
    if in_flux_recovery
        && name
            .strip_suffix(".lock")
            .or_else(|| name.strip_suffix("_job.json"))
            .is_some_and(|disk| !disk.is_empty() && disk.bytes().all(|b| b.is_ascii_digit()))
    {
        return true;
    }
    let managed_capture_zip = name
        .strip_suffix(".scp.zip")
        .and_then(|stem| stem.split_once("_attempt_"))
        .is_some_and(|(disk, attempt)| {
            disk.parse::<u32>().is_ok_and(|n| n > 0) && attempt.parse::<u32>().is_ok_and(|n| n > 0)
        })
        && path.parent().is_some_and(|p| {
            p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("Flux"))
        })
        && path.with_extension("packed.json").is_file();
    name.starts_with(".fluxvault-")
        || name.starts_with("__")
        || name.contains(".partial.")
        || name.ends_with(".tmp")
        || (name.ends_with(".zip") && !managed_capture_zip)
        || name.ends_with(".zip.sha256")
        || name == "external-tools.jsonl"
}

#[cfg(windows)]
fn is_reparse_point(path: &Path) -> Result<bool, String> {
    use std::os::windows::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
    Ok(metadata.file_attributes() & 0x400 != 0)
}

#[cfg(not(windows))]
fn is_reparse_point(_path: &Path) -> Result<bool, String> {
    Ok(false)
}

fn write_and_verify(
    output: File,
    partial_path: &Path,
    files: &[PackageFile],
    project_name: &str,
    stage: &impl Fn(&str),
) -> Result<(usize, u64), String> {
    let mut zip = ZipWriter::new(output);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));
    let mut rows = Vec::with_capacity(files.len());
    for (index, item) in files.iter().enumerate() {
        stage(&format!(
            "Packaging file {} of {}: {}",
            index + 1,
            files.len(),
            item.archive_path
        ));
        zip.start_file(&item.archive_path, options)
            .map_err(|error| error.to_string())?;
        let mut source = File::open(&item.source)
            .map_err(|error| format!("Cannot open {}: {error}", item.source.display()))?;
        let modified_utc = source
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .map(DateTime::<Utc>::from)
            .map(|time| time.to_rfc3339())
            .unwrap_or_default();
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            zip.write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
            digest.update(&buffer[..count]);
            bytes += count as u64;
        }
        rows.push(ManifestRow {
            archive_path: item.archive_path.clone(),
            bytes,
            modified_utc,
            sha256: format!("{:x}", digest.finalize()),
        });
    }
    let total_bytes = rows.iter().map(|row| row.bytes).sum();
    let manifest = manifest_text(&rows);
    let readme = readme_text(project_name, files.len(), total_bytes);
    zip.start_file("PACKAGE_MANIFEST.csv", options)
        .map_err(|error| error.to_string())?;
    zip.write_all(manifest.as_bytes())
        .map_err(|error| error.to_string())?;
    zip.start_file("PACKAGE_MANIFEST.sha256", options)
        .map_err(|error| error.to_string())?;
    zip.write_all(
        format!(
            "{:x}  PACKAGE_MANIFEST.csv\n",
            Sha256::digest(manifest.as_bytes())
        )
        .as_bytes(),
    )
    .map_err(|error| error.to_string())?;
    zip.start_file("README.txt", options)
        .map_err(|error| error.to_string())?;
    zip.write_all(readme.as_bytes())
        .map_err(|error| error.to_string())?;
    zip.finish().map_err(|error| error.to_string())?;

    stage("Verifying every ZIP member against the manifest...");
    let input = File::open(partial_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    if archive.len() != rows.len() + 3 {
        return Err("ZIP member count differs from the staged file list.".to_owned());
    }
    for row in &rows {
        let mut entry = archive
            .by_name(&row.archive_path)
            .map_err(|error| error.to_string())?;
        let mut digest = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = entry.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
            size += count as u64;
        }
        if size != row.bytes || format!("{:x}", digest.finalize()) != row.sha256 {
            return Err(format!("ZIP verification failed: {}", row.archive_path));
        }
    }
    let mut archived_manifest = String::new();
    archive
        .by_name("PACKAGE_MANIFEST.csv")
        .map_err(|error| error.to_string())?
        .read_to_string(&mut archived_manifest)
        .map_err(|error| error.to_string())?;
    if archived_manifest != manifest {
        return Err("ZIP manifest content differs from the generated manifest.".to_owned());
    }
    let mut archived_manifest_hash = String::new();
    archive
        .by_name("PACKAGE_MANIFEST.sha256")
        .map_err(|error| error.to_string())?
        .read_to_string(&mut archived_manifest_hash)
        .map_err(|error| error.to_string())?;
    let expected_manifest_hash = format!(
        "{:x}  PACKAGE_MANIFEST.csv\n",
        Sha256::digest(manifest.as_bytes())
    );
    if archived_manifest_hash != expected_manifest_hash {
        return Err("ZIP manifest SHA-256 sidecar is invalid.".to_owned());
    }
    Ok((rows.len(), total_bytes))
}

fn manifest_text(rows: &[ManifestRow]) -> String {
    let mut text = String::from("\u{feff}\"Path\",\"SizeBytes\",\"ModifiedUTC\",\"SHA256\"\r\n");
    for row in rows {
        text.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",\"{}\"\r\n",
            row.archive_path.replace('"', "\"\""),
            row.bytes,
            row.modified_utc,
            row.sha256
        ));
    }
    text
}

fn readme_text(project_name: &str, file_count: usize, total_bytes: u64) -> String {
    let mut text = format!(
        "FluxVault archival package\r\nProject: {project_name}\r\nFiles: {file_count}\r\nSource bytes: {total_bytes}\r\n\r\nImages: acquired sector images.\r\nLogs: acquisition and recovery logs.\r\nExtracted: recovered source files.\r\nConverted: customer-friendly converted copies.\r\nRecovery: preserved recovery evidence, derived images, and backups.\r\nReports: selected inventories and reports.\r\nFlux: raw flux captures, where available.\r\n\r\nWindows System Volume Information and Recycle Bin folders are excluded from delivery files; original sector images retain all captured bytes.\r\nCheck PACKAGE_MANIFEST.csv for each included file's SHA-256.\r\nA partial image or recovered file is not proof that every original byte was readable.\r\nReview audit and recovery reports for limitations before delivery.\r\n"
    );
    text.push_str("\r\nPacked flux: a managed .scp.zip contains one byte-identical original SCP, not regenerated flux. Its .scp.packed.json binds original/packed sizes and SHA-256. FluxVault decodes it transparently; a ZIP tool can restore the original SCP member.\r\n");
    text.push_str("\r\nOffline DERIVED images: Images may also contain explicitly labeled offline composite/mirrored-FAT attempts. They are saved-sector derivations, NOT new clean physical reads. OfflineDerived reports bind original images/logs/metadata and exact replayable sector copies; originals remain included. Zero remaining gaps do not certify filesystem/customer completeness.\r\n");
    text.push_str("\r\nForensic-only recovery: Recovery may contain raw .bin fragments of incomplete live files and explicitly requested deleted candidates. They are separate evidence, NOT complete/live customer documents. Read their source/offset/hash and missing-range reports; fragment counts do not increase recovered whole-file counts.\r\n");
    text.push_str("\r\nGeneration-5 recovery: Directory-Recovered preserves files from evidenced orphan directory trees; lost root names/parents and historical live/deleted status are unknown. Fragment-Hypotheses contains structurally validated missing-FAT-link alternatives, not proven original allocation. Recovery/*_word_text_v1 holds separate forensic UTF-8 Word text segments and exact missing character-position reports: NOT repaired DOCs, original replacements, complete documents or formatting recovery. These texts never increase whole-file counts or enter routine conversion/delivery. Read the native and word-text reports before using candidate content.\r\n");
    text.push_str("\r\nRecoverySelection reports explain the preferred same-acquisition recovery generation; earlier forensic results remain included. DeliveryCleanup reports audit equivalent obsolete original copies moved into local Recovery/DeliveryQuarantine. Quarantined copies and private ownership journals are excluded from this package; edited/untracked originals and prior Office derivatives are preserved.\r\n");
    text
}

fn safe_file_name(name: &str) -> String {
    let safe = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let safe = safe.trim_matches('_');
    if safe.is_empty() {
        "FluxVault".to_owned()
    } else {
        safe.to_owned()
    }
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut input = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_verified_package_without_internal_files() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-package-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = root.join("project");
        let destination = root.join("delivery");
        fs::create_dir_all(project.join("Images")).unwrap();
        fs::create_dir_all(project.join("Extracted").join("001")).unwrap();
        fs::create_dir_all(project.join("Reports")).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(project.join("project.json"), "{}").unwrap();
        fs::write(project.join("Images").join("001.img"), b"image").unwrap();
        fs::write(project.join("Images").join("001.partial.img"), b"partial").unwrap();
        let flux_recovery = project.join("Flux").join("Recovery");
        fs::create_dir_all(&flux_recovery).unwrap();
        fs::write(flux_recovery.join("001_job.json"), b"internal").unwrap();
        fs::write(flux_recovery.join("001.lock"), b"").unwrap();
        fs::write(flux_recovery.join("001_attempt_001_provenance.json"), b"{}").unwrap();
        fs::create_dir_all(project.join("Logs/Benchmark")).unwrap();
        fs::write(
            project.join("Logs/Benchmark/.fluxvault-benchmark-test.jsonl"),
            b"internal metrics",
        )
        .unwrap();
        fs::create_dir_all(project.join("Reports/Benchmark")).unwrap();
        fs::write(project.join("Reports/Benchmark/Benchmark-test.json"), b"{}").unwrap();
        fs::write(
            project.join("Reports/Benchmark/Benchmark-test.csv"),
            b"internal metrics",
        )
        .unwrap();
        fs::create_dir_all(project.join("Logs/DualBenchmark")).unwrap();
        fs::write(
            project.join("Logs/DualBenchmark/.fluxvault-dual-benchmark-test.jsonl"),
            b"private station timing",
        )
        .unwrap();
        fs::create_dir_all(project.join("Reports/DualBenchmark")).unwrap();
        for name in ["DualBenchmark-test.json", "DualBenchmark-test.csv"] {
            fs::write(project.join("Reports/DualBenchmark").join(name), b"private").unwrap();
        }
        fs::write(
            project.join("Extracted").join("001").join("customer.doc"),
            b"document",
        )
        .unwrap();
        fs::write(
            project
                .join("Extracted")
                .join("001")
                .join(".fluxvault-inventory.json"),
            b"internal",
        )
        .unwrap();
        let windows_metadata = project
            .join("Extracted")
            .join("001")
            .join("System Volume Information");
        fs::create_dir_all(&windows_metadata).unwrap();
        fs::write(windows_metadata.join("IndexerVolumeGuid"), b"OS metadata").unwrap();
        fs::write(project.join("Reports").join("EvidenceAudit.csv"), b"audit").unwrap();
        fs::create_dir_all(project.join("Reports/ConversionHistory")).unwrap();
        fs::write(
            project.join("Reports/ConversionHistory/ConversionState-test.json"),
            b"internal state",
        )
        .unwrap();
        fs::write(
            project
                .join("Reports")
                .join("OfflineRecoveryDecisions.json"),
            b"{}",
        )
        .unwrap();
        fs::write(
            project.join("Reports").join("private-working-note.txt"),
            b"private",
        )
        .unwrap();
        fs::write(
            project
                .join("Reports")
                .join("FluxVault_Jelentes_20260101.xlsx"),
            b"old",
        )
        .unwrap();
        fs::write(
            project
                .join("Reports")
                .join("FluxVault_Jelentes_20260102.xlsx"),
            b"new",
        )
        .unwrap();
        let quarantine = project.join("Recovery/DeliveryQuarantine/fixture/001");
        fs::create_dir_all(&quarantine).unwrap();
        fs::write(quarantine.join("old.doc"), b"obsolete copy").unwrap();
        fs::write(
            project.join("Reports/RecoverySelection-001-test.json"),
            b"{}",
        )
        .unwrap();
        fs::write(project.join("Reports/DeliveryCleanup-test.json"), b"{}").unwrap();
        let result = build_package(
            &PackageRequest {
                project_root: project.clone(),
                destination: destination.clone(),
                project_name: "Test project".to_owned(),
            },
            &|_| {},
        )
        .unwrap();
        assert_eq!(result.file_count, 8);
        assert_eq!(result.total_bytes, 29);
        assert!(result.sha256_path.is_file());
        let mut zip = ZipArchive::new(File::open(result.zip_path).unwrap()).unwrap();
        assert_eq!(
            zip.by_name("Images/001.img").unwrap().compression(),
            CompressionMethod::Deflated
        );
        assert!(zip.by_name("Images/001.img").is_ok());
        assert!(zip.by_name("Extracted/001/customer.doc").is_ok());
        assert!(zip.by_name("Reports/EvidenceAudit.csv").is_ok());
        assert!(
            zip.by_name("Reports/RecoverySelection-001-test.json")
                .is_ok()
        );
        assert!(zip.by_name("Reports/DeliveryCleanup-test.json").is_ok());
        assert!(
            zip.by_name("Recovery/DeliveryQuarantine/fixture/001/old.doc")
                .is_err()
        );
        assert!(zip.by_name("Reports/OfflineRecoveryDecisions.json").is_ok());
        assert!(
            zip.by_name("Reports/FluxVault_Jelentes_20260102.xlsx")
                .is_ok()
        );
        assert!(
            zip.by_name("Reports/FluxVault_Jelentes_20260101.xlsx")
                .is_err()
        );
        assert!(zip.by_name("Reports/private-working-note.txt").is_err());
        assert!(
            zip.by_name("Reports/ConversionHistory/ConversionState-test.json")
                .is_err()
        );
        assert!(
            zip.by_name("Extracted/001/System Volume Information/IndexerVolumeGuid")
                .is_err()
        );
        assert!(zip.by_name("Images/001.partial.img").is_err());
        assert!(zip.by_name("Flux/Recovery/001_job.json").is_err());
        assert!(zip.by_name("Flux/Recovery/001.lock").is_err());
        assert!(
            zip.by_name("Logs/Benchmark/.fluxvault-benchmark-test.jsonl")
                .is_err()
        );
        assert!(
            zip.by_name("Reports/Benchmark/Benchmark-test.json")
                .is_err()
        );
        assert!(zip.by_name("Reports/Benchmark/Benchmark-test.csv").is_err());
        assert!(
            zip.by_name("Logs/DualBenchmark/.fluxvault-dual-benchmark-test.jsonl")
                .is_err()
        );
        for name in ["DualBenchmark-test.json", "DualBenchmark-test.csv"] {
            assert!(
                zip.by_name(&format!("Reports/DualBenchmark/{name}"))
                    .is_err()
            );
        }
        assert!(
            zip.by_name("Flux/Recovery/001_attempt_001_provenance.json")
                .is_ok()
        );
        assert!(!should_exclude(&project.join("Extracted/001/001.lock")));
        assert!(
            zip.by_name("Extracted/001/.fluxvault-inventory.json")
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_destination_inside_project() {
        let root =
            std::env::temp_dir().join(format!("fluxvault-package-guard-{}", std::process::id()));
        fs::create_dir_all(root.join("Reports")).unwrap();
        fs::write(root.join("project.json"), "{}").unwrap();
        let result = build_package(
            &PackageRequest {
                project_root: root.clone(),
                destination: root.join("Reports"),
                project_name: "x".to_owned(),
            },
            &|_| {},
        );
        assert!(result.unwrap_err().contains("outside"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn customer_reports_include_native_recovery_exceptions_not_internal_markers() {
        assert!(is_customer_report("RecoveryExceptions.txt", None));
        assert!(is_customer_report("RecoverySelection-001-abc.json", None));
        assert!(is_customer_report("DeliveryCleanup-abc.json", None));
        assert!(!is_customer_report("DeliveryMirrorHistory", None));
        assert!(is_delivery_quarantine(Path::new(
            "Recovery/DeliveryQuarantine/abc/001/old.doc"
        )));
        assert!(!is_delivery_quarantine(Path::new(
            "Extracted/001/DeliveryQuarantine/customer.doc"
        )));
        assert!(!is_customer_report("private-recovery-note.txt", None));
        for name in [
            "BaselineComparison-123.json",
            "BaselineComparison-123.csv",
            "TestSummary-123.txt",
            "TestSummary-123.json",
        ] {
            assert!(!is_customer_report(name, None));
        }
        assert!(!should_exclude(Path::new(
            "Recovery/001/attempt_001_fat12.json"
        )));
        assert!(should_exclude(Path::new(
            "Extracted/001/attempt_001_native/.fluxvault-fat12.json"
        )));
        assert!(should_exclude(Path::new(
            "Recovery/001/.tmp-fragments-1-2/fragment.bin"
        )));
        assert!(should_exclude(Path::new(
            "Recovery/.tmp-native-001-1-2/DeletedRecovery/candidate.doc"
        )));
        assert!(!should_exclude(Path::new(
            "Recovery/001/attempt_001_fragments_v1/fragments.json"
        )));
        assert!(readme_text("test", 1, 1).contains("NOT complete/live customer documents"));
    }
}
