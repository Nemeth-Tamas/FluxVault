use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::dmde_logs;

const IMPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct ManualRecoveryImportRequest {
    pub source_directory: PathBuf,
    pub dmde_log_path: PathBuf,
    pub extracted_root: PathBuf,
    pub recovery_root: PathBuf,
    pub disk_number: u32,
}

#[derive(Debug, Clone)]
pub enum ManualRecoveryImportEvent {
    Stage(String),
    Finished(Result<ManualRecoveryImportResult, String>),
}

#[derive(Debug, Clone)]
pub struct ManualRecoveryImportResult {
    pub output_directory: PathBuf,
    pub evidence_directory: PathBuf,
    pub copied_log_path: PathBuf,
    pub manifest_path: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
struct ImportManifest {
    schema_version: u32,
    imported_unix_ms: u64,
    disk_number: u32,
    source_directory: String,
    source_dmde_log: String,
    dmde_log_sha256: String,
    dmde_status: String,
    dmde_pass_count: usize,
    dmde_bad_sector_count: usize,
    file_count: usize,
    total_bytes: u64,
    files: Vec<ImportedFile>,
    warning: String,
}

#[derive(Debug, Serialize)]
struct ImportedFile {
    relative_path: String,
    bytes: u64,
    sha256: String,
}

pub fn spawn_import(request: ManualRecoveryImportRequest) -> Receiver<ManualRecoveryImportEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let result = import_manual_recovery(&request, &|message| {
            let _ = sender.send(ManualRecoveryImportEvent::Stage(message.to_owned()));
        });
        let _ = sender.send(ManualRecoveryImportEvent::Finished(result));
    });

    receiver
}

pub(crate) fn import_manual_recovery(
    request: &ManualRecoveryImportRequest,
    send_stage: &impl Fn(&str),
) -> Result<ManualRecoveryImportResult, String> {
    if request.disk_number == 0 {
        return Err("Manual recovery import disk number must be positive.".to_owned());
    }
    if !request.source_directory.is_dir() {
        return Err(format!(
            "Recovered source directory not found: {}",
            request.source_directory.display()
        ));
    }
    if !request.dmde_log_path.is_file() {
        return Err(format!(
            "DMDE log not found: {}",
            request.dmde_log_path.display()
        ));
    }

    let parsed_dmde_log = dmde_logs::parse_dmde_log_file(&request.dmde_log_path)?;

    reject_source_inside_project(request)?;

    let disk_directory = request
        .extracted_root
        .join(format!("{:03}", request.disk_number));
    let output_directory = disk_directory.join("manual_recovery");
    if output_directory.exists() {
        return Err(format!(
            "A manual recovery import already exists for this disk: {}. Nothing overwritten.",
            output_directory.display()
        ));
    }

    fs::create_dir_all(&request.extracted_root).map_err(|error| {
        format!(
            "Cannot create Extracted directory {}: {error}",
            request.extracted_root.display()
        )
    })?;
    let temporary_directory = request.extracted_root.join(format!(
        ".manual-import-{:03}-{}-{}.partial",
        request.disk_number,
        std::process::id(),
        current_unix_ms()?
    ));
    fs::create_dir(&temporary_directory).map_err(|error| {
        format!(
            "Cannot create temporary import directory {}: {error}",
            temporary_directory.display()
        )
    })?;

    send_stage("Copying recovered files into a temporary import directory...");
    let files = match copy_tree_with_inventory(&request.source_directory, &temporary_directory) {
        Ok(files) => files,
        Err(error) => {
            cleanup_temporary(&temporary_directory);
            return Err(error);
        }
    };
    if files.is_empty() {
        cleanup_temporary(&temporary_directory);
        return Err("Selected recovery directory contains no files.".to_owned());
    }
    let file_count = files.len();
    let total_bytes = files.iter().map(|file| file.bytes).sum();

    send_stage("Recording DMDE log and import provenance...");
    let imported_unix_ms = current_unix_ms()?;
    let evidence_directory = request
        .recovery_root
        .join(format!("{:03}", request.disk_number))
        .join(format!("manual_import_{imported_unix_ms}"));
    if evidence_directory.exists() {
        cleanup_temporary(&temporary_directory);
        return Err(format!(
            "Import evidence directory already exists: {}",
            evidence_directory.display()
        ));
    }

    let dmde_log_sha256 = sha256_file(&request.dmde_log_path)?;
    let manifest = ImportManifest {
        schema_version: IMPORT_SCHEMA_VERSION,
        imported_unix_ms,
        disk_number: request.disk_number,
        source_directory: request.source_directory.display().to_string(),
        source_dmde_log: request.dmde_log_path.display().to_string(),
        dmde_log_sha256,
        dmde_status: parsed_dmde_log.status.label().to_owned(),
        dmde_pass_count: parsed_dmde_log.pass_count,
        dmde_bad_sector_count: parsed_dmde_log.bad_sectors.len(),
        file_count,
        total_bytes,
        files,
        warning: "Operator-provided DMDE recovery import. Source image evidence and physical media were not modified."
            .to_owned(),
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("Manual recovery manifest JSON hiba: {error}"))?;
    let embedded_manifest = temporary_directory.join("__fluxvault-manual-recovery.json");
    if let Err(error) = fs::write(&embedded_manifest, &manifest_json) {
        cleanup_temporary(&temporary_directory);
        return Err(format!(
            "Cannot write import provenance {}: {error}",
            embedded_manifest.display()
        ));
    }

    fs::create_dir_all(&disk_directory).map_err(|error| {
        cleanup_temporary(&temporary_directory);
        format!(
            "Cannot create disk extraction directory {}: {error}",
            disk_directory.display()
        )
    })?;
    send_stage("Atomically publishing manual recovery import...");
    if let Err(error) = fs::rename(&temporary_directory, &output_directory) {
        cleanup_temporary(&temporary_directory);
        return Err(format!(
            "Cannot publish manual recovery import {}: {error}",
            output_directory.display()
        ));
    }

    fs::create_dir_all(&evidence_directory).map_err(|error| {
        format!(
            "Recovered files imported, but evidence directory {} could not be created: {error}",
            evidence_directory.display()
        )
    })?;
    let copied_log_path = evidence_directory.join("dmde.log");
    fs::copy(&request.dmde_log_path, &copied_log_path).map_err(|error| {
        format!(
            "Recovered files imported, but DMDE log {} could not be copied: {error}",
            copied_log_path.display()
        )
    })?;
    let manifest_path = evidence_directory.join("import_manifest.json");
    fs::write(&manifest_path, manifest_json).map_err(|error| {
        format!(
            "Recovered files imported, but evidence manifest {} could not be written: {error}",
            manifest_path.display()
        )
    })?;

    Ok(ManualRecoveryImportResult {
        output_directory,
        evidence_directory,
        copied_log_path,
        manifest_path,
        file_count,
        total_bytes,
    })
}

fn reject_source_inside_project(request: &ManualRecoveryImportRequest) -> Result<(), String> {
    let source = request.source_directory.canonicalize().map_err(|error| {
        format!(
            "Cannot resolve recovered source directory {}: {error}",
            request.source_directory.display()
        )
    })?;
    for destination in [&request.extracted_root, &request.recovery_root] {
        if let Ok(destination) = destination.canonicalize()
            && source.starts_with(&destination)
        {
            return Err(format!(
                "Recovered source directory must be outside the FluxVault destination: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

fn copy_tree_with_inventory(
    source: &Path,
    destination: &Path,
) -> Result<Vec<ImportedFile>, String> {
    let mut pending = vec![source.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        let relative_directory = directory
            .strip_prefix(source)
            .map_err(|error| format!("Recovered directory relative-path error: {error}"))?;
        let destination_directory = destination.join(relative_directory);
        fs::create_dir_all(&destination_directory).map_err(|error| {
            format!(
                "Cannot create import subdirectory {}: {error}",
                destination_directory.display()
            )
        })?;

        for entry in fs::read_dir(&directory).map_err(|error| {
            format!(
                "Cannot read recovered directory {}: {error}",
                directory.display()
            )
        })? {
            let entry =
                entry.map_err(|error| format!("Invalid recovered directory entry: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Cannot inspect file type {}: {error}", path.display()))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "Cannot import a symlink/reparse point: {}",
                    path.display()
                ));
            }
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                return Err(format!("Unsupported recovered entry: {}", path.display()));
            }

            let relative_path = path
                .strip_prefix(source)
                .map_err(|error| format!("Recovered file relative-path error: {error}"))?;
            let destination_path = destination.join(relative_path);
            fs::copy(&path, &destination_path).map_err(|error| {
                format!(
                    "Recovered file copy failed {} -> {}: {error}",
                    path.display(),
                    destination_path.display()
                )
            })?;
            let bytes = fs::metadata(&destination_path)
                .map_err(|error| {
                    format!(
                        "Cannot read import metadata {}: {error}",
                        destination_path.display()
                    )
                })?
                .len();
            files.push(ImportedFile {
                relative_path: relative_path.to_string_lossy().replace('\\', "/"),
                bytes,
                sha256: sha256_file(&destination_path)?,
            });
        }
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Cannot open {} for hashing: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Hash read error {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn cleanup_temporary(path: &Path) {
    let safe = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".manual-import-") && name.ends_with(".partial"));
    if safe {
        let _ = fs::remove_dir_all(path);
    }
}

fn current_unix_ms() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| "System clock value is too large.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "fluxvault-manual-import-{}-{}",
            std::process::id(),
            current_unix_ms().unwrap()
        ))
    }

    #[test]
    fn imports_once_with_evidence_and_never_overwrites() {
        let root = test_root();
        let source = root.join("source");
        let extracted = root.join("project").join("Extracted");
        let recovery = root.join("project").join("Recovery");
        fs::create_dir_all(source.join("$Root")).unwrap();
        fs::write(source.join("$Root").join("document.doc"), b"recovered").unwrap();
        let log = root.join("copy.log");
        fs::write(
            &log,
            b"START 2026-09-22 12:00:00.000\nlogsec=512\nC 1 > > 0 : 1\nSTOP 2026-09-22 12:01:00.000\n",
        )
        .unwrap();

        let request = ManualRecoveryImportRequest {
            source_directory: source,
            dmde_log_path: log,
            extracted_root: extracted,
            recovery_root: recovery,
            disk_number: 7,
        };
        let result = import_manual_recovery(&request, &|_| {}).unwrap();
        assert_eq!(result.file_count, 1);
        assert_eq!(result.total_bytes, 9);
        assert!(
            result
                .output_directory
                .join("$Root")
                .join("document.doc")
                .is_file()
        );
        assert!(result.copied_log_path.is_file());
        assert!(result.manifest_path.is_file());

        let second = import_manual_recovery(&request, &|_| {}).unwrap_err();
        assert!(second.contains("already exists"));
        assert_eq!(
            fs::read(result.output_directory.join("$Root").join("document.doc")).unwrap(),
            b"recovered"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
