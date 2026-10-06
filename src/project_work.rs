//! Separate long-running workstation ownership from short image/publication snapshots.
//! Physical reads never hold the snapshot gate; Office conversion releases it too.
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    time::{Duration, Instant},
};

fn open(root: &Path, name: &str) -> Result<File, String> {
    refuse_floppy(root)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    refuse_floppy(&root)?;
    let path = root.join(name);
    match fs::symlink_metadata(&path) {
        Ok(m) if !m.file_type().is_file() => return Err("Unsafe project work lock".into()),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => {}
    }
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())
}

fn refuse_floppy(path: &Path) -> Result<(), String> {
    let name = path.to_string_lossy().to_ascii_uppercase();
    if [
        "A:",
        "B:",
        "\\\\?\\A:",
        "\\\\?\\B:",
        "\\\\.\\A:",
        "\\\\.\\B:",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
    {
        return Err("Project work locks must be on the workstation, never floppy media".into());
    }
    Ok(())
}

pub(crate) fn reserve(root: &Path) -> Result<File, String> {
    let file = open(root, ".fluxvault-processing.lock")?;
    file.try_lock().map_err(|_| "This project already has a processing owner. Let the scan finish, or stop it before changing extraction/conversion/reports. `processing status` remains available.".to_owned())?;
    Ok(file)
}

/// Probe an existing ownership file without creating or modifying anything.
pub(crate) fn active(root: &Path) -> Result<bool, String> {
    refuse_floppy(root)?;
    let path = root.join(".fluxvault-processing.lock");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
        Ok(m) if !m.file_type().is_file() => return Err("Unsafe project work lock".into()),
        _ => {}
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}

pub(crate) fn snapshot(root: &Path) -> Result<File, String> {
    let file = open(root, ".fluxvault-artifacts.lock")?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock)
                if started.elapsed() < Duration::from_secs(120) =>
            {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(e) => {
                return Err(format!(
                    "Image publication/report snapshot is busy or unavailable; saved evidence retained: {e}"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floppy_work_locks_are_refused_before_any_file_access() {
        for root in ["A:", "B:\\customer", "\\\\?\\A:\\", "\\\\.\\B:"] {
            assert!(
                reserve(Path::new(root))
                    .unwrap_err()
                    .contains("never floppy")
            );
            assert!(
                active(Path::new(root))
                    .unwrap_err()
                    .contains("never floppy")
            );
        }
    }
    #[test]
    fn ownership_excludes_other_writers_but_snapshot_release_allows_publication() {
        let root = std::env::temp_dir().join(format!(
            "fv-work-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms()
        ));
        fs::create_dir(&root).unwrap();
        let owner = reserve(&root).unwrap();
        assert!(reserve(&root).is_err());
        let gate = snapshot(&root).unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let other = root.clone();
        let worker = std::thread::spawn(move || {
            let _gate = snapshot(&other).unwrap();
            sent.send(()).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(60)).is_err());
        drop(gate);
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        drop(owner);
        drop(reserve(&root).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
