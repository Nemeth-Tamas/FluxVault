//! Deterministic, same-acquisition recovery preference without evidence loss.
use super::*;
use std::{collections::BTreeMap, io::Write};

#[derive(Debug, Serialize)]
pub(crate) struct ManagedSelection {
    #[serde(skip)]
    pub directory: PathBuf,
    schema_version: u32,
    attempt: u32,
    source_sha256: String,
    preferred_directory: String,
    candidates: Vec<Choice>,
    warning: &'static str,
}

#[derive(Debug, Serialize)]
struct Choice {
    directory: String,
    inventory_sha256: String,
    files: usize,
    bytes: u64,
    reachable_files: usize,
    validated_long_names: usize,
    reason: String,
}

struct Evidence {
    path: PathBuf,
    marker: ExtractionMarker,
    coverage: BTreeMap<(String, u64), usize>,
    reachable: BTreeMap<(String, u64), usize>,
    names: usize,
    inventory_hash: String,
}

fn covers(a: &BTreeMap<(String, u64), usize>, b: &BTreeMap<(String, u64), usize>) -> bool {
    b.iter()
        .all(|(key, count)| a.get(key).copied().unwrap_or(0) >= *count)
}

pub(crate) fn select_managed(
    disk: &Path,
    attempt: Option<u32>,
) -> Result<Option<ManagedSelection>, String> {
    if !disk.is_dir() {
        return Ok(None);
    }
    let canonical = disk.canonicalize().map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(disk).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() && path.join(MARKER_FILE_NAME).is_file() {
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir()
                || path.canonicalize().map_err(|e| e.to_string())?.parent()
                    != Some(canonical.as_path())
            {
                return Err("Managed recovery directory escapes disk; selection refused".into());
            }
            candidates.push(path);
            if candidates.len() > 128 {
                return Err("Recovery generation selection ceiling reached".into());
            }
        }
    }
    candidates.sort_by_key(|p| managed_directory_order(p));
    let project = disk.parent().and_then(Path::parent);
    let catalogue = if let Some(project) = project.filter(|p| p.join("project.json").is_file()) {
        let disk_number = disk
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or("Invalid recovery disk folder")?;
        Some(crate::imaging::load_attempts_for_disk(
            &project.join("Images"),
            disk_number,
        )?)
    } else {
        None
    };
    let acquisition = catalogue
        .as_ref()
        .and_then(|a| crate::imaging::best_attempt(a))
        .map(|a| a.attempt_number);
    let Some(number) = attempt
        .or(acquisition)
        .or_else(|| candidates.last().map(|p| managed_directory_order(p).0))
    else {
        return Ok(None);
    };
    candidates.retain(|p| managed_directory_order(p).0 == number);
    if candidates.is_empty() {
        return Ok(None);
    }
    let binding = if let Some(catalogue) = &catalogue {
        Some(
            catalogue
                .iter()
                .find(|a| a.attempt_number == number)
                .ok_or("Recovery generation lacks a catalogued acquisition")?,
        )
    } else {
        None
    };
    let mut evidence = Vec::new();
    for path in candidates {
        let marker: ExtractionMarker = serde_json::from_slice(
            &fs::read(path.join(MARKER_FILE_NAME)).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        verify_managed_extraction(&path, &marker.source_sha256)?;
        if let Some(images) = project.map(|p| p.join("Images")).filter(|p| p.is_dir()) {
            let source = Path::new(&marker.source_image);
            if source.components().count() != 1 || source.file_name().is_none() {
                return Err("Recovery source is not a saved image basename".into());
            }
            let image = images.join(source);
            if let Some(binding) = &binding {
                let selected_image =
                    crate::recovery_plan::resolve_image_path(&images, &binding.image_file)?;
                if marker.source_sha256 != binding.sha256
                    || image.canonicalize().map_err(|e| e.to_string())?
                        != selected_image.canonicalize().map_err(|e| e.to_string())?
                {
                    return Err("Recovery generation does not bind the selected acquisition".into());
                }
            }
            if !fs::symlink_metadata(&image)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_file()
                || image.canonicalize().map_err(|e| e.to_string())?.parent()
                    != Some(images.canonicalize().map_err(|e| e.to_string())?.as_path())
                || sha256_file(&image)? != marker.source_sha256
            {
                return Err(
                    "Recovery source image changed or escaped Images; preference refused".into(),
                );
            }
        }
        if marker.native_recovery_report_sha256.is_some() {
            let report: crate::fat12_recovery::RecoveryReport = serde_json::from_slice(
                &fs::read(path.join(FAT12_REPORT_NAME)).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if report.deleted.is_some() {
                return Err("Deleted forensic output cannot be a live recovery preference".into());
            }
        }
        if let Some(first) = evidence.first() {
            let first: &Evidence = first;
            if first.marker.source_sha256 != marker.source_sha256 {
                return Err("Competing recovery generations bind different images of the same attempt; preference refused".into());
            }
        }
        let inventory: ExtractionInventory = serde_json::from_slice(
            &fs::read(path.join(INVENTORY_FILE_NAME)).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut coverage = BTreeMap::new();
        let mut reachable = BTreeMap::new();
        for file in inventory.files {
            if file.relative_path.split(['/', '\\']).any(|p| {
                p.eq_ignore_ascii_case("System Volume Information")
                    || p.eq_ignore_ascii_case("$RECYCLE.BIN")
            }) {
                continue;
            }
            let key = (file.sha256, file.bytes);
            *coverage.entry(key.clone()).or_default() += 1;
            if !file.relative_path.split(['/', '\\']).any(|p| {
                p.eq_ignore_ascii_case("SignatureRecovery")
                    || p.eq_ignore_ascii_case("DirectoryRecovery")
                    || p.eq_ignore_ascii_case("FragmentRecovery")
                    || p.eq_ignore_ascii_case("[$Raw Files by Signatures]")
            }) {
                *reachable.entry(key).or_default() += 1;
            }
        }
        let names = if marker.native_recovery_report_sha256.is_some() {
            let report: crate::fat12_recovery::RecoveryReport = serde_json::from_slice(
                &fs::read(path.join(FAT12_REPORT_NAME)).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            report
                .analysis
                .as_ref()
                .map_or(0, |a| a.validated_long_names.len())
        } else {
            0
        };
        evidence.push(Evidence {
            inventory_hash: sha256_file(&path.join(INVENTORY_FILE_NAME))?,
            path,
            marker,
            coverage,
            reachable,
            names,
        });
    }
    if evidence.is_empty() {
        return Ok(None);
    }
    let mut preferred = 0;
    let mut choices = Vec::new();
    for (index, candidate) in evidence.iter().enumerate() {
        let incumbent = &evidence[preferred];
        let reason = if index == 0 {
            "Initial verified recovery generation"
        } else if covers(&candidate.coverage, &incumbent.coverage)
            && covers(&candidate.reachable, &incumbent.reachable)
            && candidate.names >= incumbent.names
        {
            preferred = index;
            if candidate.coverage != incumbent.coverage {
                "Promoted: retains all previous payloads and adds readable recovery coverage"
            } else if candidate.names > incumbent.names {
                "Promoted: identical byte coverage with stronger validated name evidence"
            } else {
                "Promoted: equivalent byte/ownership/name coverage; newer generation tie-break"
            }
        } else {
            "Retained as alternate: loses prior payload/name evidence or is incomparable; no union/guess made"
        };
        choices.push(Choice {
            directory: candidate.path.file_name().unwrap().to_string_lossy().into(),
            inventory_sha256: candidate.inventory_hash.clone(),
            files: candidate.coverage.values().sum(),
            bytes: candidate
                .coverage
                .iter()
                .map(|((_, bytes), count)| bytes * *count as u64)
                .sum(),
            reachable_files: candidate.reachable.values().sum(),
            validated_long_names: candidate.names,
            reason: reason.into(),
        });
    }
    let selected = &evidence[preferred];
    Ok(Some(ManagedSelection {
        directory: selected.path.clone(),
        schema_version: 1,
        attempt: number,
        source_sha256: selected.marker.source_sha256.clone(),
        preferred_directory: selected.path.file_name().unwrap().to_string_lossy().into(),
        candidates: choices,
        warning: "Same-acquisition generation preference only; byte hashes preserve multiplicity, not physical disk identity or customer completeness. Earlier and incomparable generations remain untouched. Different acquisitions are not merged or ranked by file count.",
    }))
}

pub(crate) fn record_selection(
    reports: &Path,
    disk: u32,
    selection: &ManagedSelection,
) -> Result<PathBuf, String> {
    fs::create_dir_all(reports).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(selection).map_err(|e| e.to_string())?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let path = reports.join(format!(
        "RecoverySelection-{disk:03}-{}.json",
        &digest[..16]
    ));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file
            .write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_file()
                || fs::read(&path).map_err(|e| e.to_string())? != bytes
            {
                return Err("Recovery selection snapshot changed; preserved".into());
            }
        }
        Err(e) => return Err(e.to_string()),
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fv-selection-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("Extracted/001")).unwrap();
        fs::create_dir_all(root.join("Images")).unwrap();
        fs::create_dir_all(root.join("Reports")).unwrap();
        fs::write(root.join("Images/001.img"), b"saved source").unwrap();
        root
    }
    fn generation(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let directory = root.join("Extracted/001").join(name);
        fs::create_dir_all(&directory).unwrap();
        for (path, bytes) in files {
            let path = directory.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let hash = sha256_file(&root.join("Images/001.img")).unwrap();
        let files = inventory_files(&directory).unwrap();
        let marker = ExtractionMarker {
            schema_version: 1,
            source_image: "001.img".into(),
            source_sha256: hash.clone(),
            extracted_unix_ms: 0,
            file_count: files.len(),
            total_bytes: files.iter().map(|f| f.bytes).sum(),
            native_recovery_report_sha256: None,
        };
        let inventory = ExtractionInventory {
            schema_version: 1,
            source_image: "001.img".into(),
            source_sha256: hash,
            files,
        };
        fs::write(
            directory.join(MARKER_FILE_NAME),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.join(INVENTORY_FILE_NAME),
            serde_json::to_vec(&inventory).unwrap(),
        )
        .unwrap();
        directory
    }
    #[test]
    fn superset_promotes_without_changing_prior_evidence_and_consumers_agree() {
        let root = fixture();
        let old = generation(&root, "attempt_001", &[("ALIAS.TXT", b"first")]);
        let new = generation(
            &root,
            "attempt_001_native_v4",
            &[("Readable name.txt", b"first"), ("extra.txt", b"second")],
        );
        let disk = root.join("Extracted/001");
        let selected = select_managed(&disk, Some(1)).unwrap().unwrap();
        assert_eq!(selected.directory, new);
        assert!(selected.candidates[1].reason.contains("adds readable"));
        assert!(
            matches!(inspect_extraction_presence(&root.join("Extracted"),1,1).unwrap(), ExtractionPresence::Automatic {output_directory,..} if output_directory == new)
        );
        let manifest = crate::manifest::build_manifest(
            &crate::manifest::ManifestRequest {
                extracted_root: root.join("Extracted"),
                images_directory: root.join("Images"),
                reports_directory: root.join("Reports"),
            },
            &|_| {},
        )
        .unwrap();
        assert_eq!(manifest.file_count, 2);
        let plan = crate::conversion::build_conversion_plan(
            &crate::conversion::ConversionPlanningRequest {
                extracted_root: root.join("Extracted"),
                converted_root: root.join("Converted"),
                reports_directory: root.join("Reports"),
            },
            &|_| {},
        )
        .unwrap();
        assert_eq!(plan.mirrored_files, 2);
        assert!(!root.join("Converted/001/ALIAS.TXT").exists());
        assert_eq!(fs::read(old.join("ALIAS.TXT")).unwrap(), b"first");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn regressive_incomparable_and_duplicate_losing_generations_remain_alternates() {
        let root = fixture();
        let old = generation(
            &root,
            "attempt_001",
            &[("a", b"same"), ("b", b"same"), ("c", b"other")],
        );
        generation(
            &root,
            "attempt_001_native_v2",
            &[("a", b"same"), ("c", b"other"), ("d", b"new")],
        );
        generation(
            &root,
            "attempt_001_native_v4",
            &[("a", b"same"), ("b", b"same"), ("d", b"new")],
        );
        let selected = select_managed(&root.join("Extracted/001"), None)
            .unwrap()
            .unwrap();
        assert_eq!(selected.directory, old);
        assert!(
            selected.candidates[1..]
                .iter()
                .all(|c| c.reason.contains("alternate"))
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reachable_payload_cannot_be_downgraded_to_reconstructed_or_hypothetical_only() {
        let root = fixture();
        let old = generation(&root, "attempt_001", &[("a", b"same")]);
        generation(
            &root,
            "attempt_001_native_v4",
            &[
                ("SignatureRecovery/a", b"same"),
                ("SignatureRecovery/b", b"extra"),
            ],
        );
        generation(
            &root,
            "attempt_001_native_v5",
            &[
                ("DirectoryRecovery/cluster_0002/a", b"same"),
                ("FragmentRecovery/alternative", b"extra"),
            ],
        );
        assert_eq!(
            select_managed(&root.join("Extracted/001"), Some(1))
                .unwrap()
                .unwrap()
                .directory,
            old
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn equivalent_generation_tie_break_and_snapshot_are_repeatable_not_overwritten() {
        let root = fixture();
        generation(&root, "attempt_001", &[("a", b"same")]);
        let new = generation(&root, "attempt_001_native_v4", &[("better name", b"same")]);
        let selected = select_managed(&root.join("Extracted/001"), Some(1))
            .unwrap()
            .unwrap();
        assert_eq!(selected.directory, new);
        let report = record_selection(&root.join("Reports"), 1, &selected).unwrap();
        assert_eq!(
            record_selection(&root.join("Reports"), 1, &selected).unwrap(),
            report
        );
        fs::write(&report, b"edited").unwrap();
        assert!(
            record_selection(&root.join("Reports"), 1, &selected)
                .unwrap_err()
                .contains("changed")
        );
        assert_eq!(fs::read(&report).unwrap(), b"edited");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tampered_alternate_or_source_refuses_selection_without_fallback() {
        let root = fixture();
        let old = generation(&root, "attempt_001", &[("a", b"same")]);
        generation(
            &root,
            "attempt_001_native_v4",
            &[("a", b"same"), ("b", b"extra")],
        );
        fs::write(old.join("a"), b"edit").unwrap();
        assert!(select_managed(&root.join("Extracted/001"), Some(1)).is_err());
        fs::write(old.join("a"), b"same").unwrap();
        fs::write(root.join("Images/001.img"), b"changed source").unwrap();
        assert!(
            select_managed(&root.join("Extracted/001"), Some(1))
                .unwrap_err()
                .contains("source image changed")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn explicit_acquisition_selection_never_unions_newer_scan_payloads() {
        let root = fixture();
        let old = generation(&root, "attempt_001", &[("a", b"same")]);
        generation(&root, "attempt_002", &[("b", b"different")]);
        assert_eq!(
            select_managed(&root.join("Extracted/001"), Some(1))
                .unwrap()
                .unwrap()
                .directory,
            old
        );
        fs::write(root.join("Extracted/001/operator.txt"), b"operator").unwrap();
        assert!(matches!(
            inspect_extraction_presence(&root.join("Extracted"), 1, 1).unwrap(),
            ExtractionPresence::ManualRecovery { .. }
        ));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn renamed_mirrors_are_quarantined_with_all_forensic_generations_intact() {
        let root = fixture();
        let old = generation(&root, "attempt_001", &[("ALIAS.TXT", b"same")]);
        let request = crate::conversion::ConversionPlanningRequest {
            extracted_root: root.join("Extracted"),
            converted_root: root.join("Converted"),
            reports_directory: root.join("Reports"),
        };
        crate::conversion::build_conversion_plan(&request, &|_| {}).unwrap();
        generation(
            &root,
            "attempt_001_native_v4",
            &[("Full filename.txt", b"same")],
        );
        let result = crate::conversion::build_conversion_plan(&request, &|_| {}).unwrap();
        assert_eq!(result.retired_mirrors, 1);
        assert_eq!(fs::read(old.join("ALIAS.TXT")).unwrap(), b"same");
        assert!(!root.join("Converted/001/ALIAS.TXT").exists());
        assert_eq!(
            fs::read(root.join("Converted/001/Full filename.txt")).unwrap(),
            b"same"
        );
        assert_eq!(result.cleanup_reports.len(), 1);
        assert_eq!(
            crate::conversion::build_conversion_plan(&request, &|_| {})
                .unwrap()
                .retired_mirrors,
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
}
