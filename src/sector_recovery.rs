use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

const PROVENANCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct ReconstructionRequest {
    pub image_path: PathBuf,
    pub expected_sha256: Option<String>,
    pub recovery_root: PathBuf,
    pub disk_number: u32,
    pub attempt_number: u32,
    pub bad_sectors: Vec<u64>,
}

#[derive(Debug, Clone)]
pub enum ReconstructionEvent {
    Stage(String),
    Finished(Result<ReconstructionResult, String>),
}

#[derive(Debug, Clone)]
pub struct ReconstructionResult {
    pub source_image: PathBuf,
    pub derived_image: Option<PathBuf>,
    pub provenance_path: Option<PathBuf>,
    pub source_sha256: String,
    pub derived_sha256: Option<String>,
    pub reconstructed: Vec<ReconstructionRecord>,
    pub unresolved_bad_sectors: Vec<u64>,
    pub filesystem: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconstructionRecord {
    pub target_lba: u64,
    pub source_lba: u64,
    pub method: String,
}

#[derive(Debug, Clone, Serialize)]
struct ReconstructionProvenance {
    schema_version: u32,
    created_unix_ms: u64,
    source_image: String,
    source_sha256: String,
    derived_image: String,
    derived_sha256: String,
    filesystem: String,
    reconstructed_sectors: Vec<ReconstructionRecord>,
    unresolved_bad_sectors: Vec<u64>,
    warning: String,
}

#[derive(Debug, Clone, Copy)]
struct FatLayout {
    bytes_per_sector: usize,
    total_sectors: u64,
    fat_start_lba: u64,
    fat_count: u64,
    sectors_per_fat: u64,
    fat_bits: u8,
}

pub fn spawn_reconstruction(request: ReconstructionRequest) -> Receiver<ReconstructionEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let send_stage = |message: &str| {
            let _ = sender.send(ReconstructionEvent::Stage(message.to_owned()));
        };
        let result = run_reconstruction(&request, &send_stage);
        let _ = sender.send(ReconstructionEvent::Finished(result));
    });

    receiver
}

fn run_reconstruction(
    request: &ReconstructionRequest,
    send_stage: &impl Fn(&str),
) -> Result<ReconstructionResult, String> {
    if request.bad_sectors.is_empty() {
        return Err("A FAT rekonstrukció hibás szektorlistát igényel.".to_owned());
    }

    send_stage("Forrás lemezkép és FAT geometria elemzése...");
    let metadata = fs::symlink_metadata(&request.image_path).map_err(|error| {
        format!(
            "Nem vizsgálható a forrás lemezkép {}: {error}",
            request.image_path.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err(
            "A FAT rekonstrukció csak közvetlen, legfeljebb 64 MiB méretű lemezképen használható."
                .to_owned(),
        );
    }
    let source_bytes = fs::read(&request.image_path).map_err(|error| {
        format!(
            "Nem sikerült read-only módban beolvasni a forrás lemezképet {}: {error}",
            request.image_path.display()
        )
    })?;
    let source_sha256 = sha256_bytes(&source_bytes);
    if request
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| !source_sha256.eq_ignore_ascii_case(expected))
    {
        return Err(format!(
            "A forrás lemezkép SHA-256 értéke eltér a rögzített acquisition eredménytől: {}",
            request.image_path.display()
        ));
    }
    let layout = FatLayout::parse(&source_bytes)?;
    let unique_bad = request.bad_sectors.iter().copied().collect::<BTreeSet<_>>();
    if unique_bad.len() != request.bad_sectors.len()
        || unique_bad.iter().any(|lba| *lba >= layout.total_sectors)
    {
        return Err(
            "A FAT rekonstrukció hibás LBA listája ismétlődő vagy képen kívüli értéket tartalmaz."
                .to_owned(),
        );
    }
    let filesystem = format!("FAT{}", layout.fat_bits);
    let (records, unresolved_bad_sectors) =
        plan_reconstruction(&source_bytes, layout, &request.bad_sectors);

    if records.is_empty() {
        return Ok(ReconstructionResult {
            source_image: request.image_path.clone(),
            derived_image: None,
            provenance_path: None,
            source_sha256,
            derived_sha256: None,
            reconstructed: records,
            unresolved_bad_sectors,
            filesystem,
        });
    }

    send_stage(
        "Bizonyíthatóan redundáns FAT szektorok helyreállítása külön származtatott képbe...",
    );
    let mut derived_bytes = source_bytes.clone();

    for record in &records {
        copy_sector(
            &source_bytes,
            &mut derived_bytes,
            layout.bytes_per_sector,
            record.source_lba,
            record.target_lba,
        )?;
    }

    let derived_sha256 = sha256_bytes(&derived_bytes);
    let disk_directory = request
        .recovery_root
        .join(format!("{:03}", request.disk_number));
    fs::create_dir_all(&disk_directory).map_err(|error| {
        format!(
            "Nem sikerült létrehozni a recovery mappát {}: {error}",
            disk_directory.display()
        )
    })?;

    let timestamp = current_unix_ms();
    let stem = format!(
        "{:03}_from_attempt_{:03}_fat_reconstruction_{timestamp}",
        request.disk_number, request.attempt_number
    );
    let partial_image = disk_directory.join(format!("{stem}.partial.img"));
    let final_image = disk_directory.join(format!("{stem}.img"));
    let partial_provenance = disk_directory.join(format!("{stem}.partial.json"));
    let final_provenance = disk_directory.join(format!("{stem}.json"));

    write_new_file(&partial_image, &derived_bytes)?;

    let provenance = ReconstructionProvenance {
        schema_version: PROVENANCE_SCHEMA_VERSION,
        created_unix_ms: timestamp,
        source_image: request.image_path.display().to_string(),
        source_sha256: source_sha256.clone(),
        derived_image: final_image.display().to_string(),
        derived_sha256: derived_sha256.clone(),
        filesystem: filesystem.clone(),
        reconstructed_sectors: records.clone(),
        unresolved_bad_sectors: unresolved_bad_sectors.clone(),
        warning: "DERIVED IMAGE: mirrored FAT sectors reconstructed; never treat as untouched physical capture."
            .to_owned(),
    };
    let provenance_json = serde_json::to_vec_pretty(&provenance)
        .map_err(|error| format!("Rekonstrukciós provenance JSON hiba: {error}"))?;
    write_new_file(&partial_provenance, &provenance_json)?;

    fs::rename(&partial_image, &final_image).map_err(|error| {
        format!(
            "A származtatott kép előléptetése sikertelen {}: {error}",
            final_image.display()
        )
    })?;

    if let Err(error) = fs::rename(&partial_provenance, &final_provenance) {
        let _ = fs::rename(&final_image, &partial_image);
        return Err(format!(
            "A rekonstrukciós provenance előléptetése sikertelen {}: {error}",
            final_provenance.display()
        ));
    }

    Ok(ReconstructionResult {
        source_image: request.image_path.clone(),
        derived_image: Some(final_image),
        provenance_path: Some(final_provenance),
        source_sha256,
        derived_sha256: Some(derived_sha256),
        reconstructed: records,
        unresolved_bad_sectors,
        filesystem,
    })
}

pub(crate) fn inspect_mirrored_fat(
    image: &[u8],
    bad_sectors: &[u64],
) -> Result<(usize, usize), String> {
    let layout = FatLayout::parse(image)?;
    let (reconstructed, unresolved) = plan_reconstruction(image, layout, bad_sectors);
    Ok((reconstructed.len(), unresolved.len()))
}

impl FatLayout {
    fn parse(image: &[u8]) -> Result<Self, String> {
        if image.len() < 512 {
            return Err("A lemezkép túl kicsi FAT BPB elemzéshez.".to_owned());
        }

        let bytes_per_sector = u16::from_le_bytes([image[11], image[12]]) as usize;
        let sectors_per_cluster = image[13] as u64;
        let reserved_sectors = u16::from_le_bytes([image[14], image[15]]) as u64;
        let fat_count = image[16] as u64;
        let root_entries = u16::from_le_bytes([image[17], image[18]]) as u64;
        let total_sectors_16 = u16::from_le_bytes([image[19], image[20]]) as u64;
        let sectors_per_fat = u16::from_le_bytes([image[22], image[23]]) as u64;
        let total_sectors_32 =
            u32::from_le_bytes([image[32], image[33], image[34], image[35]]) as u64;
        let total_sectors = if total_sectors_16 > 0 {
            total_sectors_16
        } else {
            total_sectors_32
        };

        if !matches!(bytes_per_sector, 128 | 256 | 512 | 1024 | 2048 | 4096) {
            return Err(format!(
                "A BPB bytes/sector értéke nem támogatott: {bytes_per_sector}."
            ));
        }

        if sectors_per_cluster == 0
            || reserved_sectors == 0
            || fat_count < 2
            || sectors_per_fat == 0
            || total_sectors == 0
        {
            return Err(
                "A BPB nem tartalmaz használható, tükrözött FAT12/16 geometriát.".to_owned(),
            );
        }

        let expected_bytes = total_sectors
            .checked_mul(bytes_per_sector as u64)
            .ok_or_else(|| "FAT lemezképméret túlcsordulás.".to_owned())?;

        if expected_bytes > image.len() as u64 {
            return Err(format!(
                "A BPB szerinti lemezképméret nagyobb a fájlnál: {expected_bytes} > {}.",
                image.len()
            ));
        }

        let root_directory_sectors = (root_entries * 32).div_ceil(bytes_per_sector as u64);
        let overhead = reserved_sectors
            .checked_add(fat_count.saturating_mul(sectors_per_fat))
            .and_then(|value| value.checked_add(root_directory_sectors))
            .ok_or_else(|| "FAT layout túlcsordulás.".to_owned())?;

        if overhead >= total_sectors {
            return Err("A BPB FAT területe érvénytelen.".to_owned());
        }

        let data_sectors = total_sectors - overhead;
        let cluster_count = data_sectors / sectors_per_cluster;
        let fat_bits = if cluster_count < 4_085 {
            12
        } else if cluster_count < 65_525 {
            16
        } else {
            return Err("A gyors rekonstrukció jelenleg csak FAT12/16 képeket támogat.".to_owned());
        };

        Ok(Self {
            bytes_per_sector,
            total_sectors,
            fat_start_lba: reserved_sectors,
            fat_count,
            sectors_per_fat,
            fat_bits,
        })
    }
}

fn plan_reconstruction(
    image: &[u8],
    layout: FatLayout,
    bad_sectors: &[u64],
) -> (Vec<ReconstructionRecord>, Vec<u64>) {
    let bad = bad_sectors.iter().copied().collect::<BTreeSet<_>>();
    let fat_end = layout.fat_start_lba + layout.fat_count * layout.sectors_per_fat;
    let mut records = Vec::new();
    let mut unresolved = Vec::new();

    for target_lba in bad.iter().copied() {
        if target_lba >= layout.total_sectors
            || target_lba < layout.fat_start_lba
            || target_lba >= fat_end
        {
            unresolved.push(target_lba);
            continue;
        }

        let within_fats = target_lba - layout.fat_start_lba;
        let sector_within_fat = within_fats % layout.sectors_per_fat;
        let source_lba = (0..layout.fat_count)
            .map(|fat_index| {
                layout.fat_start_lba + fat_index * layout.sectors_per_fat + sector_within_fat
            })
            .find(|candidate| {
                *candidate != target_lba
                    && !bad.contains(candidate)
                    && sector_exists(image, layout.bytes_per_sector, *candidate)
            });

        if let Some(source_lba) = source_lba {
            records.push(ReconstructionRecord {
                target_lba,
                source_lba,
                method: "mirrored_fat_sector".to_owned(),
            });
        } else {
            unresolved.push(target_lba);
        }
    }

    (records, unresolved)
}

fn sector_exists(image: &[u8], bytes_per_sector: usize, lba: u64) -> bool {
    let Ok(start) = usize::try_from(lba)
        .ok()
        .and_then(|value| value.checked_mul(bytes_per_sector))
        .ok_or(())
    else {
        return false;
    };

    start
        .checked_add(bytes_per_sector)
        .is_some_and(|end| end <= image.len())
}

fn copy_sector(
    source: &[u8],
    target: &mut [u8],
    bytes_per_sector: usize,
    source_lba: u64,
    target_lba: u64,
) -> Result<(), String> {
    let source_start = usize::try_from(source_lba)
        .ok()
        .and_then(|value| value.checked_mul(bytes_per_sector))
        .ok_or_else(|| "Forrás FAT szektor offset túlcsordulás.".to_owned())?;
    let target_start = usize::try_from(target_lba)
        .ok()
        .and_then(|value| value.checked_mul(bytes_per_sector))
        .ok_or_else(|| "Cél FAT szektor offset túlcsordulás.".to_owned())?;
    let source_end = source_start
        .checked_add(bytes_per_sector)
        .ok_or_else(|| "Forrás FAT szektor vége túlcsordulás.".to_owned())?;
    let target_end = target_start
        .checked_add(bytes_per_sector)
        .ok_or_else(|| "Cél FAT szektor vége túlcsordulás.".to_owned())?;

    let source_sector = source
        .get(source_start..source_end)
        .ok_or_else(|| "A forrás FAT szektor kívül esik a lemezképen.".to_owned())?;
    let target_sector = target
        .get_mut(target_start..target_end)
        .ok_or_else(|| "A cél FAT szektor kívül esik a lemezképen.".to_owned())?;
    target_sector.copy_from_slice(source_sector);
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Nem sikerült létrehozni {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("Nem sikerült írni {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("Nem sikerült szinkronizálni {}: {error}", path.display()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_fat12_image() -> Vec<u8> {
        let bytes_per_sector = 512usize;
        let total_sectors = 100usize;
        let mut image = vec![0u8; bytes_per_sector * total_sectors];
        image[11..13].copy_from_slice(&(bytes_per_sector as u16).to_le_bytes());
        image[13] = 1;
        image[14..16].copy_from_slice(&1u16.to_le_bytes());
        image[16] = 2;
        image[17..19].copy_from_slice(&16u16.to_le_bytes());
        image[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
        image[22..24].copy_from_slice(&2u16.to_le_bytes());

        for byte in &mut image[(3 * bytes_per_sector)..(4 * bytes_per_sector)] {
            *byte = 0xA5;
        }

        image
    }

    #[test]
    fn mirrored_fat_sector_is_reconstructable() {
        let image = synthetic_fat12_image();
        let layout = FatLayout::parse(&image).unwrap();
        let (records, unresolved) = plan_reconstruction(&image, layout, &[1]);

        assert!(unresolved.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].target_lba, 1);
        assert_eq!(records[0].source_lba, 3);
    }

    #[test]
    fn data_sector_is_not_guessed() {
        let image = synthetic_fat12_image();
        let layout = FatLayout::parse(&image).unwrap();
        let (records, unresolved) = plan_reconstruction(&image, layout, &[50]);

        assert!(records.is_empty());
        assert_eq!(unresolved, vec![50]);
    }

    #[test]
    fn bad_sectors_in_both_fat_copies_are_not_guessed() {
        let image = synthetic_fat12_image();
        let layout = FatLayout::parse(&image).unwrap();
        let (records, unresolved) = plan_reconstruction(&image, layout, &[1, 3]);

        assert!(records.is_empty());
        assert_eq!(unresolved, vec![1, 3]);
    }

    #[test]
    fn repairs_mirrored_fat_even_when_other_sectors_are_bad() {
        let image = synthetic_fat12_image();
        let layout = FatLayout::parse(&image).unwrap();
        let (records, unresolved) = plan_reconstruction(&image, layout, &[1, 30, 50, 70]);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].target_lba, 1);
        assert_eq!(records[0].source_lba, 3);
        assert_eq!(unresolved, vec![30, 50, 70]);
    }

    #[test]
    fn refuses_changed_source_before_writing_derived_image() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-fat-changed-source-{}-{}",
            std::process::id(),
            current_unix_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        let image_path = root.join("001.img");
        fs::write(&image_path, synthetic_fat12_image()).unwrap();
        let request = ReconstructionRequest {
            image_path,
            expected_sha256: Some("0".repeat(64)),
            recovery_root: root.join("Recovery"),
            disk_number: 1,
            attempt_number: 1,
            bad_sectors: vec![1],
        };
        assert!(
            run_reconstruction(&request, &|_| {})
                .unwrap_err()
                .contains("SHA-256")
        );
        assert!(!request.recovery_root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires FLUXVAULT_TEST_IMAGE and FLUXVAULT_TEST_OUTPUT_ROOT"]
    fn reconstructs_redundant_fat_sector_in_real_test_image() {
        let image_path = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_IMAGE").expect("FLUXVAULT_TEST_IMAGE is required"),
        );
        let output_root = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_OUTPUT_ROOT")
                .expect("FLUXVAULT_TEST_OUTPUT_ROOT is required"),
        );
        let case_root = output_root.join(format!(
            "fluxvault-sector-recovery-test-{}",
            current_unix_ms()
        ));
        let request = ReconstructionRequest {
            image_path,
            expected_sha256: None,
            recovery_root: case_root.join("Recovery"),
            disk_number: 1,
            attempt_number: 1,
            bad_sectors: vec![16, 24],
        };

        let result = run_reconstruction(&request, &|_| {})
            .expect("real test image reconstruction analysis should succeed");

        assert_eq!(result.reconstructed.len(), 1);
        assert_eq!(result.reconstructed[0].target_lba, 16);
        assert_eq!(result.reconstructed[0].source_lba, 7);
        assert_eq!(result.unresolved_bad_sectors, vec![24]);
        assert!(
            result
                .derived_image
                .as_ref()
                .is_some_and(|path| path.is_file())
        );
        assert!(
            result
                .provenance_path
                .as_ref()
                .is_some_and(|path| path.is_file())
        );

        assert!(
            case_root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("fluxvault-sector-recovery-test-"))
        );
        fs::remove_dir_all(case_root).expect("test output cleanup should succeed");
    }
}
