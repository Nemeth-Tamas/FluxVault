use std::fs::File;
use std::io::Read;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
        System::{
            IO::DeviceIoControl,
            Ioctl::{DISK_GEOMETRY, IOCTL_DISK_GET_DRIVE_GEOMETRY},
        },
    },
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

#[derive(Debug, Clone, Copy)]
pub struct DiskGeometry {
    pub cylinders: u64,
    pub heads: u32,
    pub sectors_per_track: u32,
    pub bytes_per_sector: u32,
    pub media_type: i32,
}

impl DiskGeometry {
    pub fn total_sectors(&self) -> u64 {
        self.cylinders
            .saturating_mul(self.heads as u64)
            .saturating_mul(self.sectors_per_track as u64)
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_sectors()
            .saturating_mul(self.bytes_per_sector as u64)
    }

    pub fn looks_like_floppy(&self) -> bool {
        let total_bytes = self.total_bytes();

        self.cylinders > 0
            && self.heads > 0
            && self.heads <= 2
            && self.sectors_per_track > 0
            && self.sectors_per_track <= 36
            && self.bytes_per_sector >= 128
            && self.bytes_per_sector <= 4096
            && total_bytes > 0
            && total_bytes <= 4 * 1024 * 1024
    }

    pub fn format_guess(&self) -> &'static str {
        match (
            self.cylinders,
            self.heads,
            self.sectors_per_track,
            self.bytes_per_sector,
        ) {
            (80, 2, 18, 512) => "1.44 MB HD",
            (80, 2, 9, 512) => "720 KB DD",
            (80, 2, 15, 512) => "1.2 MB",
            (40, 2, 9, 512) => "360 KB",
            (40, 2, 8, 512) => "320 KB",
            (40, 1, 9, 512) => "180 KB",
            (40, 1, 8, 512) => "160 KB",
            _ => "Ismeretlen / nem szabvanyos",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub bytes_read: usize,
    pub first_bytes: [u8; 16],
    pub boot_signature: Option<[u8; 2]>,
    pub geometry: Option<DiskGeometry>,
    pub geometry_error: Option<String>,
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

#[cfg(windows)]
fn query_geometry(file: &File) -> Result<DiskGeometry, String> {
    let mut geometry = DISK_GEOMETRY::default();
    let mut bytes_returned = 0u32;

    let handle = HANDLE(file.as_raw_handle());

    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY,
            None,
            0,
            Some((&mut geometry as *mut DISK_GEOMETRY).cast()),
            std::mem::size_of::<DISK_GEOMETRY>() as u32,
            Some(&mut bytes_returned),
            None,
        )
        .map_err(|error| format!("Lemezgeometria lekerdezesi hiba: {error}"))?;
    }

    if bytes_returned < std::mem::size_of::<DISK_GEOMETRY>() as u32 {
        return Err(format!(
            "A geometria lekerdezes csak {bytes_returned} bajtot adott vissza."
        ));
    }

    Ok(DiskGeometry {
        cylinders: geometry.Cylinders.max(0) as u64,
        heads: geometry.TracksPerCylinder,
        sectors_per_track: geometry.SectorsPerTrack,
        bytes_per_sector: geometry.BytesPerSector,
        media_type: geometry.MediaType.0,
    })
}

#[cfg(not(windows))]
fn query_geometry(_file: &File) -> Result<DiskGeometry, String> {
    Err("A lemezgeometria lekerdezese jelenleg csak Windowson tamogatott.".to_owned())
}

pub fn probe_read_only(drive: &FloppyDrive) -> Result<ProbeResult, String> {
    let mut file = File::open(&drive.device_path).map_err(|error| {
        format!(
            "Nem sikerült CSAK OLVASHATÓ módban megnyitni a(z) {} eszközt: {error}",
            drive.device_path
        )
    })?;

    let (geometry, geometry_error) = match query_geometry(&file) {
        Ok(geometry) => (Some(geometry), None),
        Err(error) => (None, Some(error)),
    };

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
        geometry,
        geometry_error,
    })
}
