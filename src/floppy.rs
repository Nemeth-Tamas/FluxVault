use std::fs::File;
use std::io::Read;

#[cfg(windows)]
use windows::{
    Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
    core::PCWSTR,
};

const PROBE_SIZE: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloppyDrive {
    pub root: String,
    pub device_path: String,
}

impl FloppyDrive {
    pub fn display_name(&self) -> String {
        format!("{}  [{}]", self.root, self.device_path)
    }
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub bytes_read: usize,
    pub first_bytes: [u8; 16],
    pub boot_signature: Option<[u8; 2]>,
}

impl ProbeResult {
    pub fn first_bytes_hex(&self) -> String {
        self.first_bytes
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn boot_signature_hex(&self) -> String {
        match self.boot_signature {
            Some(signature) => {
                format!("{:02X} {:02X}", signature[0], signature[1])
            }
            None => "nem olvasható".to_owned(),
        }
    }
}

pub fn enumerate_removable_drives() -> Result<Vec<FloppyDrive>, String> {
    enumerate_removable_drives_platform()
}

#[cfg(windows)]
fn enumerate_removable_drives_platform() -> Result<Vec<FloppyDrive>, String> {
    const DRIVE_REMOVABLE: u32 = 2;

    let drive_mask = unsafe { GetLogicalDrives() };

    if drive_mask == 0 {
        return Err(format!(
            "A Windows nem adta vissza a logikai meghajtókat: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut drives = Vec::new();

    for index in 0..26 {
        let mask = 1u32 << index;

        if drive_mask & mask == 0 {
            continue;
        }

        let letter = (b'A' + index as u8) as char;
        let root = format!("{letter}:\\");
        let device_path = format!(r"\\.\{letter}:");

        let wide_root: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

        let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide_root.as_ptr())) };

        if drive_type == DRIVE_REMOVABLE {
            drives.push(FloppyDrive { root, device_path });
        }
    }

    Ok(drives)
}

#[cfg(not(windows))]
fn enumerate_removable_drives_platform() -> Result<Vec<FloppyDrive>, String> {
    Ok(Vec::new())
}

pub fn probe_read_only(drive: &FloppyDrive) -> Result<ProbeResult, String> {
    let mut file = File::open(&drive.device_path).map_err(|error| {
        format!(
            "Nem sikerült CSAK OLVASHATÓ módban megnyitni a(z) {} eszközt: {error}",
            drive.device_path
        )
    })?;

    let mut sector = [0u8; PROBE_SIZE];
    let mut total_read = 0usize;

    while total_read < sector.len() {
        let bytes_read = file.read(&mut sector[total_read..]).map_err(|error| {
            format!("Olvasási hiba a(z) {} eszközön: {error}", drive.device_path)
        })?;

        if bytes_read == 0 {
            break;
        }

        total_read += bytes_read;
    }

    if total_read == 0 {
        return Err(format!(
            "A(z) {} meghajtó megnyílt, de 0 bájt érkezett vissza.",
            drive.device_path
        ));
    }

    let mut first_bytes = [0u8; 16];
    let preview_length = total_read.min(first_bytes.len());

    first_bytes[..preview_length].copy_from_slice(&sector[..preview_length]);

    let boot_signature = if total_read >= PROBE_SIZE {
        Some([sector[510], sector[511]])
    } else {
        None
    };

    Ok(ProbeResult {
        bytes_read: total_read,
        first_bytes,
        boot_signature,
    })
}
