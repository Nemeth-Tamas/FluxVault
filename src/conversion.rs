use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const EXTRACTION_MARKER: &str = ".fluxvault-extraction.json";

#[derive(Debug, Clone)]
pub struct ConversionPlanningRequest {
    pub extracted_root: PathBuf,
    pub converted_root: PathBuf,
    pub reports_directory: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ConversionPlanningEvent {
    Stage(String),
    Finished(Result<ConversionPlanningResult, String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionPlanningResult {
    pub disk_count: usize,
    pub mirrored_files: usize,
    pub reused_files: usize,
    pub conversion_candidates: usize,
    pub path_map: PathBuf,
    pub conversion_plan: PathBuf,
    pub jobs: Vec<ConversionJob>,
    #[serde(default)]
    pub retired_mirrors: usize,
    #[serde(default)]
    pub preserved_obsolete_mirrors: usize,
    #[serde(default)]
    pub cleanup_reports: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionJob {
    pub floppy: String,
    pub source_path: PathBuf,
    pub original_forensic_path: String,
    pub delivery_original_path: String,
    pub recovery_method: String,
    pub source_type: String,
    pub source_sha256: String,
    pub modern_format: String,
    pub modern_filter: String,
    pub pdf_filter: String,
    pub modern_path: PathBuf,
    pub pdf_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryMethod {
    Filesystem,
    NativeFat12,
    DmdeFilesystem,
    Signature,
    ReconstructedDirectory,
    FragmentHypothesis,
}

impl RecoveryMethod {
    fn label(self) -> &'static str {
        match self {
            Self::Filesystem => "Filesystem recovery",
            Self::NativeFat12 => {
                "Native FAT12 readable-chain recovery; filesystem completeness unverified"
            }
            Self::DmdeFilesystem => "DMDE filesystem recovery; artifact folders removed",
            Self::Signature => "Signature recovered; original filename unavailable",
            Self::ReconstructedDirectory => {
                "Reconstructed directory; original parent/name and live/deleted ownership unknown"
            }
            Self::FragmentHypothesis => {
                "Fragment-chain hypothesis; structurally validated, original missing FAT link unproven"
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct OfficePlan {
    modern_extension: &'static str,
    modern_filter: &'static str,
    pdf_filter: &'static str,
}

#[derive(Debug)]
struct PathMapRow {
    floppy: String,
    original_forensic_path: String,
    delivery_path: String,
    recovery_method: RecoveryMethod,
    source_sha256: String,
}

#[derive(Debug)]
struct ConversionPlanRow {
    floppy: String,
    source_path: String,
    original_forensic_path: String,
    delivery_original_path: String,
    recovery_method: RecoveryMethod,
    source_type: String,
    modern_format: String,
    modern_filter: String,
    pdf_filter: String,
    modern_path: String,
    pdf_path: String,
}

pub fn spawn_conversion_planning(
    request: ConversionPlanningRequest,
) -> Receiver<ConversionPlanningEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = build_conversion_plan(&request, &|message| {
            let _ = sender.send(ConversionPlanningEvent::Stage(message.to_owned()));
        });
        let _ = sender.send(ConversionPlanningEvent::Finished(result));
    });
    receiver
}

pub(crate) fn build_conversion_plan(
    request: &ConversionPlanningRequest,
    send_stage: &impl Fn(&str),
) -> Result<ConversionPlanningResult, String> {
    let _reservation = crate::conversion_lock::reserve(&request.reports_directory)?;
    build_conversion_plan_reserved(request, send_stage)
}

/// Caller owns the project conversion lock for the entire conversion run.
pub(crate) fn build_conversion_plan_reserved(
    request: &ConversionPlanningRequest,
    send_stage: &impl Fn(&str),
) -> Result<ConversionPlanningResult, String> {
    if !request.extracted_root.is_dir() {
        return Err(format!(
            "Az Extracted mappa nem található: {}",
            request.extracted_root.display()
        ));
    }
    fs::create_dir_all(&request.converted_root).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a Converted mappát {}: {error}",
            request.converted_root.display()
        )
    })?;
    fs::create_dir_all(&request.reports_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a Reports mappát {}: {error}",
            request.reports_directory.display()
        )
    })?;

    let mut disk_directories = fs::read_dir(&request.extracted_root)
        .map_err(|error| format!("Nem olvasható az Extracted mappa: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let number = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            Some((number, entry.path()))
        })
        .collect::<Vec<_>>();
    disk_directories.sort_by_key(|(number, _)| *number);

    let mut claimed = BTreeMap::<String, String>::new();
    let maintenance = crate::delivery_maintenance::Maintenance::open(
        &request.converted_root,
        &request.reports_directory,
    )?;
    let mut mirrors = Vec::new();
    let mut protected = BTreeSet::new();
    let mut path_rows = Vec::new();
    let mut plan_rows = Vec::new();
    let mut jobs = Vec::new();
    let mut mirrored_files = 0usize;
    let mut reused_files = 0usize;
    let mut included_disks = 0usize;

    for (disk_number, disk_directory) in disk_directories {
        send_stage(&format!(
            "Lemez {disk_number:03} delivery útvonalainak tervezése..."
        ));
        let Some((content_root, files)) =
            selected_recovered_files(&disk_directory, &request.reports_directory, disk_number)?
        else {
            continue;
        };
        included_disks += 1;
        let floppy = format!("{disk_number:03}");
        let native_recovery = content_root.join(EXTRACTION_MARKER).is_file()
            && crate::extraction::verify_native_extraction(&content_root)?;

        for source in files {
            let forensic_path = source
                .strip_prefix(&content_root)
                .map_err(|error| format!("Forensic relatívútvonal-hiba: {error}"))?;
            let forensic_text = forensic_path.to_string_lossy().replace('/', "\\");
            let (mut delivery_relative, mut recovery_method) = clean_delivery_path(forensic_path)?;
            if native_recovery
                && !matches!(
                    recovery_method,
                    RecoveryMethod::Signature
                        | RecoveryMethod::ReconstructedDirectory
                        | RecoveryMethod::FragmentHypothesis
                )
            {
                recovery_method = RecoveryMethod::NativeFat12;
            }
            let original_delivery_relative = delivery_relative.clone();

            let source_sha256 = sha256_file(&source)?;
            let clean_path = PathBuf::from(&floppy)
                .join(&delivery_relative)
                .to_string_lossy()
                .replace('\\', "/");
            if let Some(previous) =
                maintenance.previous_path(&clean_path, &forensic_text, &source_sha256)
            {
                let previous = PathBuf::from(previous);
                if let Ok(relative) = previous.strip_prefix(&floppy) {
                    delivery_relative = relative.to_path_buf();
                }
            }
            let forensic_key = forensic_text.to_ascii_lowercase();
            let mut ordinal = 2usize;
            loop {
                let delivery_with_floppy = PathBuf::from(&floppy).join(&delivery_relative);
                let key = delivery_with_floppy.to_string_lossy().to_ascii_lowercase();
                let destination = request.converted_root.join(&delivery_with_floppy);
                let claimed_by_other = claimed
                    .get(&key)
                    .is_some_and(|existing| existing != &forensic_key);
                let occupied_by_other = destination.is_file()
                    && !same_file_hash(&destination, &source_sha256).unwrap_or(false);
                if !claimed_by_other && !occupied_by_other {
                    claimed.insert(key, forensic_key.clone());
                    break;
                }
                delivery_relative = collision_path(&original_delivery_relative, ordinal)?;
                ordinal += 1;
            }

            let delivery_with_floppy = PathBuf::from(&floppy).join(&delivery_relative);
            let relative = delivery_with_floppy.to_string_lossy().replace('\\', "/");
            let target =
                crate::delivery_maintenance::safe_target(&request.converted_root, &relative)?;
            let reused = same_file_hash(&target, &source_sha256).unwrap_or(false);
            if reused {
                reused_files += 1;
            } else {
                let mut input = fs::File::open(&source).map_err(|e| e.to_string())?;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(|e| {
                        format!("New mirror reservation failed; existing files preserved: {e}")
                    })?;
                std::io::copy(&mut input, &mut output)
                    .and_then(|_| output.sync_all())
                    .map_err(|e| e.to_string())?;
                if sha256_file(&target)? != source_sha256 {
                    return Err(
                        "Source changed during mirror creation; result preserved, not claimed"
                            .into(),
                    );
                }
                mirrored_files += 1;
            }
            mirrors.push(maintenance.mirror(
                relative,
                clean_path,
                forensic_text.clone(),
                source_sha256.clone(),
                !reused,
            ));

            path_rows.push(PathMapRow {
                floppy: floppy.clone(),
                original_forensic_path: forensic_text.clone(),
                delivery_path: delivery_with_floppy.to_string_lossy().replace('/', "\\"),
                recovery_method,
                source_sha256: source_sha256.clone(),
            });

            let Some(plan) = office_plan(&source) else {
                continue;
            };
            let extension = source
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            let stem = target
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("recovered");
            let base = format!("{stem} [from {extension}]");
            let output_directory = target.parent().unwrap_or(&request.converted_root);
            let modern_path = output_directory.join(format!("{base}.{}", plan.modern_extension));
            let pdf_path = output_directory.join(format!("{base}.pdf"));
            for path in [&modern_path, &pdf_path] {
                let relative = path
                    .strip_prefix(
                        request
                            .converted_root
                            .canonicalize()
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                protected.insert(relative.to_string_lossy().replace('\\', "/").to_uppercase());
            }
            plan_rows.push(ConversionPlanRow {
                floppy: floppy.clone(),
                source_path: source.display().to_string(),
                original_forensic_path: forensic_text,
                delivery_original_path: delivery_with_floppy.to_string_lossy().replace('/', "\\"),
                recovery_method,
                source_type: format!(".{}", extension.to_ascii_lowercase()),
                modern_format: plan.modern_extension.to_ascii_uppercase(),
                modern_filter: plan.modern_filter.to_owned(),
                pdf_filter: plan.pdf_filter.to_owned(),
                modern_path: modern_path.display().to_string(),
                pdf_path: pdf_path.display().to_string(),
            });
            jobs.push(ConversionJob {
                floppy: floppy.clone(),
                source_path: source,
                original_forensic_path: plan_rows.last().unwrap().original_forensic_path.clone(),
                delivery_original_path: plan_rows.last().unwrap().delivery_original_path.clone(),
                recovery_method: recovery_method.label().to_owned(),
                source_type: format!(".{}", extension.to_ascii_lowercase()),
                source_sha256: path_rows.last().unwrap().source_sha256.clone(),
                modern_format: plan.modern_extension.to_ascii_uppercase(),
                modern_filter: plan.modern_filter.to_owned(),
                pdf_filter: plan.pdf_filter.to_owned(),
                modern_path,
                pdf_path,
            });
        }
    }

    path_rows.sort_by(|left, right| {
        (&left.floppy, &left.delivery_path).cmp(&(&right.floppy, &right.delivery_path))
    });
    plan_rows.sort_by(|left, right| {
        (&left.floppy, &left.original_forensic_path)
            .cmp(&(&right.floppy, &right.original_forensic_path))
    });
    let path_map = request.reports_directory.join("DeliveryPathMap.csv");
    let conversion_plan = request.reports_directory.join("ConversionPlan.csv");
    write_path_map(&path_map, &path_rows)?;
    write_conversion_plan(&conversion_plan, &plan_rows)?;
    let cleanup = maintenance.finish(mirrors, protected)?;
    if cleanup.retired > 0 || cleanup.preserved > 0 {
        send_stage(&format!(
            "Delivery maintenance: {} unchanged owned originals quarantined; {} modified/needed/conflicting copies preserved",
            cleanup.retired, cleanup.preserved
        ));
    }

    Ok(ConversionPlanningResult {
        disk_count: included_disks,
        mirrored_files,
        reused_files,
        conversion_candidates: plan_rows.len(),
        path_map,
        conversion_plan,
        jobs,
        retired_mirrors: cleanup.retired,
        preserved_obsolete_mirrors: cleanup.preserved,
        cleanup_reports: cleanup.reports,
    })
}

fn selected_recovered_files(
    disk_directory: &Path,
    reports: &Path,
    disk: u32,
) -> Result<Option<(PathBuf, Vec<PathBuf>)>, String> {
    let manual = collect_files(disk_directory, true)?;
    if !manual.is_empty() {
        return Ok(Some((disk_directory.to_path_buf(), manual)));
    }
    let Some(selection) = crate::extraction::select_managed(disk_directory, None)? else {
        return Ok(None);
    };
    crate::extraction::record_selection(reports, disk, &selection)?;
    let directory = selection.directory;
    let files = collect_files(&directory, false)?;
    Ok((!files.is_empty()).then_some((directory, files)))
}

fn collect_files(root: &Path, skip_managed_children: bool) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("Nem olvasható {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("Hibás recovered bejegyzés: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Nem olvasható fájltípus {}: {error}", path.display()))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.eq_ignore_ascii_case("System Volume Information")
                    || name.eq_ignore_ascii_case("$RECYCLE.BIN")
                {
                    continue;
                }
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

/// Same safe presentation normalization used for source-only comparisons.
pub(crate) fn comparison_path(path: &Path) -> Result<PathBuf, String> {
    clean_delivery_path(path).map(|(path, _)| path)
}

fn clean_delivery_path(path: &Path) -> Result<(PathBuf, RecoveryMethod), String> {
    let mut clean = Vec::new();
    let mut signature = false;
    let mut reconstructed_directory = false;
    let mut fragment = false;
    let mut dmde = false;
    for component in path.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        let text = part.to_string_lossy();
        if text.eq_ignore_ascii_case("FragmentRecovery") {
            fragment = true;
            continue;
        }
        if text.eq_ignore_ascii_case("DirectoryRecovery") {
            reconstructed_directory = true;
            continue;
        }
        if text.eq_ignore_ascii_case("[$Raw Files by Signatures]")
            || text.eq_ignore_ascii_case("$Raw")
            || text.eq_ignore_ascii_case("SignatureRecovery")
        {
            signature = true;
            continue;
        }
        let lower = text.to_ascii_lowercase();
        if lower == "$root"
            || lower == "$noname"
            || lower
                .strip_prefix("$noname ")
                .is_some_and(|suffix| suffix.parse::<u32>().is_ok())
        {
            dmde = true;
            continue;
        }
        if !text.is_empty() {
            clean.push(part.to_os_string());
        }
    }
    if signature {
        clean.insert(0, "Signature-Recovered".into());
    } else if reconstructed_directory {
        clean.insert(0, "Directory-Recovered".into());
    } else if fragment {
        clean.insert(0, "Fragment-Hypotheses".into());
    }
    if clean.is_empty() {
        clean.push("Unsorted-Recovery".into());
        clean.push(
            path.file_name()
                .ok_or_else(|| "A recovered útvonalnak nincs fájlneve.".to_owned())?
                .to_os_string(),
        );
    }
    let method = if signature {
        RecoveryMethod::Signature
    } else if reconstructed_directory {
        RecoveryMethod::ReconstructedDirectory
    } else if fragment {
        RecoveryMethod::FragmentHypothesis
    } else if dmde {
        RecoveryMethod::DmdeFilesystem
    } else {
        RecoveryMethod::Filesystem
    };
    Ok((clean.iter().collect(), method))
}

/// Use exactly the same recovery labels as delivery planning, without copying files.
pub(crate) fn report_recovery_method(path: &Path, native: bool) -> Result<String, String> {
    let (_, mut method) = clean_delivery_path(path)?;
    if native && method == RecoveryMethod::Filesystem {
        method = RecoveryMethod::NativeFat12;
    }
    Ok(method.label().to_owned())
}

fn collision_path(path: &Path, ordinal: usize) -> Result<PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Nem képezhető collision útvonal: {}", path.display()))?;
    let extension = path.extension().and_then(|value| value.to_str());
    let name = match extension {
        Some(extension) => format!("{stem} [recovered copy {ordinal}].{extension}"),
        None => format!("{stem} [recovered copy {ordinal}]"),
    };
    Ok(path.parent().unwrap_or(Path::new("")).join(name))
}

fn office_plan(path: &Path) -> Option<OfficePlan> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "doc" | "rtf" | "wps" | "wri" | "wpd" | "sdw" => Some(OfficePlan {
            modern_extension: "docx",
            modern_filter: "Office Open XML Text",
            pdf_filter: "writer_pdf_Export",
        }),
        "xls" | "xlw" | "xlt" | "wk1" | "wk3" | "wk4" | "wks" | "123" | "wb1" | "wb2" | "wq1"
        | "wq2" | "sdc" => Some(OfficePlan {
            modern_extension: "xlsx",
            modern_filter: "Calc Office Open XML",
            pdf_filter: "calc_pdf_Export",
        }),
        "ppt" | "pps" | "pot" | "sdd" => Some(OfficePlan {
            modern_extension: "pptx",
            modern_filter: "Impress Office Open XML",
            pdf_filter: "impress_pdf_Export",
        }),
        _ => None,
    }
}

pub(crate) fn requires_conversion(path: &Path) -> bool {
    office_plan(path).is_some()
}

pub(crate) fn delivery_eligible(path: &Path) -> bool {
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    !filename.starts_with("__")
        && !filename.starts_with(".fluxvault-")
        && !path.components().any(|c| {
            let name = c.as_os_str().to_string_lossy();
            name.eq_ignore_ascii_case("System Volume Information")
                || name.eq_ignore_ascii_case("$RECYCLE.BIN")
        })
}

fn same_file_hash(path: &Path, expected_sha256: &str) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    Ok(sha256_file(path)? == expected_sha256)
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Nem nyitható meg hash-eléshez {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        crate::cancellation::check()?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Hash olvasási hiba {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn write_path_map(path: &Path, rows: &[PathMapRow]) -> Result<(), String> {
    let mut csv = String::from(
        "\u{feff}\"Floppy\",\"OriginalForensicPath\",\"DeliveryPath\",\"RecoveryMethod\",\"SourceSHA256\"\r\n",
    );
    for row in rows {
        csv.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\r\n",
            escape(&row.floppy),
            escape(&row.original_forensic_path),
            escape(&row.delivery_path),
            escape(row.recovery_method.label()),
            escape(&row.source_sha256)
        ));
    }
    fs::write(path, csv).map_err(|error| format!("Nem írható {}: {error}", path.display()))
}

fn write_conversion_plan(path: &Path, rows: &[ConversionPlanRow]) -> Result<(), String> {
    let mut csv = String::from(
        "\u{feff}\"Floppy\",\"SourcePath\",\"OriginalForensicPath\",\"DeliveryOriginalPath\",\"RecoveryMethod\",\"SourceType\",\"ModernFormat\",\"ModernFilter\",\"PdfFilter\",\"ModernPath\",\"PdfPath\"\r\n",
    );
    for row in rows {
        let values = [
            row.floppy.as_str(),
            row.source_path.as_str(),
            row.original_forensic_path.as_str(),
            row.delivery_original_path.as_str(),
            row.recovery_method.label(),
            row.source_type.as_str(),
            row.modern_format.as_str(),
            row.modern_filter.as_str(),
            row.pdf_filter.as_str(),
            row.modern_path.as_str(),
            row.pdf_path.as_str(),
        ];
        csv.push_str(
            &values
                .iter()
                .map(|value| format!("\"{}\"", escape(value)))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
    }
    fs::write(path, csv).map_err(|error| format!("Nem írható {}: {error}", path.display()))
}

fn escape(value: &str) -> String {
    value.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_dmde_artifacts_and_labels_recovery() {
        let (path, method) = clean_delivery_path(Path::new("$Noname 2/$Root/docs/a.doc")).unwrap();
        assert_eq!(path, PathBuf::from("docs").join("a.doc"));
        assert_eq!(method, RecoveryMethod::DmdeFilesystem);
    }

    #[test]
    fn signature_recovery_is_separated() {
        let (path, method) =
            clean_delivery_path(Path::new("[$Raw Files by Signatures]/DOC/file.doc")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("Signature-Recovered")
                .join("DOC")
                .join("file.doc")
        );
        assert_eq!(method, RecoveryMethod::Signature);
    }

    #[test]
    fn reconstructed_directories_and_fragment_hypotheses_keep_their_labels() {
        for (input, expected, kind) in [
            (
                "DirectoryRecovery/cluster_0002/folder/file.doc",
                "Directory-Recovered/cluster_0002/folder/file.doc",
                RecoveryMethod::ReconstructedDirectory,
            ),
            (
                "FragmentRecovery/carved.doc",
                "Fragment-Hypotheses/carved.doc",
                RecoveryMethod::FragmentHypothesis,
            ),
        ] {
            let (path, method) = clean_delivery_path(Path::new(input)).unwrap();
            assert_eq!(path, PathBuf::from(expected));
            assert_eq!(method, kind);
            assert!(!method.label().contains("Native FAT12"));
        }
    }

    #[test]
    fn legacy_extensions_have_expected_output_families() {
        assert_eq!(
            office_plan(Path::new("a.wpd")).unwrap().modern_extension,
            "docx"
        );
        assert_eq!(
            office_plan(Path::new("a.wk1")).unwrap().modern_extension,
            "xlsx"
        );
        assert_eq!(
            office_plan(Path::new("a.sdd")).unwrap().modern_extension,
            "pptx"
        );
        assert!(office_plan(Path::new("a.txt")).is_none());
    }

    #[test]
    fn collision_names_are_deterministic() {
        assert_eq!(
            collision_path(Path::new("docs/a.doc"), 2).unwrap(),
            PathBuf::from("docs").join("a [recovered copy 2].doc")
        );
    }

    #[test]
    fn builds_delivery_tree_without_changing_recovered_sources() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-conversion-plan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let extracted = root.join("Extracted");
        let converted = root.join("Converted");
        let reports = root.join("Reports");
        let root_doc = extracted
            .join("001")
            .join("$Root")
            .join("docs")
            .join("a.doc");
        let noname_doc = extracted
            .join("001")
            .join("$Noname 2")
            .join("docs")
            .join("a.doc");
        fs::create_dir_all(root_doc.parent().unwrap()).unwrap();
        fs::create_dir_all(noname_doc.parent().unwrap()).unwrap();
        fs::write(&root_doc, b"first").unwrap();
        fs::write(&noname_doc, b"second").unwrap();
        let os_metadata = extracted
            .join("001")
            .join("System Volume Information")
            .join("IndexerVolumeGuid");
        fs::create_dir_all(os_metadata.parent().unwrap()).unwrap();
        fs::write(&os_metadata, b"Windows metadata").unwrap();

        let result = build_conversion_plan(
            &ConversionPlanningRequest {
                extracted_root: extracted.clone(),
                converted_root: converted.clone(),
                reports_directory: reports,
            },
            &|_| {},
        )
        .unwrap();

        assert_eq!(result.disk_count, 1);
        assert_eq!(result.mirrored_files, 2);
        assert_eq!(result.conversion_candidates, 2);
        assert!(
            !converted
                .join("001")
                .join("System Volume Information")
                .exists()
        );
        assert!(converted.join("001").join("docs").join("a.doc").is_file());
        assert!(
            converted
                .join("001")
                .join("docs")
                .join("a [recovered copy 2].doc")
                .is_file()
        );
        assert_eq!(fs::read(&root_doc).unwrap(), b"first");
        assert_eq!(fs::read(&noname_doc).unwrap(), b"second");
        assert!(result.path_map.is_file());
        assert!(result.conversion_plan.is_file());

        fs::remove_dir_all(root).unwrap();
    }
}
