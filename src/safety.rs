#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMediaAccess {
    ReadOnly,
}

pub struct MediaSafetyPolicy;

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
