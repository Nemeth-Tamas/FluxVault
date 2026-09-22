use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::external_tools;

const EXTRACTION_SCHEMA_VERSION: u32 = 1;
const MARKER_FILE_NAME: &str = ".fluxvault-extraction.json";
const INVENTORY_FILE_NAME: &str = ".fluxvault-inventory.json";

#[derive(Debug, Clone)]
pub struct ExtractionRequest {
    pub seven_zip_executable: PathBuf,
    pub image_path: PathBuf,
    pub disk_number: u32,
    pub attempt_number: u32,
    pub extracted_root: PathBuf,
    pub logs_directory: PathBuf,
    pub command_audit_path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ExtractionEvent {
    Stage(String),
    Finished(Result<ExtractionResult, String>),
}

#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub image_path: PathBuf,
    pub output_directory: PathBuf,
    pub listing_path: PathBuf,
    pub inventory_path: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
    pub source_sha256: String,
    pub reused: bool,
}

#[derive(Debug, Clone)]
pub enum ExtractionPresence {
    Missing {
        expected_directory: PathBuf,
    },
    Automatic {
        output_directory: PathBuf,
        file_count: usize,
        total_bytes: u64,
        source_sha256: String,
    },
    ManualRecovery {
        output_directory: PathBuf,
        file_count: usize,
        total_bytes: u64,
    },
    InvalidAutomatic {
        output_directory: PathBuf,
        detail: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractionMarker {
    schema_version: u32,
    source_image: String,
    source_sha256: String,
    extracted_unix_ms: u64,
    file_count: usize,
    total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractionInventory {
    schema_version: u32,
    source_image: String,
    source_sha256: String,
    files: Vec<ExtractedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractedFile {
    relative_path: String,
    bytes: u64,
    modified_unix_ms: Option<u64>,
    attributes: String,
    sha256: String,
}

pub fn spawn_extraction(request: ExtractionRequest) -> Receiver<ExtractionEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let send_stage = |message: &str| {
            let _ = sender.send(ExtractionEvent::Stage(message.to_owned()));
        };

        send_stage("Forrás lemezkép SHA-256 ellenőrzése...");
        let result = run_extraction(&request, &send_stage);
        let _ = sender.send(ExtractionEvent::Finished(result));
    });

    receiver
}

pub fn inspect_extraction_presence(
    extracted_root: &Path,
    disk_number: u32,
    attempt_number: u32,
) -> Result<ExtractionPresence, String> {
    let disk_directory = extracted_root.join(format!("{disk_number:03}"));
    let expected_directory = if attempt_number == 0 {
        disk_directory.join("legacy")
    } else {
        disk_directory.join(format!("attempt_{attempt_number:03}"))
    };

    if expected_directory.is_dir() {
        return inspect_candidate_directory(&expected_directory);
    }

    if disk_directory.is_dir() {
        let (file_count, total_bytes) = count_manual_files(&disk_directory, true)?;

        if file_count > 0 {
            return Ok(ExtractionPresence::ManualRecovery {
                output_directory: disk_directory,
                file_count,
                total_bytes,
            });
        }
    }

    Ok(ExtractionPresence::Missing { expected_directory })
}

fn inspect_candidate_directory(path: &Path) -> Result<ExtractionPresence, String> {
    let marker_path = path.join(MARKER_FILE_NAME);

    if marker_path.is_file() {
        let marker_json = fs::read_to_string(&marker_path).map_err(|error| {
            format!(
                "Nem olvasható extraction marker {}: {error}",
                marker_path.display()
            )
        })?;

        return match serde_json::from_str::<ExtractionMarker>(&marker_json) {
            Ok(marker) => Ok(ExtractionPresence::Automatic {
                output_directory: path.to_path_buf(),
                file_count: marker.file_count,
                total_bytes: marker.total_bytes,
                source_sha256: marker.source_sha256,
            }),
            Err(error) => Ok(ExtractionPresence::InvalidAutomatic {
                output_directory: path.to_path_buf(),
                detail: format!("Hibás extraction marker: {error}"),
            }),
        };
    }

    let (file_count, total_bytes) = count_manual_files(path, false)?;

    if file_count > 0 {
        Ok(ExtractionPresence::ManualRecovery {
            output_directory: path.to_path_buf(),
            file_count,
            total_bytes,
        })
    } else {
        Ok(ExtractionPresence::Missing {
            expected_directory: path.to_path_buf(),
        })
    }
}

fn count_manual_files(root: &Path, skip_managed_children: bool) -> Result<(usize, u64), String> {
    let mut pending = vec![root.to_path_buf()];
    let mut file_count = 0usize;
    let mut total_bytes = 0u64;

    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            format!(
                "Nem sikerült megvizsgálni a recovery mappát {}: {error}",
                directory.display()
            )
        })?;

        for entry in entries {
            let entry = entry.map_err(|error| format!("Hibás recovery bejegyzés: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Nem olvasható fájltípus {}: {error}", path.display()))?;

            if file_type.is_dir() {
                if skip_managed_children && path.join(MARKER_FILE_NAME).is_file() {
                    continue;
                }

                pending.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with("__") || name == MARKER_FILE_NAME || name == INVENTORY_FILE_NAME {
                continue;
            }

            let metadata = entry.metadata().map_err(|error| {
                format!("Nem olvasható recovery fájl {}: {error}", path.display())
            })?;
            file_count += 1;
            total_bytes = total_bytes.saturating_add(metadata.len());
        }
    }

    Ok((file_count, total_bytes))
}

fn run_extraction(
    request: &ExtractionRequest,
    send_stage: &impl Fn(&str),
) -> Result<ExtractionResult, String> {
    if !request.image_path.is_file() {
        return Err(format!(
            "A forrás lemezkép nem található: {}",
            request.image_path.display()
        ));
    }

    if !request.seven_zip_executable.is_file() {
        return Err(format!(
            "A 7-Zip futtatható fájl nem található: {}",
            request.seven_zip_executable.display()
        ));
    }

    let disk_number = request.disk_number;
    let attempt_number = request.attempt_number;

    if disk_number == 0 {
        return Err("Az extraction lemezszáma nem lehet nulla.".to_owned());
    }

    let source_sha256 = sha256_file(&request.image_path)?;
    let disk_directory = request.extracted_root.join(format!("{disk_number:03}"));
    let output_directory = if attempt_number == 0 {
        disk_directory.join("legacy")
    } else {
        disk_directory.join(format!("attempt_{attempt_number:03}"))
    };
    let inventory_path = output_directory.join(INVENTORY_FILE_NAME);
    let listing_path = request.logs_directory.join(format!(
        "{disk_number:03}_attempt_{attempt_number:03}_7zip-listing.txt"
    ));

    if output_directory.exists() {
        return reuse_existing_extraction(
            &request.image_path,
            &source_sha256,
            &output_directory,
            &listing_path,
            &inventory_path,
        );
    }

    fs::create_dir_all(&request.logs_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a naplómappát {}: {error}",
            request.logs_directory.display()
        )
    })?;

    send_stage("FAT olvashatóság és 7-Zip tartalomjegyzék ellenőrzése...");
    let listing_arguments = vec![
        "l".to_owned(),
        "-slt".to_owned(),
        "--".to_owned(),
        request.image_path.display().to_string(),
    ];
    let listing = external_tools::run_audited_command(
        "7-Zip FAT listing",
        &request.seven_zip_executable,
        &listing_arguments,
        &request.command_audit_path,
    );

    write_listing_log(&listing_path, &listing.audit.stdout, &listing.audit.stderr)?;

    if let Some(error) = listing.audit_error {
        return Err(format!("A parancsnapló nem írható: {error}"));
    }

    if !listing.audit.success {
        return Err(format!(
            "A 7-Zip nem tudta olvasni a lemezkép fájlrendszerét (kilépési kód: {:?}). Részletek: {}",
            listing.audit.exit_code,
            concise_command_error(&listing.audit.stderr, &listing.audit.stdout)
        ));
    }

    fs::create_dir_all(&disk_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni az extraction mappát {}: {error}",
            disk_directory.display()
        )
    })?;

    let temporary_directory = request.extracted_root.join(format!(
        ".tmp-{disk_number:03}-attempt-{attempt_number:03}-{}",
        current_unix_ms()
    ));

    if temporary_directory.exists() {
        return Err(format!(
            "Az ideiglenes extraction mappa már létezik: {}",
            temporary_directory.display()
        ));
    }

    fs::create_dir_all(&temporary_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni az ideiglenes extraction mappát {}: {error}",
            temporary_directory.display()
        )
    })?;

    send_stage("Fájlok kibontása ideiglenes munkamappába...");
    let extraction_arguments = vec![
        "x".to_owned(),
        "-y".to_owned(),
        format!("-o{}", temporary_directory.display()),
        "--".to_owned(),
        request.image_path.display().to_string(),
    ];
    let extraction = external_tools::run_audited_command(
        "7-Zip extraction",
        &request.seven_zip_executable,
        &extraction_arguments,
        &request.command_audit_path,
    );

    if let Some(error) = extraction.audit_error {
        cleanup_temporary_directory(&temporary_directory);
        return Err(format!("A parancsnapló nem írható: {error}"));
    }

    if !extraction.audit.success {
        cleanup_temporary_directory(&temporary_directory);
        return Err(format!(
            "A 7-Zip extraction sikertelen (kilépési kód: {:?}). Részletek: {}",
            extraction.audit.exit_code,
            concise_command_error(&extraction.audit.stderr, &extraction.audit.stdout)
        ));
    }

    send_stage("Kinyert fájlok leltározása és hash-elése...");
    let files = match inventory_files(&temporary_directory) {
        Ok(files) => files,
        Err(error) => {
            cleanup_temporary_directory(&temporary_directory);
            return Err(error);
        }
    };
    let file_count = files.len();
    let total_bytes = files.iter().map(|file| file.bytes).sum();
    let source_image = request
        .image_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ismeretlen")
        .to_owned();
    let inventory = ExtractionInventory {
        schema_version: EXTRACTION_SCHEMA_VERSION,
        source_image: source_image.clone(),
        source_sha256: source_sha256.clone(),
        files,
    };
    let marker = ExtractionMarker {
        schema_version: EXTRACTION_SCHEMA_VERSION,
        source_image,
        source_sha256: source_sha256.clone(),
        extracted_unix_ms: current_unix_ms(),
        file_count,
        total_bytes,
    };

    if let Err(error) = write_json(
        &temporary_directory.join(INVENTORY_FILE_NAME),
        &inventory,
        "fájlleltár",
    ) {
        cleanup_temporary_directory(&temporary_directory);
        return Err(error);
    }

    if let Err(error) = write_json(
        &temporary_directory.join(MARKER_FILE_NAME),
        &marker,
        "extraction marker",
    ) {
        cleanup_temporary_directory(&temporary_directory);
        return Err(error);
    }

    send_stage("Sikeres extraction atomikus előléptetése...");
    if let Err(error) = fs::rename(&temporary_directory, &output_directory) {
        cleanup_temporary_directory(&temporary_directory);
        return Err(format!(
            "Nem sikerült előléptetni az extraction mappát {} -> {}: {error}",
            temporary_directory.display(),
            output_directory.display()
        ));
    }

    Ok(ExtractionResult {
        image_path: request.image_path.clone(),
        output_directory,
        listing_path,
        inventory_path,
        file_count,
        total_bytes,
        source_sha256,
        reused: false,
    })
}

fn reuse_existing_extraction(
    image_path: &Path,
    source_sha256: &str,
    output_directory: &Path,
    listing_path: &Path,
    inventory_path: &Path,
) -> Result<ExtractionResult, String> {
    let marker_path = output_directory.join(MARKER_FILE_NAME);
    let marker_json = fs::read_to_string(&marker_path).map_err(|error| {
        format!(
            "Az extraction célmappa már létezik, de nincs olvasható FluxVault marker: {} ({error}). A mappa változatlan maradt.",
            output_directory.display()
        )
    })?;
    let marker: ExtractionMarker = serde_json::from_str(&marker_json).map_err(|error| {
        format!(
            "Hibás extraction marker {}: {error}. A mappa változatlan maradt.",
            marker_path.display()
        )
    })?;

    if marker.source_sha256 != source_sha256 {
        return Err(format!(
            "Az extraction célmappa más forrás-hashhez tartozik: {}. Nem történt felülírás.",
            output_directory.display()
        ));
    }

    Ok(ExtractionResult {
        image_path: image_path.to_path_buf(),
        output_directory: output_directory.to_path_buf(),
        listing_path: listing_path.to_path_buf(),
        inventory_path: inventory_path.to_path_buf(),
        file_count: marker.file_count,
        total_bytes: marker.total_bytes,
        source_sha256: source_sha256.to_owned(),
        reused: true,
    })
}

#[cfg(test)]
fn parse_attempt_name(image_path: &Path) -> Result<(u32, u32), String> {
    let stem = image_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Érvénytelen lemezkép fájlnév: {}", image_path.display()))?;
    let Some((disk, attempt)) = stem.split_once("_attempt_") else {
        return Err(format!(
            "A lemezkép neve nem FluxVault attempt formátumú: {}",
            image_path.display()
        ));
    };
    let disk_number = disk
        .parse::<u32>()
        .map_err(|_| format!("Érvénytelen lemezszám a fájlnévben: {stem}"))?;
    let attempt_number = attempt
        .parse::<u32>()
        .map_err(|_| format!("Érvénytelen próbálkozásszám a fájlnévben: {stem}"))?;

    if disk_number == 0 || attempt_number == 0 {
        return Err(format!(
            "A lemez- és próbálkozásszám nem lehet nulla: {stem}"
        ));
    }

    Ok((disk_number, attempt_number))
}

fn inventory_files(root: &Path) -> Result<Vec<ExtractedFile>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            format!(
                "Nem sikerült leltározni a kinyert mappát {}: {error}",
                directory.display()
            )
        })?;

        for entry in entries {
            let entry = entry.map_err(|error| format!("Hibás extraction bejegyzés: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Nem olvasható fájltípus {}: {error}", path.display()))?;

            if file_type.is_dir() {
                pending.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let metadata = entry
                .metadata()
                .map_err(|error| format!("Nem olvasható metadata {}: {error}", path.display()))?;
            let relative_path = path
                .strip_prefix(root)
                .map_err(|error| format!("Extraction relatívútvonal-hiba: {error}"))?
                .to_string_lossy()
                .replace('\\', "/");
            let modified_unix_ms = metadata.modified().ok().and_then(system_time_unix_ms);

            files.push(ExtractedFile {
                relative_path,
                bytes: metadata.len(),
                modified_unix_ms,
                attributes: file_attributes(&metadata),
                sha256: sha256_file(&path)?,
            });
        }
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| {
        format!(
            "Nem sikerült megnyitni hash-eléshez {}: {error}",
            path.display()
        )
    })?;
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

#[cfg(windows)]
fn file_attributes(metadata: &fs::Metadata) -> String {
    use std::os::windows::fs::MetadataExt;

    format!("0x{:08X}", metadata.file_attributes())
}

#[cfg(not(windows))]
fn file_attributes(metadata: &fs::Metadata) -> String {
    if metadata.permissions().readonly() {
        "READ_ONLY".to_owned()
    } else {
        "NORMAL".to_owned()
    }
}

fn write_listing_log(path: &Path, stdout: &str, stderr: &str) -> Result<(), String> {
    let content = format!(
        "FluxVault 7-Zip listing\n\n--- stdout ---\n{stdout}\n\n--- stderr ---\n{stderr}\n"
    );

    fs::write(path, content).map_err(|error| {
        format!(
            "Nem sikerült menteni a 7-Zip listinget {}: {error}",
            path.display()
        )
    })
}

fn write_json(path: &Path, value: &impl Serialize, description: &str) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("{description} JSON hiba: {error}"))?;

    fs::write(path, json).map_err(|error| {
        format!(
            "Nem sikerült menteni a(z) {description} fájlt {}: {error}",
            path.display()
        )
    })
}

fn cleanup_temporary_directory(path: &Path) {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".tmp-"))
    {
        let _ = fs::remove_dir_all(path);
    }
}

fn concise_command_error(stderr: &str, stdout: &str) -> String {
    let source = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };

    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn current_unix_ms() -> u64 {
    system_time_unix_ms(SystemTime::now()).unwrap_or(0)
}

fn system_time_unix_ms(value: SystemTime) -> Option<u64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fluxvault-extraction-presence-{name}-{}-{}",
            std::process::id(),
            current_unix_ms()
        ))
    }

    #[test]
    fn parses_fluxvault_attempt_image_name() {
        let path = Path::new(r"C:\Archive\Images\007_attempt_012.img");

        assert_eq!(parse_attempt_name(path).unwrap(), (7, 12));
    }

    #[test]
    fn rejects_non_attempt_image_name() {
        let path = Path::new(r"C:\Archive\Images\007.img");

        assert!(parse_attempt_name(path).is_err());
    }

    #[test]
    fn cleanup_guard_only_accepts_fluxvault_temp_prefix() {
        assert!(
            Path::new(".tmp-001-attempt-001-123")
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".tmp-"))
        );
        assert!(
            !Path::new("attempt_001")
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".tmp-"))
        );
    }

    #[test]
    fn detects_operator_created_recovery_without_marker() {
        let root = test_root("manual");
        let disk_directory = root.join("001");
        fs::create_dir_all(disk_directory.join("$Root")).unwrap();
        fs::write(disk_directory.join("$Root").join("recovered.doc"), b"data").unwrap();

        let presence = inspect_extraction_presence(&root, 1, 0).unwrap();

        match presence {
            ExtractionPresence::ManualRecovery {
                output_directory,
                file_count,
                total_bytes,
            } => {
                assert_eq!(output_directory, disk_directory);
                assert_eq!(file_count, 1);
                assert_eq!(total_bytes, 4);
            }
            other => panic!("expected manual recovery, got {other:?}"),
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_managed_extraction_from_marker() {
        let root = test_root("automatic");
        let output_directory = root.join("001").join("attempt_001");
        fs::create_dir_all(&output_directory).unwrap();
        let marker = ExtractionMarker {
            schema_version: EXTRACTION_SCHEMA_VERSION,
            source_image: "001_attempt_001.img".to_owned(),
            source_sha256: "a".repeat(64),
            extracted_unix_ms: 1,
            file_count: 2,
            total_bytes: 42,
        };
        write_json(
            &output_directory.join(MARKER_FILE_NAME),
            &marker,
            "test marker",
        )
        .unwrap();

        let presence = inspect_extraction_presence(&root, 1, 1).unwrap();

        assert!(matches!(
            presence,
            ExtractionPresence::Automatic {
                file_count: 2,
                total_bytes: 42,
                ..
            }
        ));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires FLUXVAULT_TEST_7Z, FLUXVAULT_TEST_IMAGE, and FLUXVAULT_TEST_OUTPUT_ROOT"]
    fn seven_zip_adapter_extracts_and_reuses_a_test_image() {
        let seven_zip_executable = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_7Z").expect("FLUXVAULT_TEST_7Z is required"),
        );
        let image_path = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_IMAGE").expect("FLUXVAULT_TEST_IMAGE is required"),
        );
        let output_root = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_OUTPUT_ROOT")
                .expect("FLUXVAULT_TEST_OUTPUT_ROOT is required"),
        );
        let case_root =
            output_root.join(format!("fluxvault-extraction-test-{}", current_unix_ms()));
        let request = ExtractionRequest {
            seven_zip_executable,
            image_path,
            disk_number: 1,
            attempt_number: 1,
            extracted_root: case_root.join("Extracted"),
            logs_directory: case_root.join("Logs"),
            command_audit_path: case_root.join("Logs").join("external-tools.jsonl"),
        };

        let first = run_extraction(&request, &|_| {}).expect("first extraction should succeed");
        assert!(!first.reused);
        assert!(first.file_count > 0);
        assert!(first.inventory_path.is_file());

        let second = run_extraction(&request, &|_| {}).expect("second extraction should reuse");
        assert!(second.reused);
        assert_eq!(second.source_sha256, first.source_sha256);

        assert!(
            case_root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("fluxvault-extraction-test-"))
        );
        fs::remove_dir_all(case_root).expect("test output cleanup should succeed");
    }
}
