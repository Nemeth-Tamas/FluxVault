//! Physical USB/GW exclusion across this user's CLI processes/projects.
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

pub(super) struct GreaseweazleReservation {
    _lock: File,
}

pub(super) struct UsbReservation {
    _lock: File,
}

impl UsbReservation {
    pub(super) fn acquire(drive: &str) -> Result<Self, String> {
        let drive = drive.trim().to_ascii_uppercase();
        if drive.len() != 2 || !drive.as_bytes()[0].is_ascii_uppercase() || !drive.ends_with(':') {
            return Err("USB drive must be a Windows letter such as A:".into());
        }
        let audit = crate::external_tools::default_audit_path();
        Self::at(
            audit.parent().ok_or("Missing USB reservation folder")?,
            &drive,
        )
    }

    fn at(root: &Path, drive: &str) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let path = root.join(format!(".fluxvault-usb-{}.lock", &drive[..1]));
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.file_type().is_file() => return Err("Unsafe USB reservation path".into()),
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.try_lock()
            .map_err(|_| "USB drive is reserved by another FluxVault command".to_string())?;
        Ok(Self { _lock: file })
    }
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

    #[test]
    fn usb_reservation_is_per_drive_and_releases_with_its_owner() {
        let root = std::env::temp_dir().join(format!(
            "fv-usb-reservation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let a = UsbReservation::at(&root, "A:").unwrap();
        assert!(UsbReservation::at(&root, "A:").is_err());
        let b = UsbReservation::at(&root, "B:").unwrap();
        drop(a);
        drop(UsbReservation::at(&root, "A:").unwrap());
        drop(b);
        fs::remove_dir_all(root).unwrap();
    }
}
