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
pub struct CompositeSource {
    pub attempt_number: u32,
    pub image_path: PathBuf,
    pub total_sectors: usize,
    pub bad_sectors: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct CompositeRequest {
    pub recovery_root: PathBuf,
    pub disk_number: u32,
    pub sources: Vec<CompositeSource>,
}

#[derive(Debug, Clone)]
pub enum CompositeEvent {
    Stage(String),
    Finished(Result<CompositeResult, String>),
}

#[derive(Debug, Clone)]
pub struct CompositeResult {
    pub base_attempt: u32,
    pub derived_image: Option<PathBuf>,
    pub provenance_path: Option<PathBuf>,
    pub derived_sha256: Option<String>,
    pub replacements: Vec<CompositeReplacement>,
    pub unresolved_bad_sectors: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompositeReplacement {
    pub target_lba: u64,
    pub source_attempt: u32,
    pub source_image: String,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
struct CompositeSourceProvenance {
    attempt_number: u32,
    image: String,
    sha256: String,
    bad_sectors: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct CompositeProvenance {
    schema_version: u32,
    created_unix_ms: u64,
    disk_number: u32,
    base_attempt: u32,
    derived_image: String,
    derived_sha256: String,
    bytes_per_sector: usize,
    total_sectors: usize,
    sources: Vec<CompositeSourceProvenance>,
    replacements: Vec<CompositeReplacement>,
    unresolved_bad_sectors: Vec<u64>,
    warning: String,
}

struct LoadedSource {
    source: CompositeSource,
    bytes: Vec<u8>,
    sha256: String,
    bad_sectors: BTreeSet<u64>,
}

pub fn spawn_composite(request: CompositeRequest) -> Receiver<CompositeEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let send_stage = |message: &str| {
            let _ = sender.send(CompositeEvent::Stage(message.to_owned()));
        };
        let result = run_composite(&request, &send_stage);
        let _ = sender.send(CompositeEvent::Finished(result));
    });

    receiver
}

fn run_composite(
    request: &CompositeRequest,
    send_stage: &impl Fn(&str),
) -> Result<CompositeResult, String> {
    if request.sources.len() < 2 {
        return Err("Kompozit képhez legalább két acquisition próbálkozás szükséges.".to_owned());
    }

    send_stage("Forráspróbálkozások read-only ellenőrzése és hash-elése...");
    let mut loaded = Vec::with_capacity(request.sources.len());

    for source in &request.sources {
        if source.total_sectors == 0 {
            return Err(format!(
                "A(z) {:03} próbálkozás szektorszáma nulla.",
                source.attempt_number
            ));
        }

        let bytes = fs::read(&source.image_path).map_err(|error| {
            format!(
                "Nem sikerült read-only módban beolvasni a forrásképet {}: {error}",
                source.image_path.display()
            )
        })?;
        let sha256 = sha256_bytes(&bytes);

        loaded.push(LoadedSource {
            source: source.clone(),
            bytes,
            sha256,
            bad_sectors: source.bad_sectors.iter().copied().collect(),
        });
    }

    loaded.sort_by(|left, right| {
        left.bad_sectors
            .len()
            .cmp(&right.bad_sectors.len())
            .then_with(|| right.source.attempt_number.cmp(&left.source.attempt_number))
    });

    let total_sectors = loaded[0].source.total_sectors;
    let image_length = loaded[0].bytes.len();

    if image_length % total_sectors != 0 {
        return Err("A báziskép mérete nem osztható a rögzített szektorszámmal.".to_owned());
    }

    let bytes_per_sector = image_length / total_sectors;

    if !matches!(bytes_per_sector, 128 | 256 | 512 | 1024 | 2048 | 4096) {
        return Err(format!(
            "A számított szektorméret nem támogatott: {bytes_per_sector} bájt."
        ));
    }

    for source in &loaded {
        if source.source.total_sectors != total_sectors || source.bytes.len() != image_length {
            return Err(format!(
                "A(z) {:03} próbálkozás geometriája vagy mérete eltér a bázisképtől.",
                source.source.attempt_number
            ));
        }

        if source
            .bad_sectors
            .iter()
            .any(|lba| *lba >= total_sectors as u64)
        {
            return Err(format!(
                "A(z) {:03} próbálkozás hibás LBA listája a képen kívüli értéket tartalmaz.",
                source.source.attempt_number
            ));
        }
    }

    let base = &loaded[0];
    let base_attempt = base.source.attempt_number;
    let mut derived_bytes = base.bytes.clone();
    let mut replacements = Vec::new();
    let mut unresolved_bad_sectors = Vec::new();

    send_stage("Olvasható szektorok keresése a többi próbálkozásban...");

    for target_lba in base.bad_sectors.iter().copied() {
        let replacement_source = loaded
            .iter()
            .skip(1)
            .find(|candidate| !candidate.bad_sectors.contains(&target_lba));

        let Some(source) = replacement_source else {
            unresolved_bad_sectors.push(target_lba);
            continue;
        };

        copy_sector(
            &source.bytes,
            &mut derived_bytes,
            bytes_per_sector,
            target_lba,
        )?;
        replacements.push(CompositeReplacement {
            target_lba,
            source_attempt: source.source.attempt_number,
            source_image: source.source.image_path.display().to_string(),
            source_sha256: source.sha256.clone(),
        });
    }

    if replacements.is_empty() {
        return Ok(CompositeResult {
            base_attempt,
            derived_image: None,
            provenance_path: None,
            derived_sha256: None,
            replacements,
            unresolved_bad_sectors,
        });
    }

    send_stage("Külön származtatott kompozit kép és provenance készítése...");
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
        "{:03}_from_attempt_{base_attempt:03}_composite_{timestamp}",
        request.disk_number
    );
    let partial_image = disk_directory.join(format!("{stem}.partial.img"));
    let final_image = disk_directory.join(format!("{stem}.img"));
    let partial_provenance = disk_directory.join(format!("{stem}.partial.json"));
    let final_provenance = disk_directory.join(format!("{stem}.json"));

    write_new_file(&partial_image, &derived_bytes)?;

    let provenance = CompositeProvenance {
        schema_version: PROVENANCE_SCHEMA_VERSION,
        created_unix_ms: timestamp,
        disk_number: request.disk_number,
        base_attempt,
        derived_image: final_image.display().to_string(),
        derived_sha256: derived_sha256.clone(),
        bytes_per_sector,
        total_sectors,
        sources: loaded
            .iter()
            .map(|source| CompositeSourceProvenance {
                attempt_number: source.source.attempt_number,
                image: source.source.image_path.display().to_string(),
                sha256: source.sha256.clone(),
                bad_sectors: source.bad_sectors.iter().copied().collect(),
            })
            .collect(),
        replacements: replacements.clone(),
        unresolved_bad_sectors: unresolved_bad_sectors.clone(),
        warning: "DERIVED IMAGE: sectors copied from independently acquired attempts; never treat as an untouched physical capture."
            .to_owned(),
    };
    let provenance_json = serde_json::to_vec_pretty(&provenance)
        .map_err(|error| format!("Kompozit provenance JSON hiba: {error}"))?;
    write_new_file(&partial_provenance, &provenance_json)?;

    fs::rename(&partial_image, &final_image).map_err(|error| {
        format!(
            "A kompozit kép előléptetése sikertelen {}: {error}",
            final_image.display()
        )
    })?;

    if let Err(error) = fs::rename(&partial_provenance, &final_provenance) {
        let _ = fs::rename(&final_image, &partial_image);
        return Err(format!(
            "A kompozit provenance előléptetése sikertelen {}: {error}",
            final_provenance.display()
        ));
    }

    Ok(CompositeResult {
        base_attempt,
        derived_image: Some(final_image),
        provenance_path: Some(final_provenance),
        derived_sha256: Some(derived_sha256),
        replacements,
        unresolved_bad_sectors,
    })
}

fn copy_sector(
    source: &[u8],
    target: &mut [u8],
    bytes_per_sector: usize,
    lba: u64,
) -> Result<(), String> {
    let start = usize::try_from(lba)
        .ok()
        .and_then(|value| value.checked_mul(bytes_per_sector))
        .ok_or_else(|| "Kompozit szektor offset túlcsordulás.".to_owned())?;
    let end = start
        .checked_add(bytes_per_sector)
        .ok_or_else(|| "Kompozit szektor vége túlcsordulás.".to_owned())?;
    let source_sector = source
        .get(start..end)
        .ok_or_else(|| "A donor szektor kívül esik a forrásképen.".to_owned())?;
    let target_sector = target
        .get_mut(start..end)
        .ok_or_else(|| "A cél szektor kívül esik a kompozit képen.".to_owned())?;
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

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fluxvault-composite-{name}-{}-{}",
            std::process::id(),
            current_unix_ms()
        ))
    }

    #[test]
    fn replaces_bad_base_sector_from_clean_attempt_with_provenance() {
        let root = test_root("replace");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("001_attempt_001.img");
        let second = root.join("001_attempt_002.img");
        let mut first_bytes = vec![0u8; 4 * 512];
        let mut second_bytes = vec![0u8; 4 * 512];
        first_bytes[512..1024].fill(0x11);
        second_bytes[512..1024].fill(0xA5);
        fs::write(&first, first_bytes).unwrap();
        fs::write(&second, second_bytes).unwrap();

        let request = CompositeRequest {
            recovery_root: root.join("Recovery"),
            disk_number: 1,
            sources: vec![
                CompositeSource {
                    attempt_number: 1,
                    image_path: first,
                    total_sectors: 4,
                    bad_sectors: vec![1],
                },
                CompositeSource {
                    attempt_number: 2,
                    image_path: second,
                    total_sectors: 4,
                    bad_sectors: vec![2, 3],
                },
            ],
        };

        let result = run_composite(&request, &|_| {}).unwrap();
        let derived = fs::read(result.derived_image.as_ref().unwrap()).unwrap();

        assert_eq!(result.base_attempt, 1);
        assert_eq!(result.replacements.len(), 1);
        assert_eq!(result.replacements[0].target_lba, 1);
        assert_eq!(result.replacements[0].source_attempt, 2);
        assert!(result.unresolved_bad_sectors.is_empty());
        assert!(derived[512..1024].iter().all(|byte| *byte == 0xA5));
        assert!(result.provenance_path.as_ref().unwrap().is_file());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn never_replaces_sector_marked_bad_in_every_attempt() {
        let root = test_root("unresolved");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("001_attempt_001.img");
        let second = root.join("001_attempt_002.img");
        fs::write(&first, vec![0u8; 2 * 512]).unwrap();
        fs::write(&second, vec![0u8; 2 * 512]).unwrap();

        let request = CompositeRequest {
            recovery_root: root.join("Recovery"),
            disk_number: 1,
            sources: vec![
                CompositeSource {
                    attempt_number: 1,
                    image_path: first,
                    total_sectors: 2,
                    bad_sectors: vec![1],
                },
                CompositeSource {
                    attempt_number: 2,
                    image_path: second,
                    total_sectors: 2,
                    bad_sectors: vec![1],
                },
            ],
        };

        let result = run_composite(&request, &|_| {}).unwrap();

        assert!(result.derived_image.is_none());
        assert!(result.replacements.is_empty());
        assert_eq!(result.unresolved_bad_sectors, vec![1]);

        fs::remove_dir_all(root).unwrap();
    }
}
