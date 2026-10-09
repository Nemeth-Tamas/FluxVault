//! Generation-bound local stop requests. Never kills a PID or accesses a device.
use crate::{cancellation, project::ProjectState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
const CONTROL: &str = ".fluxvault-run-control.json";
const LOCK: &str = ".fluxvault-run-control.lock";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
thread_local! {
    static INHERITED_SESSION: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    generation: String,
    pid: u32,
    project: PathBuf,
    operation: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    generation: String,
    stop: bool,
}
fn regular(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_file() => Err("Unsafe run-control file".into()),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    regular(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 8192 {
        return Err("Oversized run-control record".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
fn root(project: &ProjectState) -> Result<PathBuf, String> {
    crate::processing::validate_workspace(project)?;
    project.root().canonicalize().map_err(|e| e.to_string())
}
fn record(root: &Path) -> Result<Record, String> {
    let r: Record = read(&root.join(CONTROL))?;
    if r.schema != 1
        || r.project != root
        || r.generation.is_empty()
        || r.generation.len() > 100
        || !r
            .generation
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-')
    {
        return Err("Invalid run-control identity".into());
    }
    Ok(r)
}
fn active(root: &Path) -> Result<bool, String> {
    let path = root.join(LOCK);
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(false);
    }
    regular(&path)?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    match f.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}
fn request_path(root: &Path, generation: &str) -> PathBuf {
    root.join(format!(".fluxvault-stop-{generation}.json"))
}
pub fn status(project: &ProjectState) -> Result<Value, String> {
    let root = root(project)?;
    let active = active(&root)?;
    let r = if root.join(CONTROL).exists() {
        Some(record(&root)?)
    } else {
        None
    };
    Ok(
        json!({"active":active,"record":r,"physical_media_access":false,"record_is_not_proof_of_active_work":!active}),
    )
}
pub fn stop(project: &ProjectState) -> Result<Value, String> {
    let root = root(project)?;
    if !active(&root)? {
        return Err("No active controllable operation in this project; nothing stopped".into());
    }
    let r = record(&root)?;
    let request = Request {
        schema: 1,
        generation: r.generation.clone(),
        stop: true,
    };
    let path = request_path(&root, &r.generation);
    regular(&path)?;
    match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(mut f) => {
            f.write_all(&serde_json::to_vec(&request).map_err(|e| e.to_string())?)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let old: Request = read(&path)?;
            if old.schema != 1 || old.generation != r.generation || !old.stop {
                return Err("Invalid existing stop request".into());
            }
        }
        Err(e) => return Err(e.to_string()),
    };
    Ok(
        json!({"stop_requested":true,"generation":r.generation,"operation":r.operation,"physical_media_access":false,"wait_for_original_console_to_confirm_stopped":true}),
    )
}
pub(crate) struct Session {
    _owner: Option<File>,
    root: PathBuf,
    _scope: cancellation::Scope,
    wake: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Session {
    pub(crate) fn start(project: &ProjectState, operation: &str) -> Result<Self, String> {
        let root = root(project)?;
        if let Some(parent) = INHERITED_SESSION.with(|slot| slot.borrow().clone()) {
            if parent != root {
                return Err("Inherited stop controller belongs to another project".into());
            }
            return Ok(Self {
                _owner: None,
                root,
                _scope: cancellation::enter(cancellation::current()),
                wake: None,
                worker: None,
            });
        }
        regular(&root.join(LOCK))?;
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(LOCK))
            .map_err(|e| e.to_string())?;
        owner
            .try_lock()
            .map_err(|_| "This project already has a controllable operation")?;
        let r = Record {
            schema: 1,
            generation: format!(
                "{}-{}-{}",
                crate::external_tools::current_unix_ms(),
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            pid: std::process::id(),
            project: root.clone(),
            operation: operation.into(),
        };
        let temporary = root.join(format!(".fluxvault-run-{}.partial.json", r.generation));
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        f.write_all(&serde_json::to_vec(&r).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        regular(&root.join(CONTROL))?;
        fs::rename(temporary, root.join(CONTROL)).map_err(|e| e.to_string())?;
        let token = cancellation::current();
        let scope = cancellation::enter(token.clone());
        let (wake, rx) = mpsc::channel();
        let controller_root = root.clone();
        let worker = thread::spawn(move || {
            loop {
                if token.requested() {
                    break;
                }
                if let Ok(request) = read::<Request>(&request_path(&root, &r.generation)) {
                    if request.schema == 1 && request.generation == r.generation && request.stop {
                        token.request();
                        break;
                    }
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            }
        });
        Ok(Self {
            _owner: Some(owner),
            root: controller_root,
            _scope: scope,
            wake: Some(wake),
            worker: Some(worker),
        })
    }

    /// Keep one generation/stop watcher across all synchronous production phases.
    pub(crate) fn with_children<T>(&self, work: impl FnOnce() -> T) -> T {
        let previous = INHERITED_SESSION.with(|slot| slot.replace(Some(self.root.clone())));
        struct Restore(Option<PathBuf>);
        impl Drop for Restore {
            fn drop(&mut self) {
                INHERITED_SESSION.with(|slot| {
                    slot.replace(self.0.take());
                });
            }
        }
        let _restore = Restore(previous);
        work()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod parent_tests {
    use super::*;
    #[test]
    fn inherited_children_keep_generation_and_stop_request_scoped_to_the_parent() {
        let root = std::env::temp_dir().join(format!(
            "fv-parent-control-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let _scope = cancellation::enter(cancellation::Token::default());
        let parent = Session::start(&project, "production_start").unwrap();
        let before = fs::read(root.join(CONTROL)).unwrap();
        parent.with_children(|| {
            let child = Session::start(&project, "scan").unwrap();
            assert_eq!(fs::read(root.join(CONTROL)).unwrap(), before);
            drop(child);
            assert!(status(&project).unwrap()["active"] == true);
            stop(&project).unwrap();
            let until = std::time::Instant::now() + Duration::from_secs(2);
            while !cancellation::requested() && std::time::Instant::now() < until {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(cancellation::requested());
        });
        assert_eq!(fs::read(root.join(CONTROL)).unwrap(), before);
        drop(parent);
        assert_eq!(status(&project).unwrap()["active"], false);
        fs::remove_dir_all(root).unwrap();
    }
}
/// Live stdin is read on a detached reader so stop also interrupts a swap prompt.
pub(crate) struct Input {
    receiver: mpsc::Receiver<Vec<u8>>,
    buffer: Vec<u8>,
    position: usize,
}
impl Input {
    pub(crate) fn stdin() -> Self {
        let (tx, rx) = mpsc::sync_channel(8);
        let stop_token = cancellation::current();
        thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            loop {
                let mut line = Vec::new();
                match input.by_ref().take(4097).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() > 4096 => break,
                    Ok(_) => {
                        if String::from_utf8_lossy(&line)
                            .trim()
                            .eq_ignore_ascii_case("STOP")
                        {
                            stop_token.request();
                            break;
                        }
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            receiver: rx,
            buffer: Vec::new(),
            position: 0,
        }
    }
}
impl std::io::BufRead for Input {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        while self.position == self.buffer.len() {
            if cancellation::requested() {
                return Err(std::io::Error::other(cancellation::MESSAGE));
            }
            match self.receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(bytes) => {
                    self.buffer = bytes;
                    self.position = 0;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Ok(&[]),
            }
        }
        Ok(&self.buffer[self.position..])
    }
    fn consume(&mut self, n: usize) {
        self.position = (self.position + n).min(self.buffer.len());
    }
}
impl Read for Input {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        use std::io::BufRead;
        let bytes = self.fill_buf()?;
        let n = bytes.len().min(out.len());
        out[..n].copy_from_slice(&bytes[..n]);
        self.consume(n);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> ProjectState {
        ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-control-test-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )))
        .unwrap()
    }
    #[test]
    fn owner_stop_and_generation_are_scoped_and_inactive_requests_refused() {
        let other = project();
        let project = project();
        assert!(stop(&project).is_err());
        let session = Session::start(&project, "test").unwrap();
        assert!(Session::start(&project, "second").is_err());
        assert!(stop(&other).is_err());
        let token = cancellation::current();
        assert_eq!(status(&project).unwrap()["active"], true);
        stop(&project).unwrap();
        stop(&project).unwrap();
        for _ in 0..100 {
            if token.requested() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(token.requested());
        drop(session);
        assert_eq!(status(&project).unwrap()["active"], false);
        assert!(stop(&project).is_err());
        let resumed = Session::start(&project, "resume").unwrap();
        thread::sleep(Duration::from_millis(150));
        assert!(
            !cancellation::requested(),
            "old request must not cancel new generation"
        );
        drop(resumed);
        fs::remove_dir_all(project.root()).unwrap();
        fs::remove_dir_all(other.root()).unwrap();
    }
    #[test]
    fn malformed_foreign_and_oversized_request_records_never_stop_an_owner() {
        let project = project();
        let session = Session::start(&project, "test").unwrap();
        let r = record(&root(&project).unwrap()).unwrap();
        let path = request_path(project.root(), &r.generation);
        for bytes in [
            b"broken".to_vec(),
            vec![b' '; 8193],
            serde_json::to_vec(&Request {
                schema: 1,
                generation: "foreign".into(),
                stop: true,
            })
            .unwrap(),
        ] {
            fs::write(&path, bytes).unwrap();
            thread::sleep(Duration::from_millis(150));
            assert!(!cancellation::requested());
            assert!(stop(&project).is_err());
        }
        drop(session);
        fs::remove_dir_all(project.root()).unwrap();
    }
    #[test]
    fn token_scope_is_inherited_by_workers_and_restored_for_unrelated_work() {
        let token = cancellation::Token::default();
        let scope = cancellation::enter(token.clone());
        let worker = cancellation::spawn(move || {
            while !cancellation::requested() {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(cancellation::check().is_err());
        });
        token.request();
        worker.join().unwrap();
        drop(scope);
        assert!(!cancellation::requested());
    }
}
