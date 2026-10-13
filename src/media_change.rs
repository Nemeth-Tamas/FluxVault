//! Metadata-only removable-media observation. FILE_READ_ATTRIBUTES avoids a
//! filesystem mount; no data stream, write access, eject or lock IOCTL is used.
//! Some USB adapters cache a successful response even after removal. This is
//! driver-reported metadata only, never independent physical custody proof.
//! https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddstor/ni-ntddstor-ioctl_storage_check_verify2
use crate::floppy::FloppyDrive;
use serde::Serialize;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Observation {
    Present { change_counter: Option<u32> },
    Absent,
    Unknown { code: Option<u32>, detail: String },
}
#[derive(Default)]
pub struct Detector {
    candidate: Option<Observation>,
    repeats: u8,
    stable: Option<Observation>,
}
impl Detector {
    pub fn update(&mut self, reading: &Observation) -> Option<&'static str> {
        if matches!(reading, Observation::Unknown { .. }) {
            self.candidate = None;
            self.repeats = 0;
            return None;
        }
        if self.candidate.as_ref() == Some(reading) {
            self.repeats = self.repeats.saturating_add(1);
        } else {
            self.candidate = Some(reading.clone());
            self.repeats = 1;
        }
        if self.repeats < 3 || self.stable.as_ref() == Some(reading) {
            return None;
        }
        let event = match (self.stable.as_ref(), reading) {
            (None, _) => "initial_stable_observation",
            (Some(Observation::Absent), Observation::Present { .. }) => "stable_insertion",
            (Some(Observation::Present { .. }), Observation::Absent) => "stable_removal",
            (
                Some(Observation::Present {
                    change_counter: Some(a),
                }),
                Observation::Present {
                    change_counter: Some(b),
                },
            ) if a != b => "media_counter_changed_not_label_identity",
            _ => "observation_capability_changed_no_swap_proof",
        };
        self.stable = Some(reading.clone());
        Some(event)
    }
}
fn classify(code: u32, detail: String) -> Observation {
    // NOT_READY, MEDIA_CHANGED, access errors and unsupported IOCTLs are not
    // proof of removal. Only Windows' explicit NO_MEDIA_IN_DRIVE qualifies.
    if code == 1112 {
        Observation::Absent
    } else {
        Observation::Unknown {
            code: Some(code),
            detail,
        }
    }
}
#[cfg(windows)]
pub fn probe(drive: &FloppyDrive) -> Observation {
    use std::{
        fs::OpenOptions,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::FILE_READ_ATTRIBUTES,
        System::{IO::DeviceIoControl, Ioctl::IOCTL_STORAGE_CHECK_VERIFY2},
    };
    let bytes = drive.root.as_bytes();
    if bytes.len() != 3
        || !bytes[0].is_ascii_uppercase()
        || &bytes[1..] != b":\\"
        || drive.device_path != format!(r"\\.\{}:", bytes[0] as char)
    {
        return Observation::Unknown {
            code: None,
            detail: "Invalid removable drive identity; no device opened".into(),
        };
    }
    let file = match OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .open(&drive.device_path)
    {
        Ok(f) => f,
        Err(e) => return classify(e.raw_os_error().unwrap_or(-1) as u32, e.to_string()),
    };
    let mut counter = 0u32;
    let mut returned = 0u32;
    let result = unsafe {
        DeviceIoControl(
            HANDLE(file.as_raw_handle()),
            IOCTL_STORAGE_CHECK_VERIFY2,
            None,
            0,
            Some((&mut counter as *mut u32).cast()),
            4,
            Some(&mut returned),
            None,
        )
    };
    match result {
        Ok(()) => Observation::Present {
            change_counter: (returned == 4).then_some(counter),
        },
        Err(e) => classify((e.code().0 as u32) & 0xffff, e.to_string()),
    }
}
#[cfg(not(windows))]
pub fn probe(_drive: &FloppyDrive) -> Observation {
    Observation::Unknown {
        code: None,
        detail: "Metadata-only media observation requires Windows".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insertion_removal_and_change_counts_require_three_consistent_observations() {
        let mut d = Detector::default();
        let present = Observation::Present {
            change_counter: Some(1),
        };
        assert!(d.update(&Observation::Absent).is_none());
        assert!(d.update(&present).is_none());
        assert!(d.update(&Observation::Absent).is_none());
        assert!(d.update(&Observation::Absent).is_none());
        assert_eq!(
            d.update(&Observation::Absent),
            Some("initial_stable_observation")
        );
        assert!(d.update(&present).is_none());
        assert!(d.update(&present).is_none());
        assert_eq!(d.update(&present), Some("stable_insertion"));
        let next = Observation::Present {
            change_counter: Some(2),
        };
        for _ in 0..2 {
            assert!(d.update(&next).is_none());
        }
        assert_eq!(
            d.update(&next),
            Some("media_counter_changed_not_label_identity")
        );
        for _ in 0..2 {
            assert!(d.update(&Observation::Absent).is_none());
        }
        assert_eq!(d.update(&Observation::Absent), Some("stable_removal"));
    }
    #[test]
    fn not_ready_unsupported_disconnects_and_unknown_reset_debounce_not_custody() {
        let mut d = Detector::default();
        for _ in 0..3 {
            d.update(&Observation::Present {
                change_counter: None,
            });
        }
        for code in [1, 2, 5, 21, 1110, 1167] {
            let o = classify(code, "raw Windows diagnostic".into());
            assert!(matches!(o, Observation::Unknown { .. }));
            assert!(d.update(&o).is_none());
        }
        assert_eq!(classify(1112, "no media".into()), Observation::Absent);
        assert!(d.update(&Observation::Absent).is_none());
        assert!(d.update(&Observation::Absent).is_none());
        assert_eq!(d.update(&Observation::Absent), Some("stable_removal"));
    }
}
