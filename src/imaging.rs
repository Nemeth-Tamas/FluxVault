use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

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
    pub sha256: String,
    pub total_sectors: usize,
    pub bad_sectors: Vec<u64>,
    pub retry_recovered: usize,
    pub bytes_written: u64,
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
    sector_retries: usize,
) -> Receiver<ImagingEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        if let Err(error) = run_imaging(drive, geometry, sector_retries, &sender) {
            let _ = sender.send(ImagingEvent::Failed(error));
        }
    });

    receiver
}

fn run_imaging(
    drive: FloppyDrive,
    geometry: DiskGeometry,
    sector_retries: usize,
    sender: &Sender<ImagingEvent>,
) -> Result<(), String> {
    MediaSafetyPolicy::assert_invariants();

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

    fs::create_dir_all("captures")
        .map_err(|error| format!("Nem sikerult letrehozni a captures mappat: {error}"))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Rendszerido hiba: {error}"))?
        .as_millis();

    let stem = format!("capture_{}_{}", timestamp, std::process::id());

    let partial_path = PathBuf::from("captures").join(format!("{stem}.partial.img"));
    let final_path = PathBuf::from("captures").join(format!("{stem}.img"));

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

    fs::rename(&partial_path, &final_path).map_err(|error| {
        format!(
            "A kesz lemezkep atnevezese sikertelen. A partial fajl megmarad: {}: {error}",
            partial_path.display()
        )
    })?;

    let sha256 = format!("{:x}", hasher.finalize());

    sender
        .send(ImagingEvent::Log(format!(
            "Kepkeszites befejezve: {}",
            final_path.display()
        )))
        .ok();

    sender
        .send(ImagingEvent::Log(format!("SHA-256: {sha256}")))
        .ok();

    sender
        .send(ImagingEvent::Completed(ImagingResult {
            output_path: final_path,
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
