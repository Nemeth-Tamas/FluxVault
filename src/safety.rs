#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMediaAccess {
    ReadOnly,
}

pub struct MediaSafetyPolicy;

/// Reject known floppy/device spellings before resolving a workstation path.
/// Check both the supplied and canonical path to catch junction aliases too.
pub(crate) fn workstation_path(path: &std::path::Path) -> Result<(), String> {
    let value = path
        .to_string_lossy()
        .to_ascii_uppercase()
        .replace('/', "\\");
    if [
        "A:",
        "B:",
        "\\\\?\\A:",
        "\\\\?\\B:",
        "\\\\.\\",
        "\\\\?\\GLOBALROOT\\",
        "\\\\?\\VOLUME{",
        "\\??\\",
        "\\DEVICE\\",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
    {
        return Err("Workstation folders only, never floppy/device paths".into());
    }
    Ok(())
}

impl MediaSafetyPolicy {
    pub const SOURCE_MEDIA_ACCESS: SourceMediaAccess = SourceMediaAccess::ReadOnly;

    pub const ALLOW_PHYSICAL_MEDIA_WRITES: bool = false;

    pub const ALLOW_GREASEWEAZLE_WRITES: bool = false;

    pub fn assert_invariants() {
        assert_eq!(
            Self::SOURCE_MEDIA_ACCESS,
            SourceMediaAccess::ReadOnly,
            "FluxVault source media access must remain read-only"
        );

        assert!(
            !Self::ALLOW_PHYSICAL_MEDIA_WRITES,
            "FluxVault must never enable writes to source floppy media"
        );

        assert!(
            !Self::ALLOW_GREASEWEAZLE_WRITES,
            "FluxVault must never enable Greaseweazle write operations"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workstation_guard_rejects_floppy_and_device_aliases_without_io() {
        for path in [
            "A:",
            "B:\\delivery",
            "a:/delivery",
            "\\\\?\\B:\\delivery",
            "\\\\.\\PhysicalDrive0",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1",
            "\\\\?\\Volume{fixture}\\",
            "\\??\\A:\\",
            "\\Device\\Floppy0",
        ] {
            assert!(
                workstation_path(std::path::Path::new(path)).is_err(),
                "{path}"
            );
        }
        for path in [
            "C:\\delivery",
            "\\\\?\\C:\\delivery",
            "\\\\server\\archive",
            "relative-output",
        ] {
            assert!(
                workstation_path(std::path::Path::new(path)).is_ok(),
                "{path}"
            );
        }
    }
}
