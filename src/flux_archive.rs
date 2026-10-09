//! Lossless workstation capture retention. Acquisition metadata and logical SCP
//! identity never change. Publication and retirement are separate durable steps.
use crate::{flux_capture, project::ProjectState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

#[path = "flux_storage_work.rs"]
mod work;

#[cfg(test)]
#[derive(Default)]
struct TestHooks<'a> {
    checkpoint: Option<&'a mut dyn FnMut(&str) -> Result<(), String>>,
    fail_after: Option<u64>,
    binding_fail_after: Option<u64>,
}
macro_rules! checkpoint {
    ($hooks:expr, $stage:expr) => {
        #[cfg(test)]
        if let Some(callback) = &mut $hooks.checkpoint {
            callback($stage)?;
        }
    };
}

// Test-only byte budgets exercise real short writes/ENOSPC propagation through
// the ZIP writer; no environment flag or fault-injection CLI ships to operators.
struct ArchiveOutput {
    file: File,
    #[cfg(test)]
    remaining: Option<u64>,
}
impl Write for ArchiveOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        #[cfg(test)]
        if let Some(remaining) = &mut self.remaining {
            if *remaining == 0 {
                return Err(std::io::Error::from_raw_os_error(if cfg!(windows) {
                    112
                } else {
                    28
                }));
            }
            let n = self
                .file
                .write(&bytes[..bytes.len().min(*remaining as usize)])?;
            *remaining -= n as u64;
            return Ok(n);
        }
        self.file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
impl std::io::Seek for ArchiveOutput {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.file, from)
    }
}

const LIMIT: u64 = 512 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
fn require_space(directory: &Path, bytes: u64) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{Win32::Storage::FileSystem::GetDiskFreeSpaceExW, core::PCWSTR};
    let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(path.as_ptr()), Some(&mut available), None, None) }
        .map_err(|e| format!("Cannot check capture storage space: {e}"))?;
    if available < bytes {
        return Err(format!(
            "Insufficient capture storage space: need {bytes} bytes, available {available}; original evidence retained"
        ));
    }
    Ok(())
}
#[cfg(not(windows))]
fn require_space(_directory: &Path, _bytes: u64) -> Result<(), String> {
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedCapture {
    pub schema_version: u32,
    pub raw_file: String,
    pub raw_bytes: u64,
    pub raw_sha256: String,
    pub packed_file: String,
    pub packed_bytes: u64,
    pub packed_sha256: String,
    pub codec: String,
    pub codec_version: String,
}

fn regular(path: &Path) -> Result<u64, String> {
    let info = fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
    if !info.file_type().is_file() {
        return Err(format!("Not a regular file: {}", path.display()));
    }
    Ok(info.len())
}
fn present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            regular(path)?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
fn sidecar(raw: &Path) -> PathBuf {
    raw.with_extension("scp.packed.json")
}
fn packed_path(raw: &Path) -> PathBuf {
    raw.with_extension("scp.zip")
}
fn nonce() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
fn lock(raw: &Path, exclusive: bool) -> Result<File, String> {
    let name = raw
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid capture name")?;
    let path = raw.with_file_name(format!(".fluxvault-storage-{name}.lock"));
    if fs::symlink_metadata(&path).is_ok() {
        regular(&path)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    if exclusive {
        file.try_lock().map_err(|_| {
            "Capture storage is busy; retry after the current reader/packer finishes".to_owned()
        })?;
    } else {
        file.lock_shared().map_err(|e| e.to_string())?;
    }
    Ok(file)
}
fn hash(
    mut input: impl Read,
    expected_size: u64,
    expected_hash: &str,
    mut sink: impl Write,
) -> Result<(), String> {
    if expected_size == 0
        || expected_size > LIMIT
        || expected_hash.len() != 64
        || !expected_hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid or oversized raw capture identity".to_owned());
    }
    let mut digest = Sha256::new();
    let mut count = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > expected_size {
            return Err("Capture exceeds recorded byte count".to_owned());
        }
        digest.update(&buffer[..n]);
        sink.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
    }
    if count != expected_size
        || !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(expected_hash)
    {
        return Err("Capture size/SHA-256 mismatch".to_owned());
    }
    Ok(())
}
fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut b = [0; 64 * 1024];
    loop {
        let n = file.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&b[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn unpack(
    raw: &Path,
    packed: &Path,
    size: u64,
    digest: &str,
    sink: impl Write,
) -> Result<(), String> {
    regular(packed)?;
    let mut archive = ZipArchive::new(File::open(packed).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid packed capture: {e}"))?;
    if archive.len() != 1 {
        return Err("Packed capture must contain exactly one SCP".to_owned());
    }
    let member = archive.by_index(0).map_err(|e| e.to_string())?;
    if Some(member.name()) != raw.file_name().and_then(|n| n.to_str())
        || member.size() != size
        || member.compression() != CompressionMethod::Deflated
    {
        return Err("Packed capture member identity/codec mismatch".to_owned());
    }
    hash(member, size, digest, sink)
}
fn record(raw: &Path, size: u64, digest: &str) -> Result<PackedCapture, String> {
    if size == 0
        || size > LIMIT
        || digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid or oversized raw capture identity".to_owned());
    }
    let meta = sidecar(raw);
    if regular(&meta)? > 16 * 1024 {
        return Err("Oversized packed metadata".to_owned());
    }
    let value: PackedCapture = serde_json::from_slice(&fs::read(meta).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid packed metadata: {e}"))?;
    if value.schema_version != 1
        || Some(value.raw_file.as_str()) != raw.file_name().and_then(|n| n.to_str())
        || value.raw_bytes != size
        || value.raw_sha256 != digest
        || Some(value.packed_file.as_str()) != packed_path(raw).file_name().and_then(|n| n.to_str())
        || value.codec != "zip/deflate-level-6"
        || value.codec_version != "zip-8"
    {
        return Err("Packed metadata does not bind this original capture".to_owned());
    }
    let packed = packed_path(raw);
    if value.packed_bytes == 0
        || value.packed_bytes > size + size / 100 + 1024 * 1024
        || regular(&packed)? != value.packed_bytes
        || file_hash(&packed)? != value.packed_sha256
    {
        return Err("Packed capture changed since publication".to_owned());
    }
    Ok(value)
}
fn verify_unlocked(raw: &Path, size: u64, digest: &str) -> Result<(), String> {
    let packed = present(&sidecar(raw))?;
    if packed {
        record(raw, size, digest)?;
        unpack(raw, &packed_path(raw), size, digest, std::io::sink())?;
    }
    if present(raw)? {
        hash(
            File::open(raw).map_err(|e| e.to_string())?,
            size,
            digest,
            std::io::sink(),
        )
    } else if packed {
        Ok(())
    } else {
        Err("Original raw capture and verified packed binding are missing".to_owned())
    }
}
pub(crate) fn verify(raw: &Path, size: u64, digest: &str) -> Result<(), String> {
    let _guard = lock(raw, false)?;
    verify_unlocked(raw, size, digest)
}

pub(crate) fn verify_packed(project: &ProjectState, disk: u32, attempt: u32) -> Result<(), String> {
    let (raw, size, digest) = flux_capture::raw_identity(project, disk, attempt)?;
    let _guard = lock(&raw, false)?;
    record(&raw, size, &digest)?;
    unpack(&raw, &packed_path(&raw), size, &digest, std::io::sink())
}

pub(crate) struct Source {
    pub path: PathBuf,
    // Drop scratch before releasing the shared per-capture lock.
    _temporary: Option<work::Work>,
    _guard: File,
}
/// Private verified materialization remains locked for the entire host conversion.
pub(crate) fn open_source(raw: &Path, size: u64, digest: &str) -> Result<Source, String> {
    open_source_inner(
        raw,
        size,
        digest,
        #[cfg(test)]
        &mut TestHooks::default(),
    )
}
fn open_source_inner(
    raw: &Path,
    size: u64,
    digest: &str,
    #[cfg(test)] hooks: &mut TestHooks<'_>,
) -> Result<Source, String> {
    let guard = lock(raw, false)?;
    if present(raw)? {
        verify_unlocked(raw, size, digest)?;
        return Ok(Source {
            path: raw.to_owned(),
            _temporary: None,
            _guard: guard,
        });
    }
    record(raw, size, digest)?;
    require_space(
        raw.parent().ok_or("Capture has no parent")?,
        size + 16 * 1024 * 1024,
    )?;
    let temporary = work::Work::create(raw, size, digest, "unpack")?;
    let path = temporary.path("materialized.scp");
    let source = Source {
        path: path.clone(),
        _temporary: Some(temporary),
        _guard: guard,
    };
    checkpoint!(hooks, "unpack-created");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut output = ArchiveOutput {
        file,
        #[cfg(test)]
        remaining: hooks.fail_after,
    };
    unpack(raw, &packed_path(raw), size, digest, &mut output)?;
    checkpoint!(hooks, "unpack-written");
    output.file.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    checkpoint!(hooks, "unpack-ready");
    Ok(source)
}

pub fn pack(
    project: &ProjectState,
    disk: u32,
    attempt: u32,
    retire_raw: bool,
) -> Result<PackedCapture, String> {
    let (raw, size, _) = flux_capture::raw_identity(project, disk, attempt)?;
    // Admission precedes the exclusive capture-storage lock: a decoder must
    // never wait on a packer which is itself waiting for background resources.
    let _budget = crate::resource_budget::background(
        crate::resource_budget::Kind::Packing,
        64 * crate::resource_budget::MIB,
        &[(
            raw.parent().ok_or("Capture has no directory")?,
            size.saturating_add(16 * crate::resource_budget::MIB),
        )],
        &|_| {},
    )?;
    pack_inner(
        project,
        disk,
        attempt,
        retire_raw,
        #[cfg(test)]
        &mut TestHooks::default(),
    )
}
fn pack_inner(
    project: &ProjectState,
    disk: u32,
    attempt: u32,
    retire_raw: bool,
    #[cfg(test)] hooks: &mut TestHooks<'_>,
) -> Result<PackedCapture, String> {
    let (raw, size, digest) = flux_capture::raw_identity(project, disk, attempt)?;
    let _guard = lock(&raw, true)?;
    let final_zip = packed_path(&raw);
    let final_meta = sidecar(&raw);
    // Verification precedes scratch cleanup and retirement, including restart.
    verify_unlocked(&raw, size, &digest)?;
    work::prune(&raw, size, &digest)?;
    if present(&final_meta)? {
        let value = record(&raw, size, &digest)?;
        unpack(&raw, &final_zip, size, &digest, std::io::sink())?;
        if retire_raw && present(&raw)? {
            verify_unlocked(&raw, size, &digest)?;
            fs::remove_file(&raw)
                .map_err(|e| format!("Packed evidence verified, but raw retirement failed: {e}"))?;
            checkpoint!(hooks, "raw-retired");
        }
        return Ok(value);
    }
    let scratch = work::Work::create(&raw, size, &digest, "pack")?;
    checkpoint!(hooks, "work-created");
    if !present(&final_zip)? {
        require_space(
            raw.parent().ok_or("Capture has no parent")?,
            size + size / 100 + 16 * 1024 * 1024,
        )?;
        let temp = scratch.path("capture.partial.zip");
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let output = ArchiveOutput {
            file,
            #[cfg(test)]
            remaining: hooks.fail_after,
        };
        let mut writer = ZipWriter::new(output);
        writer
            .start_file(
                raw.file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("Capture filename missing")?,
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Deflated)
                    .compression_level(Some(6)),
            )
            .map_err(|e| e.to_string())?;
        checkpoint!(hooks, "zip-started");
        hash(
            File::open(&raw).map_err(|e| e.to_string())?,
            size,
            &digest,
            &mut writer,
        )?;
        checkpoint!(hooks, "zip-written");
        let output = writer.finish().map_err(|e| e.to_string())?;
        output.file.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        checkpoint!(hooks, "zip-synced");
        unpack(&raw, &temp, size, &digest, std::io::sink())?;
        checkpoint!(hooks, "zip-verified");
        crate::flux_recovery::publish_image_no_replace(&temp, &final_zip)?;
        checkpoint!(hooks, "zip-published");
    }
    // Also covers restart after archive publication but before sidecar publication.
    unpack(&raw, &final_zip, size, &digest, std::io::sink())?;
    let value = PackedCapture {
        schema_version: 1,
        raw_file: raw.file_name().unwrap().to_string_lossy().into_owned(),
        raw_bytes: size,
        raw_sha256: digest.clone(),
        packed_file: final_zip
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        packed_bytes: regular(&final_zip)?,
        packed_sha256: file_hash(&final_zip)?,
        codec: "zip/deflate-level-6".to_owned(),
        codec_version: "zip-8".to_owned(),
    };
    let temporary = scratch.path("binding.partial.json");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let mut output = ArchiveOutput {
        file,
        #[cfg(test)]
        remaining: hooks.binding_fail_after,
    };
    checkpoint!(hooks, "binding-created");
    output
        .write_all(&serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?)
        .and_then(|_| output.file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(output);
    checkpoint!(hooks, "binding-synced");
    crate::flux_recovery::publish_image_no_replace(&temporary, &final_meta)?;
    checkpoint!(hooks, "binding-published");
    // Reopen the published pair. Never retire on a write/verification failure.
    record(&raw, size, &digest)?;
    unpack(&raw, &final_zip, size, &digest, std::io::sink())?;
    checkpoint!(hooks, "pair-verified");
    if retire_raw {
        verify_unlocked(&raw, size, &digest)?;
        fs::remove_file(&raw).map_err(|e| e.to_string())?;
        checkpoint!(hooks, "raw-retired");
    }
    Ok(value)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    schema_version: u32,
    disk: u32,
    attempt: u32,
    retire_raw: bool,
}

fn cleanup_project_scratch(project: &ProjectState) -> Vec<String> {
    let sources = flux_capture::project_flux_dir(project).and_then(|flux| work::sources(&flux));
    let sources = match sources {
        Ok(s) => s,
        Err(e) => return vec![e],
    };
    let mut errors = Vec::new();
    for (disk, attempt) in sources {
        let result = (|| {
            let (raw, size, digest) = flux_capture::raw_identity(project, disk, attempt)?;
            let _guard = match lock(&raw, true) {
                Ok(guard) => guard,
                // A current decode/packer is not abandoned and is never interrupted.
                Err(e) if e.starts_with("Capture storage is busy;") => return Ok(()),
                Err(e) => return Err(e),
            };
            verify_unlocked(&raw, size, &digest)?;
            work::prune(&raw, size, &digest)?;
            Ok(())
        })();
        if let Err(e) = result {
            errors.push(format!(
                "Scratch cleanup {disk:03}/{attempt:03}: {e}; temporary evidence preserved"
            ));
        }
    }
    errors
}

/// One disk-streaming packer, coalesced wakeups and durable per-capture task files.
/// No worker writes to the terminal while the operator is waiting at a swap cue.
pub(crate) struct Queue {
    directory: PathBuf,
    wake: Option<mpsc::SyncSender<()>>,
    finish: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Vec<String>>>,
}
impl Queue {
    pub fn start(project: &ProjectState) -> Result<Self, String> {
        let flux = flux_capture::project_flux_dir(project)?;
        let directory = flux.join(".fluxvault-storage-queue");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        if directory.parent() != Some(flux.as_path()) {
            return Err("Storage queue escapes Flux".to_owned());
        }
        let ownership = directory.join(".fluxvault-owner.lock");
        if fs::symlink_metadata(&ownership).is_ok() {
            regular(&ownership)?;
        }
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(ownership)
            .map_err(|e| e.to_string())?;
        owner
            .try_lock()
            .map_err(|_| "Storage queue already has an owner".to_owned())?;
        let (wake, receiver) = mpsc::sync_channel(1);
        let finish = Arc::new(AtomicBool::new(false));
        let ending = finish.clone();
        let root = project.root().to_owned();
        let jobs = directory.clone();
        let worker = thread::spawn(move || {
            let _owner = owner;
            let mut errors = Vec::new();
            let mut attempted = std::collections::BTreeSet::new();
            let project = match ProjectState::open_without_session(root) {
                Ok(p) => p,
                Err(e) => return vec![e],
            };
            // Also reclaim interrupted decode copies whose packing task already
            // completed. Only captures named by valid scratch ownership are hashed.
            errors.extend(cleanup_project_scratch(&project));
            loop {
                // Acquire the producer's finished flag before enumerating jobs:
                // finish must not miss a last task published after an earlier
                // directory snapshot. Otherwise notifications drive the next loop.
                let draining = ending.load(Ordering::Acquire);
                let entries = match fs::read_dir(&jobs) {
                    Ok(e) => e,
                    Err(e) => {
                        errors.push(e.to_string());
                        break;
                    }
                };
                let entries = match entries.collect::<Result<Vec<_>, _>>() {
                    Ok(entries) => entries,
                    Err(error) => {
                        errors.push(format!("Cannot enumerate durable tasks: {error}"));
                        break;
                    }
                };
                let mut pending = entries
                    .into_iter()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension().is_some_and(|e| e == "json") && !attempted.contains(p)
                    })
                    .collect::<Vec<_>>();
                pending.sort();
                if let Some(path) = pending.into_iter().next() {
                    attempted.insert(path.clone());
                    let result = (|| {
                        if regular(&path)? > 4096 {
                            return Err("Oversized storage task".to_owned());
                        }
                        let task: Task =
                            serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                                .map_err(|e| e.to_string())?;
                        if task.schema_version != 1
                            || path.file_name().and_then(|n| n.to_str())
                                != Some(
                                    format!("{:03}_{:03}.json", task.disk, task.attempt).as_str(),
                                )
                        {
                            return Err("Invalid storage task identity".to_owned());
                        }
                        pack(&project, task.disk, task.attempt, task.retire_raw)?;
                        fs::remove_file(&path).map_err(|e| e.to_string())
                    })();
                    if let Err(e) = result {
                        errors.push(format!(
                            "{}: {e}; durable task retained for retry",
                            path.display()
                        ));
                    }
                    continue;
                }
                if draining {
                    break;
                }
                if receiver.recv().is_err() {
                    break;
                }
            }
            errors
        });
        Ok(Self {
            directory,
            wake: Some(wake),
            finish,
            worker: Some(worker),
        })
    }
    pub fn enqueue(&self, disk: u32, attempt: u32) -> Result<(), String> {
        if disk == 0 || attempt == 0 {
            return Err("Storage task numbers must be positive".to_owned());
        }
        let raw = self
            .directory
            .parent()
            .ok_or("Storage queue parent missing")?
            .join(format!("{disk:03}_attempt_{attempt:03}.scp"));
        if !present(&raw)? && present(&sidecar(&raw))? {
            let root = raw
                .parent()
                .and_then(Path::parent)
                .ok_or("Storage project parent missing")?;
            let project = ProjectState::open_without_session(root.to_owned())?;
            verify_packed(&project, disk, attempt)?;
            return Ok(());
        }
        let path = self.directory.join(format!("{disk:03}_{attempt:03}.json"));
        if !present(&path)? {
            let temporary = self
                .directory
                .join(format!(".fluxvault-task-{}.partial", nonce()));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(
                &serde_json::to_vec(&Task {
                    schema_version: 1,
                    disk,
                    attempt,
                    retire_raw: true,
                })
                .map_err(|e| e.to_string())?,
            )
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
            drop(file);
            fs::rename(temporary, path).map_err(|e| e.to_string())?;
        }
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
        Ok(())
    }
    pub fn finish(mut self) -> Vec<String> {
        self.stop()
    }
    fn stop(&mut self) -> Vec<String> {
        self.finish.store(true, Ordering::Release);
        self.wake.take();
        self.worker
            .take()
            .map(|w| {
                w.join().unwrap_or_else(|_| {
                    vec!["Storage worker panicked; raw evidence/tasks retained".to_owned()]
                })
            })
            .unwrap_or_default()
    }
}
impl Drop for Queue {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn child(project: &ProjectState, mode: &str, stage: &str, id: usize) -> (ChildGuard, PathBuf) {
        let ready = project.root().join(format!("child-{id}.ready"));
        let process = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "flux_archive::tests::storage_child_entry",
                "--nocapture",
            ])
            .env("FV_STORAGE_TEST_PROJECT", project.root())
            .env("FV_STORAGE_TEST_MODE", mode)
            .env("FV_STORAGE_TEST_STAGE", stage)
            .env("FV_STORAGE_TEST_READY", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut process = ChildGuard(process);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.is_file() {
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "Storage child exited before {stage}"
            );
            assert!(
                Instant::now() < deadline,
                "Storage child did not reach {stage}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        (process, ready)
    }
    fn scratch_dirs(raw: &Path) -> Vec<PathBuf> {
        fs::read_dir(raw.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".fluxvault-storage-work-")
            })
            .collect()
    }

    // Invoked only in an isolated test executable, with child-local environment.
    // The shipped binary never reads any of these environment variables.
    #[test]
    fn storage_child_entry() {
        let Some(root) = std::env::var_os("FV_STORAGE_TEST_PROJECT") else {
            return;
        };
        let project = ProjectState::open_without_session(PathBuf::from(root)).unwrap();
        let ready = PathBuf::from(std::env::var_os("FV_STORAGE_TEST_READY").unwrap());
        let wanted = std::env::var("FV_STORAGE_TEST_STAGE").unwrap();
        let mut pause = |stage: &str| -> Result<(), String> {
            if stage == wanted {
                let mut file = File::create(&ready).map_err(|e| e.to_string())?;
                file.write_all(stage.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())?;
                let deadline = Instant::now() + Duration::from_secs(30);
                while !ready.with_extension("release").is_file() {
                    if Instant::now() > deadline {
                        return Err("Test child release timeout".into());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
            Ok(())
        };
        let mut hooks = TestHooks {
            checkpoint: Some(&mut pause),
            fail_after: None,
            binding_fail_after: None,
        };
        match std::env::var("FV_STORAGE_TEST_MODE").unwrap().as_str() {
            "pack" => {
                pack_inner(&project, 1, 1, true, &mut hooks).unwrap();
            }
            "read" => {
                let (raw, size, hash) = flux_capture::raw_identity(&project, 1, 1).unwrap();
                let source = open_source_inner(&raw, size, &hash, &mut hooks).unwrap();
                assert_eq!(file_hash(&source.path).unwrap(), hash);
            }
            "queue" => {
                let queue = Queue::start(&project).unwrap();
                pause("queue-owned").unwrap();
                assert!(queue.finish().is_empty());
            }
            _ => panic!("Invalid storage test child mode"),
        }
    }

    #[test]
    fn forced_process_termination_at_each_pack_boundary_resumes_verified_evidence() {
        for (id, stage) in [
            "work-created",
            "zip-started",
            "zip-written",
            "zip-synced",
            "zip-verified",
            "zip-published",
            "binding-created",
            "binding-synced",
            "binding-published",
            "pair-verified",
            "raw-retired",
        ]
        .iter()
        .enumerate()
        {
            let (project, root, bytes) = fixture();
            let (raw, size, hash) = flux_capture::raw_identity(&project, 1, 1).unwrap();
            let task_dir = raw.parent().unwrap().join(".fluxvault-storage-queue");
            fs::create_dir(&task_dir).unwrap();
            let task = task_dir.join("001_001.json");
            fs::write(
                &task,
                br#"{"schema_version":1,"disk":1,"attempt":1,"retire_raw":true}"#,
            )
            .unwrap();
            let (mut process, _) = child(&project, "pack", stage, id);
            if *stage != "raw-retired" {
                assert_eq!(fs::read(&raw).unwrap(), bytes);
            }
            process.0.kill().unwrap();
            process.0.wait().unwrap();
            drop(process);
            assert!(task.is_file());
            assert!(
                verify(&raw, size, &hash).is_ok(),
                "Lost evidence at {stage}"
            );
            assert!(!scratch_dirs(&raw).is_empty());
            assert!(
                Queue::start(&project).unwrap().finish().is_empty(),
                "Resume failed at {stage}"
            );
            assert!(!task.exists());
            assert!(
                scratch_dirs(&raw).is_empty(),
                "Abandoned scratch at {stage}"
            );
            let source = open_source(&raw, size, &hash).unwrap();
            assert_eq!(fs::read(&source.path).unwrap(), bytes);
            drop(source);
            pack(&project, 1, 1, true).unwrap();
            assert!(
                flux_capture::inspect_disk(&project, 1)
                    .unwrap()
                    .evidence_healthy
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn injected_mid_write_disk_full_retains_original_and_allows_retry() {
        let (project, root, bytes) = fixture();
        let (raw, size, hash) = flux_capture::raw_identity(&project, 1, 1).unwrap();
        // Refuse in a ZIP header and later inside compressed payload/finalization.
        for budget in [12, 80] {
            let error = pack_inner(
                &project,
                1,
                1,
                true,
                &mut TestHooks {
                    checkpoint: None,
                    fail_after: Some(budget),
                    binding_fail_after: None,
                },
            )
            .unwrap_err();
            assert!(error.contains(if cfg!(windows) { "112" } else { "28" }));
            assert_eq!(fs::read(&raw).unwrap(), bytes);
            assert!(!packed_path(&raw).exists());
            assert!(!sidecar(&raw).exists());
            assert!(scratch_dirs(&raw).is_empty());
        }
        assert!(
            pack_inner(
                &project,
                1,
                1,
                true,
                &mut TestHooks {
                    binding_fail_after: Some(24),
                    ..TestHooks::default()
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(&raw).unwrap(), bytes);
        assert!(packed_path(&raw).is_file());
        assert!(!sidecar(&raw).exists());
        assert!(scratch_dirs(&raw).is_empty());
        let unfinished_binding_zip = fs::read(packed_path(&raw)).unwrap();
        pack(&project, 1, 1, true).unwrap();
        assert_eq!(fs::read(packed_path(&raw)).unwrap(), unfinished_binding_zip);
        let archive_before = fs::read(packed_path(&raw)).unwrap();
        assert!(
            open_source_inner(
                &raw,
                size,
                &hash,
                &mut TestHooks {
                    checkpoint: None,
                    fail_after: Some(4096),
                    binding_fail_after: None,
                }
            )
            .is_err()
        );
        assert!(scratch_dirs(&raw).is_empty());
        assert_eq!(fs::read(packed_path(&raw)).unwrap(), archive_before);
        assert_eq!(
            fs::read(&open_source(&raw, size, &hash).unwrap().path).unwrap(),
            bytes
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn killed_materializations_are_cleaned_only_after_evidence_verification() {
        for (id, stage) in ["unpack-created", "unpack-written", "unpack-ready"]
            .iter()
            .enumerate()
        {
            let (project, root, bytes) = fixture();
            let (raw, size, hash) = flux_capture::raw_identity(&project, 1, 1).unwrap();
            pack(&project, 1, 1, true).unwrap();
            let zip_before = fs::read(packed_path(&raw)).unwrap();
            let (mut process, _) = child(&project, "read", stage, id);
            assert!(pack(&project, 1, 1, true).unwrap_err().contains("busy"));
            process.0.kill().unwrap();
            process.0.wait().unwrap();
            drop(process);
            let abandoned = scratch_dirs(&raw);
            assert_eq!(abandoned.len(), 1);
            if *stage == "unpack-ready" {
                assert_eq!(
                    fs::read(abandoned[0].join("materialized.scp")).unwrap(),
                    bytes
                );
            }
            fs::write(packed_path(&raw), b"truncated").unwrap();
            assert!(pack(&project, 1, 1, true).is_err());
            assert!(abandoned[0].exists()); // Last materialization is not swept on corrupt evidence.
            fs::write(packed_path(&raw), &zip_before).unwrap();
            assert!(Queue::start(&project).unwrap().finish().is_empty());
            assert!(scratch_dirs(&raw).is_empty());
            verify(&raw, size, &hash).unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cross_process_reader_packer_and_queue_ownership_soak() {
        let (project, root, bytes) = fixture();
        let (raw, size, hash) = flux_capture::raw_identity(&project, 1, 1).unwrap();
        pack(&project, 1, 1, true).unwrap();
        let zip_before = fs::read(packed_path(&raw)).unwrap();
        let binding_before = fs::read(sidecar(&raw)).unwrap();
        for round in 0..8 {
            let (mut first, first_ready) = child(&project, "read", "unpack-ready", round * 2);
            let (mut second, second_ready) = child(&project, "read", "unpack-ready", round * 2 + 1);
            assert_eq!(scratch_dirs(&raw).len(), 2);
            for _ in 0..3 {
                assert!(pack(&project, 1, 1, true).unwrap_err().contains("busy"));
            }
            fs::write(first_ready.with_extension("release"), b"release").unwrap();
            assert!(first.0.wait().unwrap().success());
            drop(first);
            assert!(pack(&project, 1, 1, true).unwrap_err().contains("busy"));
            fs::write(second_ready.with_extension("release"), b"release").unwrap();
            assert!(second.0.wait().unwrap().success());
            drop(second);
            pack(&project, 1, 1, true).unwrap();
            assert!(scratch_dirs(&raw).is_empty());
            assert_eq!(fs::read(packed_path(&raw)).unwrap(), zip_before);
            assert_eq!(fs::read(sidecar(&raw)).unwrap(), binding_before);
        }
        let (mut owner, _) = child(&project, "queue", "queue-owned", 100);
        assert!(Queue::start(&project).is_err());
        owner.0.kill().unwrap();
        owner.0.wait().unwrap();
        drop(owner);
        assert!(Queue::start(&project).unwrap().finish().is_empty());
        assert_eq!(
            fs::read(&open_source(&raw, size, &hash).unwrap().path).unwrap(),
            bytes
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scratch_cleanup_preserves_unknown_foreign_and_modified_entries() {
        let (project, root, _) = fixture();
        let (raw, size, hash) = flux_capture::raw_identity(&project, 1, 1).unwrap();
        let foreign = work::Work::create(
            &raw.with_file_name("002_attempt_001.scp"),
            size,
            &hash,
            "pack",
        )
        .unwrap();
        let foreign_dir = foreign.path("owner.json").parent().unwrap().to_owned();
        std::mem::forget(foreign);
        let modified = work::Work::create(&raw, size, &hash, "pack").unwrap();
        let modified_dir = modified.path("owner.json").parent().unwrap().to_owned();
        fs::write(modified.path("operator-note.txt"), b"keep").unwrap();
        drop(modified);
        let malformed = work::Work::create(&raw, size, &hash, "pack").unwrap();
        let malformed_dir = malformed.path("owner.json").parent().unwrap().to_owned();
        fs::write(malformed.path("owner.json"), b"broken ownership record").unwrap();
        drop(malformed);
        let changed = work::Work::create(&raw, size, &hash, "pack").unwrap();
        let changed_dir = changed.path("owner.json").parent().unwrap().to_owned();
        let mut changed_owner: serde_json::Value =
            serde_json::from_slice(&fs::read(changed.path("owner.json")).unwrap()).unwrap();
        changed_owner["sha256"] = serde_json::Value::String("0".repeat(64));
        fs::write(
            changed.path("owner.json"),
            serde_json::to_vec(&changed_owner).unwrap(),
        )
        .unwrap();
        drop(changed);
        let unknown = raw
            .parent()
            .unwrap()
            .join(".fluxvault-storage-work-unknown");
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("capture.partial.zip"), b"keep").unwrap();
        let old = raw.with_file_name(".fluxvault-pack-legacy.partial.zip");
        fs::write(&old, b"unbound old scratch").unwrap();
        pack(&project, 1, 1, true).unwrap();
        assert!(foreign_dir.is_dir());
        assert!(modified_dir.is_dir());
        assert!(unknown.is_dir());
        assert!(malformed_dir.is_dir());
        assert!(changed_dir.is_dir());
        assert_eq!(fs::read(old).unwrap(), b"unbound old scratch");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queue_resume_skips_active_materializations_and_refuses_corrupt_packed_enqueue() {
        let (project, root, _) = fixture();
        let (raw, size, hash) = flux_capture::raw_identity(&project, 1, 1).unwrap();
        pack(&project, 1, 1, true).unwrap();
        let (mut reader, ready) = child(&project, "read", "unpack-ready", 0);
        assert!(Queue::start(&project).unwrap().finish().is_empty());
        assert_eq!(scratch_dirs(&raw).len(), 1);
        fs::write(ready.with_extension("release"), b"release").unwrap();
        assert!(reader.0.wait().unwrap().success());
        drop(reader);
        verify(&raw, size, &hash).unwrap();
        fs::write(packed_path(&raw), b"truncated").unwrap();
        let queue = Queue::start(&project).unwrap();
        assert!(queue.enqueue(1, 1).is_err());
        assert!(queue.finish().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    fn fixture() -> (ProjectState, PathBuf, Vec<u8>) {
        let directory = std::env::temp_dir().join(format!("fluxvault-retention-{}", nonce()));
        let project = ProjectState::create_without_session(directory.clone()).unwrap();
        let flux = flux_capture::project_flux_dir(&project).unwrap();
        let raw = flux.join("001_attempt_001.scp");
        let bytes = vec![0x42; 128 * 1024];
        fs::write(&raw, &bytes).unwrap();
        fs::write(flux.join("001_attempt_001.json"),serde_json::to_vec(&serde_json::json!({"schema_version":1,"disk_number":1,"attempt_number":1,"profile":"ibm.1440","drive":"B","revolutions":2,"status":"complete","flux_file":"001_attempt_001.scp","bytes":bytes.len(),"sha256":file_hash(&raw).unwrap(),"command":[],"detail":null})).unwrap()).unwrap();
        (project, directory, bytes)
    }
    #[test]
    fn verified_retirement_materialization_locks_and_export_preserve_original_identity() {
        let (project, root, bytes) = fixture();
        let (raw, size, digest) = flux_capture::raw_identity(&project, 1, 1).unwrap();
        let value = pack(&project, 1, 1, false).unwrap();
        assert!(value.packed_bytes < size);
        assert_eq!(fs::read(&raw).unwrap(), bytes);
        let zip_before = fs::read(packed_path(&raw)).unwrap();
        let meta_before = fs::read(sidecar(&raw)).unwrap();
        // Simulate interruption between verified ZIP and sidecar publication.
        fs::remove_file(sidecar(&raw)).unwrap();
        pack(&project, 1, 1, false).unwrap();
        assert_eq!(fs::read(packed_path(&raw)).unwrap(), zip_before);
        assert_eq!(fs::read(sidecar(&raw)).unwrap(), meta_before);
        let reader = open_source(&raw, size, &digest).unwrap();
        assert!(pack(&project, 1, 1, true).unwrap_err().contains("busy"));
        drop(reader);
        pack(&project, 1, 1, true).unwrap();
        assert!(!raw.exists());
        assert!(
            flux_capture::inspect_disk(&project, 1)
                .unwrap()
                .evidence_healthy
        );
        let first = open_source(&raw, size, &digest).unwrap();
        let second = open_source(&raw, size, &digest).unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(fs::read(&first.path).unwrap(), bytes);
        assert_eq!(fs::read(&second.path).unwrap(), bytes);
        let temp = first.path.clone();
        drop(first);
        assert!(!temp.exists());
        drop(second);
        pack(&project, 1, 1, true).unwrap();
        assert_eq!(fs::read(packed_path(&raw)).unwrap(), zip_before);
        assert_eq!(
            flux_capture::next_capture_attempt(raw.parent().unwrap(), 1).unwrap(),
            2
        );
        let destination = root.with_file_name(format!("fluxvault-retention-export-{}", nonce()));
        fs::create_dir(&destination).unwrap();
        let exported = crate::package::build_package(
            &crate::package::PackageRequest {
                project_root: root.clone(),
                destination: destination.clone(),
                project_name: "retention".to_owned(),
            },
            &|_| {},
        )
        .unwrap();
        let mut zip = ZipArchive::new(File::open(exported.zip_path).unwrap()).unwrap();
        assert!(zip.by_name("Flux/001_attempt_001.scp.zip").is_ok());
        assert!(zip.by_name("Flux/001_attempt_001.scp.packed.json").is_ok());
        assert!(zip.by_name("Flux/001_attempt_001.scp").is_err());
        drop(zip);
        fs::write(packed_path(&raw), b"truncated").unwrap();
        assert!(
            !flux_capture::inspect_disk(&project, 1)
                .unwrap()
                .evidence_healthy
        );
        assert!(open_source(&raw, size, &digest).is_err());
        assert!(pack(&project, 1, 1, true).is_err());
        assert!(
            crate::package::build_package(
                &crate::package::PackageRequest {
                    project_root: root.clone(),
                    destination: destination.clone(),
                    project_name: "tampered".to_owned()
                },
                &|_| {}
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(destination).unwrap();
    }
    #[test]
    fn durable_queue_retains_failed_tasks_and_resumes_without_duplicate_ownership() {
        let (project, root, bytes) = fixture();
        let (raw, _, _) = flux_capture::raw_identity(&project, 1, 1).unwrap();
        fs::write(&raw, b"damaged").unwrap();
        let queue = Queue::start(&project).unwrap();
        assert!(Queue::start(&project).is_err());
        queue.enqueue(1, 1).unwrap();
        queue.enqueue(1, 1).unwrap();
        assert!(!queue.finish().is_empty());
        let task = raw
            .parent()
            .unwrap()
            .join(".fluxvault-storage-queue/001_001.json");
        assert!(task.is_file());
        assert!(raw.is_file());
        fs::write(&raw, &bytes).unwrap();
        let queue = Queue::start(&project).unwrap();
        assert!(queue.finish().is_empty());
        assert!(!task.exists());
        assert!(!raw.exists());
        assert!(
            flux_capture::inspect_disk(&project, 1)
                .unwrap()
                .evidence_healthy
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn space_check_refuses_unavailable_capacity() {
        assert!(
            require_space(&std::env::temp_dir(), u64::MAX)
                .unwrap_err()
                .contains("Insufficient")
        );
    }
    #[test]
    fn roundtrip_is_bounded_and_refuses_wrong_size_hash_and_zip_members() {
        let directory = std::env::temp_dir().join(format!("fluxvault-pack-test-{}", nonce()));
        fs::create_dir(&directory).unwrap();
        let raw = directory.join("001_attempt_001.scp");
        let bytes = vec![42u8; 10000];
        fs::write(&raw, &bytes).unwrap();
        let digest = file_hash(&raw).unwrap();
        assert!(verify(&raw, 10001, &digest).is_err());
        assert!(verify(&raw, 10000, &"0".repeat(64)).is_err());
        let mut zip = ZipWriter::new(File::create(packed_path(&raw)).unwrap());
        zip.start_file(
            "../escape.scp",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(&bytes).unwrap();
        zip.finish().unwrap();
        assert!(unpack(&raw, &packed_path(&raw), 10000, &digest, std::io::sink()).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
