//! Immutable raw-flux captures and separately derived, unverified sector images.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    greaseweazle::{
        CaptureSettings, GreaseweazleBackend, GreaseweazleCommand, GreaseweazleProfile,
    },
    imaging,
    project::ProjectState,
    recovery_plan,
    safety::MediaSafetyPolicy,
};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct CaptureRequest {
    pub disk_number: u32,
    pub profile: GreaseweazleProfile,
    pub drive: char,
    pub revolutions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureRecord {
    schema_version: u32,
    disk_number: u32,
    attempt_number: u32,
    profile: String,
    drive: char,
    revolutions: u32,
    status: String,
    flux_file: Option<String>,
    bytes: Option<u64>,
    sha256: Option<String>,
    command: Vec<String>,
    detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_version: Option<String>,
    #[serde(default)]
    capture_settings: Option<CaptureSettings>,
}

#[derive(Debug, Clone)]
pub struct CaptureResult {
    pub disk_number: u32,
    pub attempt_number: u32,
    pub flux_path: PathBuf,
    pub metadata_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub host_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecodeRecord {
    schema_version: u32,
    disk_number: u32,
    capture_attempt: u32,
    decode_attempt: u32,
    profile: String,
    source_flux_file: String,
    source_sha256: String,
    output_file: String,
    output_sha256: String,
    bytes: u64,
    status: String,
    #[serde(default)]
    detail: Option<String>,
    command: Vec<String>,
    #[serde(default)]
    reported_found_sectors: Option<usize>,
    #[serde(default)]
    reported_total_sectors: Option<usize>,
    #[serde(default)]
    gw_bad_lbas: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_version: Option<String>,
    #[serde(default)]
    capture_settings: Option<CaptureSettings>,
}

#[derive(Debug, Clone)]
pub struct DecodeResult {
    pub disk_number: u32,
    pub capture_attempt: u32,
    pub decode_attempt: u32,
    pub image_path: PathBuf,
    pub metadata_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub reported_sectors: Option<(usize, usize)>,
    pub gw_bad_lbas: Option<Vec<u64>>,
    pub host_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureInspection {
    pub attempt: u32,
    pub status: String,
    pub profile: Option<String>,
    pub metadata: PathBuf,
    pub raw_flux: Option<PathBuf>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub hash_matches: bool,
    pub host_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecodeInspection {
    pub capture_attempt: u32,
    pub decode_attempt: u32,
    pub profile: String,
    pub metadata: PathBuf,
    pub image: PathBuf,
    pub output_sha256: String,
    pub output_hash_matches: bool,
    pub source_hash_matches: bool,
    pub gw_reported_found_sectors: Option<usize>,
    pub gw_reported_total_sectors: Option<usize>,
    pub gw_bad_lbas: Option<Vec<u64>>,
    pub sector_quality: String,
    pub detail: Option<String>,
    pub host_version: Option<String>,
    pub capture_settings: Option<CaptureSettings>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FluxDiskStatus {
    pub disk_number: u32,
    pub captures: Vec<CaptureInspection>,
    pub decodes: Vec<DecodeInspection>,
    pub evidence_healthy: bool,
    pub attention_required: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FluxComparison {
    pub disk_number: u32,
    pub usb_attempt: u32,
    pub usb_image: PathBuf,
    pub usb_sha256: String,
    pub capture_attempt: u32,
    pub decode_attempt: u32,
    pub flux_image: PathBuf,
    pub flux_sha256: String,
    pub total_sectors: usize,
    pub usb_bad_lbas: Vec<u64>,
    pub gw_reported_bad_lbas: Vec<u64>,
    pub candidate_flux_donor_lbas: Vec<u64>,
    pub usb_only_good_lbas: Vec<u64>,
    pub unresolved_lbas: Vec<u64>,
    pub conflicting_good_lbas: Vec<u64>,
    pub matching_good_sectors: usize,
    pub no_reported_good_byte_conflicts: bool,
    pub physical_media_access: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FluxConsensus {
    pub disk_number: u32,
    pub older_capture_attempt: u32,
    pub newer_capture_attempt: u32,
    pub older_decode_attempt: u32,
    pub newer_decode_attempt: u32,
    pub older_image_sha256: String,
    pub newer_image_sha256: String,
    pub total_sectors: usize,
    pub matching_reported_good_lbas: Vec<u64>,
    pub conflicting_reported_good_lbas: Vec<u64>,
    pub older_only_reported_good_lbas: Vec<u64>,
    pub newer_only_reported_good_lbas: Vec<u64>,
    pub both_reported_bad_lbas: Vec<u64>,
    pub physical_media_access: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FluxRecoveryPlan {
    pub disk_number: u32,
    pub usb_attempt: u32,
    pub usb_sha256: String,
    pub older_capture_attempt: u32,
    pub older_decode_attempt: u32,
    pub older_sha256: String,
    pub newer_capture_attempt: u32,
    pub newer_decode_attempt: u32,
    pub newer_sha256: String,
    pub total_sectors: usize,
    pub matching_control_sectors: usize,
    pub corroborated_donor_lbas: Vec<u64>,
    pub single_flux_read_lbas: Vec<u64>,
    pub unresolved_lbas: Vec<u64>,
    pub usb_flux_conflict_lbas: Vec<u64>,
    pub flux_flux_conflict_lbas: Vec<u64>,
    pub image_promoted: bool,
    pub physical_media_access: bool,
}

/// Integrate three saved, hash-checked sources without creating a composite.
/// Two separate raw captures must report a sector good and agree byte-for-byte
/// before it becomes a corroborated donor candidate for a USB-bad LBA.
pub fn plan_flux_recovery(
    project: &ProjectState,
    disk_number: u32,
) -> Result<FluxRecoveryPlan, String> {
    let images_dir = project.images_dir();
    let attempts = imaging::load_attempts_for_disk(&images_dir, disk_number)?;
    let usb = attempts
        .iter()
        .filter(|attempt| attempt.total_sectors > 0)
        .min_by_key(|attempt| {
            (
                attempt.bad_sectors.len(),
                std::cmp::Reverse(attempt.attempt_number),
            )
        })
        .ok_or_else(|| format!("No saved USB image for disk {disk_number:03}"))?;
    let status = inspect_disk(project, disk_number)?;
    let (older, newer) = latest_two_decodes(&status)?;
    if older.profile != newer.profile {
        return Err("Latest two raw-capture decodes use different sector profiles".to_owned());
    }
    let sectors = match newer.profile.as_str() {
        "ibm.1440" => 2880,
        "ibm.720" => 1440,
        _ => return Err("Unsupported Greaseweazle sector profile".to_owned()),
    };
    if usb.total_sectors != sectors {
        return Err("USB and Greaseweazle geometry disagree".to_owned());
    }
    let usb_path = recovery_plan::resolve_image_path(&images_dir, &usb.image_file)?;
    let usb_bytes = read_regular_sector_image(&usb_path, sectors)?;
    let usb_sha256 = sha256_bytes(&usb_bytes);
    if !usb_sha256.eq_ignore_ascii_case(&usb.sha256) {
        return Err("Saved USB image changed since acquisition; recovery plan refused".to_owned());
    }
    let usb_bad = validated_bad_set(&usb.bad_sectors, sectors, "USB")?;
    let (older_bytes, older_bad) = verified_decode(project, older, sectors)?;
    let (newer_bytes, newer_bad) = verified_decode(project, newer, sectors)?;
    let mut plan = FluxRecoveryPlan {
        disk_number,
        usb_attempt: usb.attempt_number,
        usb_sha256,
        older_capture_attempt: older.capture_attempt,
        older_decode_attempt: older.decode_attempt,
        older_sha256: sha256_bytes(&older_bytes),
        newer_capture_attempt: newer.capture_attempt,
        newer_decode_attempt: newer.decode_attempt,
        newer_sha256: sha256_bytes(&newer_bytes),
        total_sectors: sectors,
        matching_control_sectors: 0,
        corroborated_donor_lbas: Vec::new(),
        single_flux_read_lbas: Vec::new(),
        unresolved_lbas: Vec::new(),
        usb_flux_conflict_lbas: Vec::new(),
        flux_flux_conflict_lbas: Vec::new(),
        image_promoted: false,
        physical_media_access: false,
    };
    for lba in 0..sectors {
        let lba_u64 = lba as u64;
        let range = lba * 512..(lba + 1) * 512;
        let usb_good = !usb_bad.contains(&lba_u64);
        let older_good = !older_bad.contains(&lba_u64);
        let newer_good = !newer_bad.contains(&lba_u64);
        let older_matches_usb =
            older_good && usb_bytes[range.clone()] == older_bytes[range.clone()];
        let newer_matches_usb =
            newer_good && usb_bytes[range.clone()] == newer_bytes[range.clone()];
        let flux_agrees =
            older_good && newer_good && older_bytes[range.clone()] == newer_bytes[range];
        if older_good && newer_good && !flux_agrees {
            plan.flux_flux_conflict_lbas.push(lba_u64);
        }
        if usb_good {
            if (older_good && !older_matches_usb) || (newer_good && !newer_matches_usb) {
                plan.usb_flux_conflict_lbas.push(lba_u64);
            } else if flux_agrees {
                plan.matching_control_sectors += 1;
            }
        } else if flux_agrees {
            plan.corroborated_donor_lbas.push(lba_u64);
        } else if older_good ^ newer_good {
            plan.single_flux_read_lbas.push(lba_u64);
        } else if !older_good && !newer_good {
            plan.unresolved_lbas.push(lba_u64);
        }
    }
    Ok(plan)
}

fn latest_two_decodes(
    status: &FluxDiskStatus,
) -> Result<(&DecodeInspection, &DecodeInspection), String> {
    let mut chosen = Vec::new();
    for decode in status.decodes.iter().rev() {
        if chosen
            .iter()
            .all(|prior: &&DecodeInspection| prior.capture_attempt != decode.capture_attempt)
        {
            chosen.push(decode);
            if chosen.len() == 2 {
                return Ok((chosen[1], chosen[0]));
            }
        }
    }
    Err("Flux analysis requires decodes from two distinct raw-flux captures".to_owned())
}

/// Re-decoding one SCP twice is not independent evidence. This comparison
/// requires two distinct raw captures and never promotes any image bytes.
pub fn compare_flux_captures(
    project: &ProjectState,
    disk_number: u32,
) -> Result<FluxConsensus, String> {
    let status = inspect_disk(project, disk_number)?;
    let (older, newer) = latest_two_decodes(&status)?;
    if newer.profile != older.profile {
        return Err("Latest two raw-capture decodes use different sector profiles".to_owned());
    }
    let sectors = match newer.profile.as_str() {
        "ibm.1440" => 2880,
        "ibm.720" => 1440,
        _ => return Err("Unsupported Greaseweazle sector profile".to_owned()),
    };
    let (newer_bytes, newer_bad) = verified_decode(project, newer, sectors)?;
    let (older_bytes, older_bad) = verified_decode(project, older, sectors)?;
    let mut result = FluxConsensus {
        disk_number,
        older_capture_attempt: older.capture_attempt,
        newer_capture_attempt: newer.capture_attempt,
        older_decode_attempt: older.decode_attempt,
        newer_decode_attempt: newer.decode_attempt,
        older_image_sha256: sha256_bytes(&older_bytes),
        newer_image_sha256: sha256_bytes(&newer_bytes),
        total_sectors: sectors,
        matching_reported_good_lbas: Vec::new(),
        conflicting_reported_good_lbas: Vec::new(),
        older_only_reported_good_lbas: Vec::new(),
        newer_only_reported_good_lbas: Vec::new(),
        both_reported_bad_lbas: Vec::new(),
        physical_media_access: false,
    };
    for lba in 0..sectors {
        match (
            older_bad.contains(&(lba as u64)),
            newer_bad.contains(&(lba as u64)),
        ) {
            (false, false) => {
                let range = lba * 512..(lba + 1) * 512;
                if older_bytes[range.clone()] == newer_bytes[range] {
                    result.matching_reported_good_lbas.push(lba as u64);
                } else {
                    result.conflicting_reported_good_lbas.push(lba as u64);
                }
            }
            (false, true) => result.older_only_reported_good_lbas.push(lba as u64),
            (true, false) => result.newer_only_reported_good_lbas.push(lba as u64),
            (true, true) => result.both_reported_bad_lbas.push(lba as u64),
        }
    }
    Ok(result)
}

pub(crate) fn verified_decode(
    project: &ProjectState,
    decode: &DecodeInspection,
    sectors: usize,
) -> Result<(Vec<u8>, BTreeSet<u64>), String> {
    if !decode.output_hash_matches || !decode.source_hash_matches {
        return Err("Greaseweazle decode or raw capture changed; comparison refused".to_owned());
    }
    let bad = decode
        .gw_bad_lbas
        .as_ref()
        .ok_or("Greaseweazle decode lacks a complete, consistent sector map; comparison refused")?;
    let bad_set = validated_bad_set(bad, sectors, "Greaseweazle")?;
    let covered_sectors = match decode
        .capture_settings
        .as_ref()
        .and_then(|s| s.cylinders.as_ref())
    {
        Some(cylinders) => {
            decode.capture_settings.as_ref().unwrap().validate()?;
            cylinders.len() * 2 * (sectors / 160)
        }
        None => sectors,
    };
    if decode.gw_reported_total_sectors != Some(covered_sectors)
        || decode.gw_reported_found_sectors != Some(sectors - bad_set.len())
    {
        return Err("Greaseweazle sector counts disagree with its map".to_owned());
    }
    let flux_dir = project_flux_dir(project)?;
    let derived_dir = resolve_derived_dir(&flux_dir, false)?;
    let image = decode
        .image
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if image.parent() != Some(derived_dir.as_path()) {
        return Err("Decoded image escapes the project's Flux/Derived directory".to_owned());
    }
    let bytes = read_regular_sector_image(&image, sectors)?;
    if !sha256_bytes(&bytes).eq_ignore_ascii_case(&decode.output_sha256) {
        return Err("Decoded Greaseweazle image changed during comparison".to_owned());
    }
    Ok((bytes, bad_set))
}

/// Compare saved USB and Greaseweazle evidence without promoting either image.
/// A Greaseweazle dot in the complete grid is vendor-reported, not proof of
/// independently validated bytes; candidates require further validation.
pub fn compare_with_usb(
    project: &ProjectState,
    disk_number: u32,
) -> Result<FluxComparison, String> {
    let images_dir = project.images_dir();
    let attempts = imaging::load_attempts_for_disk(&images_dir, disk_number)?;
    let usb = attempts
        .iter()
        .filter(|attempt| attempt.total_sectors > 0)
        .min_by_key(|attempt| {
            (
                attempt.bad_sectors.len(),
                std::cmp::Reverse(attempt.attempt_number),
            )
        })
        .ok_or_else(|| format!("No saved USB image for disk {disk_number:03}"))?;
    let usb_path = recovery_plan::resolve_image_path(&images_dir, &usb.image_file)?;
    let status = inspect_disk(project, disk_number)?;
    let flux = status
        .decodes
        .last()
        .ok_or_else(|| format!("No saved Greaseweazle decode for disk {disk_number:03}"))?;
    let expected_sectors = match flux.profile.as_str() {
        "ibm.1440" => 2880usize,
        "ibm.720" => 1440usize,
        _ => return Err("Unsupported Greaseweazle sector profile".to_owned()),
    };
    if usb.total_sectors != expected_sectors {
        return Err("USB and Greaseweazle geometry or sector counts disagree".to_owned());
    }
    let usb_bytes = read_regular_sector_image(&usb_path, expected_sectors)?;
    let (flux_bytes, gw_bad_set) = verified_decode(project, flux, expected_sectors)?;
    let usb_sha256 = sha256_bytes(&usb_bytes);
    if !usb_sha256.eq_ignore_ascii_case(&usb.sha256) {
        return Err("Saved USB image changed since acquisition; comparison refused".to_owned());
    }
    let flux_sha256 = sha256_bytes(&flux_bytes);
    let usb_bad = validated_bad_set(&usb.bad_sectors, expected_sectors, "USB")?;
    let mut candidate_flux_donor_lbas = Vec::new();
    let mut usb_only_good_lbas = Vec::new();
    let mut unresolved_lbas = Vec::new();
    let mut conflicting_good_lbas = Vec::new();
    let mut matching_good_sectors = 0;
    for lba in 0..expected_sectors {
        let usb_bad_here = usb_bad.contains(&(lba as u64));
        let gw_bad_here = gw_bad_set.contains(&(lba as u64));
        match (usb_bad_here, gw_bad_here) {
            (true, false) => candidate_flux_donor_lbas.push(lba as u64),
            (false, true) => usb_only_good_lbas.push(lba as u64),
            (true, true) => unresolved_lbas.push(lba as u64),
            (false, false) => {
                let range = lba * 512..(lba + 1) * 512;
                if usb_bytes[range.clone()] == flux_bytes[range] {
                    matching_good_sectors += 1;
                } else {
                    conflicting_good_lbas.push(lba as u64);
                }
            }
        }
    }
    Ok(FluxComparison {
        disk_number,
        usb_attempt: usb.attempt_number,
        usb_image: usb_path,
        usb_sha256,
        capture_attempt: flux.capture_attempt,
        decode_attempt: flux.decode_attempt,
        flux_image: flux.image.clone(),
        flux_sha256,
        total_sectors: expected_sectors,
        usb_bad_lbas: usb_bad.into_iter().collect(),
        gw_reported_bad_lbas: gw_bad_set.into_iter().collect(),
        no_reported_good_byte_conflicts: conflicting_good_lbas.is_empty(),
        candidate_flux_donor_lbas,
        usb_only_good_lbas,
        unresolved_lbas,
        conflicting_good_lbas,
        matching_good_sectors,
        physical_media_access: false,
    })
}

fn read_regular_sector_image(path: &Path, sectors: usize) -> Result<Vec<u8>, String> {
    let info = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect image {}: {error}", path.display()))?;
    if !info.file_type().is_file() || info.len() != (sectors * 512) as u64 {
        return Err(format!(
            "Image is not a regular {sectors}-sector file: {}",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| format!("Cannot read image {}: {error}", path.display()))
}

fn validated_bad_set(lbas: &[u64], sectors: usize, label: &str) -> Result<BTreeSet<u64>, String> {
    let set = lbas.iter().copied().collect::<BTreeSet<_>>();
    if set.len() != lbas.len() || set.iter().any(|lba| *lba >= sectors as u64) {
        return Err(format!(
            "{label} bad-sector map has duplicates or out-of-range LBAs"
        ));
    }
    Ok(set)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn inspect_disk(project: &ProjectState, disk_number: u32) -> Result<FluxDiskStatus, String> {
    if disk_number == 0 {
        return Err("Greaseweazle status requires a positive disk number".to_owned());
    }
    let flux_dir = project_flux_dir(project)?;
    let prefix = format!("{disk_number:03}_attempt_");
    let mut captures = Vec::new();
    for entry in fs::read_dir(&flux_dir)
        .map_err(|error| format!("Cannot list raw-flux captures: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Cannot inspect raw-flux capture: {error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let partial = name.ends_with(".partial.json");
        let number_text = name
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(if partial { ".partial.json" } else { ".json" }));
        let Some(attempt) = number_text.and_then(|number| number.parse::<u32>().ok()) else {
            continue;
        };
        let metadata = entry.path();
        let record = fs::read(&metadata)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CaptureRecord>(&bytes).ok());
        let expected_raw = flux_dir.join(format!("{disk_number:03}_attempt_{attempt:03}.scp"));
        let hash_matches = record.as_ref().is_some_and(|record| {
            !partial
                && record.schema_version == SCHEMA_VERSION
                && record.disk_number == disk_number
                && record.attempt_number == attempt
                && record.status == "complete"
                && record.flux_file.as_deref()
                    == expected_raw.file_name().and_then(|name| name.to_str())
                && fs::symlink_metadata(&expected_raw)
                    .ok()
                    .is_some_and(|info| {
                        info.file_type().is_file() && Some(info.len()) == record.bytes
                    })
                && record.sha256.as_deref() == hash_file(&expected_raw).ok().as_deref()
        });
        captures.push(CaptureInspection {
            attempt,
            status: record
                .as_ref()
                .map(|record| record.status.clone())
                .unwrap_or_else(|| "invalid_metadata".to_owned()),
            profile: record.as_ref().map(|record| record.profile.clone()),
            metadata,
            raw_flux: (!partial).then_some(expected_raw),
            bytes: record.as_ref().and_then(|record| record.bytes),
            sha256: record.as_ref().and_then(|record| record.sha256.clone()),
            hash_matches,
            host_version: record
                .as_ref()
                .and_then(|record| record.host_version.clone()),
        });
    }
    captures.sort_by_key(|capture| capture.attempt);

    let mut decodes = Vec::new();
    let derived_dir = flux_dir.join("Derived");
    if path_occupied(&derived_dir)? {
        let derived_dir = resolve_derived_dir(&flux_dir, false)?;
        for entry in fs::read_dir(&derived_dir)
            .map_err(|error| format!("Cannot list decoded images: {error}"))?
        {
            let entry = entry.map_err(|error| format!("Cannot inspect decoded image: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&format!("{disk_number:03}_flux_")) || !name.ends_with(".json") {
                continue;
            }
            let partial = name.ends_with(".partial.json");
            let metadata = entry.path();
            let record: DecodeRecord =
                serde_json::from_slice(&fs::read(&metadata).map_err(|error| {
                    format!(
                        "Cannot read decode metadata {}: {error}",
                        metadata.display()
                    )
                })?)
                .map_err(|error| {
                    format!("Invalid decode metadata {}: {error}", metadata.display())
                })?;
            if record.disk_number != disk_number || record.schema_version != SCHEMA_VERSION {
                return Err(format!(
                    "Mismatched decode metadata: {}",
                    metadata.display()
                ));
            }
            let expected_name = format!(
                "{disk_number:03}_flux_{:03}_{}_decode_{:03}.img",
                record.capture_attempt,
                record.profile.replace('.', "_"),
                record.decode_attempt
            );
            let expected_metadata_name = format!(
                "{}{}",
                expected_name.trim_end_matches(".img"),
                if partial { ".partial.json" } else { ".json" }
            );
            if record.output_file != expected_name || name != expected_metadata_name {
                return Err(format!(
                    "Unexpected decode artifact filename in {}",
                    metadata.display()
                ));
            }
            let image = derived_dir.join(&record.output_file);
            let output_hash_matches = !partial
                && fs::symlink_metadata(&image)
                    .ok()
                    .is_some_and(|info| info.file_type().is_file() && info.len() == record.bytes)
                && hash_file(&image).ok().as_deref() == Some(record.output_sha256.as_str());
            let expected_source =
                format!("{disk_number:03}_attempt_{:03}.scp", record.capture_attempt);
            let source_hash_matches = !partial
                && record.source_flux_file == expected_source
                && captures.iter().any(|capture| {
                    capture.attempt == record.capture_attempt
                        && capture.hash_matches
                        && capture.sha256.as_deref() == Some(record.source_sha256.as_str())
                })
                && hash_file(&flux_dir.join(&expected_source)).ok().as_deref()
                    == Some(record.source_sha256.as_str());
            decodes.push(DecodeInspection {
                capture_attempt: record.capture_attempt,
                decode_attempt: record.decode_attempt,
                profile: record.profile,
                metadata,
                image,
                output_sha256: record.output_sha256,
                output_hash_matches,
                source_hash_matches,
                gw_reported_found_sectors: record.reported_found_sectors,
                gw_reported_total_sectors: record.reported_total_sectors,
                gw_bad_lbas: record.gw_bad_lbas,
                sector_quality: record.status,
                detail: record.detail,
                host_version: record.host_version,
                capture_settings: record.capture_settings,
            });
        }
    }
    decodes.sort_by_key(|decode| (decode.capture_attempt, decode.decode_attempt));
    if captures.is_empty() && decodes.is_empty() {
        return Err(format!(
            "No Greaseweazle artifacts for disk {disk_number:03}"
        ));
    }
    let evidence_healthy = captures.iter().all(|capture| capture.hash_matches)
        && decodes
            .iter()
            .all(|decode| decode.output_hash_matches && decode.source_hash_matches);
    Ok(FluxDiskStatus {
        disk_number,
        captures,
        decodes,
        evidence_healthy,
        attention_required: true, // Sector quality is not yet proven by FluxVault.
    })
}

pub fn latest_capture_attempt(project: &ProjectState, disk_number: u32) -> Result<u32, String> {
    let flux_dir = project_flux_dir(project)?;
    let prefix = format!("{disk_number:03}_attempt_");
    let mut latest = None;
    for entry in fs::read_dir(&flux_dir)
        .map_err(|error| format!("Cannot list raw-flux captures: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Cannot inspect raw-flux capture: {error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(number) = name
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|number| number.parse::<u32>().ok())
        {
            latest = Some(latest.map_or(number, |old: u32| old.max(number)));
        }
    }
    latest.ok_or_else(|| format!("No completed raw-flux capture for disk {disk_number:03}"))
}

pub fn capture(
    project: &ProjectState,
    request: CaptureRequest,
    backend: &mut impl GreaseweazleBackend,
) -> Result<CaptureResult, String> {
    capture_with_settings(
        project,
        request,
        &CaptureSettings {
            cylinders: None,
            retries: 3,
        },
        backend,
    )
}

pub fn capture_with_settings(
    project: &ProjectState,
    request: CaptureRequest,
    settings: &CaptureSettings,
    backend: &mut impl GreaseweazleBackend,
) -> Result<CaptureResult, String> {
    settings.validate()?;
    MediaSafetyPolicy::assert_invariants();
    if request.disk_number == 0 {
        return Err("Capture requires a positive disk number".to_owned());
    }
    if !(1..=10).contains(&request.revolutions) {
        return Err("Capture revolutions must be from 1 to 10".to_owned());
    }
    let flux_dir = project_flux_dir(project)?;
    let attempt_number = next_capture_attempt(&flux_dir, request.disk_number)?;
    let stem = format!("{:03}_attempt_{attempt_number:03}", request.disk_number);
    let partial_flux = flux_dir.join(format!("{stem}.partial.scp"));
    let final_flux = flux_dir.join(format!("{stem}.scp"));
    let partial_metadata = flux_dir.join(format!("{stem}.partial.json"));
    let final_metadata = flux_dir.join(format!("{stem}.json"));
    let command = GreaseweazleCommand::raw_flux_read_with_settings(
        request.profile,
        request.drive,
        request.revolutions,
        &partial_flux,
        settings,
    )?;
    let mut record = CaptureRecord {
        schema_version: SCHEMA_VERSION,
        disk_number: request.disk_number,
        attempt_number,
        profile: request.profile.argument().to_owned(),
        drive: request.drive.to_ascii_uppercase(),
        revolutions: request.revolutions,
        status: "started".to_owned(),
        flux_file: None,
        bytes: None,
        sha256: None,
        command: command.arguments().to_vec(),
        detail: None,
        host_version: None,
        capture_settings: Some(settings.clone()),
    };
    reserve_record(&partial_metadata, &record)?;

    let execution = match backend.execute(&command) {
        Ok(execution) => execution,
        Err(error) => {
            record.status = "failed".to_owned();
            record.detail = Some(error.clone());
            save_record(&partial_metadata, &record)?;
            return Err(format!(
                "Raw capture failed; attempt evidence remains at {}: {error}",
                partial_metadata.display()
            ));
        }
    };
    record.host_version = execution.host_version.clone();
    if !execution.success {
        record.status = "failed".to_owned();
        let reason = if execution.timed_out {
            format!("gw timed out: {}", execution.stderr)
        } else {
            format!(
                "gw exited {:?}: {} {}",
                execution.exit_code, execution.stdout, execution.stderr
            )
        };
        record.detail = Some(reason);
        save_record(&partial_metadata, &record)?;
        return Err(format!(
            "Raw capture failed; attempt evidence remains at {}: {}",
            partial_metadata.display(),
            record.detail.as_deref().unwrap_or("unknown error")
        ));
    }
    let raw_info = fs::symlink_metadata(&partial_flux)
        .map_err(|error| format!("Successful gw run produced no SCP file: {error}"))?;
    if !raw_info.file_type().is_file() {
        return Err("Greaseweazle output is not a regular SCP file".to_owned());
    }
    let bytes = raw_info.len();
    if bytes == 0 {
        return Err(format!(
            "Greaseweazle produced an empty SCP file: {}",
            partial_flux.display()
        ));
    }
    let sha256 = hash_file(&partial_flux)?;
    if path_occupied(&final_flux)? || path_occupied(&final_metadata)? {
        return Err("Capture destination already exists; refusing to overwrite".to_owned());
    }
    fs::rename(&partial_flux, &final_flux)
        .map_err(|error| format!("Cannot finalize raw-flux capture: {error}"))?;
    record.status = "complete".to_owned();
    record.flux_file = Some(format!("{stem}.scp"));
    record.bytes = Some(bytes);
    record.sha256 = Some(sha256.clone());
    save_record(&partial_metadata, &record)?;
    fs::rename(&partial_metadata, &final_metadata)
        .map_err(|error| format!("Cannot finalize capture metadata: {error}"))?;
    Ok(CaptureResult {
        disk_number: request.disk_number,
        attempt_number,
        flux_path: final_flux,
        metadata_path: final_metadata,
        bytes,
        sha256,
        host_version: execution.host_version,
    })
}

pub fn decode(
    project: &ProjectState,
    disk_number: u32,
    capture_attempt: u32,
    profile_override: Option<GreaseweazleProfile>,
    backend: &mut impl GreaseweazleBackend,
) -> Result<DecodeResult, String> {
    MediaSafetyPolicy::assert_invariants();
    if disk_number == 0 || capture_attempt == 0 {
        return Err("Decode requires positive disk and capture-attempt numbers".to_owned());
    }
    let flux_dir = project_flux_dir(project)?;
    let stem = format!("{disk_number:03}_attempt_{capture_attempt:03}");
    let capture_metadata = flux_dir.join(format!("{stem}.json"));
    let record: CaptureRecord =
        serde_json::from_slice(&fs::read(&capture_metadata).map_err(|error| {
            format!(
                "Cannot read capture metadata {}: {error}",
                capture_metadata.display()
            )
        })?)
        .map_err(|error| format!("Invalid capture metadata: {error}"))?;
    if record.schema_version != SCHEMA_VERSION
        || record.disk_number != disk_number
        || record.attempt_number != capture_attempt
        || record.status != "complete"
    {
        return Err("Capture metadata does not identify a completed matching attempt".to_owned());
    }
    let flux_file = record
        .flux_file
        .as_deref()
        .ok_or("Capture has no SCP filename")?;
    if flux_file != format!("{stem}.scp") {
        return Err("Capture metadata contains an unexpected SCP filename".to_owned());
    }
    let input = flux_dir.join(flux_file);
    if !fs::symlink_metadata(&input)
        .map_err(|error| format!("Cannot inspect raw-flux source: {error}"))?
        .file_type()
        .is_file()
    {
        return Err("Raw-flux source is not a regular file".to_owned());
    }
    let source_hash = record
        .sha256
        .as_deref()
        .ok_or("Capture has no saved SHA-256")?;
    if hash_file(&input)? != source_hash {
        return Err("Raw-flux capture changed since acquisition; decode refused".to_owned());
    }
    let profile = match profile_override {
        Some(profile) => profile,
        None => GreaseweazleProfile::parse(&record.profile)?,
    };
    let derived_dir = resolve_derived_dir(&flux_dir, true)?;
    let profile_slug = profile.argument().replace('.', "_");
    let prefix = format!("{disk_number:03}_flux_{capture_attempt:03}_{profile_slug}");
    let decode_attempt = next_decode_attempt(&derived_dir, &prefix)?;
    let derived_stem = format!("{prefix}_decode_{decode_attempt:03}");
    let partial_image = derived_dir.join(format!("{derived_stem}.partial.img"));
    let final_image = derived_dir.join(format!("{derived_stem}.img"));
    let final_metadata = derived_dir.join(format!("{derived_stem}.json"));
    let partial_metadata = derived_dir.join(format!("{derived_stem}.partial.json"));
    let command =
        GreaseweazleCommand::convert_flux_to_sector_image(profile, &input, &partial_image)?;
    if path_occupied(&final_image)?
        || path_occupied(&final_metadata)?
        || path_occupied(&partial_image)?
        || path_occupied(&partial_metadata)?
    {
        return Err("Decode destination already exists; refusing to overwrite".to_owned());
    }
    let mut decode_record = DecodeRecord {
        schema_version: SCHEMA_VERSION,
        disk_number,
        capture_attempt,
        decode_attempt,
        profile: profile.argument().to_owned(),
        source_flux_file: flux_file.to_owned(),
        source_sha256: source_hash.to_owned(),
        output_file: format!("{derived_stem}.img"),
        output_sha256: String::new(),
        bytes: 0,
        status: "started".to_owned(),
        detail: None,
        command: command.arguments().to_vec(),
        reported_found_sectors: None,
        reported_total_sectors: None,
        gw_bad_lbas: None,
        host_version: None,
        capture_settings: record.capture_settings.clone(),
    };
    reserve_decode_record(&partial_metadata, &decode_record)?;
    let execution = match backend.execute(&command) {
        Ok(execution) => execution,
        Err(error) => {
            decode_record.status = "failed".to_owned();
            decode_record.detail = Some(error.clone());
            save_decode_record(&partial_metadata, &decode_record)?;
            return Err(format!(
                "Greaseweazle decode failed; partial evidence remains at {}: {error}",
                partial_metadata.display()
            ));
        }
    };
    decode_record.host_version = execution.host_version.clone();
    if !execution.success {
        decode_record.status = "failed".to_owned();
        let reason = if execution.timed_out {
            format!("gw timed out: {}", execution.stderr)
        } else {
            format!(
                "gw exited {:?}: {} {}",
                execution.exit_code, execution.stdout, execution.stderr
            )
        };
        decode_record.detail = Some(reason);
        save_decode_record(&partial_metadata, &decode_record)?;
        return Err(format!(
            "Greaseweazle decode failed (exit {:?}); partial evidence remains at {} and {}: {} {}",
            execution.exit_code,
            partial_image.display(),
            partial_metadata.display(),
            execution.stdout,
            execution.stderr
        ));
    }
    let bytes = match fs::symlink_metadata(&partial_image) {
        Ok(info) if info.file_type().is_file() => info.len(),
        Ok(_) => {
            decode_record.status = "failed".to_owned();
            decode_record.detail =
                Some("Greaseweazle output is not a regular image file".to_owned());
            save_decode_record(&partial_metadata, &decode_record)?;
            return Err(decode_record.detail.unwrap());
        }
        Err(error) => {
            decode_record.status = "failed".to_owned();
            decode_record.detail =
                Some(format!("Successful gw convert produced no image: {error}"));
            save_decode_record(&partial_metadata, &decode_record)?;
            return Err(decode_record.detail.unwrap());
        }
    };
    if bytes != profile.expected_sector_image_bytes() {
        decode_record.status = "failed".to_owned();
        decode_record.detail = Some(format!("Unexpected decoded image size: {bytes} bytes"));
        save_decode_record(&partial_metadata, &decode_record)?;
        return Err(format!(
            "Decoded image has {bytes} bytes, expected {}; partial evidence remains at {}",
            profile.expected_sector_image_bytes(),
            partial_image.display()
        ));
    }
    let output_hash = hash_file(&partial_image)?;
    let reported_sectors =
        parse_sector_summary(&execution.stdout).or_else(|| parse_sector_summary(&execution.stderr));
    let selected = record
        .capture_settings
        .as_ref()
        .and_then(|s| s.cylinders.as_deref());
    let gw_bad_lbas = parse_sector_map_selected(&execution.stdout, profile, selected)
        .or_else(|| parse_sector_map_selected(&execution.stderr, profile, selected));
    fs::rename(&partial_image, &final_image)
        .map_err(|error| format!("Cannot finalize decoded image: {error}"))?;
    decode_record.output_sha256 = output_hash.clone();
    decode_record.bytes = bytes;
    decode_record.status = "unverified_sector_quality".to_owned();
    decode_record.reported_found_sectors = reported_sectors.map(|(found, _)| found);
    decode_record.reported_total_sectors = reported_sectors.map(|(_, total)| total);
    decode_record.gw_bad_lbas = gw_bad_lbas.clone();
    save_decode_record(&partial_metadata, &decode_record)?;
    fs::rename(&partial_metadata, &final_metadata)
        .map_err(|error| format!("Cannot finalize decode metadata: {error}"))?;
    Ok(DecodeResult {
        disk_number,
        capture_attempt,
        decode_attempt,
        image_path: final_image,
        metadata_path: final_metadata,
        bytes,
        sha256: output_hash,
        reported_sectors,
        gw_bad_lbas,
        host_version: execution.host_version,
    })
}

fn parse_sector_summary(output: &str) -> Option<(usize, usize)> {
    output.lines().rev().find_map(|line| {
        let line = line.trim();
        let (found, rest) = line.strip_prefix("Found ")?.split_once(" sectors of ")?;
        let total = rest.split_whitespace().next()?;
        let found = found.parse::<usize>().ok()?;
        let total = total.parse::<usize>().ok()?;
        (total > 0 && found <= total).then_some((found, total))
    })
}

/// Parse only a complete IBM 80-cylinder grid matching gw's final sector total.
/// This is a report from gw, not independent CRC verification of the .img bytes.
#[cfg(test)]
fn parse_sector_map(output: &str, profile: GreaseweazleProfile) -> Option<Vec<u64>> {
    parse_sector_map_selected(output, profile, None)
}

fn parse_sector_map_selected(
    output: &str,
    profile: GreaseweazleProfile,
    selected: Option<&[u32]>,
) -> Option<Vec<u64>> {
    let sectors_per_track = match profile {
        GreaseweazleProfile::Ibm1440 => 18usize,
        GreaseweazleProfile::Ibm720 => 9usize,
    };
    let cylinders = 80usize;
    let heads = 2usize;
    let selected_set = selected.map(|values| values.iter().copied().collect::<BTreeSet<_>>());
    if let Some(values) = selected
        && (values.is_empty()
            || selected_set.as_ref()?.len() != values.len()
            || values.iter().any(|c| *c >= 80))
    {
        return None;
    }
    let expected_total =
        selected.map_or(cylinders, |values| values.len()) * heads * sectors_per_track;
    let (reported_found, reported_total) = parse_sector_summary(output)?;
    if reported_total != expected_total {
        return None;
    }
    let lines = output.lines().collect::<Vec<_>>();
    let header_index = lines.iter().rposition(|line| line.starts_with("H. S: "))?;
    if header_index == 0 || !lines[header_index - 1].starts_with("Cyl-> ") {
        return None;
    }
    let cylinder_digits = lines[header_index].strip_prefix("H. S: ")?;
    if cylinder_digits.chars().count() != cylinders
        || !cylinder_digits
            .chars()
            .enumerate()
            .all(|(cylinder, digit)| digit.to_digit(10) == Some((cylinder % 10) as u32))
    {
        return None;
    }
    let mut seen_rows = vec![false; heads * sectors_per_track];
    let mut bad_lbas = Vec::new();
    let mut found = 0usize;
    for line in &lines[header_index + 1..] {
        if line.starts_with("Found ") {
            break;
        }
        let (label, cells) = line.split_once(':')?;
        let (head, sector) = label.split_once('.')?;
        let head = head.trim().parse::<usize>().ok()?;
        let sector = sector.trim().parse::<usize>().ok()?;
        if head >= heads || sector >= sectors_per_track {
            return None;
        }
        let row = head * sectors_per_track + sector;
        if seen_rows[row] {
            return None;
        }
        seen_rows[row] = true;
        let cells = cells.strip_prefix(' ')?;
        if cells.chars().count() != cylinders {
            return None;
        }
        for (cylinder, cell) in cells.chars().enumerate() {
            let in_capture = selected_set
                .as_ref()
                .is_none_or(|set| set.contains(&(cylinder as u32)));
            if !in_capture {
                if cell != ' ' {
                    return None;
                }
                bad_lbas.push(((cylinder * heads + head) * sectors_per_track + sector) as u64);
                continue;
            }
            match cell {
                '.' => found += 1,
                'X' => {
                    bad_lbas.push(((cylinder * heads + head) * sectors_per_track + sector) as u64)
                }
                _ => return None,
            }
        }
    }
    if !seen_rows.iter().all(|seen| *seen) || found != reported_found {
        return None;
    }
    bad_lbas.sort_unstable();
    Some(bad_lbas)
}

pub(crate) fn project_flux_dir(project: &ProjectState) -> Result<PathBuf, String> {
    let root = project
        .root()
        .canonicalize()
        .map_err(|error| format!("Cannot resolve project directory: {error}"))?;
    let root_text = root.to_string_lossy().to_ascii_uppercase();
    if ["A:\\", "B:\\", "\\\\?\\A:\\", "\\\\?\\B:\\"]
        .iter()
        .any(|prefix| root_text.starts_with(prefix))
    {
        return Err("Refusing to put capture output on a floppy drive letter".to_owned());
    }
    let flux_dir = root.join("Flux");
    fs::create_dir_all(&flux_dir)
        .map_err(|error| format!("Cannot create Flux directory: {error}"))?;
    let flux_dir = flux_dir
        .canonicalize()
        .map_err(|error| format!("Cannot resolve Flux directory: {error}"))?;
    if !flux_dir.starts_with(&root) || flux_dir == root {
        return Err("Flux directory escapes the project root".to_owned());
    }
    Ok(flux_dir)
}

fn resolve_derived_dir(flux_dir: &Path, create: bool) -> Result<PathBuf, String> {
    let derived_dir = flux_dir.join("Derived");
    if create {
        fs::create_dir_all(&derived_dir)
            .map_err(|error| format!("Cannot create derived-image directory: {error}"))?;
    }
    let resolved = derived_dir
        .canonicalize()
        .map_err(|error| format!("Cannot resolve derived-image directory: {error}"))?;
    if resolved.parent() != Some(flux_dir) {
        return Err("Derived-image directory escapes the project's Flux directory".to_owned());
    }
    Ok(resolved)
}

pub(crate) fn next_capture_attempt(directory: &Path, disk_number: u32) -> Result<u32, String> {
    for attempt in 1..=999_999u32 {
        let stem = format!("{disk_number:03}_attempt_{attempt:03}");
        let mut available = true;
        for suffix in [".scp", ".partial.scp", ".json", ".partial.json"] {
            if path_occupied(&directory.join(format!("{stem}{suffix}")))? {
                available = false;
                break;
            }
        }
        if available {
            return Ok(attempt);
        }
    }
    Err("Too many raw-flux attempts for this disk".to_owned())
}

fn next_decode_attempt(directory: &Path, prefix: &str) -> Result<u32, String> {
    for attempt in 1..=999_999u32 {
        let stem = format!("{prefix}_decode_{attempt:03}");
        let mut available = true;
        for suffix in [".img", ".partial.img", ".json", ".partial.json"] {
            if path_occupied(&directory.join(format!("{stem}{suffix}")))? {
                available = false;
                break;
            }
        }
        if available {
            return Ok(attempt);
        }
    }
    Err("Too many decodes for this capture/profile".to_owned())
}

fn path_occupied(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Cannot inspect output slot {}: {error}",
            path.display()
        )),
    }
}

fn reserve_record(path: &Path, record: &CaptureRecord) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Cannot reserve capture attempt: {error}"))?;
    output
        .write_all(&serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?)
        .map_err(|error| format!("Cannot record capture attempt: {error}"))
}

fn reserve_decode_record(path: &Path, record: &DecodeRecord) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Cannot reserve decode attempt: {error}"))?;
    output
        .write_all(&serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?)
        .map_err(|error| format!("Cannot record decode attempt: {error}"))
}

fn save_decode_record(path: &Path, record: &DecodeRecord) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot update decode metadata: {error}"))
}

fn save_record(path: &Path, record: &CaptureRecord) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Cannot update capture metadata: {error}"))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut input =
        File::open(path).map_err(|error| format!("Cannot hash {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| format!("Cannot read {} while hashing: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::greaseweazle::{BackendMode, GreaseweazleExecution};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct ArtifactBackend {
        commands: Vec<GreaseweazleCommand>,
        fail: bool,
    }

    impl GreaseweazleBackend for ArtifactBackend {
        fn mode(&self) -> BackendMode {
            BackendMode::MockNoHardware
        }

        fn execute(
            &mut self,
            command: &GreaseweazleCommand,
        ) -> Result<GreaseweazleExecution, String> {
            self.commands.push(command.clone());
            if !self.fail {
                let output = PathBuf::from(command.arguments().last().unwrap());
                match command.arguments()[0].as_str() {
                    "read" => fs::write(output, b"SCP synthetic flux").unwrap(),
                    "convert" => fs::write(output, vec![0x33; 737_280]).unwrap(),
                    _ => panic!("unexpected command"),
                }
            }
            Ok(GreaseweazleExecution {
                mode: BackendMode::MockNoHardware,
                command: command.clone(),
                success: !self.fail,
                exit_code: Some(if self.fail { 1 } else { 0 }),
                stdout: if command.arguments()[0] == "convert" {
                    synthetic_gw_grid(GreaseweazleProfile::Ibm720, 711)
                } else {
                    String::new()
                },
                stderr: if self.fail {
                    "synthetic failure".to_owned()
                } else {
                    String::new()
                },
                timed_out: false,
                host_version: Some("1.23-mock".to_owned()),
                started_unix_ms: 0,
                duration_ms: 0,
            })
        }
    }

    fn fixture() -> (ProjectState, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-flux-fixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        (project, root)
    }

    #[test]
    fn raw_capture_and_offline_decode_are_immutable_and_hash_bound() {
        let (project, root) = fixture();
        let mut backend = ArtifactBackend::default();
        let request = CaptureRequest {
            disk_number: 7,
            profile: GreaseweazleProfile::Ibm720,
            drive: 'A',
            revolutions: 3,
        };
        let first = capture(&project, request, &mut backend).unwrap();
        assert_eq!(first.attempt_number, 1);
        assert_eq!(first.bytes, 18);
        assert!(first.flux_path.is_file());
        assert!(first.metadata_path.is_file());
        assert!(
            backend.commands[0]
                .arguments()
                .contains(&"--raw".to_owned())
        );
        assert!(
            backend.commands[0]
                .arguments()
                .contains(&"--no-clobber".to_owned())
        );
        let first_bytes = fs::read(&first.flux_path).unwrap();
        let second = capture(&project, request, &mut backend).unwrap();
        assert_eq!(second.attempt_number, 2);
        assert_eq!(fs::read(&first.flux_path).unwrap(), first_bytes);
        assert_eq!(latest_capture_attempt(&project, 7).unwrap(), 2);

        let derived = decode(&project, 7, 1, None, &mut backend).unwrap();
        assert_eq!(derived.bytes, 737_280);
        assert_eq!(derived.reported_sectors, Some((1439, 1440)));
        assert_eq!(derived.gw_bad_lbas, Some(vec![711]));
        assert!(
            derived
                .image_path
                .starts_with(project.root().canonicalize().unwrap().join("Flux"))
        );
        assert!(!derived.image_path.starts_with(project.images_dir()));
        let metadata: DecodeRecord =
            serde_json::from_slice(&fs::read(&derived.metadata_path).unwrap()).unwrap();
        assert_eq!(metadata.status, "unverified_sector_quality");
        assert_eq!(metadata.source_sha256, first.sha256);
        assert_eq!(metadata.reported_found_sectors, Some(1439));
        assert_eq!(metadata.gw_bad_lbas, Some(vec![711]));
        let decoded_again = decode(&project, 7, 1, None, &mut backend).unwrap();
        assert_eq!(decoded_again.decode_attempt, 2);
        assert!(
            compare_flux_captures(&project, 7)
                .unwrap_err()
                .contains("two distinct")
        );
        assert_eq!(
            fs::read(&derived.image_path).unwrap(),
            fs::read(&decoded_again.image_path).unwrap()
        );

        let status = inspect_disk(&project, 7).unwrap();
        assert_eq!(status.captures.len(), 2);
        assert_eq!(status.decodes.len(), 2);
        assert!(status.evidence_healthy);
        assert!(status.attention_required);
        assert_eq!(status.decodes[0].gw_reported_found_sectors, Some(1439));
        assert_eq!(status.decodes[0].gw_bad_lbas, Some(vec![711]));

        fs::write(&first.flux_path, b"changed").unwrap();
        assert!(!inspect_disk(&project, 7).unwrap().evidence_healthy);
        let command_count = backend.commands.len();
        assert!(
            decode(&project, 7, 1, None, &mut backend)
                .unwrap_err()
                .contains("changed")
        );
        assert_eq!(backend.commands.len(), command_count);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_capture_keeps_partial_evidence_and_advances_attempt_number() {
        let (project, root) = fixture();
        let request = CaptureRequest {
            disk_number: 1,
            profile: GreaseweazleProfile::Ibm1440,
            drive: 'B',
            revolutions: 3,
        };
        let mut backend = ArtifactBackend {
            fail: true,
            ..Default::default()
        };
        assert!(
            capture(&project, request, &mut backend)
                .unwrap_err()
                .contains("failed")
        );
        let partial = project
            .root()
            .join("Flux")
            .join("001_attempt_001.partial.json");
        let record: CaptureRecord = serde_json::from_slice(&fs::read(&partial).unwrap()).unwrap();
        assert_eq!(record.status, "failed");
        backend.fail = false;
        let next = capture(&project, request, &mut backend).unwrap();
        assert_eq!(next.attempt_number, 2);
        assert!(partial.is_file());
        let status = inspect_disk(&project, 1).unwrap();
        assert!(!status.evidence_healthy);
        assert_eq!(status.captures.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_decode_keeps_metadata_and_uses_a_new_attempt_number() {
        let (project, root) = fixture();
        let mut backend = ArtifactBackend::default();
        let captured = capture(
            &project,
            CaptureRequest {
                disk_number: 3,
                profile: GreaseweazleProfile::Ibm720,
                drive: 'A',
                revolutions: 3,
            },
            &mut backend,
        )
        .unwrap();
        backend.fail = true;
        assert!(
            decode(&project, 3, captured.attempt_number, None, &mut backend)
                .unwrap_err()
                .contains("partial evidence")
        );
        let status = inspect_disk(&project, 3).unwrap();
        assert!(!status.evidence_healthy);
        assert_eq!(status.decodes.len(), 1);
        assert_eq!(status.decodes[0].sector_quality, "failed");
        assert!(
            status.decodes[0]
                .metadata
                .ends_with("003_flux_001_ibm_720_decode_001.partial.json")
        );
        backend.fail = false;
        let success = decode(&project, 3, captured.attempt_number, None, &mut backend).unwrap();
        assert_eq!(success.decode_attempt, 2);
        assert!(status.decodes[0].metadata.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn dangling_windows_links_still_occupy_capture_and_decode_slots() {
        let (project, root) = fixture();
        let flux_dir = project.root().join("Flux");
        let link = flux_dir.join("011_attempt_001.partial.scp");
        let target = root.join("nonexistent-capture-target.scp");
        if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
            eprintln!("Skipping symlink assertions; Windows denied symlink creation: {error}");
            fs::remove_dir_all(root).unwrap();
            return;
        }
        assert!(!link.exists());
        assert!(path_occupied(&link).unwrap());
        assert_eq!(next_capture_attempt(&flux_dir, 11).unwrap(), 2);

        let derived_dir = flux_dir.join("Derived");
        fs::create_dir_all(&derived_dir).unwrap();
        let decode_link = derived_dir.join("011_flux_001_ibm_1440_decode_001.partial.img");
        std::os::windows::fs::symlink_file(root.join("nonexistent-image-target.img"), &decode_link)
            .unwrap();
        assert_eq!(
            next_decode_attempt(&derived_dir, "011_flux_001_ibm_1440").unwrap(),
            2
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sector_summary_parser_ignores_ambiguous_or_impossible_counts() {
        assert_eq!(
            parse_sector_summary("T0.0: IBM MFM\nFound 2879 sectors of 2880 (99%)\n"),
            Some((2879, 2880))
        );
        assert_eq!(
            parse_sector_summary("Found 2881 sectors of 2880 (100%)"),
            None
        );
        assert_eq!(parse_sector_summary("Found 10 sectors of zero"), None);
        assert_eq!(parse_sector_summary("No summary"), None);
    }

    fn synthetic_gw_grid(profile: GreaseweazleProfile, bad_lba: u64) -> String {
        let sectors = match profile {
            GreaseweazleProfile::Ibm1440 => 18usize,
            GreaseweazleProfile::Ibm720 => 9usize,
        };
        let total = 80 * 2 * sectors;
        let tens = (0..80)
            .map(|cylinder| {
                if cylinder % 10 == 0 {
                    char::from_digit((cylinder / 10) as u32, 10).unwrap()
                } else {
                    ' '
                }
            })
            .collect::<String>();
        let units = (0..80)
            .map(|cylinder| char::from_digit((cylinder % 10) as u32, 10).unwrap())
            .collect::<String>();
        let mut output = format!("Cyl-> {tens}\nH. S: {units}\n");
        for head in 0..2 {
            for sector in 0..sectors {
                let cells = (0..80)
                    .map(|cylinder| {
                        let lba = ((cylinder * 2 + head) * sectors + sector) as u64;
                        if lba == bad_lba { 'X' } else { '.' }
                    })
                    .collect::<String>();
                output.push_str(&format!("{head}.{sector:>2}: {cells}\n"));
            }
        }
        output.push_str(&format!("Found {} sectors of {total} (99%)\n", total - 1));
        output
    }

    #[test]
    fn conservative_grid_parser_maps_only_complete_consistent_reports() {
        let hd = synthetic_gw_grid(GreaseweazleProfile::Ibm1440, 1600);
        assert_eq!(
            parse_sector_map(&hd, GreaseweazleProfile::Ibm1440),
            Some(vec![1600])
        );
        assert_eq!(parse_sector_map(&hd, GreaseweazleProfile::Ibm720), None);
        let dd = synthetic_gw_grid(GreaseweazleProfile::Ibm720, 711);
        assert_eq!(
            parse_sector_map(&dd, GreaseweazleProfile::Ibm720),
            Some(vec![711])
        );

        let truncated = hd
            .lines()
            .filter(|line| !line.starts_with("1.17:"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            parse_sector_map(&truncated, GreaseweazleProfile::Ibm1440),
            None
        );
        let inconsistent = hd.replace("Found 2879", "Found 2880");
        assert_eq!(
            parse_sector_map(&inconsistent, GreaseweazleProfile::Ibm1440),
            None
        );
        let unknown_cell = hd.replacen("0. 0: .", "0. 0:  ", 1);
        assert_eq!(
            parse_sector_map(&unknown_cell, GreaseweazleProfile::Ibm1440),
            None
        );
    }

    #[test]
    fn offline_comparison_keeps_donor_candidates_separate_from_conflicts() {
        let (project, root) = fixture();
        let mut backend = ArtifactBackend::default();
        let captured = capture(
            &project,
            CaptureRequest {
                disk_number: 7,
                profile: GreaseweazleProfile::Ibm720,
                drive: 'A',
                revolutions: 3,
            },
            &mut backend,
        )
        .unwrap();
        decode(&project, 7, captured.attempt_number, None, &mut backend).unwrap();
        let image_name = "007_attempt_001.img";
        let image_path = project.images_dir().join(image_name);
        let mut bytes = vec![0x33; 737_280];
        bytes[2 * 512] = 0;
        fs::write(&image_path, &bytes).unwrap();
        let metadata = serde_json::json!({
            "fluxvault_version": "test", "status": "PARTIAL", "disk_number": 7,
            "attempt_number": 1, "source_device": "synthetic",
            "image_file": image_name, "timestamp_unix_ms": 1,
            "geometry": {"cylinders": 80, "heads": 2, "sectors_per_track": 9,
                "bytes_per_sector": 512, "total_bytes": 737280, "format_guess": "720KB"},
            "sector_retries": 2, "total_sectors": 1440, "bytes_written": 737280,
            "retry_recovered_sectors": 0, "bad_sector_count": 2,
            "bad_sectors": [
                {"lba": 2, "cylinder": 0, "head": 0, "sector": 3},
                {"lba": 711, "cylinder": 39, "head": 1, "sector": 1}
            ],
            "sha256": sha256_bytes(&bytes)
        });
        let metadata_path = project.images_dir().join("007_attempt_001.json");
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let comparison = compare_with_usb(&project, 7).unwrap();
        assert_eq!(comparison.candidate_flux_donor_lbas, vec![2]);
        assert_eq!(comparison.unresolved_lbas, vec![711]);
        assert!(comparison.conflicting_good_lbas.is_empty());
        assert_eq!(comparison.matching_good_sectors, 1438);
        assert!(comparison.no_reported_good_byte_conflicts);

        bytes[3 * 512] = 0x44;
        fs::write(&image_path, &bytes).unwrap();
        assert!(
            compare_with_usb(&project, 7)
                .unwrap_err()
                .contains("changed")
        );
        let mut metadata = metadata;
        metadata["sha256"] = serde_json::json!(sha256_bytes(&bytes));
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let conflict = compare_with_usb(&project, 7).unwrap();
        assert_eq!(conflict.conflicting_good_lbas, vec![3]);
        assert!(!conflict.no_reported_good_byte_conflicts);

        assert!(
            plan_flux_recovery(&project, 7)
                .unwrap_err()
                .contains("two distinct")
        );
        let second_capture = capture(
            &project,
            CaptureRequest {
                disk_number: 7,
                profile: GreaseweazleProfile::Ibm720,
                drive: 'A',
                revolutions: 3,
            },
            &mut backend,
        )
        .unwrap();
        let second_decode = decode(
            &project,
            7,
            second_capture.attempt_number,
            None,
            &mut backend,
        )
        .unwrap();
        let plan = plan_flux_recovery(&project, 7).unwrap();
        assert_eq!(plan.corroborated_donor_lbas, vec![2]);
        assert_eq!(plan.unresolved_lbas, vec![711]);
        assert_eq!(plan.usb_flux_conflict_lbas, vec![3]);
        assert!(plan.flux_flux_conflict_lbas.is_empty());
        assert_eq!(plan.matching_control_sectors, 1437);
        assert!(!plan.image_promoted);
        assert!(!plan.physical_media_access);
        assert!(
            fs::read_dir(project.root().join("Recovery"))
                .unwrap()
                .next()
                .is_none()
        );

        let mut changed_flux = fs::read(&second_decode.image_path).unwrap();
        changed_flux[2 * 512] = 0x55;
        fs::write(&second_decode.image_path, &changed_flux).unwrap();
        assert!(
            plan_flux_recovery(&project, 7)
                .unwrap_err()
                .contains("changed")
        );
        let mut decode_metadata: DecodeRecord =
            serde_json::from_slice(&fs::read(&second_decode.metadata_path).unwrap()).unwrap();
        decode_metadata.output_sha256 = sha256_bytes(&changed_flux);
        fs::write(
            &second_decode.metadata_path,
            serde_json::to_vec(&decode_metadata).unwrap(),
        )
        .unwrap();
        let disputed = plan_flux_recovery(&project, 7).unwrap();
        assert!(disputed.corroborated_donor_lbas.is_empty());
        assert_eq!(disputed.flux_flux_conflict_lbas, vec![2]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flux_consensus_requires_distinct_capture_hashes_and_detects_byte_conflicts() {
        let (project, root) = fixture();
        let mut backend = ArtifactBackend::default();
        let request = CaptureRequest {
            disk_number: 9,
            profile: GreaseweazleProfile::Ibm720,
            drive: 'A',
            revolutions: 3,
        };
        let first = capture(&project, request, &mut backend).unwrap();
        decode(&project, 9, first.attempt_number, None, &mut backend).unwrap();
        let second = capture(&project, request, &mut backend).unwrap();
        let second_decode = decode(&project, 9, second.attempt_number, None, &mut backend).unwrap();
        let consensus = compare_flux_captures(&project, 9).unwrap();
        assert_eq!(consensus.matching_reported_good_lbas.len(), 1439);
        assert_eq!(consensus.both_reported_bad_lbas, vec![711]);
        assert!(consensus.conflicting_reported_good_lbas.is_empty());

        let mut changed = fs::read(&second_decode.image_path).unwrap();
        changed[3 * 512] = 0x44;
        fs::write(&second_decode.image_path, &changed).unwrap();
        assert!(
            compare_flux_captures(&project, 9)
                .unwrap_err()
                .contains("changed")
        );
        let mut metadata: DecodeRecord =
            serde_json::from_slice(&fs::read(&second_decode.metadata_path).unwrap()).unwrap();
        metadata.output_sha256 = sha256_bytes(&changed);
        fs::write(
            &second_decode.metadata_path,
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let conflict = compare_flux_captures(&project, 9).unwrap();
        assert_eq!(conflict.conflicting_reported_good_lbas, vec![3]);

        fs::write(&second.flux_path, b"changed SCP evidence").unwrap();
        metadata.source_sha256 = hash_file(&second.flux_path).unwrap();
        fs::write(
            &second_decode.metadata_path,
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        assert!(!inspect_disk(&project, 9).unwrap().decodes[1].source_hash_matches);
        assert!(
            compare_flux_captures(&project, 9)
                .unwrap_err()
                .contains("changed")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
