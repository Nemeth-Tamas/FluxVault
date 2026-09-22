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

const BACKUP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct RecoveryBackupRequest {
    pub recovery_root: PathBuf,
    pub disk_number: u32,
    pub attempt_number: u32,
    pub image_path: PathBuf,
    pub log_path: Option<PathBuf>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub enum RecoveryBackupEvent {
    Stage(String),
    Finished(Result<RecoveryBackupResult, String>),
}

#[derive(Debug, Clone)]
pub struct RecoveryBackupResult {
    pub directory: PathBuf,
    pub created: bool,
    pub image_backup: Option<PathBuf>,
    pub log_backup: Option<PathBuf>,
    pub manifest_path: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct RecoveryBackupManifest {
    schema_version: u32,
    created_unix_ms: u64,
    disk_number: u32,
    attempt_number: u32,
    reason: String,
    source_image: String,
    source_image_sha256: String,
    source_log: Option<String>,
    source_log_sha256: Option<String>,
    warning: String,
}

pub fn spawn_backup(request: RecoveryBackupRequest) -> Receiver<RecoveryBackupEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let send_stage = |message: &str| {
            let _ = sender.send(RecoveryBackupEvent::Stage(message.to_owned()));
        };
        let result = ensure_first_backup(&request, &send_stage);
        let _ = sender.send(RecoveryBackupEvent::Finished(result));
    });

    receiver
}

fn ensure_first_backup(
    request: &RecoveryBackupRequest,
    send_stage: &impl Fn(&str),
) -> Result<RecoveryBackupResult, String> {
    if request.disk_number == 0 {
        return Err("A recovery backup lemezszáma nem lehet nulla.".to_owned());
    }

    if !request.image_path.is_file() {
        return Err(format!(
            "A recovery backup forrásképe nem található: {}",
            request.image_path.display()
        ));
    }

    let disk_directory = request
        .recovery_root
        .join(format!("{:03}", request.disk_number));
    let final_directory = disk_directory.join("pass1");

    if final_directory.is_dir() {
        return Ok(existing_result(final_directory));
    }

    fs::create_dir_all(&disk_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a recovery lemezmappát {}: {error}",
            disk_directory.display()
        )
    })?;

    let temporary_directory = disk_directory.join(format!(
        ".pass1-{}-{}.partial",
        std::process::id(),
        current_unix_ms()
    ));
    fs::create_dir(&temporary_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni az ideiglenes recovery backup mappát {}: {error}",
            temporary_directory.display()
        )
    })?;

    let result = build_temporary_backup(request, &temporary_directory, send_stage);

    let (image_backup, log_backup, manifest_path) = match result {
        Ok(paths) => paths,
        Err(error) => {
            cleanup_temporary_backup(&temporary_directory);
            return Err(error);
        }
    };

    send_stage("Immutable pass1 recovery backup előléptetése...");

    match fs::rename(&temporary_directory, &final_directory) {
        Ok(()) => Ok(RecoveryBackupResult {
            directory: final_directory.clone(),
            created: true,
            image_backup: image_backup
                .and_then(|path| path.file_name().map(|name| final_directory.join(name))),
            log_backup: log_backup
                .and_then(|path| path.file_name().map(|name| final_directory.join(name))),
            manifest_path: manifest_path
                .and_then(|path| path.file_name().map(|name| final_directory.join(name))),
        }),
        Err(_) if final_directory.is_dir() => {
            cleanup_temporary_backup(&temporary_directory);
            Ok(existing_result(final_directory))
        }
        Err(error) => {
            cleanup_temporary_backup(&temporary_directory);
            Err(format!(
                "A pass1 recovery backup előléptetése sikertelen {}: {error}",
                final_directory.display()
            ))
        }
    }
}

fn build_temporary_backup(
    request: &RecoveryBackupRequest,
    temporary_directory: &Path,
    send_stage: &impl Fn(&str),
) -> Result<(Option<PathBuf>, Option<PathBuf>, Option<PathBuf>), String> {
    send_stage("Forráskép hash-elése és másolása az első recovery backupba...");
    let image_sha256 = sha256_file(&request.image_path)?;
    let image_name = request
        .image_path
        .file_name()
        .ok_or_else(|| "A recovery forráskép neve érvénytelen.".to_owned())?;
    let image_backup = temporary_directory.join(image_name);
    fs::copy(&request.image_path, &image_backup).map_err(|error| {
        format!("Nem sikerült a forrásképet a recovery backupba másolni: {error}")
    })?;

    let (log_backup, source_log, source_log_sha256) =
        match request.log_path.as_ref().filter(|path| path.is_file()) {
            Some(log_path) => {
                send_stage("Forrásnapló hash-elése és megőrzése...");
                let hash = sha256_file(log_path)?;
                let log_name = log_path
                    .file_name()
                    .ok_or_else(|| "A recovery forrásnapló neve érvénytelen.".to_owned())?;
                let backup = temporary_directory.join(log_name);
                fs::copy(log_path, &backup).map_err(|error| {
                    format!("Nem sikerült a forrásnaplót a recovery backupba másolni: {error}")
                })?;
                (
                    Some(backup),
                    Some(log_path.display().to_string()),
                    Some(hash),
                )
            }
            None => (None, None, None),
        };

    let manifest = RecoveryBackupManifest {
        schema_version: BACKUP_SCHEMA_VERSION,
        created_unix_ms: current_unix_ms(),
        disk_number: request.disk_number,
        attempt_number: request.attempt_number,
        reason: request.reason.clone(),
        source_image: request.image_path.display().to_string(),
        source_image_sha256: image_sha256,
        source_log,
        source_log_sha256,
        warning: "IMMUTABLE FIRST RECOVERY BACKUP: never overwrite with a later attempt."
            .to_owned(),
    };
    let manifest_path = temporary_directory.join("backup_info.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("Recovery backup manifest JSON hiba: {error}"))?;
    fs::write(&manifest_path, manifest_json).map_err(|error| {
        format!(
            "Nem sikerült menteni a recovery backup manifestet {}: {error}",
            manifest_path.display()
        )
    })?;

    Ok((Some(image_backup), log_backup, Some(manifest_path)))
}

fn existing_result(directory: PathBuf) -> RecoveryBackupResult {
    RecoveryBackupResult {
        image_backup: find_first_image(&directory),
        log_backup: find_first_extension(&directory, "log"),
        manifest_path: directory
            .join("backup_info.json")
            .is_file()
            .then(|| directory.join("backup_info.json")),
        directory,
        created: false,
    }
}

fn find_first_image(directory: &Path) -> Option<PathBuf> {
    ["bin", "img", "ima"]
        .iter()
        .find_map(|extension| find_first_extension(directory, extension))
}

fn find_first_extension(directory: &Path, extension: &str) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        })
}

fn cleanup_temporary_backup(path: &Path) {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".pass1-") && name.ends_with(".partial"))
    {
        let _ = fs::remove_dir_all(path);
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Nem sikerült hash-elni {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|error| format!("Hash olvasási hiba {}: {error}", path.display()))?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_first_backup_once_and_never_replaces_it() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-recovery-backup-{}-{}",
            std::process::id(),
            current_unix_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        let image = root.join("001_attempt_001.img");
        let log = root.join("001_attempt_001.log");
        fs::write(&image, b"first image").unwrap();
        fs::write(&log, b"first log").unwrap();
        let request = RecoveryBackupRequest {
            recovery_root: root.join("Recovery"),
            disk_number: 1,
            attempt_number: 1,
            image_path: image.clone(),
            log_path: Some(log),
            reason: "PARTIAL".to_owned(),
        };

        let first = ensure_first_backup(&request, &|_| {}).unwrap();
        assert!(first.created);
        assert_eq!(
            fs::read(first.image_backup.as_ref().unwrap()).unwrap(),
            b"first image"
        );

        fs::write(&image, b"later image").unwrap();
        let second = ensure_first_backup(&request, &|_| {}).unwrap();
        assert!(!second.created);
        assert_eq!(
            fs::read(second.image_backup.as_ref().unwrap()).unwrap(),
            b"first image"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
