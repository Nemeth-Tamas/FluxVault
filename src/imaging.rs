use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    floppy::{DiskGeometry, FloppyDrive},
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
    source_device: String,
    image_file: String,
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
    disk_number: u32,
    sector_retries: usize,
) -> Receiver<ImagingEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        if let Err(error) = run_imaging(
            drive,
            geometry,
            output_directory,
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

    sender
        .send(ImagingEvent::Log(format!(
            "Kepkeszites indul: {} -> {}",
            drive.device_path,
            partial_path.display()
        )))
        .ok();

    sender
        .send(ImagingEvent::Log(format!(
            "Geometria: {} cilinder, {} fej, {} szektor/sav, {} bajt/szektor.",
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
            geometry.bytes_per_sector
        )))
        .ok();

    let mut hasher = Sha256::new();
    let mut completed = 0usize;
    let mut bad_sectors = Vec::new();
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

                hasher.update(&track_buffer);

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
                sender
                    .send(ImagingEvent::Log(format!(
                        "Savolvasasi hiba C{cylinder:02} H{head}: {track_error}. \
                         Atallas szektoronkenti olvasasra."
                    )))
                    .ok();

                for sector_index in 0..sectors_per_track {
                    let lba = track_lba + sector_index;

                    let sector_offset = (lba as u64)
                        .checked_mul(geometry.bytes_per_sector as u64)
                        .ok_or_else(|| "Szektor offset tulcsordulas.".to_owned())?;

                    let mut sector_buffer = vec![0u8; bytes_per_sector];

                    let state = read_sector_with_retries(
                        &mut source,
                        sector_offset,
                        &mut sector_buffer,
                        sector_retries,
                    );

                    let state = match state {
                        Ok(state) => {
                            if state == SectorReadState::RetryRecovered {
                                retry_recovered += 1;

                                sender
                                    .send(ImagingEvent::Log(format!(
                                        "Szektor retry utan olvashato: LBA {lba}, \
                                         C{cylinder:02} H{head} S{:02}.",
                                        sector_index + 1
                                    )))
                                    .ok();
                            }

                            state
                        }
                        Err(error) => {
                            sector_buffer.fill(0);
                            bad_sectors.push(lba as u64);

                            sender
                                .send(ImagingEvent::Log(format!(
                                    "HIBAS SZEKTOR: LBA {lba}, C{cylinder:02} H{head} S{:02}: {error}",
                                    sector_index + 1
                                )))
                                .ok();

                            SectorReadState::Bad
                        }
                    };

                    output
                        .write_all(&sector_buffer)
                        .map_err(|error| format!("Lemezkep irasi hiba: {error}"))?;

                    hasher.update(&sector_buffer);

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

    let sha256 = format!("{:x}", hasher.finalize());

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

    let metadata = AcquisitionMetadata {
        fluxvault_version: env!("CARGO_PKG_VERSION").to_owned(),
        status: acquisition_status.to_owned(),
        disk_number,
        attempt_number,
        source_device: drive.device_path.clone(),
        image_file: final_path.display().to_string(),
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

fn read_sector_with_retries(
    source: &mut File,
    offset: u64,
    buffer: &mut [u8],
    retry_count: usize,
) -> Result<SectorReadState, String> {
    let mut last_error = None;

    for attempt in 0..=retry_count {
        let result = source
            .seek(SeekFrom::Start(offset))
            .and_then(|_| source.read_exact(buffer));

        match result {
            Ok(()) => {
                return Ok(if attempt == 0 {
                    SectorReadState::Good
                } else {
                    SectorReadState::RetryRecovered
                });
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }

    Err(last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "Ismeretlen szektorolvasasi hiba.".to_owned()))
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

        attempts.push(AttemptSummary {
            attempt_number: metadata.attempt_number,
            status: metadata.status,
            timestamp_unix_ms: metadata.timestamp_unix_ms,
            image_file: metadata.image_file,
            metadata_path: path,
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

    attempts.sort_by_key(|attempt| attempt.attempt_number);

    Ok(attempts)
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
