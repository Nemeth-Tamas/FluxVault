//! Guarded materialization of intact FAT12 files and validated signature candidates.
use crate::{
    carving,
    dmde_logs::DmdeLogStatus,
    extraction::{self, ExtractionPresence},
    fat12,
    imaging::AttemptSummary,
    legacy_logs::ArchiverLogStatus,
    recovery_plan,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub schema_version: u32,
    #[serde(default)]
    pub native_engine_version: u32,
    pub method: String,
    pub source_image: String,
    pub source_sha256: String,
    pub disk: u32,
    pub attempt: u32,
    pub analysis: Option<fat12::Analysis>,
    #[serde(default)]
    pub carving: Option<carving::Analysis>,
    #[serde(default)]
    pub filesystem_error: Option<String>,
    pub warning: String,
}

impl RecoveryReport {
    pub(crate) fn payloads(&self) -> Vec<(String, u64, String)> {
        let mut files = self
            .analysis
            .iter()
            .flat_map(|a| &a.recovered_files)
            .map(|f| (f.path.clone(), f.bytes as u64, f.sha256.clone()))
            .chain(
                self.carving
                    .iter()
                    .flat_map(|a| &a.files)
                    .map(|f| (f.path.clone(), f.bytes as u64, f.sha256.clone())),
            )
            .collect::<Vec<_>>();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }
    fn bad_lbas(&self) -> &[u64] {
        self.analysis
            .as_ref()
            .map(|a| a.bad_lbas.as_slice())
            .or_else(|| self.carving.as_ref().map(|a| a.bad_lbas.as_slice()))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryResult {
    pub native_engine_version: u32,
    pub layout_method: String,
    pub layout_warning: Option<String>,
    pub disk: u32,
    pub attempt: u32,
    pub output_directory: PathBuf,
    pub report_path: PathBuf,
    pub source_sha256: String,
    pub files: usize,
    pub carved_files: usize,
    pub orphan_chains_scanned: usize,
    pub rejected_candidates: usize,
    pub carving_warning: Option<String>,
    pub carving_limits_reached: bool,
    pub bytes: u64,
    pub skipped_entries: usize,
    pub validated_long_names: usize,
    pub name_fallbacks: usize,
    pub directory_gaps: Vec<u64>,
    pub reused: bool,
    pub customer_delivery_certified: bool,
}

pub fn recover_attempt(
    images_directory: &Path,
    extracted_root: &Path,
    recovery_root: &Path,
    disk: u32,
    attempt: &AttemptSummary,
    progress: &impl Fn(&str),
) -> Result<RecoveryResult, String> {
    if disk == 0 {
        return Err("Native recovery requires a positive disk number".into());
    }
    let images = images_directory.canonicalize().map_err(|e| e.to_string())?;
    let extracted = extracted_root.canonicalize().map_err(|e| e.to_string())?;
    let recovery = recovery_root.canonicalize().map_err(|e| e.to_string())?;
    let project = images.parent().ok_or("Invalid Images root")?;
    if extracted.parent() != Some(project)
        || recovery.parent() != Some(project)
        || images
            .file_name()
            .is_none_or(|n| !n.eq_ignore_ascii_case("Images"))
        || extracted
            .file_name()
            .is_none_or(|n| !n.eq_ignore_ascii_case("Extracted"))
        || recovery
            .file_name()
            .is_none_or(|n| !n.eq_ignore_ascii_case("Recovery"))
    {
        return Err("Native recovery requires regular sibling project directories".into());
    }
    let upper = project.to_string_lossy().to_ascii_uppercase();
    if ["A:\\", "B:\\", "\\\\?\\A:\\", "\\\\?\\B:\\"]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
    {
        return Err("Recovery refuses floppy drive-letter output".into());
    }
    if matches!(
        extraction::inspect_extraction_presence(&extracted, disk, attempt.attempt_number)?,
        ExtractionPresence::ManualRecovery { .. }
    ) {
        return Err("Operator recovery preserved; native recovery will not replace it".into());
    }
    let image = recovery_plan::resolve_image_path(&images, &attempt.image_file)?;
    let image = image.canonicalize().map_err(|e| e.to_string())?;
    if image.parent() != Some(images.as_path())
        || !fs::symlink_metadata(&image)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
    {
        return Err("Source image escapes Images or is not a regular file".into());
    }
    let size = fs::metadata(&image).map_err(|e| e.to_string())?.len();
    if size > fat12::MAX_IMAGE_BYTES as u64 || size < 512 {
        return Err("Unsupported native recovery image size".into());
    }
    let snapshot = read_snapshot(&image)?;
    let source_sha256 = hash(&snapshot);
    if attempt.sha256.len() != 64 || !attempt.sha256.eq_ignore_ascii_case(&source_sha256) {
        return Err("Image hash differs from saved acquisition; no native files published".into());
    }
    validate_sector_evidence(attempt, snapshot.len() / 512, &source_sha256)?;
    let disk_directory = extracted.join(format!("{disk:03}"));
    fs::create_dir_all(&disk_directory).map_err(|e| e.to_string())?;
    let disk_directory = disk_directory.canonicalize().map_err(|e| e.to_string())?;
    if disk_directory.parent() != Some(extracted.as_path()) {
        return Err("Extraction disk directory escapes project".into());
    }
    let recovery_disk = recovery.join(format!("{disk:03}"));
    fs::create_dir_all(&recovery_disk).map_err(|e| e.to_string())?;
    let recovery_disk = recovery_disk.canonicalize().map_err(|e| e.to_string())?;
    if recovery_disk.parent() != Some(recovery.as_path()) {
        return Err("Recovery disk directory escapes project".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(extracted.join(format!(".fluxvault-native-{disk:03}.lock")))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "Native recovery already running for this disk")?;
    let folder = if attempt.attempt_number == 0 {
        format!("legacy_native_v{}", fat12::RECOVERY_ENGINE_VERSION)
    } else {
        format!(
            "attempt_{:03}_native_v{}",
            attempt.attempt_number,
            fat12::RECOVERY_ENGINE_VERSION
        )
    };
    let output = disk_directory.join(folder);
    let report_path = recovery_disk.join(format!(
        "attempt_{:03}_fat12_v{}.json",
        attempt.attempt_number,
        fat12::RECOVERY_ENGINE_VERSION
    ));
    if output.exists() {
        let output = output.canonicalize().map_err(|e| e.to_string())?;
        if output.parent() != Some(disk_directory.as_path()) {
            return Err("Native output escapes disk directory".into());
        }
        extraction::verify_managed_extraction(&output, &source_sha256)?;
        let internal = fs::read(output.join(extraction::FAT12_REPORT_NAME))
            .map_err(|_| "Existing extraction is not native recovery; left unchanged")?;
        let report: RecoveryReport =
            serde_json::from_slice(&internal).map_err(|e| e.to_string())?;
        let mut expected_bad = attempt.bad_sectors.clone();
        expected_bad.sort_unstable();
        if report.disk != disk
            || report.attempt != attempt.attempt_number
            || report.native_engine_version != fat12::RECOVERY_ENGINE_VERSION
            || report.bad_lbas() != expected_bad
        {
            return Err("Native recovery evidence/settings changed; reuse refused".into());
        }
        save_report_copy(&report_path, &internal)?;
        return Ok(result(&report, output, report_path, true));
    }
    progress("Native FAT12: checking directories, FAT copies and intact file chains...");
    let (analysis, filesystem_error) = match fat12::analyze(&snapshot, &attempt.bad_sectors) {
        Ok(a) => (Some(a), None),
        Err(error) => (None, Some(error)),
    };
    if let Some(warning) = analysis
        .as_ref()
        .and_then(|a| a.layout_evidence.warning.as_ref())
    {
        progress(&format!("Native FAT12 layout WARNING: {warning}"));
    }
    progress(
        "Native recovery: validating orphan-chain/signature candidates from readable image bytes...",
    );
    let (regions, chains, issues, hashes) = if let Some(a) = &analysis {
        let (mut regions, chains, issues) = fat12::orphan_regions(&snapshot, a);
        regions.extend(fat12::partial_file_regions(a));
        (
            regions,
            chains,
            issues,
            a.recovered_files
                .iter()
                .map(|f| f.sha256.clone())
                .collect::<Vec<_>>(),
        )
    } else {
        (
            carving::raw_readable_regions(&snapshot, &attempt.bad_sectors),
            0,
            vec![],
            vec![],
        )
    };
    let mut carved = carving::analyze(&snapshot, &attempt.bad_sectors, &regions, &hashes)?;
    carved.orphan_chains_scanned = chains;
    carved.allocation_issues = issues;
    carved.bad_lbas.sort_unstable();
    if carved.limits_reached {
        progress(
            "Native recovery WARNING: signature validation work ceiling reached; this search is not exhaustive.",
        );
    }
    if analysis.is_none() || !carved.files.is_empty() {
        carved.allocation_warning = Some("Reconstructed candidate names; original live/deleted ownership unknown. Known deleted chains and free clusters are excluded when FAT layout is readable; unknown filesystems cannot establish deletion status. Structural validation is not original-content/customer certification.".into());
    }
    let report = RecoveryReport { schema_version:1, native_engine_version:fat12::RECOVERY_ENGINE_VERSION, method:"native_fat12_and_validated_carving".into(),
        source_image:image.file_name().unwrap_or_default().to_string_lossy().to_string(), source_sha256:source_sha256.clone(),
        disk, attempt:attempt.attempt_number, analysis, carving:Some(carved), filesystem_error,
        warning:"Recovered bytes came only from acquisition-reported readable sectors. Layout hypotheses never fabricate boot bytes. Carved names/paths are reconstructed, not original; recorded extents bind every payload to the saved image. No missing bytes are guessed or joined across holes. Known deleted/free allocation is excluded when readable FAT metadata exists; raw fallback cannot determine live/deleted status. Pixel/CRC/container validation does not prove document semantics or original custody. Single-capture confidence is unchanged; disk/customer completeness remains unverified.".into() };
    let staging = extracted.join(format!(
        ".tmp-native-{disk:03}-{}-{}",
        std::process::id(),
        crate::external_tools::current_unix_ms()
    ));
    fs::create_dir(&staging).map_err(|e| format!("Cannot reserve native recovery staging: {e}"))?;
    for record in report.analysis.iter().flat_map(|a| &a.recovered_files) {
        let path = safe_file_path(&staging, &record.path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        write_new(&path, &fat12::file_bytes(&snapshot, record))?;
    }
    for record in report.carving.iter().flat_map(|a| &a.files) {
        let path = safe_file_path(&staging, &record.path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        write_new(&path, &carving::file_bytes(&snapshot, record)?)?;
    }
    let serialized = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    write_new(&staging.join(extraction::FAT12_REPORT_NAME), &serialized)?;
    extraction::write_native_managed_metadata(&staging, &image, &source_sha256, hash(&serialized))?;
    if hash(&read_snapshot(&image)?) != source_sha256 {
        return Err("Source changed during recovery; unpublished staging retained".into());
    }
    // Destination was not present: never replace another managed/manual folder.
    if output.exists() {
        return Err("Recovery destination appeared during extraction; staging retained".into());
    }
    fs::rename(&staging, &output)
        .map_err(|e| format!("Native staging retained; cannot publish: {e}"))?;
    save_report_copy(&report_path, &serialized)?;
    progress(&format!(
        "Native recovery: {} intact reachable files, {} validated signature candidates; {} rejected candidates; completeness remains unverified",
        report
            .analysis
            .as_ref()
            .map_or(0, |a| a.recovered_files.len()),
        report.carving.as_ref().map_or(0, |a| a.files.len()),
        report.carving.as_ref().map_or(0, |a| a.rejected.len())
    ));
    Ok(result(&report, output, report_path, false))
}

fn result(
    report: &RecoveryReport,
    output_directory: PathBuf,
    report_path: PathBuf,
    reused: bool,
) -> RecoveryResult {
    RecoveryResult {
        native_engine_version: report.native_engine_version,
        layout_method: report.analysis.as_ref().map_or_else(
            || "signature_only_unknown_filesystem".into(),
            |a| a.layout_evidence.method.clone(),
        ),
        layout_warning: report
            .analysis
            .as_ref()
            .and_then(|a| a.layout_evidence.warning.clone())
            .or_else(|| report.filesystem_error.clone()),
        disk: report.disk,
        attempt: report.attempt,
        output_directory,
        report_path,
        source_sha256: report.source_sha256.clone(),
        files: report.payloads().len(),
        carved_files: report.carving.as_ref().map_or(0, |a| a.files.len()),
        orphan_chains_scanned: report
            .carving
            .as_ref()
            .map_or(0, |a| a.orphan_chains_scanned),
        rejected_candidates: report.carving.as_ref().map_or(0, |a| a.rejected.len()),
        carving_warning: report.carving.as_ref().and_then(|a| {
            a.allocation_warning.clone().or_else(|| {
                a.limits_reached
                    .then(|| "Signature work ceiling reached; search is not exhaustive.".into())
            })
        }),
        carving_limits_reached: report.carving.as_ref().is_some_and(|a| a.limits_reached),
        bytes: report.payloads().iter().map(|f| f.1).sum(),
        skipped_entries: report.analysis.as_ref().map_or(0, |a| a.skipped.len()),
        validated_long_names: report
            .analysis
            .as_ref()
            .map_or(0, |a| a.validated_long_names.len()),
        name_fallbacks: report
            .analysis
            .as_ref()
            .map_or(0, |a| a.name_fallbacks.len()),
        directory_gaps: report
            .analysis
            .as_ref()
            .map_or_else(Vec::new, |a| a.directory_gaps.clone()),
        reused,
        customer_delivery_certified: false,
    }
}

pub(crate) fn validate_sector_evidence(
    attempt: &AttemptSummary,
    sectors: usize,
    sha256: &str,
) -> Result<(), String> {
    if attempt.total_sectors != sectors || !matches!(attempt.status.as_str(), "OK" | "PARTIAL") {
        return Err("Acquisition is unfinished/unknown or geometry differs from image".into());
    }
    let mut expected = attempt.bad_sectors.clone();
    expected.sort_unstable();
    if let Some(log) = &attempt.parsed_log {
        let mut actual = log.bad_sectors.clone();
        actual.sort_unstable();
        if !log.end_seen
            || !matches!(
                log.status,
                ArchiverLogStatus::Ok | ArchiverLogStatus::Partial
            )
            || log.geometry.bytes_per_sector != Some(512)
            || log.geometry.total_sectors != Some(sectors as u64)
            || log
                .sha256
                .as_ref()
                .is_some_and(|h| !h.eq_ignore_ascii_case(sha256))
            || actual != expected
        {
            return Err(
                "Acquisition log/map/hash does not establish complete sector evidence".into(),
            );
        }
    } else if let Some(log) = &attempt.parsed_dmde_log {
        let mut actual = log.bad_sectors.clone();
        actual.sort_unstable();
        if log.status == DmdeLogStatus::InProgress
            || log.sector_size != Some(512)
            || log.highest_sector_exclusive != sectors as u64
            || log.copied_sectors + actual.len() != sectors
            || actual != expected
        {
            return Err("DMDE log does not cover every image sector; unknown bytes will not be treated as readable".into());
        }
    } else {
        return Err(
            "Native recovery requires recognized, completed acquisition sector evidence".into(),
        );
    }
    Ok(())
}

fn safe_file_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty()
        || relative.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.chars()
                    .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
                || p.ends_with([' ', '.'])
        })
    {
        return Err("Unsafe native recovery filename".into());
    }
    Ok(root.join(relative))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_snapshot(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(fat12::MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > fat12::MAX_IMAGE_BYTES || !bytes.len().is_multiple_of(512) {
        return Err("Image size changed or is not a supported complete sector image".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("Cannot reserve {}: {e}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
fn save_report_copy(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        if !fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
            || fs::read(path).map_err(|e| e.to_string())? != bytes
        {
            return Err("Existing native report differs; preserved, not overwritten".into());
        }
        Ok(())
    } else {
        write_new(path, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audit, batch_extraction, conversion, imaging, project::ProjectState};

    fn rebind_fixture(
        project: &ProjectState,
        attempt: &AttemptSummary,
        image: &[u8],
    ) -> AttemptSummary {
        let new_hash = hash(image);
        fs::write(project.images_dir().join(&attempt.image_file), image).unwrap();
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(&attempt.metadata_path).unwrap()).unwrap();
        metadata["sha256"] = serde_json::json!(new_hash);
        fs::write(
            &attempt.metadata_path,
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let log = fs::read_to_string(&attempt.log_file)
            .unwrap()
            .replace(&attempt.sha256, &new_hash);
        fs::write(&attempt.log_file, log).unwrap();
        imaging::load_attempts_for_disk(&project.images_dir(), 1)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn automatic_raw_fallback_handles_severe_damage_and_keeps_delivery_separate() {
        let bad = (0..400).collect::<Vec<u64>>();
        let (project, attempt, mut image) = fixture(&bad);
        let payload = b"{\\rtf1 recovered from readable bytes}";
        image[800 * 512..800 * 512 + payload.len()].copy_from_slice(payload);
        let attempt = rebind_fixture(&project, &attempt, &image);
        let request = batch_extraction::BatchExtractionRequest {
            seven_zip_executable: std::env::current_exe().unwrap(),
            images_directory: project.images_dir(),
            logs_directory: project.logs_dir(),
            extracted_root: project.extracted_dir(),
            recovery_root: project.recovery_dir(),
            reports_directory: project.reports_dir(),
            command_audit_path: project.logs_dir().join("external-tools.jsonl"),
        };
        let batch = batch_extraction::run_single_disk_extraction(&request, 1, &|_| {}).unwrap();
        assert_eq!(batch.file_count, 1);
        let result = recover(&project, &attempt).unwrap();
        assert!(result.reused);
        assert_eq!(result.carved_files, 1);
        let report: RecoveryReport =
            serde_json::from_slice(&fs::read(&result.report_path).unwrap()).unwrap();
        assert!(report.analysis.is_none());
        let carving = report.carving.unwrap();
        assert_eq!(carving.bad_lbas.len(), 400);
        assert!(carving.allocation_warning.is_some());
        assert_eq!(
            carving.files[0].source_extents[0].source_byte_offset,
            800 * 512
        );
        assert_eq!(
            fs::read(result.output_directory.join(&carving.files[0].path)).unwrap(),
            payload
        );
        let plan = conversion::build_conversion_plan(&planning(&project), &|_| {}).unwrap();
        let mapping = fs::read_to_string(&plan.path_map).unwrap();
        assert!(mapping.contains("Signature recovered"));
        assert!(mapping.contains("Signature-Recovered"));
        assert!(!mapping.contains("Native FAT12 readable-chain"));
        let audit = audit::run_audit(&project, &|_| {}).unwrap();
        assert_eq!(audit.verified_disks, 0);
        assert_eq!(audit.attention_disks, 1);
        assert!(!request.command_audit_path.exists());
        assert_eq!(
            fs::read(project.images_dir().join(&attempt.image_file)).unwrap(),
            image
        );
        fs::write(
            result.output_directory.join(&carving.files[0].path),
            b"changed candidate",
        )
        .unwrap();
        assert!(recover(&project, &attempt).is_err());
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn zero_candidate_unknown_filesystem_publishes_reusable_failure_evidence_not_files() {
        let (project, attempt, _) = fixture(&[34]);
        let image = vec![0; 2880 * 512];
        let attempt = rebind_fixture(&project, &attempt, &image);
        let result = recover(&project, &attempt).unwrap();
        assert_eq!(result.files, 0);
        assert!(recover(&project, &attempt).unwrap().reused);
        let report: RecoveryReport =
            serde_json::from_slice(&fs::read(&result.report_path).unwrap()).unwrap();
        assert!(report.analysis.is_none() && report.filesystem_error.is_some());
        assert!(!report.carving.unwrap().limits_reached);
        assert!(
            extraction::verify_managed_extraction(&result.output_directory, &attempt.sha256)
                .is_ok()
        );
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn carving_work_limit_warning_survives_reuse_even_without_exported_candidates() {
        let (project, attempt, mut image) = fixture(&[34]);
        for cluster in 10..=15 {
            for copy in 0..2 {
                fat12::tests::set_fat(
                    &mut image,
                    copy,
                    cluster,
                    if cluster == 15 { 0xfff } else { cluster + 1 },
                );
            }
        }
        for at in (41 * 512..47 * 512).step_by(2) {
            image[at..at + 2].copy_from_slice(b"BM");
        }
        let attempt = rebind_fixture(&project, &attempt, &image);
        let first = recover(&project, &attempt).unwrap();
        assert_eq!(first.carved_files, 0);
        assert!(first.carving_limits_reached);
        assert!(
            first
                .carving_warning
                .as_ref()
                .unwrap()
                .contains("not exhaustive")
        );
        let reused = recover(&project, &attempt).unwrap();
        assert!(reused.reused);
        assert_eq!(reused.carving_warning, first.carving_warning);
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn intact_embedded_candidate_survives_partial_parent_without_exporting_parent_or_slack() {
        let (project, attempt, mut image) = fixture(&[34]);
        fat12::tests::entry(&mut image, 19 * 512, b"PARTIAL BIN", 2, 1024 + 20, false);
        image[19 * 512 + 32..19 * 512 + 64].fill(0);
        for copy in 0..2 {
            fat12::tests::set_fat(&mut image, copy, 2, 3);
            fat12::tests::set_fat(&mut image, copy, 3, 4);
            fat12::tests::set_fat(&mut image, copy, 4, 0xfff);
        }
        let good = b"{\\rtf1 embedded}";
        image[35 * 512..35 * 512 + good.len()].copy_from_slice(good);
        let slack = b"{\\rtf1 slack not a file}";
        image[35 * 512 + 20..35 * 512 + 20 + slack.len()].copy_from_slice(slack);
        let attempt = rebind_fixture(&project, &attempt, &image);
        let result = recover(&project, &attempt).unwrap();
        assert_eq!(result.files, 1);
        assert_eq!(result.carved_files, 1);
        let report: RecoveryReport =
            serde_json::from_slice(&fs::read(&result.report_path).unwrap()).unwrap();
        let a = report.analysis.unwrap();
        assert!(a.recovered_files.is_empty());
        assert_eq!(a.unrecovered_files[0].unreadable_ranges[0].file_offset, 512);
        let carved = report.carving.unwrap();
        assert_eq!(carved.files[0].parent_file.as_deref(), Some("PARTIAL.BIN"));
        assert_eq!(
            carved.files[0].recovery_method,
            "readable_partial_file_embedded_signature"
        );
        assert_eq!(
            fs::read(result.output_directory.join(&carved.files[0].path)).unwrap(),
            good
        );
        assert!(!result.output_directory.join("PARTIAL.BIN").exists());
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    #[ignore = "requires FLUXVAULT_CARVING_SOURCE_PROJECT and FLUXVAULT_CARVING_OUTPUT; saved images copied to a NEW project only"]
    fn saved_cohort_native_carving_uses_isolated_images_and_preserves_every_source() {
        let source = ProjectState::open_without_session(PathBuf::from(
            std::env::var("FLUXVAULT_CARVING_SOURCE_PROJECT").unwrap(),
        ))
        .unwrap();
        let output = PathBuf::from(std::env::var("FLUXVAULT_CARVING_OUTPUT").unwrap());
        assert!(!output.exists());
        let project = ProjectState::create_without_session(output).unwrap();
        let stats = imaging::load_project_statistics(&source.images_dir()).unwrap();
        let mut rows = vec![];
        for disk in stats.disks {
            let attempts =
                imaging::load_attempts_for_disk(&source.images_dir(), disk.disk_number).unwrap();
            let mut attempt = attempts
                .into_iter()
                .find(|a| a.attempt_number == disk.best_attempt_number)
                .unwrap();
            let original =
                recovery_plan::resolve_image_path(&source.images_dir(), &attempt.image_file)
                    .unwrap();
            let before = read_snapshot(&original).unwrap();
            attempt.image_file = original.file_name().unwrap().to_string_lossy().into();
            fs::copy(&original, project.images_dir().join(&attempt.image_file)).unwrap();
            // Keep the isolated project usable by ordinary offline CLI commands.
            if !attempt.log_file.is_empty() {
                let log = Path::new(&attempt.log_file);
                let copied = project.logs_dir().join(log.file_name().unwrap());
                fs::copy(log, &copied).unwrap();
                attempt.log_file = copied.to_string_lossy().into();
            }
            if !attempt.legacy_image {
                let mut metadata: serde_json::Value =
                    serde_json::from_slice(&fs::read(&attempt.metadata_path).unwrap()).unwrap();
                metadata["image_file"] = serde_json::json!(attempt.image_file);
                metadata["log_file"] = serde_json::json!(attempt.log_file);
                attempt.metadata_path = project
                    .images_dir()
                    .join(attempt.metadata_path.file_name().unwrap());
                fs::write(
                    &attempt.metadata_path,
                    serde_json::to_vec_pretty(&metadata).unwrap(),
                )
                .unwrap();
            }
            let result = recover_attempt(
                &project.images_dir(),
                &project.extracted_dir(),
                &project.recovery_dir(),
                disk.disk_number,
                &attempt,
                &|s| eprintln!("{s}"),
            )
            .unwrap();
            assert!(
                extraction::verify_managed_extraction(&result.output_directory, &attempt.sha256)
                    .is_ok()
            );
            assert_eq!(read_snapshot(&original).unwrap(), before);
            eprintln!(
                "DISK {:03}: {} payloads / {} carved / {} orphan chains / {} rejected",
                disk.disk_number,
                result.files,
                result.carved_files,
                result.orphan_chains_scanned,
                result.rejected_candidates
            );
            rows.push(serde_json::to_value(&result).unwrap());
        }
        fs::write(
            project.reports_dir().join("NativeCarvingValidation.json"),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
        eprintln!(
            "Isolated recovery validation retained: {}",
            project.root().display()
        );
    }

    fn fixture(bad: &[u64]) -> (ProjectState, AttemptSummary, Vec<u8>) {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-native-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root).unwrap();
        let mut bytes = fat12::tests::image();
        fat12::tests::file(
            &mut bytes,
            19 * 512,
            b"GOOD    TXT",
            2,
            b"known intact bytes",
        );
        fat12::tests::file(
            &mut bytes,
            19 * 512 + 32,
            b"BROKEN  TXT",
            3,
            b"must not invent this file",
        );
        for lba in bad {
            bytes[*lba as usize * 512..(*lba as usize + 1) * 512].fill(0);
        }
        let sha = hash(&bytes);
        let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
        let log = project.logs_dir().join("001_attempt_001.log");
        let mut text = "BEGIN | disk=1 | attempt=1\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\n".to_owned();
        for lba in bad {
            text.push_str(&format!("BAD_SECTOR | lba={lba}\n"));
        }
        text.push_str(&format!(
            "END | status={status} | bytes=1474560 | sha256={sha}\n"
        ));
        fs::write(&log, text).unwrap();
        fs::write(project.images_dir().join("001_attempt_001.img"), &bytes).unwrap();
        let metadata = serde_json::json!({
            "fluxvault_version":"test", "status":status, "disk_number":1, "attempt_number":1,
            "source_backend":"synthetic-test", "source_device":"none", "image_file":"001_attempt_001.img",
            "log_file":log, "timestamp_unix_ms":1,
            "geometry":{"cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":1474560,"format_guess":"FAT12 fixture"},
            "sector_retries":0,"total_sectors":2880,"bytes_written":1474560,"retry_recovered_sectors":0,
            "bad_sector_count":bad.len(),"bad_sectors":bad.iter().map(|lba| serde_json::json!({"lba":lba,"cylinder":lba/36,"head":lba/18%2,"sector":lba%18+1})).collect::<Vec<_>>(),"sha256":sha
        });
        fs::write(
            project.images_dir().join("001_attempt_001.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let attempt = imaging::load_attempts_for_disk(&project.images_dir(), 1)
            .unwrap()
            .remove(0);
        (project, attempt, bytes)
    }
    fn recover(project: &ProjectState, attempt: &AttemptSummary) -> Result<RecoveryResult, String> {
        recover_attempt(
            &project.images_dir(),
            &project.extracted_dir(),
            &project.recovery_dir(),
            1,
            attempt,
            &|_| {},
        )
    }
    fn planning(project: &ProjectState) -> conversion::ConversionPlanningRequest {
        conversion::ConversionPlanningRequest {
            extracted_root: project.extracted_dir(),
            converted_root: project.converted_dir(),
            reports_directory: project.reports_dir(),
        }
    }
    #[test]
    fn partial_image_recovers_only_intact_files_and_reuses_verified_inventory() {
        let (project, attempt, bytes) = fixture(&[34]);
        let recovered = recover(&project, &attempt).unwrap();
        assert_eq!(recovered.files, 1);
        assert_eq!(recovered.skipped_entries, 1);
        assert!(!recovered.customer_delivery_certified);
        assert_eq!(
            fs::read(recovered.output_directory.join("GOOD.TXT")).unwrap(),
            b"known intact bytes"
        );
        assert!(!recovered.output_directory.join("BROKEN.TXT").exists());
        assert!(recovered.report_path.is_file());
        let report: RecoveryReport =
            serde_json::from_slice(&fs::read(&recovered.report_path).unwrap()).unwrap();
        assert_eq!(
            report.analysis.as_ref().unwrap().recovered_files[0].data_lbas,
            vec![33]
        );
        assert!(recover(&project, &attempt).unwrap().reused);
        assert_eq!(
            fs::read(project.images_dir().join(&attempt.image_file)).unwrap(),
            bytes
        );
        fs::write(recovered.output_directory.join("GOOD.TXT"), b"tampered").unwrap();
        assert!(recover(&project, &attempt).is_err());
        assert!(conversion::build_conversion_plan(&planning(&project), &|_| {}).is_err());
        fs::remove_dir_all(project.root()).unwrap();
    }
    #[test]
    fn recovery_reports_and_acquisition_map_cannot_be_silently_replaced() {
        let (project, mut attempt, _) = fixture(&[34]);
        let recovered = recover(&project, &attempt).unwrap();
        fs::write(&recovered.report_path, b"different report").unwrap();
        assert!(recover(&project, &attempt).is_err());
        assert_eq!(
            fs::read(&recovered.report_path).unwrap(),
            b"different report"
        );
        fs::remove_file(&recovered.report_path).unwrap();
        assert!(recover(&project, &attempt).unwrap().reused);
        let internal = recovered
            .output_directory
            .join(extraction::FAT12_REPORT_NAME);
        fs::write(internal, b"tampered provenance").unwrap();
        assert!(recover(&project, &attempt).is_err());
        attempt.bad_sectors.clear();
        assert!(recover(&project, &attempt).is_err());
        fs::remove_dir_all(project.root()).unwrap();
    }
    #[test]
    fn uncertain_maps_changed_sources_and_operator_files_are_preserved() {
        let (project, mut attempt, bytes) = fixture(&[34]);
        let original = attempt.clone();
        attempt.parsed_log = None;
        assert!(
            recover(&project, &attempt)
                .unwrap_err()
                .contains("sector evidence")
        );
        attempt = original.clone();
        attempt.parsed_log.as_mut().unwrap().end_seen = false;
        assert!(recover(&project, &attempt).is_err());
        attempt = original.clone();
        attempt.sha256 = "0".repeat(64);
        assert!(
            recover(&project, &attempt)
                .unwrap_err()
                .contains("hash differs")
        );
        let manual = project.extracted_dir().join("001");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("manual.txt"), b"operator content").unwrap();
        assert!(
            recover(&project, &original)
                .unwrap_err()
                .contains("Operator recovery preserved")
        );
        assert_eq!(
            fs::read(manual.join("manual.txt")).unwrap(),
            b"operator content"
        );
        assert_eq!(
            fs::read(project.images_dir().join(&original.image_file)).unwrap(),
            bytes
        );
        fs::remove_dir_all(project.root()).unwrap();
    }
    #[test]
    fn partial_native_extraction_flows_through_batch_manifest_conversion_and_audit() {
        let (project, _, bytes) = fixture(&[0, 34]);
        let request = batch_extraction::BatchExtractionRequest {
            seven_zip_executable: project.root().join("must-not-be-invoked.exe"),
            images_directory: project.images_dir(),
            logs_directory: project.logs_dir(),
            extracted_root: project.extracted_dir(),
            recovery_root: project.recovery_dir(),
            reports_directory: project.reports_dir(),
            command_audit_path: project.logs_dir().join("external-tools.jsonl"),
        };
        fs::write(&request.seven_zip_executable, []).unwrap();
        let first = batch_extraction::run_single_disk_extraction(&request, 1, &|_| {}).unwrap();
        assert_eq!(first.status, "partial_recovered");
        assert_eq!(first.file_count, 1);
        assert!(
            fs::read_to_string(&first.manifest_path)
                .unwrap()
                .contains("GOOD.TXT")
        );
        assert!(
            !fs::read_to_string(&first.manifest_path)
                .unwrap()
                .contains("BROKEN.TXT")
        );
        assert!(project.recovery_dir().join("001/pass1").is_dir());
        assert!(!request.command_audit_path.exists());
        let second = batch_extraction::run_single_disk_extraction(&request, 1, &|_| {}).unwrap();
        assert!(second.reused);
        let conversion = conversion::build_conversion_plan(&planning(&project), &|_| {}).unwrap();
        assert_eq!(conversion.mirrored_files, 1);
        assert!(
            fs::read_to_string(conversion.path_map)
                .unwrap()
                .contains("Native FAT12")
        );
        let audit = audit::run_audit(&project, &|_| {}).unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(audit.json_path).unwrap()).unwrap();
        assert_eq!(
            report["disks"][0]["extraction_status"],
            "PARTIAL_VERIFIED_FILES"
        );
        assert_eq!(report["disks"][0]["extracted_hashes_verified"], true);
        assert_eq!(audit.verified_disks, 0);
        assert_eq!(audit.attention_disks, 1);
        assert_eq!(
            fs::read(project.images_dir().join("001_attempt_001.img")).unwrap(),
            bytes
        );
        fs::remove_dir_all(project.root()).unwrap();
    }
    #[test]
    fn native_recovery_does_not_certify_even_a_readable_image_or_replace_sevenzip_output() {
        let (project, attempt, _) = fixture(&[]);
        let recovered = recover(&project, &attempt).unwrap();
        assert_eq!(recovered.files, 2);
        let audit = audit::run_audit(&project, &|_| {}).unwrap();
        assert_eq!(audit.verified_disks, 0);
        let old = project.extracted_dir().join("001/attempt_001");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("old.txt"), b"previous extraction").unwrap();
        assert!(recover(&project, &attempt).unwrap().reused);
        assert_eq!(
            fs::read(old.join("old.txt")).unwrap(),
            b"previous extraction"
        );
        assert!(
            recovered
                .output_directory
                .ends_with("attempt_001_native_v4")
        );
        fs::remove_dir_all(project.root()).unwrap();
    }
    #[test]
    fn upgrading_native_generation_preserves_old_files_reports_and_schema_compatibility() {
        let (project, attempt, _) = fixture(&[34]);
        let initial = recover(&project, &attempt).unwrap();
        let old = project.extracted_dir().join("001/attempt_001_native");
        fs::rename(&initial.output_directory, &old).unwrap();
        let mut report: serde_json::Value =
            serde_json::from_slice(&fs::read(&initial.report_path).unwrap()).unwrap();
        report
            .as_object_mut()
            .unwrap()
            .remove("native_engine_version");
        report["analysis"]
            .as_object_mut()
            .unwrap()
            .remove("validated_long_names");
        report["analysis"]
            .as_object_mut()
            .unwrap()
            .remove("name_fallbacks");
        report["analysis"]["recovered_files"][0]
            .as_object_mut()
            .unwrap()
            .remove("long_name");
        report["analysis"]["recovered_files"][0]
            .as_object_mut()
            .unwrap()
            .remove("short_name_case_flags");
        let old_bytes = serde_json::to_vec_pretty(&report).unwrap();
        fs::write(old.join(extraction::FAT12_REPORT_NAME), &old_bytes).unwrap();
        extraction::write_native_managed_metadata(
            &old,
            &project.images_dir().join(&attempt.image_file),
            &attempt.sha256,
            hash(&old_bytes),
        )
        .unwrap();
        let old_report = project.recovery_dir().join("001/attempt_001_fat12.json");
        fs::rename(&initial.report_path, &old_report).unwrap();
        fs::write(&old_report, &old_bytes).unwrap();
        let old_marker = fs::read(old.join(".fluxvault-extraction.json")).unwrap();
        assert!(extraction::verify_managed_extraction(&old, &attempt.sha256).is_ok());
        let upgraded = recover(&project, &attempt).unwrap();
        assert!(!upgraded.reused);
        assert_eq!(
            upgraded.native_engine_version,
            fat12::RECOVERY_ENGINE_VERSION
        );
        assert_ne!(upgraded.output_directory, old);
        assert_eq!(fs::read(&old_report).unwrap(), old_bytes);
        assert_eq!(
            fs::read(old.join(".fluxvault-extraction.json")).unwrap(),
            old_marker
        );
        assert_eq!(
            fs::read(old.join("GOOD.TXT")).unwrap(),
            b"known intact bytes"
        );
        assert!(recover(&project, &attempt).unwrap().reused);
        let presence =
            extraction::inspect_extraction_presence(&project.extracted_dir(), 1, 1).unwrap();
        assert!(
            matches!(presence, ExtractionPresence::Automatic { output_directory, .. } if output_directory == upgraded.output_directory || output_directory.canonicalize().unwrap() == upgraded.output_directory)
        );
        fs::remove_dir_all(project.root()).unwrap();
    }
    #[test]
    fn validated_os_metadata_stays_forensic_but_is_excluded_from_delivery() {
        let (project, _, mut bytes) = fixture(&[34]);
        let root = 19 * 512;
        bytes[root..33 * 512].fill(0);
        let folder_alias = b"SYSTEM~1   ";
        let at =
            fat12::tests::long_name(&mut bytes, root, folder_alias, "System Volume Information");
        fat12::tests::entry(&mut bytes, at, folder_alias, 4, 0, true);
        for copy in 0..2 {
            fat12::tests::set_fat(&mut bytes, copy, 4, 0xfff);
        }
        let file_alias = b"INDEXE~1   ";
        let at = fat12::tests::long_name(&mut bytes, 35 * 512, file_alias, "IndexerVolumeGuid");
        fat12::tests::file(&mut bytes, at, file_alias, 5, b"Windows metadata");
        let at =
            fat12::tests::long_name(&mut bytes, root + 3 * 32, b"DOCUME~1TXT", "Árvíztűrő.txt");
        fat12::tests::file(&mut bytes, at, b"DOCUME~1TXT", 2, b"customer text");
        let sha = hash(&bytes);
        fs::write(project.images_dir().join("001_attempt_001.img"), &bytes).unwrap();
        let metadata_path = project.images_dir().join("001_attempt_001.json");
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        metadata["sha256"] = serde_json::json!(sha);
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let log = project.logs_dir().join("001_attempt_001.log");
        let mut text = fs::read_to_string(&log).unwrap();
        let original_sha = text.rsplit("sha256=").next().unwrap().trim().to_owned();
        text = text.replace(&original_sha, &sha);
        fs::write(log, text).unwrap();
        let attempt = imaging::load_attempts_for_disk(&project.images_dir(), 1)
            .unwrap()
            .remove(0);
        let recovered = recover(&project, &attempt).unwrap();
        assert_eq!(recovered.files, 2);
        assert_eq!(recovered.validated_long_names, 3);
        assert!(
            recovered
                .output_directory
                .join("System Volume Information/IndexerVolumeGuid")
                .is_file()
        );
        assert_eq!(
            fs::read(recovered.output_directory.join("Árvíztűrő.txt")).unwrap(),
            b"customer text"
        );
        let plan = conversion::build_conversion_plan(&planning(&project), &|_| {}).unwrap();
        assert_eq!(plan.mirrored_files, 1);
        assert!(project.converted_dir().join("001/Árvíztűrő.txt").is_file());
        assert!(
            !project
                .converted_dir()
                .join("001/System Volume Information")
                .exists()
        );
        let audit = audit::run_audit(&project, &|_| {}).unwrap();
        assert_eq!(audit.attention_disks, 1);
        let audit: serde_json::Value =
            serde_json::from_slice(&fs::read(audit.json_path).unwrap()).unwrap();
        assert_eq!(audit["disks"][0]["extracted_files"], 2);
        assert_eq!(audit["disks"][0]["extracted_hashes_verified"], true);
        assert_eq!(
            fs::read(project.images_dir().join("001_attempt_001.img")).unwrap(),
            bytes
        );
        let destination = project.root().with_file_name(format!(
            "{}-delivery",
            project.root().file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir(&destination).unwrap();
        let package = crate::package::build_package(
            &crate::package::PackageRequest {
                project_root: project.root().to_path_buf(),
                destination: destination.clone(),
                project_name: "native long names".into(),
            },
            &|_| {},
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(fs::File::open(package.zip_path).unwrap()).unwrap();
        assert!(
            archive
                .by_name("Extracted/001/attempt_001_native_v4/Árvíztűrő.txt")
                .is_ok()
        );
        assert!(archive.by_name("Extracted/001/attempt_001_native_v4/System Volume Information/IndexerVolumeGuid").is_err());
        assert!(
            archive
                .by_name("Recovery/001/attempt_001_fat12_v4.json")
                .is_ok()
        );
        drop(archive);
        fs::remove_dir_all(destination).unwrap();
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn missing_boot_recovery_is_managed_reusable_and_keeps_image_hash_and_attention() {
        let (project, attempt, bytes) = fixture(&[0, 34]);
        let recovered = recover(&project, &attempt).unwrap();
        assert_eq!(recovered.files, 1);
        assert_eq!(recovered.skipped_entries, 1);
        assert_eq!(
            recovered.native_engine_version,
            fat12::RECOVERY_ENGINE_VERSION
        );
        let report: RecoveryReport =
            serde_json::from_slice(&fs::read(&recovered.report_path).unwrap()).unwrap();
        assert_eq!(
            report.analysis.as_ref().unwrap().layout_evidence.method,
            "inferred_standard_layout"
        );
        assert!(
            !report.analysis.as_ref().unwrap().recovered_files[0]
                .metadata_lbas
                .contains(&0)
        );
        assert!(recover(&project, &attempt).unwrap().reused);
        assert!(
            extraction::verify_managed_extraction(&recovered.output_directory, &attempt.sha256)
                .is_ok()
        );
        let audit = audit::run_audit(&project, &|_| {}).unwrap();
        assert_eq!(audit.verified_disks, 0);
        assert_eq!(audit.attention_disks, 1);
        assert_eq!(
            fs::read(project.images_dir().join(&attempt.image_file)).unwrap(),
            bytes
        );
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn version_two_generation_and_report_survive_layout_engine_upgrade() {
        let (project, attempt, _) = fixture(&[34]);
        let initial = recover(&project, &attempt).unwrap();
        let old = project.extracted_dir().join("001/attempt_001_native_v2");
        fs::rename(&initial.output_directory, &old).unwrap();
        let mut report: serde_json::Value =
            serde_json::from_slice(&fs::read(&initial.report_path).unwrap()).unwrap();
        report["native_engine_version"] = serde_json::json!(2);
        report["method"] = serde_json::json!("native_fat12_readable_chains");
        report.as_object_mut().unwrap().remove("carving");
        report.as_object_mut().unwrap().remove("filesystem_error");
        report["analysis"]
            .as_object_mut()
            .unwrap()
            .remove("layout_evidence");
        report["analysis"]
            .as_object_mut()
            .unwrap()
            .remove("unrecovered_files");
        let old_bytes = serde_json::to_vec_pretty(&report).unwrap();
        fs::write(old.join(extraction::FAT12_REPORT_NAME), &old_bytes).unwrap();
        extraction::write_native_managed_metadata(
            &old,
            &project.images_dir().join(&attempt.image_file),
            &attempt.sha256,
            hash(&old_bytes),
        )
        .unwrap();
        let old_report = project.recovery_dir().join("001/attempt_001_fat12_v2.json");
        fs::rename(&initial.report_path, &old_report).unwrap();
        fs::write(&old_report, &old_bytes).unwrap();
        let upgraded = recover(&project, &attempt).unwrap();
        assert!(upgraded.output_directory.ends_with("attempt_001_native_v4"));
        assert!(extraction::verify_managed_extraction(&old, &attempt.sha256).is_ok());
        assert_eq!(fs::read(old_report).unwrap(), old_bytes);
        assert!(old.join("GOOD.TXT").is_file());
        assert!(recover(&project, &attempt).unwrap().reused);
        fs::remove_dir_all(project.root()).unwrap();
    }
}
