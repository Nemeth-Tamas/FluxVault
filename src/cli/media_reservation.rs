//! One physical Greaseweazle owner across this user's CLI processes/projects.
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

pub(super) struct GreaseweazleReservation {
    _lock: File,
}

impl GreaseweazleReservation {
    pub(super) fn acquire() -> Result<Self, String> {
        let audit = crate::external_tools::default_audit_path();
        Self::at(
            audit
                .parent()
                .ok_or("Cannot locate Greaseweazle reservation folder")?,
        )
    }

    fn at(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let path = root.join(".fluxvault-greaseweazle.lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err("Unsafe Greaseweazle reservation path".to_owned());
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.to_string());
            }
            _ => {}
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.try_lock().map_err(|_| {
            "Greaseweazle is reserved by another FluxVault command; finish that session first"
                .to_owned()
        })?;
        Ok(Self { _lock: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservation_blocks_another_owner_and_releases_when_owner_exits() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-device-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = GreaseweazleReservation::at(&root).unwrap();
        assert!(matches!(GreaseweazleReservation::at(&root), Err(e) if e.contains("reserved")));
        drop(first);
        drop(GreaseweazleReservation::at(&root).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
