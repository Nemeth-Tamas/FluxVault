//! Immutable raw-flux captures and separately derived, unverified sector images.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    greaseweazle::{GreaseweazleBackend, GreaseweazleCommand, GreaseweazleProfile},
    project::ProjectState,
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
}

#[derive(Debug, Clone)]
pub struct CaptureResult {
    pub disk_number: u32,
    pub attempt_number: u32,
    pub flux_path: PathBuf,
    pub metadata_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
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
    command: Vec<String>,
    #[serde(default)]
    reported_found_sectors: Option<usize>,
    #[serde(default)]
    reported_total_sectors: Option<usize>,
    #[serde(default)]
    gw_bad_lbas: Option<Vec<u64>>,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct DecodeInspection {
    pub capture_attempt: u32,
    pub decode_attempt: u32,
    pub profile: String,
    pub metadata: PathBuf,
    pub image: PathBuf,
    pub output_hash_matches: bool,
    pub source_hash_matches: bool,
    pub gw_reported_found_sectors: Option<usize>,
    pub gw_reported_total_sectors: Option<usize>,
    pub gw_bad_lbas: Option<Vec<u64>>,
    pub sector_quality: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FluxDiskStatus {
    pub disk_number: u32,
    pub captures: Vec<CaptureInspection>,
    pub decodes: Vec<DecodeInspection>,
    pub evidence_healthy: bool,
    pub attention_required: bool,
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
                && fs::metadata(&expected_raw)
                    .ok()
                    .is_some_and(|info| Some(info.len()) == record.bytes)
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
        });
    }
    captures.sort_by_key(|capture| capture.attempt);

    let mut decodes = Vec::new();
    let derived_dir = flux_dir.join("Derived");
    if derived_dir.is_dir() {
        for entry in fs::read_dir(&derived_dir)
            .map_err(|error| format!("Cannot list decoded images: {error}"))?
        {
            let entry = entry.map_err(|error| format!("Cannot inspect decoded image: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&format!("{disk_number:03}_flux_")) || !name.ends_with(".json") {
                continue;
            }
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
            if record.output_file != expected_name {
                return Err(format!(
                    "Unexpected decoded-image filename in {}",
                    metadata.display()
                ));
            }
            let image = derived_dir.join(&record.output_file);
            let output_hash_matches = fs::metadata(&image)
                .ok()
                .is_some_and(|info| info.len() == record.bytes)
                && hash_file(&image).ok().as_deref() == Some(record.output_sha256.as_str());
            let expected_source =
                format!("{disk_number:03}_attempt_{:03}.scp", record.capture_attempt);
            let source_hash_matches = record.source_flux_file == expected_source
                && hash_file(&flux_dir.join(&expected_source)).ok().as_deref()
                    == Some(record.source_sha256.as_str());
            decodes.push(DecodeInspection {
                capture_attempt: record.capture_attempt,
                decode_attempt: record.decode_attempt,
                profile: record.profile,
                metadata,
                image,
                output_hash_matches,
                source_hash_matches,
                gw_reported_found_sectors: record.reported_found_sectors,
                gw_reported_total_sectors: record.reported_total_sectors,
                gw_bad_lbas: record.gw_bad_lbas,
                sector_quality: record.status,
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
    let command = GreaseweazleCommand::raw_flux_read(
        request.profile,
        request.drive,
        request.revolutions,
        &partial_flux,
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
    if !execution.success {
        record.status = "failed".to_owned();
        record.detail = Some(format!(
            "gw exited {:?}: {} {}",
            execution.exit_code, execution.stdout, execution.stderr
        ));
        save_record(&partial_metadata, &record)?;
        return Err(format!(
            "Raw capture failed; attempt evidence remains at {}: {}",
            partial_metadata.display(),
            record.detail.as_deref().unwrap_or("unknown error")
        ));
    }
    let bytes = fs::metadata(&partial_flux)
        .map_err(|error| format!("Successful gw run produced no SCP file: {error}"))?
        .len();
    if bytes == 0 {
        return Err(format!(
            "Greaseweazle produced an empty SCP file: {}",
            partial_flux.display()
        ));
    }
    let sha256 = hash_file(&partial_flux)?;
    if final_flux.exists() || final_metadata.exists() {
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
    let derived_dir = flux_dir.join("Derived");
    fs::create_dir_all(&derived_dir)
        .map_err(|error| format!("Cannot create derived-image directory: {error}"))?;
    let profile_slug = profile.argument().replace('.', "_");
    let prefix = format!("{disk_number:03}_flux_{capture_attempt:03}_{profile_slug}");
    let decode_attempt = next_decode_attempt(&derived_dir, &prefix)?;
    let derived_stem = format!("{prefix}_decode_{decode_attempt:03}");
    let partial_image = derived_dir.join(format!("{derived_stem}.partial.img"));
    let final_image = derived_dir.join(format!("{derived_stem}.img"));
    let final_metadata = derived_dir.join(format!("{derived_stem}.json"));
    let command =
        GreaseweazleCommand::convert_flux_to_sector_image(profile, &input, &partial_image)?;
    if final_image.exists() || final_metadata.exists() || partial_image.exists() {
        return Err("Decode destination already exists; refusing to overwrite".to_owned());
    }
    let execution = backend.execute(&command)?;
    if !execution.success {
        return Err(format!(
            "Greaseweazle decode failed (exit {:?}); any partial image remains at {}: {} {}",
            execution.exit_code,
            partial_image.display(),
            execution.stdout,
            execution.stderr
        ));
    }
    let bytes = fs::metadata(&partial_image)
        .map_err(|error| format!("Successful gw convert produced no image: {error}"))?
        .len();
    if bytes != profile.expected_sector_image_bytes() {
        return Err(format!(
            "Decoded image has {bytes} bytes, expected {}; partial evidence remains at {}",
            profile.expected_sector_image_bytes(),
            partial_image.display()
        ));
    }
    let output_hash = hash_file(&partial_image)?;
    let reported_sectors =
        parse_sector_summary(&execution.stdout).or_else(|| parse_sector_summary(&execution.stderr));
    let gw_bad_lbas = parse_sector_map(&execution.stdout, profile)
        .or_else(|| parse_sector_map(&execution.stderr, profile));
    fs::rename(&partial_image, &final_image)
        .map_err(|error| format!("Cannot finalize decoded image: {error}"))?;
    let decode_record = DecodeRecord {
        schema_version: SCHEMA_VERSION,
        disk_number,
        capture_attempt,
        decode_attempt,
        profile: profile.argument().to_owned(),
        source_flux_file: flux_file.to_owned(),
        source_sha256: source_hash.to_owned(),
        output_file: format!("{derived_stem}.img"),
        output_sha256: output_hash.clone(),
        bytes,
        status: "unverified_sector_quality".to_owned(),
        command: command.arguments().to_vec(),
        reported_found_sectors: reported_sectors.map(|(found, _)| found),
        reported_total_sectors: reported_sectors.map(|(_, total)| total),
        gw_bad_lbas: gw_bad_lbas.clone(),
    };
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&final_metadata)
        .map_err(|error| format!("Cannot create decode metadata: {error}"))?;
    output
        .write_all(&serde_json::to_vec_pretty(&decode_record).map_err(|error| error.to_string())?)
        .map_err(|error| format!("Cannot save decode metadata: {error}"))?;
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
    })
}

fn parse_sector_summary(output: &str) -> Option<(usize, usize)> {
    output.lines().rev().find_map(|line| {
        let line = line.trim();
        let (found, rest) = line.strip_prefix("Found ")?.split_once(" sectors of ")?;
        let (total, _) = rest.split_once(' ')?;
        let found = found.parse::<usize>().ok()?;
        let total = total.parse::<usize>().ok()?;
        (total > 0 && found <= total).then_some((found, total))
    })
}

/// Parse only a complete IBM 80-cylinder grid matching gw's final sector total.
/// This is a report from gw, not independent CRC verification of the .img bytes.
fn parse_sector_map(output: &str, profile: GreaseweazleProfile) -> Option<Vec<u64>> {
    let sectors_per_track = match profile {
        GreaseweazleProfile::Ibm1440 => 18usize,
        GreaseweazleProfile::Ibm720 => 9usize,
    };
    let cylinders = 80usize;
    let heads = 2usize;
    let expected_total = cylinders * heads * sectors_per_track;
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

fn project_flux_dir(project: &ProjectState) -> Result<PathBuf, String> {
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

fn next_capture_attempt(directory: &Path, disk_number: u32) -> Result<u32, String> {
    for attempt in 1..=999_999u32 {
        let stem = format!("{disk_number:03}_attempt_{attempt:03}");
        if [".scp", ".partial.scp", ".json", ".partial.json"]
            .iter()
            .all(|suffix| !directory.join(format!("{stem}{suffix}")).exists())
        {
            return Ok(attempt);
        }
    }
    Err("Too many raw-flux attempts for this disk".to_owned())
}

fn next_decode_attempt(directory: &Path, prefix: &str) -> Result<u32, String> {
    for attempt in 1..=999_999u32 {
        let stem = format!("{prefix}_decode_{attempt:03}");
        if [".img", ".partial.img", ".json"]
            .iter()
            .all(|suffix| !directory.join(format!("{stem}{suffix}")).exists())
        {
            return Ok(attempt);
        }
    }
    Err("Too many decodes for this capture/profile".to_owned())
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
}
