//! Gated physical acquisition using the GUI's read-only USB backend.

use serde_json::json;

use crate::{
    floppy::{self, DiskGeometry, ProbeResult, WriteProtectionStatus},
    imaging::{self, ImagingEvent},
    project::ProjectState,
    safety::MediaSafetyPolicy,
};

use super::{CliResponse, select_removable_drive};

pub(super) fn run(
    project: &ProjectState,
    json_output: bool,
    drive_name: Option<&str>,
    disk_number: Option<u32>,
    retries: usize,
    write_blocker_verified: bool,
) -> Result<CliResponse, String> {
    MediaSafetyPolicy::assert_invariants();
    if !write_blocker_verified {
        return Err("Acquisition is blocked until the drive/write blocker has been independently verified with a known-good disposable disk. Then pass --write-blocker-verified. Do not validate using customer media.".to_owned());
    }
    let drive_name = drive_name.ok_or("acquire requires --drive LETTER:")?;
    let disk_number = disk_number.ok_or("acquire requires --disk N")?;
    let drives = floppy::enumerate_removable_drives()?;
    let drive = select_removable_drive(&drives, drive_name)?;
    let probe = floppy::probe_read_only(&drive)?;
    let geometry = acquisition_eligible(&probe)?;
    eprintln!(
        "READ ONLY: imaging disk {disk_number:03} from {} ({}; {} bytes; {} retry passes)",
        drive.root,
        geometry.format_guess(),
        geometry.total_bytes(),
        retries
    );
    let events = imaging::start_imaging(
        drive,
        geometry,
        project.images_dir(),
        project.logs_dir(),
        disk_number,
        retries,
    );
    let mut last_progress_bucket = 0;
    for event in events {
        match event {
            ImagingEvent::Started { output_path, .. } => {
                eprintln!("Saving acquisition: {}", output_path.display());
            }
            ImagingEvent::Progress { completed, total } if total > 0 => {
                let bucket = completed.saturating_mul(10) / total;
                if bucket > last_progress_bucket {
                    last_progress_bucket = bucket;
                    eprintln!("Read {}%", bucket * 10);
                }
            }
            ImagingEvent::Log(message) => eprintln!("{message}"),
            ImagingEvent::Completed(result) => {
                let attention = !result.bad_sectors.is_empty();
                return Ok(CliResponse {
                    output: if json_output {
                        json!({
                            "disk": result.disk_number,
                            "attempt": result.attempt_number,
                            "image": result.output_path,
                            "metadata": result.metadata_path,
                            "log": result.log_path,
                            "sha256": result.sha256,
                            "total_sectors": result.total_sectors,
                            "bad_sectors": result.bad_sectors,
                            "retry_recovered": result.retry_recovered,
                            "bytes_written_to_image": result.bytes_written,
                            "source_media_access": "read_only"
                        })
                        .to_string()
                    } else {
                        format!(
                            "Disk {:03}, attempt #{:03}: {} bytes imaged, {} bad sectors, {} retry-recovered.\nImage: {}\nSHA-256: {}",
                            result.disk_number,
                            result.attempt_number,
                            result.bytes_written,
                            result.bad_sectors.len(),
                            result.retry_recovered,
                            result.output_path.display(),
                            result.sha256
                        )
                    },
                    exit_code: if attention { 3 } else { 0 },
                });
            }
            ImagingEvent::Failed(error) => return Err(error),
            ImagingEvent::Sector { .. } | ImagingEvent::Progress { .. } => {}
        }
    }
    Err("Imaging worker stopped without a completion result; inspect partial files in Images and Logs".to_owned())
}

fn acquisition_eligible(probe: &ProbeResult) -> Result<DiskGeometry, String> {
    if probe.write_protection != WriteProtectionStatus::Protected {
        return Err(format!(
            "Acquisition blocked: physical write protection was not positively reported ({:?})",
            probe.write_protection
        ));
    }
    let geometry = probe
        .geometry
        .ok_or("Acquisition blocked: drive geometry could not be read")?;
    if !geometry.looks_like_floppy() {
        return Err("Acquisition blocked: geometry does not look like a floppy".to_owned());
    }
    Ok(geometry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquisition_guards_reject_unverified_media_without_a_drive_open() {
        let root =
            std::env::temp_dir().join(format!("fluxvault-acquire-gate-{}", std::process::id()));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let error = run(&project, true, Some("A:"), Some(1), 2, false).unwrap_err();
        assert!(error.contains("independently verified"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acquisition_requires_positive_protection_and_floppy_geometry() {
        let geometry = DiskGeometry {
            cylinders: 80,
            heads: 2,
            sectors_per_track: 18,
            bytes_per_sector: 512,
            media_type: 0,
        };
        let probe = ProbeResult {
            bytes_read: 512,
            first_bytes: [0; 16],
            boot_signature: Some([0x55, 0xaa]),
            geometry: Some(geometry),
            geometry_error: None,
            write_protection: WriteProtectionStatus::Writable,
        };
        assert!(acquisition_eligible(&probe).is_err());
        assert_eq!(
            acquisition_eligible(&ProbeResult {
                write_protection: WriteProtectionStatus::Protected,
                ..probe
            })
            .unwrap()
            .total_bytes(),
            1_474_560
        );
    }
}
