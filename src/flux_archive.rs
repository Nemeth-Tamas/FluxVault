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
    temporary: Option<PathBuf>,
    _guard: File,
}
impl Drop for Source {
    fn drop(&mut self) {
        if let Some(directory) = &self.temporary {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir(directory);
        }
    }
}
/// Private verified materialization remains locked for the entire host conversion.
pub(crate) fn open_source(raw: &Path, size: u64, digest: &str) -> Result<Source, String> {
    let guard = lock(raw, false)?;
    if present(raw)? {
        verify_unlocked(raw, size, digest)?;
        return Ok(Source {
            path: raw.to_owned(),
            temporary: None,
            _guard: guard,
        });
    }
    record(raw, size, digest)?;
    let directory = raw
        .parent()
        .ok_or("Capture has no parent")?
        .join(format!(".fluxvault-unpack-{}", nonce()));
    require_space(
        raw.parent().ok_or("Capture has no parent")?,
        size + 16 * 1024 * 1024,
    )?;
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(raw.file_name().ok_or("Capture filename missing")?);
    let source = Source {
        path: path.clone(),
        temporary: Some(directory),
        _guard: guard,
    };
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    unpack(raw, &packed_path(raw), size, digest, &mut output)?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    Ok(source)
}

pub fn pack(
    project: &ProjectState,
    disk: u32,
    attempt: u32,
    retire_raw: bool,
) -> Result<PackedCapture, String> {
    let (raw, size, digest) = flux_capture::raw_identity(project, disk, attempt)?;
    let _guard = lock(&raw, true)?;
    let final_zip = packed_path(&raw);
    let final_meta = sidecar(&raw);
    if present(&final_meta)? {
        let value = record(&raw, size, &digest)?;
        unpack(&raw, &final_zip, size, &digest, std::io::sink())?;
        if retire_raw && present(&raw)? {
            verify_unlocked(&raw, size, &digest)?;
            fs::remove_file(&raw)
                .map_err(|e| format!("Packed evidence verified, but raw retirement failed: {e}"))?;
        }
        return Ok(value);
    }
    verify_unlocked(&raw, size, &digest)?;
    if !present(&final_zip)? {
        require_space(
            raw.parent().ok_or("Capture has no parent")?,
            size + size / 100 + 16 * 1024 * 1024,
        )?;
        let temp = raw.with_file_name(format!(".fluxvault-pack-{}.partial.zip", nonce()));
        let output = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
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
        hash(
            File::open(&raw).map_err(|e| e.to_string())?,
            size,
            &digest,
            &mut writer,
        )?;
        let output = writer.finish().map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        unpack(&raw, &temp, size, &digest, std::io::sink())?;
        crate::flux_recovery::publish_image_no_replace(&temp, &final_zip)?;
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
    let temporary = raw.with_file_name(format!(".fluxvault-pack-{}.partial.json", nonce()));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    output
        .write_all(&serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    drop(output);
    crate::flux_recovery::publish_image_no_replace(&temporary, &final_meta)?;
    // Reopen the published pair. Never retire on a write/verification failure.
    record(&raw, size, &digest)?;
    unpack(&raw, &final_zip, size, &digest, std::io::sink())?;
    if retire_raw {
        verify_unlocked(&raw, size, &digest)?;
        fs::remove_file(&raw).map_err(|e| e.to_string())?;
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
