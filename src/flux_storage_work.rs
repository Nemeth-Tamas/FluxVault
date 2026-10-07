//! Private, capture-bound scratch directories. Abandoned cleanup requires the capture's
//! exclusive storage lock and an independently verified surviving source.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const PREFIX: &str = ".fluxvault-storage-work-";
const MARKER: &str = "owner.json";
const LIMIT: usize = 4096;

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema: u32,
    raw_file: String,
    bytes: u64,
    sha256: String,
    purpose: String,
}

fn plain_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir() && !is_reparse(&m))
}
fn plain_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file() && !is_reparse(&m))
}
#[cfg(windows)]
fn is_reparse(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(_m: &fs::Metadata) -> bool {
    false
}

fn owner(raw: &Path, bytes: u64, sha256: &str, purpose: &str) -> Result<Owner, String> {
    Ok(Owner {
        schema: 1,
        raw_file: raw
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid scratch source name")?
            .to_owned(),
        bytes,
        sha256: sha256.to_owned(),
        purpose: purpose.to_owned(),
    })
}

fn read_owner(directory: &Path) -> Result<Option<Owner>, String> {
    let marker = directory.join(MARKER);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.file_type().is_file() || is_reparse(&metadata) || metadata.len() > 4096 {
        return Ok(None);
    }
    // Another capture's active reader can finish while this directory is being
    // discovered. A disappearing private marker is not a storage failure.
    let file = match fs::File::open(marker) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Ok(None);
    }
    Ok(serde_json::from_slice(&bytes).ok())
}

pub(super) struct Work {
    directory: PathBuf,
    owner: Owner,
}
impl Work {
    pub(super) fn create(
        raw: &Path,
        bytes: u64,
        sha256: &str,
        purpose: &str,
    ) -> Result<Self, String> {
        use std::io::Write;
        let parent = raw.parent().ok_or("Scratch source has no parent")?;
        if !plain_directory(parent) {
            return Err("Unsafe capture scratch parent".into());
        }
        let directory = parent.join(format!("{PREFIX}{}", super::nonce()));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let work = Self {
            directory,
            owner: owner(raw, bytes, sha256, purpose)?,
        };
        let mut marker = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(work.directory.join(MARKER))
            .map_err(|e| e.to_string())?;
        marker
            .write_all(&serde_json::to_vec(&work.owner).map_err(|e| e.to_string())?)
            .and_then(|_| marker.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(work)
    }
    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

// Never recurse or remove unknown entries. A renamed/modified owner marker or
// operator-added file makes the whole directory ineligible, even on normal Drop.
fn remove_owned(directory: &Path, expected: &Owner) -> Result<bool, String> {
    if !plain_directory(directory) {
        return Ok(false);
    }
    let marker = directory.join(MARKER);
    let Some(actual) = read_owner(directory)? else {
        return Ok(false);
    };
    if actual != *expected {
        return Ok(false);
    }
    let allowed: &[&str] = match actual.purpose.as_str() {
        "pack" => &[MARKER, "capture.partial.zip", "binding.partial.json"],
        "unpack" => &[MARKER, "materialized.scp"],
        _ => return Ok(false),
    };
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if files.len() >= allowed.len()
            || !allowed.iter().any(|n| entry.file_name() == *n)
            || !plain_file(&entry.path())
        {
            return Ok(false);
        }
        files.push(entry.path());
    }
    for path in files
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n != MARKER))
    {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    fs::remove_file(marker).map_err(|e| e.to_string())?;
    fs::remove_dir(directory).map_err(|e| e.to_string())?;
    Ok(true)
}
impl Drop for Work {
    fn drop(&mut self) {
        let _ = remove_owned(&self.directory, &self.owner);
    }
}

/// Caller holds the exclusive per-capture lock and has verified raw/packed
/// evidence. Active readers hold its shared counterpart until host decode ends.
pub(super) fn prune(raw: &Path, bytes: u64, sha256: &str) -> Result<usize, String> {
    let parent = raw.parent().ok_or("Scratch source has no parent")?;
    if !plain_directory(parent) {
        return Err("Unsafe capture scratch parent".into());
    }
    let mut removed = 0;
    let mut seen = 0;
    for entry in fs::read_dir(parent).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_name().to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        seen += 1;
        if seen > LIMIT {
            return Err("Too many capture scratch directories; evidence retained".into());
        }
        if !plain_directory(&entry.path()) {
            continue;
        }
        for purpose in ["pack", "unpack"] {
            if remove_owned(&entry.path(), &owner(raw, bytes, sha256, purpose)?)? {
                removed += 1;
                break;
            }
        }
    }
    Ok(removed)
}

/// Discover only bounded, well-formed ownership records. Names never become
/// arbitrary paths: callers resolve the positive disk/attempt through the catalog.
pub(super) fn sources(parent: &Path) -> Result<std::collections::BTreeSet<(u32, u32)>, String> {
    let mut sources = std::collections::BTreeSet::new();
    let mut count = 0;
    for entry in fs::read_dir(parent).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_name().to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        count += 1;
        if count > LIMIT {
            return Err("Too many capture scratch directories; evidence retained".into());
        }
        if !plain_directory(&entry.path()) {
            continue;
        }
        let Some(value) = read_owner(&entry.path())? else {
            continue;
        };
        if value.schema != 1
            || !matches!(value.purpose.as_str(), "pack" | "unpack")
            || value.bytes == 0
            || value.bytes > super::LIMIT
            || value.sha256.len() != 64
            || !value.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            continue;
        }
        let Some(stem) = value.raw_file.strip_suffix(".scp") else {
            continue;
        };
        let Some((disk, attempt)) = stem.split_once("_attempt_") else {
            continue;
        };
        let (Ok(disk), Ok(attempt)) = (disk.parse::<u32>(), attempt.parse::<u32>()) else {
            continue;
        };
        if disk > 0
            && attempt > 0
            && value.raw_file == format!("{disk:03}_attempt_{attempt:03}.scp")
        {
            sources.insert((disk, attempt));
        }
    }
    Ok(sources)
}
