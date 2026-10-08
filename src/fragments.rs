//! Evidence-bound raw readable runs of incomplete live files, never whole files.
use crate::{carving::Extent, fat12, fat12_recovery::RecoveryReport};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fragment {
    pub path: String,
    pub parent_file: String,
    pub directory_entry_offset: usize,
    pub parent_file_offset: usize,
    pub bytes: usize,
    pub sha256: String,
    pub source_extents: Vec<Extent>,
    pub metadata_lbas: Vec<u64>,
    pub complete_file: bool,
    pub customer_delivery_certified: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Report {
    schema_version: u32,
    source_image: String,
    source_sha256: String,
    disk: u32,
    attempt: u32,
    bad_lbas: Vec<u64>,
    partial_parents: Vec<fat12::UnrecoveredFile>,
    fragments: Vec<Fragment>,
    warning: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FragmentResult {
    pub output_directory: PathBuf,
    pub report_path: PathBuf,
    pub files: usize,
    pub bytes: usize,
    pub incomplete_parents: usize,
    pub reused: bool,
    pub complete_files: usize,
}

/// Caller holds the per-disk native lock and has verified acquisition and roots.
pub(crate) fn preserve(
    image: &[u8],
    source: &Path,
    recovery_disk: &Path,
    native: &RecoveryReport,
) -> Result<Option<FragmentResult>, String> {
    let Some(analysis) = &native.analysis else {
        return Ok(None);
    };
    if analysis.unrecovered_files.is_empty() || native.deleted.is_some() {
        return Ok(None);
    }
    let bad = analysis.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    let mut records = Vec::new();
    let mut payloads = Vec::new();
    let mut total = 0usize;
    for parent in &analysis.unrecovered_files {
        if !fat12::partial_file_safe(parent, analysis) {
            continue;
        }
        // Build only offsets/mappings here. Full FAT evidence lives once in the
        // parent record, not thousands of copies for an alternating-hole file.
        let mut runs: Vec<(usize, Vec<u64>, usize)> = Vec::new();
        let mut interrupted = true;
        for (index, lba) in parent.record.data_lbas.iter().enumerate() {
            let length = parent.record.bytes.saturating_sub(index * 512).min(512);
            if length == 0 {
                break;
            }
            if bad.contains(lba) {
                interrupted = true;
                continue;
            }
            if interrupted {
                runs.push((index * 512, vec![], 0));
                interrupted = false;
            }
            let run = runs.last_mut().unwrap();
            run.1.push(*lba);
            run.2 += length;
        }
        for (parent_offset, lbas, size) in runs {
            if parent_offset
                .checked_add(size)
                .is_none_or(|end| end > parent.record.bytes)
            {
                return Err("Fragment crosses declared parent EOF".into());
            }
            let mut bytes = Vec::new();
            let mut extents: Vec<Extent> = Vec::new();
            let mut unique = BTreeSet::new();
            for lba in &lbas {
                if !unique.insert(*lba) || bad.contains(lba) || *lba >= (image.len() / 512) as u64 {
                    return Err(
                        "Fragment contains unknown, repeated or out-of-range source bytes".into(),
                    );
                }
                let count = size.saturating_sub(bytes.len()).min(512);
                let source_at = *lba as usize * 512;
                let file_at = bytes.len();
                bytes.extend_from_slice(&image[source_at..source_at + count]);
                if let Some(last) = extents.last_mut()
                    && last.source_byte_offset + last.bytes == source_at
                {
                    last.bytes += count;
                } else {
                    extents.push(Extent {
                        file_offset: file_at,
                        source_byte_offset: source_at,
                        bytes: count,
                    });
                }
            }
            if bytes.len() != size {
                return Err("Fragment mapping is shorter than declared range".into());
            }
            total += size;
            if records.len() >= 4096 || total > fat12::MAX_IMAGE_BYTES {
                return Err(
                    "Raw fragment output ceiling reached; no new generation published".into(),
                );
            }
            let hash = sha(&bytes);
            records.push(Fragment {
                path: format!(
                    "fragment_{:08x}_{parent_offset:08x}_{}.bin",
                    parent.record.directory_entry_offset,
                    &hash[..12]
                ),
                parent_file: parent.record.path.clone(),
                directory_entry_offset: parent.record.directory_entry_offset,
                parent_file_offset: parent_offset,
                bytes: size,
                sha256: hash,
                source_extents: extents,
                metadata_lbas: parent.record.metadata_lbas.clone(),
                complete_file: false,
                customer_delivery_certified: false,
            });
            payloads.push(bytes);
        }
    }
    let report = Report {
        schema_version: 1, source_image: native.source_image.clone(), source_sha256: native.source_sha256.clone(),
        disk: native.disk, attempt: native.attempt, bad_lbas: analysis.bad_lbas.clone(),
        partial_parents: analysis.unrecovered_files.clone(), fragments: records,
        warning: if native.native_engine_version >= 5 {
            "Raw fragments of incomplete file candidates, NOT complete documents or customer delivery. Reconstructed-directory parents may have unknown original path/live-versus-deleted status. Logical parent offsets, physical extents, metadata/FAT provenance and exact missing ranges/unmapped tails are retained. Unknown bytes are not filled or joined. Final-sector slack is excluded. Ambiguous/crosslinked ownership is not exported. Bytes are acquisition-reported readable, not certified original content. Unsupported/unknown filesystem layouts remain unresolved."
        } else {
            "Raw fragments of incomplete live files, NOT complete documents or customer delivery. Logical parent offsets, physical extents, metadata/FAT provenance and exact missing ranges/unmapped tails are retained. Unknown bytes are not filled or joined. Final-sector slack is excluded. Ambiguous/crosslinked ownership is not exported. Bytes are acquisition-reported readable, not certified original content. Unsupported/unknown filesystem layouts remain unresolved."
        }.into(),
    };
    let serialized = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    if serialized.len() > 16 * 1024 * 1024 {
        return Err("Fragment provenance size ceiling reached; no publication".into());
    }
    let generation = if native.native_engine_version >= 5 {
        2
    } else {
        1
    };
    let output = recovery_disk.join(format!(
        "attempt_{:03}_fragments_v{generation}",
        native.attempt
    ));
    let report_path = output.join("fragments.json");
    let reused = output.exists();
    if reused {
        if !fs::symlink_metadata(&output)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_dir()
            || output.canonicalize().map_err(|e| e.to_string())?.parent() != Some(recovery_disk)
        {
            return Err("Raw fragment output escapes Recovery; preserved".into());
        }
        if regular_read(&report_path)? != serialized {
            return Err("Raw fragment report changed; preserved, reuse refused".into());
        }
        if fs::read_dir(&output).map_err(|e| e.to_string())?.count() != report.fragments.len() + 1 {
            return Err("Raw fragment inventory changed; preserved, reuse refused".into());
        }
        for (record, bytes) in report.fragments.iter().zip(&payloads) {
            if regular_read(&output.join(&record.path))? != *bytes {
                return Err("Raw fragment payload changed; preserved, reuse refused".into());
            }
        }
    } else {
        let staging = recovery_disk.join(format!(
            ".tmp-fragments-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms()
        ));
        fs::create_dir(&staging).map_err(|e| format!("Cannot reserve fragment staging: {e}"))?;
        for (record, bytes) in report.fragments.iter().zip(&payloads) {
            write_new(&staging.join(&record.path), bytes)?;
        }
        write_new(&staging.join("fragments.json"), &serialized)?;
        if sha(&regular_read(source)?) != native.source_sha256 {
            return Err("Source changed; fragment staging retained, no publication".into());
        }
        if output.exists() {
            return Err("Fragment destination appeared; staging retained".into());
        }
        fs::rename(&staging, &output).map_err(|e| format!("Fragment staging retained: {e}"))?;
    }
    Ok(Some(FragmentResult {
        output_directory: output,
        report_path,
        files: report.fragments.len(),
        bytes: total,
        incomplete_parents: report.partial_parents.len(),
        reused,
        complete_files: 0,
    }))
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn regular_read(path: &Path) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Fragment evidence is not a regular file".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Fragment evidence read ceiling exceeded".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
