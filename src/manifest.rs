use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use chrono::{DateTime, Local};

use crate::imaging;

const EXTRACTION_MARKER: &str = ".fluxvault-extraction.json";

#[derive(Debug, Clone)]
pub struct ManifestRequest {
    pub extracted_root: PathBuf,
    pub images_directory: PathBuf,
    pub reports_directory: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ManifestEvent {
    Stage(String),
    Finished(Result<ManifestResult, String>),
}

#[derive(Debug, Clone)]
pub struct ManifestResult {
    pub path: PathBuf,
    pub disk_count: usize,
    pub file_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug)]
struct ManifestRow {
    floppy: String,
    path: String,
    size_bytes: u64,
    modified: String,
    attributes: String,
    image_sha256: String,
}

pub fn spawn_manifest(request: ManifestRequest) -> Receiver<ManifestEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let send_stage = |message: &str| {
            let _ = sender.send(ManifestEvent::Stage(message.to_owned()));
        };
        let result = build_manifest(&request, &send_stage);
        let _ = sender.send(ManifestEvent::Finished(result));
    });

    receiver
}

pub(crate) fn build_manifest(
    request: &ManifestRequest,
    send_stage: &impl Fn(&str),
) -> Result<ManifestResult, String> {
    send_stage("Recovered fájlmappák és forrásképek összerendelése...");
    let mut disk_directories = fs::read_dir(&request.extracted_root)
        .map_err(|error| {
            format!(
                "Nem sikerült beolvasni az Extracted mappát {}: {error}",
                request.extracted_root.display()
            )
        })?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let number = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            Some((number, entry.path()))
        })
        .collect::<Vec<_>>();
    disk_directories.sort_by_key(|(number, _)| *number);

    let mut rows = Vec::new();
    let mut included_disks = 0usize;

    for (disk_number, disk_directory) in disk_directories {
        let manual_files = collect_files(&disk_directory, true)?;
        let (content_root, files) = if manual_files.is_empty() {
            let Some(managed_directory) = latest_managed_directory(&disk_directory)? else {
                continue;
            };
            let files = collect_files(&managed_directory, false)?;
            (managed_directory, files)
        } else {
            (disk_directory, manual_files)
        };

        if files.is_empty() {
            continue;
        }

        let image_sha256 = imaging::load_attempts_for_disk(&request.images_directory, disk_number)?
            .last()
            .map(|attempt| attempt.sha256.clone())
            .unwrap_or_default();

        included_disks += 1;

        for file in files {
            let metadata = fs::metadata(&file).map_err(|error| {
                format!("Nem olvasható recovered fájl {}: {error}", file.display())
            })?;
            let relative_path = file
                .strip_prefix(&content_root)
                .map_err(|error| format!("Manifest relatívútvonal-hiba: {error}"))?
                .to_string_lossy()
                .replace('/', "\\");
            let modified = metadata
                .modified()
                .ok()
                .map(DateTime::<Local>::from)
                .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();

            rows.push(ManifestRow {
                floppy: format!("{disk_number:03}"),
                path: relative_path,
                size_bytes: metadata.len(),
                modified,
                attributes: file_attributes(&metadata),
                image_sha256: image_sha256.clone(),
            });
        }
    }

    rows.sort_by(|left, right| {
        left.floppy
            .cmp(&right.floppy)
            .then_with(|| left.path.cmp(&right.path))
    });
    let total_bytes = rows.iter().map(|row| row.size_bytes).sum();
    let file_count = rows.len();

    send_stage("MasterFileList.csv írása...");
    fs::create_dir_all(&request.reports_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a Reports mappát {}: {error}",
            request.reports_directory.display()
        )
    })?;
    let output_path = request.reports_directory.join("MasterFileList.csv");
    let mut csv = String::from(
        "\u{feff}\"Floppy\",\"Path\",\"SizeBytes\",\"Modified\",\"Attributes\",\"ImageSHA256\"\r\n",
    );

    for row in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\r\n",
            csv_field(&row.floppy),
            csv_field(&row.path),
            csv_field(&row.size_bytes.to_string()),
            csv_field(&row.modified),
            csv_field(&row.attributes),
            csv_field(&row.image_sha256)
        ));
    }

    fs::write(&output_path, csv).map_err(|error| {
        format!(
            "Nem sikerült menteni a recovered fájl manifestet {}: {error}",
            output_path.display()
        )
    })?;

    Ok(ManifestResult {
        path: output_path,
        disk_count: included_disks,
        file_count,
        total_bytes,
    })
}

fn latest_managed_directory(disk_directory: &Path) -> Result<Option<PathBuf>, String> {
    let mut candidates = fs::read_dir(disk_directory)
        .map_err(|error| {
            format!(
                "Nem sikerült megvizsgálni az extraction mappát {}: {error}",
                disk_directory.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(EXTRACTION_MARKER).is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    Ok(candidates.pop())
}

fn collect_files(root: &Path, skip_managed_children: bool) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("Nem sikerült beolvasni {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("Hibás manifest bejegyzés: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Nem olvasható fájltípus {}: {error}", path.display()))?;

            if file_type.is_dir() {
                if skip_managed_children && path.join(EXTRACTION_MARKER).is_file() {
                    continue;
                }
                pending.push(path);
            } else if file_type.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("__") && !name.starts_with(".fluxvault-") {
                    files.push(path);
                }
            }
        }
    }

    files.sort();
    Ok(files)
}

fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "fluxvault-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ))
    }

    #[test]
    fn csv_fields_escape_quotes() {
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn builds_manifest_from_manual_recovery_tree() {
        let root = test_root();
        let extracted = root.join("Extracted");
        let images = root.join("Images");
        let reports = root.join("Reports");
        fs::create_dir_all(extracted.join("001").join("$Root")).unwrap();
        fs::create_dir_all(&images).unwrap();
        fs::write(
            extracted.join("001").join("$Root").join("recovered.doc"),
            b"recovered",
        )
        .unwrap();
        let request = ManifestRequest {
            extracted_root: extracted,
            images_directory: images,
            reports_directory: reports,
        };

        let result = build_manifest(&request, &|_| {}).unwrap();
        let csv = fs::read_to_string(&result.path).unwrap();

        assert_eq!(result.disk_count, 1);
        assert_eq!(result.file_count, 1);
        assert_eq!(result.total_bytes, 9);
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("\"001\",\"$Root\\recovered.doc\""));

        fs::remove_dir_all(root).unwrap();
    }
}
