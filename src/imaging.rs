use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::Local;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    dmde_logs::{self, ParsedDmdeLog},
    floppy::{DiskGeometry, FloppyDrive},
    legacy_logs::{self, ParsedArchiverLog},
    safety::MediaSafetyPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorReadState {
    Unread,
    Good,
    RetryRecovered,
    Bad,
}

#[derive(Debug, Clone)]
pub struct ImagingResult {
    pub output_path: PathBuf,
    pub metadata_path: PathBuf,
    pub log_path: PathBuf,
    pub disk_number: u32,
    pub attempt_number: u32,
    pub sha256: String,
    pub total_sectors: usize,
    pub bad_sectors: Vec<u64>,
    pub retry_recovered: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BadSectorMetadata {
    lba: u64,
    cylinder: u64,
    head: u32,
    sector: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcquisitionMetadata {
    fluxvault_version: String,
    status: String,
    disk_number: u32,
    attempt_number: u32,

    #[serde(default)]
    source_backend: String,

    source_device: String,
    image_file: String,

    #[serde(default)]
    log_file: String,

    timestamp_unix_ms: u128,

    geometry: GeometryMetadata,

    sector_retries: usize,
    total_sectors: usize,
    bytes_written: u64,
    retry_recovered_sectors: usize,
    bad_sector_count: usize,
    bad_sectors: Vec<BadSectorMetadata>,

    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeometryMetadata {
    cylinders: u64,
    heads: u32,
    sectors_per_track: u32,
    bytes_per_sector: u32,
    total_bytes: u64,
    format_guess: String,
}

#[derive(Debug, Clone)]
pub struct AttemptSummary {
    pub attempt_number: u32,
    pub status: String,
    pub timestamp_unix_ms: u128,
    pub image_file: String,
    pub metadata_path: PathBuf,
    pub log_file: String,
    pub parsed_log: Option<ParsedArchiverLog>,
    pub parsed_dmde_log: Option<ParsedDmdeLog>,
    pub legacy_image: bool,
    pub attention_required: bool,
    pub sha256: String,
    pub total_sectors: usize,
    pub retry_recovered_sectors: usize,
    pub bad_sectors: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct AttemptComparison {
    pub older_attempt: u32,
    pub newer_attempt: u32,
    pub older_bad_count: usize,
    pub newer_bad_count: usize,
    pub recovered_sectors: Vec<u64>,
    pub newly_bad_sectors: Vec<u64>,
    pub still_bad_sectors: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct DiskSummary {
    pub disk_number: u32,
    pub attempt_count: usize,
    pub latest_attempt_number: u32,
    pub latest_status: String,
    pub latest_bad_sectors: usize,
    pub latest_timestamp_unix_ms: u128,
    pub best_attempt_number: u32,
    pub best_bad_sectors: usize,
    pub attention_required: bool,
    pub total_sectors: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectStatistics {
    pub disk_count: usize,
    pub total_attempts: usize,
    pub ok_disks: usize,
    pub partial_disks: usize,
    pub latest_bad_sectors: usize,
    pub best_known_bad_sectors: usize,
    pub disks: Vec<DiskSummary>,
}

#[derive(Debug)]
pub enum ImagingEvent {
    Started {
        output_path: PathBuf,
        total_sectors: usize,
    },
    Sector {
        lba: usize,
        state: SectorReadState,
    },
    Progress {
        completed: usize,
        total: usize,
    },
    Log(String),
    Completed(ImagingResult),
    Failed(String),
}

pub fn start_imaging(
    drive: FloppyDrive,
    geometry: DiskGeometry,
    output_directory: PathBuf,
    log_directory: PathBuf,
    disk_number: u32,
    sector_retries: usize,
) -> Receiver<ImagingEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        if let Err(error) = run_imaging(
            drive,
            geometry,
            output_directory,
            log_directory,
            disk_number,
            sector_retries,
            &sender,
        ) {
            let _ = sender.send(ImagingEvent::Failed(error));
        }
    });

    receiver
}

fn run_imaging(
    drive: FloppyDrive,
    geometry: DiskGeometry,
    output_directory: PathBuf,
    log_directory: PathBuf,
    disk_number: u32,
    sector_retries: usize,
    sender: &Sender<ImagingEvent>,
) -> Result<(), String> {
    MediaSafetyPolicy::assert_invariants();

    if disk_number == 0 {
        return Err("A lemezszám nem lehet 000.".to_owned());
    }

    if !geometry.looks_like_floppy() {
        return Err(format!(
            "A(z) {} meghajto geometriája nem tunik floppy geometrianak. \
             A teljes kepkeszites biztonsagi okbol megtagadva.",
            drive.root
        ));
    }

    let bytes_per_sector = geometry.bytes_per_sector as usize;
    let sectors_per_track = geometry.sectors_per_track as usize;
    let total_sectors = usize::try_from(geometry.total_sectors())
        .map_err(|_| "Tul nagy szektorszam.".to_owned())?;

    let expected_bytes = geometry.total_bytes();

    let track_size = sectors_per_track
        .checked_mul(bytes_per_sector)
        .ok_or_else(|| "Savmeret tulcsordulas.".to_owned())?;

    let total_tracks = usize::try_from(geometry.cylinders.saturating_mul(geometry.heads as u64))
        .map_err(|_| "Tul nagy savszam.".to_owned())?;

    fs::create_dir_all(&output_directory).map_err(|error| {
        format!(
            "Nem sikerult letrehozni a kimeneti mappat {}: {error}",
            output_directory.display()
        )
    })?;

    fs::create_dir_all(&log_directory).map_err(|error| {
        format!(
            "Nem sikerult letrehozni a naplo mappat {}: {error}",
            log_directory.display()
        )
    })?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Rendszerido hiba: {error}"))?
        .as_millis();

    let attempt_number = next_attempt_number(&output_directory, disk_number)?;

    let stem = format!("{disk_number:03}_attempt_{attempt_number:03}");

    let partial_path = output_directory.join(format!("{stem}.partial.img"));

    let final_path = output_directory.join(format!("{stem}.img"));

    let metadata_partial_path = output_directory.join(format!("{stem}.partial.json"));

    let metadata_final_path = output_directory.join(format!("{stem}.json"));

    let log_partial_path = log_directory.join(format!("{stem}.partial.log"));

    let log_final_path = log_directory.join(format!("{stem}.log"));

    let mut source = File::open(&drive.device_path).map_err(|error| {
        format!(
            "Nem sikerult CSAK OLVASHATO modban megnyitni a(z) {} eszkozt: {error}",
            drive.device_path
        )
    })?;

    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial_path)
        .map_err(|error| {
            format!(
                "Nem sikerult letrehozni a reszleges lemezkepet {}: {error}",
                partial_path.display()
            )
        })?;

    sender
        .send(ImagingEvent::Started {
            output_path: final_path.clone(),
            total_sectors,
        })
        .ok();

    let mut human_log = Vec::new();

    record_attempt_log(
        sender,
        &mut human_log,
        format!(
            "BEGIN | disk={disk_number:03} | attempt={attempt_number:03} | source={}",
            drive.device_path
        ),
    );

    record_attempt_log(
        sender,
        &mut human_log,
        format!(
            "IMAGE | partial={} | final={}",
            partial_path.display(),
            final_path.display()
        ),
    );

    record_attempt_log(
        sender,
        &mut human_log,
        format!(
            "GEOMETRY | cylinders={} | heads={} | sectors_per_track={} | bytes_per_sector={} | total_sectors={} | total_bytes={}",
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
            geometry.bytes_per_sector,
            total_sectors,
            expected_bytes
        ),
    );

    record_attempt_log(
        sender,
        &mut human_log,
        format!(
            "RETRY_POLICY | retries={} | total_attempts_per_failed_sector={} | retry_1=backward | retry_2=forward | additional=alternating",
            sector_retries,
            sector_retries + 1
        ),
    );

    let mut completed = 0usize;
    let mut pending_bad_sectors = BTreeMap::<usize, Vec<String>>::new();
    let mut retry_recovered = 0usize;

    for track_index in 0..total_tracks {
        let track_lba = track_index
            .checked_mul(sectors_per_track)
            .ok_or_else(|| "LBA tulcsordulas.".to_owned())?;

        let track_offset = (track_lba as u64)
            .checked_mul(geometry.bytes_per_sector as u64)
            .ok_or_else(|| "Sav offset tulcsordulas.".to_owned())?;

        let cylinder = track_index / geometry.heads as usize;
        let head = track_index % geometry.heads as usize;

        let mut track_buffer = vec![0u8; track_size];

        let track_read = source
            .seek(SeekFrom::Start(track_offset))
            .and_then(|_| source.read_exact(&mut track_buffer));

        match track_read {
            Ok(()) => {
                output
                    .write_all(&track_buffer)
                    .map_err(|error| format!("Lemezkep irasi hiba: {error}"))?;

                for sector_index in 0..sectors_per_track {
                    let lba = track_lba + sector_index;

                    sender
                        .send(ImagingEvent::Sector {
                            lba,
                            state: SectorReadState::Good,
                        })
                        .ok();

                    completed += 1;

                    sender
                        .send(ImagingEvent::Progress {
                            completed,
                            total: total_sectors,
                        })
                        .ok();
                }
            }
            Err(track_error) => {
                record_attempt_log(
                    sender,
                    &mut human_log,
                    format!(
                        "TRACK_READ_FAILED | C{cylinder:02} H{head} | {track_error} | fallback=sector"
                    ),
                );

                for sector_index in 0..sectors_per_track {
                    let lba = track_lba + sector_index;

                    let sector_offset = (lba as u64)
                        .checked_mul(geometry.bytes_per_sector as u64)
                        .ok_or_else(|| "Szektor offset tulcsordulas.".to_owned())?;

                    let mut sector_buffer = vec![0u8; bytes_per_sector];

                    let read_result = source
                        .seek(SeekFrom::Start(sector_offset))
                        .and_then(|_| source.read_exact(&mut sector_buffer));

                    let state = match read_result {
                        Ok(()) => SectorReadState::Good,
                        Err(error) => {
                            let error = error.to_string();

                            record_attempt_log(
                                sender,
                                &mut human_log,
                                format!(
                                    "SECTOR_READ_FAILED | LBA={lba} | C{cylinder:02} H{head} S{:02} | pass=initial_forward | attempt=1/{} | error={error}",
                                    sector_index + 1,
                                    sector_retries + 1
                                ),
                            );

                            sector_buffer.fill(0);
                            pending_bad_sectors.insert(lba, vec![error]);

                            SectorReadState::Bad
                        }
                    };

                    output
                        .write_all(&sector_buffer)
                        .map_err(|error| format!("Lemezkep irasi hiba: {error}"))?;

                    sender.send(ImagingEvent::Sector { lba, state }).ok();

                    completed += 1;

                    sender
                        .send(ImagingEvent::Progress {
                            completed,
                            total: total_sectors,
                        })
                        .ok();
                }
            }
        }
    }

    for retry_pass in 1..=sector_retries {
        if pending_bad_sectors.is_empty() {
            break;
        }

        let direction = retry_direction(retry_pass);
        let retry_lbas = retry_lba_order(pending_bad_sectors.keys().copied().collect(), retry_pass);
        let pending_at_start = retry_lbas.len();
        let mut recovered_in_pass = 0usize;

        record_attempt_log(
            sender,
            &mut human_log,
            format!(
                "RETRY_PASS_BEGIN | pass={retry_pass}/{sector_retries} | direction={direction} | pending={pending_at_start}"
            ),
        );

        for lba in retry_lbas {
            let sector_offset = (lba as u64)
                .checked_mul(geometry.bytes_per_sector as u64)
                .ok_or_else(|| "Retry szektor offset tulcsordulas.".to_owned())?;
            let mut sector_buffer = vec![0u8; bytes_per_sector];
            let read_result = source
                .seek(SeekFrom::Start(sector_offset))
                .and_then(|_| source.read_exact(&mut sector_buffer));
            let location = bad_sector_metadata(lba as u64, geometry);

            match read_result {
                Ok(()) => {
                    let previous_failures = pending_bad_sectors
                        .get(&lba)
                        .map(Vec::len)
                        .unwrap_or(retry_pass);

                    output
                        .seek(SeekFrom::Start(sector_offset))
                        .and_then(|_| output.write_all(&sector_buffer))
                        .map_err(|error| {
                            format!("Retry utan visszanyert szektor irasi hiba: {error}")
                        })?;

                    pending_bad_sectors.remove(&lba);
                    retry_recovered += 1;
                    recovered_in_pass += 1;

                    sender
                        .send(ImagingEvent::Sector {
                            lba,
                            state: SectorReadState::RetryRecovered,
                        })
                        .ok();

                    record_attempt_log(
                        sender,
                        &mut human_log,
                        format!(
                            "SECTOR_RECOVERED_AFTER_RETRY | LBA={lba} | C{:02} H{} S{:02} | retry_pass={retry_pass} | direction={direction} | attempts_used={}",
                            location.cylinder,
                            location.head,
                            location.sector,
                            previous_failures + 1
                        ),
                    );
                }
                Err(error) => {
                    let error = error.to_string();

                    if let Some(errors) = pending_bad_sectors.get_mut(&lba) {
                        errors.push(error.clone());
                    }

                    record_attempt_log(
                        sender,
                        &mut human_log,
                        format!(
                            "SECTOR_RETRY_FAILED | LBA={lba} | C{:02} H{} S{:02} | retry_pass={retry_pass}/{sector_retries} | direction={direction} | attempt={}/{} | error={error}",
                            location.cylinder,
                            location.head,
                            location.sector,
                            retry_pass + 1,
                            sector_retries + 1
                        ),
                    );
                }
            }
        }

        record_attempt_log(
            sender,
            &mut human_log,
            format!(
                "RETRY_PASS_END | pass={retry_pass}/{sector_retries} | direction={direction} | recovered={recovered_in_pass} | remaining={}",
                pending_bad_sectors.len()
            ),
        );
    }

    let bad_sectors = pending_bad_sectors
        .keys()
        .map(|lba| *lba as u64)
        .collect::<Vec<_>>();

    for (lba, errors) in &pending_bad_sectors {
        let location = bad_sector_metadata(*lba as u64, geometry);
        let last_error = errors
            .last()
            .map(String::as_str)
            .unwrap_or("Ismeretlen olvasasi hiba.");

        record_attempt_log(
            sender,
            &mut human_log,
            format!(
                "BAD_SECTOR | LBA={lba} | C{:02} H{} S{:02} | attempts={} | zero_filled=true | error={last_error}",
                location.cylinder,
                location.head,
                location.sector,
                errors.len()
            ),
        );
    }

    output
        .flush()
        .map_err(|error| format!("Lemezkep flush hiba: {error}"))?;

    output
        .sync_all()
        .map_err(|error| format!("Lemezkep sync hiba: {error}"))?;

    drop(output);

    let actual_bytes = fs::metadata(&partial_path)
        .map_err(|error| format!("Nem sikerult ellenorizni a lemezkep meretet: {error}"))?
        .len();

    if actual_bytes != expected_bytes {
        return Err(format!(
            "HIBAS LEMEZKEPMERET: vart {expected_bytes} bajt, kapott {actual_bytes} bajt. \
             A .partial.img fajl megmarad: {}",
            partial_path.display()
        ));
    }

    let sha256 = sha256_file(&partial_path)?;

    let bad_sector_metadata = bad_sectors
        .iter()
        .copied()
        .map(|lba| bad_sector_metadata(lba, geometry))
        .collect::<Vec<_>>();

    let acquisition_status = if bad_sectors.is_empty() {
        "OK"
    } else {
        "PARTIAL"
    };

    record_attempt_log(
        sender,
        &mut human_log,
        format!(
            "END | status={acquisition_status} | bad_sectors={} | retry_recovered={} | bytes={} | sha256={sha256}",
            bad_sectors.len(),
            retry_recovered,
            actual_bytes
        ),
    );

    record_attempt_log(
        sender,
        &mut human_log,
        format!("METADATA | {}", metadata_final_path.display()),
    );

    record_attempt_log(
        sender,
        &mut human_log,
        format!("LOG | {}", log_final_path.display()),
    );

    let metadata = AcquisitionMetadata {
        fluxvault_version: env!("CARGO_PKG_VERSION").to_owned(),
        status: acquisition_status.to_owned(),
        disk_number,
        attempt_number,
        source_backend: "windows-raw-sector".to_owned(),
        source_device: drive.device_path.clone(),
        image_file: final_path.display().to_string(),
        log_file: log_final_path.display().to_string(),
        timestamp_unix_ms: timestamp,

        geometry: GeometryMetadata {
            cylinders: geometry.cylinders,
            heads: geometry.heads,
            sectors_per_track: geometry.sectors_per_track,
            bytes_per_sector: geometry.bytes_per_sector,
            total_bytes: geometry.total_bytes(),
            format_guess: geometry.format_guess().to_owned(),
        },

        sector_retries,
        total_sectors,
        bytes_written: actual_bytes,
        retry_recovered_sectors: retry_recovered,
        bad_sector_count: bad_sectors.len(),
        bad_sectors: bad_sector_metadata,

        sha256: sha256.clone(),
    };

    let metadata_json = serde_json::to_string_pretty(&metadata)
        .map_err(|error| format!("Metadata JSON generalasi hiba: {error}"))?;

    fs::write(&metadata_partial_path, metadata_json).map_err(|error| {
        format!(
            "Nem sikerult kiirni a metadata fajlt {}: {error}",
            metadata_partial_path.display()
        )
    })?;

    let human_log_text = format!("{}\r\n", human_log.join("\r\n"));

    let mut log_output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log_partial_path)
        .map_err(|error| {
            format!(
                "Nem sikerult letrehozni a human-readable naplot {}: {error}",
                log_partial_path.display()
            )
        })?;

    log_output
        .write_all(human_log_text.as_bytes())
        .map_err(|error| format!("Napló írási hiba: {error}"))?;

    log_output
        .flush()
        .map_err(|error| format!("Napló flush hiba: {error}"))?;

    log_output
        .sync_all()
        .map_err(|error| format!("Napló sync hiba: {error}"))?;

    drop(log_output);

    fs::rename(&partial_path, &final_path).map_err(|error| {
        format!(
            "A kesz lemezkep atnevezese sikertelen. A partial fajl megmarad: {}: {error}",
            partial_path.display()
        )
    })?;

    fs::rename(&metadata_partial_path, &metadata_final_path).map_err(|error| {
        format!(
            "A metadata fajl atnevezese sikertelen. A partial metadata megmarad: {}: {error}",
            metadata_partial_path.display()
        )
    })?;

    fs::rename(&log_partial_path, &log_final_path).map_err(|error| {
        format!(
            "A naplofajl atnevezese sikertelen. A partial naplo megmarad: {}: {error}",
            log_partial_path.display()
        )
    })?;

    sender
        .send(ImagingEvent::Log(format!(
            "Kepkeszites befejezve: {}",
            final_path.display()
        )))
        .ok();

    sender
        .send(ImagingEvent::Log(format!(
            "Metadata: {}",
            metadata_final_path.display()
        )))
        .ok();

    sender
        .send(ImagingEvent::Log(format!("SHA-256: {sha256}")))
        .ok();

    sender
        .send(ImagingEvent::Completed(ImagingResult {
            output_path: final_path,
            metadata_path: metadata_final_path,
            log_path: log_final_path,
            disk_number,
            attempt_number,
            sha256,
            total_sectors,
            bad_sectors,
            retry_recovered,
            bytes_written: actual_bytes,
        }))
        .ok();

    Ok(())
}

fn record_attempt_log(
    sender: &Sender<ImagingEvent>,
    human_log: &mut Vec<String>,
    message: impl Into<String>,
) {
    let message = message.into();

    human_log.push(format!(
        "[{}] {}",
        Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        message
    ));

    sender.send(ImagingEvent::Log(message)).ok();
}

fn retry_direction(retry_pass: usize) -> &'static str {
    if retry_pass % 2 == 1 {
        "backward"
    } else {
        "forward"
    }
}

fn retry_lba_order(mut lbas: Vec<usize>, retry_pass: usize) -> Vec<usize> {
    lbas.sort_unstable();

    if retry_pass % 2 == 1 {
        lbas.reverse();
    }

    lbas
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| {
        format!(
            "Nem sikerult megnyitni a lemezkepet hash-eleshez {}: {error}",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|error| format!("Lemezkep hash olvasasi hiba {}: {error}", path.display()))?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn next_attempt_number(directory: &Path, disk_number: u32) -> Result<u32, String> {
    let prefix = format!("{disk_number:03}_attempt_");
    let mut highest_attempt = 0u32;

    let entries = fs::read_dir(directory)
        .map_err(|error| format!("Nem sikerult megvizsgalni a captures mappat: {error}"))?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("Hibas captures mappa bejegyzes: {error}"))?;

        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();

        let Some(rest) = file_name.strip_prefix(&prefix) else {
            continue;
        };

        let attempt_text = rest.split('.').next().unwrap_or_default();

        let Ok(attempt) = attempt_text.parse::<u32>() else {
            continue;
        };

        highest_attempt = highest_attempt.max(attempt);
    }

    Ok(highest_attempt.saturating_add(1).max(1))
}

fn bad_sector_metadata(lba: u64, geometry: DiskGeometry) -> BadSectorMetadata {
    let sectors_per_cylinder = geometry.heads as u64 * geometry.sectors_per_track as u64;

    let cylinder = lba / sectors_per_cylinder;
    let within_cylinder = lba % sectors_per_cylinder;

    let head = (within_cylinder / geometry.sectors_per_track as u64) as u32;

    let sector = (within_cylinder % geometry.sectors_per_track as u64) as u32 + 1;

    BadSectorMetadata {
        lba,
        cylinder,
        head,
        sector,
    }
}

pub fn load_attempts_for_disk(
    directory: &Path,
    disk_number: u32,
) -> Result<Vec<AttemptSummary>, String> {
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let prefix = format!("{disk_number:03}_attempt_");
    let mut attempts = Vec::new();

    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "Nem sikerült megvizsgálni a(z) {} mappát: {error}",
            directory.display()
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("Hibás captures mappa bejegyzés: {error}"))?;

        let path = entry.path();

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if !file_name.starts_with(&prefix)
            || !file_name.ends_with(".json")
            || file_name.ends_with(".partial.json")
        {
            continue;
        }

        let json = fs::read_to_string(&path).map_err(|error| {
            format!(
                "Nem sikerült beolvasni a metadata fájlt {}: {error}",
                path.display()
            )
        })?;

        let metadata: AcquisitionMetadata = serde_json::from_str(&json)
            .map_err(|error| format!("Hibás metadata JSON {}: {error}", path.display()))?;

        if metadata.disk_number != disk_number {
            continue;
        }

        let resolved_log_path =
            resolve_archiver_log_path(directory, &metadata.log_file, metadata.disk_number);
        let parsed_log = resolved_log_path
            .as_deref()
            .and_then(|log_path| legacy_logs::parse_archiver_log_file(log_path).ok());
        let log_file = resolved_log_path
            .map(|path| path.display().to_string())
            .unwrap_or(metadata.log_file);

        let attention_required =
            !metadata.status.eq_ignore_ascii_case("OK") || !metadata.bad_sectors.is_empty();

        attempts.push(AttemptSummary {
            attempt_number: metadata.attempt_number,
            status: metadata.status,
            timestamp_unix_ms: metadata.timestamp_unix_ms,
            image_file: metadata.image_file,
            metadata_path: path,
            log_file,
            parsed_log,
            parsed_dmde_log: None,
            legacy_image: false,
            attention_required,
            sha256: metadata.sha256,
            total_sectors: metadata.total_sectors,
            retry_recovered_sectors: metadata.retry_recovered_sectors,
            bad_sectors: metadata
                .bad_sectors
                .into_iter()
                .map(|sector| sector.lba)
                .collect(),
        });
    }

    if let Some(legacy_attempt) = load_legacy_attempt(directory, disk_number)? {
        attempts.push(legacy_attempt);
    }

    attempts.sort_by_key(|attempt| attempt.attempt_number);

    Ok(attempts)
}

fn load_legacy_attempt(
    directory: &Path,
    disk_number: u32,
) -> Result<Option<AttemptSummary>, String> {
    let disk_stem = format!("{disk_number:03}");
    let mut candidates = fs::read_dir(directory)
        .map_err(|error| format!("Nem sikerült megvizsgálni a legacy képeket: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_stem().and_then(|stem| stem.to_str()) == Some(disk_stem.as_str())
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "bin" | "img" | "ima"
                        )
                    })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "bin" => 0,
            "img" => 1,
            "ima" => 2,
            _ => 3,
        }
    });

    let Some(image_path) = candidates.into_iter().next() else {
        return Ok(None);
    };
    let resolved_log_path = resolve_archiver_log_path(directory, "", disk_number);
    let parsed_log = resolved_log_path
        .as_deref()
        .and_then(|path| legacy_logs::parse_archiver_log_file(path).ok());
    let parsed_dmde_log = if parsed_log.is_none() {
        resolved_log_path
            .as_deref()
            .and_then(|path| dmde_logs::parse_dmde_log_file(path).ok())
    } else {
        None
    };
    let image_metadata = fs::metadata(&image_path).map_err(|error| {
        format!(
            "Nem olvasható a legacy lemezkép metadata {}: {error}",
            image_path.display()
        )
    })?;
    let timestamp_unix_ms = image_metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let image_sha256 = parsed_log
        .as_ref()
        .and_then(|log| log.sha256.clone())
        .unwrap_or(sha256_file(&image_path)?);
    let sector_size = parsed_log
        .as_ref()
        .and_then(|log| log.geometry.bytes_per_sector)
        .or_else(|| parsed_dmde_log.as_ref().and_then(|log| log.sector_size))
        .unwrap_or(512) as u64;
    let total_sectors = parsed_log
        .as_ref()
        .and_then(|log| log.geometry.total_sectors)
        .or_else(|| {
            parsed_dmde_log
                .as_ref()
                .map(|log| log.highest_sector_exclusive)
        })
        .unwrap_or_else(|| image_metadata.len() / sector_size) as usize;
    let bad_sectors = parsed_log
        .as_ref()
        .map(|log| log.bad_sectors.clone())
        .or_else(|| parsed_dmde_log.as_ref().map(|log| log.bad_sectors.clone()))
        .unwrap_or_default();
    let retry_recovered_sectors = parsed_log
        .as_ref()
        .map(|log| log.retry_recovered)
        .unwrap_or(0);
    let status = parsed_log
        .as_ref()
        .map(|log| log.status.label().to_owned())
        .or_else(|| {
            parsed_dmde_log
                .as_ref()
                .map(|log| log.status.label().to_owned())
        })
        .unwrap_or_else(|| "MISSING LOG".to_owned());
    let attention_required = !status.eq_ignore_ascii_case("OK") || !bad_sectors.is_empty();
    let image_file = image_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();

    Ok(Some(AttemptSummary {
        attempt_number: 0,
        status,
        timestamp_unix_ms,
        image_file,
        metadata_path: PathBuf::new(),
        log_file: resolved_log_path
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        parsed_log,
        parsed_dmde_log,
        legacy_image: true,
        attention_required,
        sha256: image_sha256,
        total_sectors,
        retry_recovered_sectors,
        bad_sectors,
    }))
}

fn resolve_archiver_log_path(
    image_directory: &Path,
    configured_log: &str,
    disk_number: u32,
) -> Option<PathBuf> {
    if !configured_log.is_empty() {
        let configured_path = PathBuf::from(configured_log);
        let candidates = if configured_path.is_absolute() {
            vec![configured_path]
        } else {
            let mut candidates = vec![image_directory.join(&configured_path)];

            if let Some(project_root) = image_directory.parent() {
                candidates.push(project_root.join(&configured_path));
            }

            candidates
        };

        if let Some(existing) = candidates.into_iter().find(|path| path.is_file()) {
            return Some(existing);
        }
    }

    let logs_directory = image_directory.parent()?.join("Logs");
    let log_paths = fs::read_dir(logs_directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("log"))
        })
        .collect::<Vec<_>>();

    legacy_logs::choose_primary_log(log_paths.iter(), disk_number)
}

pub fn compare_latest_attempts(attempts: &[AttemptSummary]) -> Option<AttemptComparison> {
    if attempts.len() < 2 {
        return None;
    }

    let older = &attempts[attempts.len() - 2];
    let newer = &attempts[attempts.len() - 1];

    if older.total_sectors != newer.total_sectors {
        return None;
    }

    let older_bad = older
        .bad_sectors
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();

    let newer_bad = newer
        .bad_sectors
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();

    let recovered_sectors = older_bad.difference(&newer_bad).copied().collect();

    let newly_bad_sectors = newer_bad.difference(&older_bad).copied().collect();

    let still_bad_sectors = older_bad.intersection(&newer_bad).copied().collect();

    Some(AttemptComparison {
        older_attempt: older.attempt_number,
        newer_attempt: newer.attempt_number,
        older_bad_count: older.bad_sectors.len(),
        newer_bad_count: newer.bad_sectors.len(),
        recovered_sectors,
        newly_bad_sectors,
        still_bad_sectors,
    })
}

pub fn load_project_statistics(directory: &Path) -> Result<ProjectStatistics, String> {
    let mut statistics = ProjectStatistics::default();

    if !directory.exists() {
        return Ok(statistics);
    }

    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "Nem sikerült megvizsgálni a(z) {} mappát: {error}",
            directory.display()
        )
    })?;

    let mut disk_numbers = BTreeSet::new();

    for entry in entries {
        let entry = entry.map_err(|error| format!("Hibás acquisition mappa bejegyzés: {error}"))?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if file_name.to_ascii_lowercase().contains(".partial.") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };

        if let Some((disk_text, _)) = stem.split_once("_attempt_") {
            if let Ok(disk_number) = disk_text.parse::<u32>() {
                disk_numbers.insert(disk_number);
            }
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "bin" | "img" | "ima"
                )
            })
        {
            if let Ok(disk_number) = stem.parse::<u32>() {
                disk_numbers.insert(disk_number);
            }
        }
    }

    for disk_number in disk_numbers {
        let attempts = load_attempts_for_disk(directory, disk_number)?;

        let Some(latest) = attempts.last() else {
            continue;
        };

        let Some(best) = attempts.iter().min_by(|left, right| {
            left.attention_required
                .cmp(&right.attention_required)
                .then_with(|| left.bad_sectors.len().cmp(&right.bad_sectors.len()))
                .then_with(|| right.attempt_number.cmp(&left.attempt_number))
        }) else {
            continue;
        };

        let attempt_count = attempts.len();

        statistics.total_attempts += attempt_count;
        statistics.latest_bad_sectors += latest.bad_sectors.len();
        statistics.best_known_bad_sectors += best.bad_sectors.len();

        if !best.attention_required {
            statistics.ok_disks += 1;
        } else {
            statistics.partial_disks += 1;
        }

        statistics.disks.push(DiskSummary {
            disk_number,
            attempt_count,
            latest_attempt_number: latest.attempt_number,
            latest_status: latest.status.clone(),
            latest_bad_sectors: latest.bad_sectors.len(),
            latest_timestamp_unix_ms: latest.timestamp_unix_ms,
            best_attempt_number: best.attempt_number,
            best_bad_sectors: best.bad_sectors.len(),
            attention_required: best.attention_required,
            total_sectors: best.total_sectors,
        });
    }

    statistics.disk_count = statistics.disks.len();

    Ok(statistics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_retry_pass_reads_bad_sectors_backward() {
        assert_eq!(retry_direction(1), "backward");
        assert_eq!(retry_lba_order(vec![16, 24, 3], 1), vec![24, 16, 3]);
    }

    #[test]
    fn second_retry_pass_reads_remaining_sectors_forward() {
        assert_eq!(retry_direction(2), "forward");
        assert_eq!(retry_lba_order(vec![16, 24, 3], 2), vec![3, 16, 24]);
    }

    #[test]
    fn additional_retry_passes_continue_alternating() {
        assert_eq!(retry_direction(3), "backward");
        assert_eq!(retry_direction(4), "forward");
    }

    #[test]
    #[ignore = "requires FLUXVAULT_LEGACY_FIXTURE_ROOT"]
    fn imports_real_legacy_dmde_and_archiver_images() {
        let fixture_root = PathBuf::from(
            std::env::var("FLUXVAULT_LEGACY_FIXTURE_ROOT")
                .expect("FLUXVAULT_LEGACY_FIXTURE_ROOT is required"),
        );
        let images = fixture_root.join("Images");

        let disk_001 = load_attempts_for_disk(&images, 1).unwrap();
        assert_eq!(disk_001.len(), 1);
        assert!(disk_001[0].legacy_image);
        assert!(disk_001[0].parsed_dmde_log.is_some());
        assert_eq!(disk_001[0].status, "OK");

        let disk_007 = load_attempts_for_disk(&images, 7).unwrap();
        assert_eq!(disk_007.len(), 1);
        assert!(disk_007[0].parsed_log.is_some());
        assert_eq!(disk_007[0].status, "OK");

        let disk_009 = load_attempts_for_disk(&images, 9).unwrap();
        assert_eq!(disk_009.len(), 1);
        assert!(disk_009[0].parsed_dmde_log.is_some());
        assert_eq!(disk_009[0].bad_sectors.len(), 954);

        let statistics = load_project_statistics(&images).unwrap();
        assert_eq!(statistics.disk_count, 3);
        assert_eq!(statistics.ok_disks, 2);
        assert_eq!(statistics.partial_disks, 1);
    }
}
