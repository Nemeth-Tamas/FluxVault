//! Cross-process ownership of one project's delivery plan, conversion outputs and state.

use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

pub(crate) fn reserve(reports: &Path) -> Result<File, String> {
    let root = reports
        .parent()
        .ok_or("Conversion reports directory has no project parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = root.join(".fluxvault-conversion.lock");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("Unsafe conversion lock path".to_owned());
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => {}
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|_| "Another conversion or delivery-plan job is running in this project; no shared output changed".to_owned())?;
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_lock_refuses_second_owner_and_releases_without_deleting_state() {
        let root = std::env::temp_dir().join(format!(
            "fv-conversion-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let reports = root.join("Reports");
        let first = reserve(&reports).unwrap();
        assert!(
            reserve(&reports)
                .unwrap_err()
                .contains("Another conversion")
        );
        drop(first);
        drop(reserve(&reports).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
