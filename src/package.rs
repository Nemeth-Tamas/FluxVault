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
    for directory in INCLUDED_DIRECTORIES {
        let root = project.join(directory);
        if !root.exists() {
            continue;
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
                if should_exclude(&path) {
                    continue;
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

fn should_exclude(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    name.starts_with(".fluxvault-")
        || name.starts_with("__")
        || name.contains(".partial.")
        || name.ends_with(".tmp")
        || name.ends_with(".zip")
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
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
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
    format!(
        "FluxVault archival package\r\nProject: {project_name}\r\nFiles: {file_count}\r\nSource bytes: {total_bytes}\r\n\r\nImages: original and derived sector images.\r\nLogs: acquisition and recovery logs.\r\nExtracted: recovered source files.\r\nConverted: customer-friendly converted copies.\r\nRecovery: preserved recovery evidence and backups.\r\nReports: generated inventories and reports.\r\nFlux: raw flux captures, where available.\r\n\r\nCheck PACKAGE_MANIFEST.csv for each file's SHA-256.\r\nA partial image or recovered file is not proof that every original byte was readable.\r\nReview audit and recovery reports for limitations before delivery.\r\n"
    )
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
        fs::create_dir_all(&destination).unwrap();
        fs::write(project.join("project.json"), "{}").unwrap();
        fs::write(project.join("Images").join("001.img"), b"image").unwrap();
        fs::write(project.join("Images").join("001.partial.img"), b"partial").unwrap();
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
        let result = build_package(
            &PackageRequest {
                project_root: project.clone(),
                destination: destination.clone(),
                project_name: "Test project".to_owned(),
            },
            &|_| {},
        )
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert_eq!(result.total_bytes, 13);
        assert!(result.sha256_path.is_file());
        let mut zip = ZipArchive::new(File::open(result.zip_path).unwrap()).unwrap();
        assert!(zip.by_name("Images/001.img").is_ok());
        assert!(zip.by_name("Extracted/001/customer.doc").is_ok());
        assert!(zip.by_name("Images/001.partial.img").is_err());
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
}
