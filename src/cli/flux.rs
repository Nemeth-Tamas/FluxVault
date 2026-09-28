//! Greaseweazle CLI commands. Physical capture is read-only; decoding is image-only.

use serde_json::json;

use crate::{
    external_tools::{self, ToolKind},
    flux_capture::{self, CaptureRequest},
    greaseweazle::{GreaseweazleProfile, ProcessGreaseweazleBackend},
    imaging,
    project::ProjectState,
};

use super::CliResponse;

pub(super) fn capture(
    project: &ProjectState,
    disk_number: u32,
    profile_override: Option<GreaseweazleProfile>,
    drive: char,
    revolutions: u32,
    source_write_protected: bool,
    json_output: bool,
) -> Result<CliResponse, String> {
    if !source_write_protected {
        return Err("Raw capture requires --source-write-protected after checking the floppy's physical write-protect tab; this assertion does not prove the hardware blocks writes".to_owned());
    }
    let profile = match profile_override {
        Some(profile) => profile,
        None => infer_profile(project, disk_number)?,
    };
    let settings = external_tools::load_settings()?;
    let audit_path = project.logs_dir().join("external-tools.jsonl");
    let executable = external_tools::find_ready_tool(
        ToolKind::Greaseweazle,
        settings.path(ToolKind::Greaseweazle),
        &audit_path,
    )?;
    let mut backend = ProcessGreaseweazleBackend::new(executable, audit_path)?;
    eprintln!(
        "READ ONLY: preserving raw flux for disk {disk_number:03} on Greaseweazle drive {drive} ({}; {revolutions} revolutions)",
        profile.argument()
    );
    let result = flux_capture::capture(
        project,
        CaptureRequest {
            disk_number,
            profile,
            drive,
            revolutions,
        },
        &mut backend,
    )?;
    Ok(CliResponse {
        output: if json_output {
            json!({
                "disk": result.disk_number,
                "attempt": result.attempt_number,
                "raw_flux": result.flux_path,
                "metadata": result.metadata_path,
                "bytes": result.bytes,
                "sha256": result.sha256,
                "profile": profile.argument(),
                "source_media_access": "read_only"
            })
            .to_string()
        } else {
            format!(
                "Raw-flux capture {:03} attempt #{:03}: {} bytes, SHA-256 {}\nSCP: {}\nMetadata: {}",
                result.disk_number,
                result.attempt_number,
                result.bytes,
                result.sha256,
                result.flux_path.display(),
                result.metadata_path.display()
            )
        },
        exit_code: 0,
    })
}

pub(super) fn decode(
    project: &ProjectState,
    disk_number: u32,
    capture_attempt: Option<u32>,
    profile_override: Option<GreaseweazleProfile>,
    json_output: bool,
) -> Result<CliResponse, String> {
    let capture_attempt = match capture_attempt {
        Some(attempt) => attempt,
        None => flux_capture::latest_capture_attempt(project, disk_number)?,
    };
    let settings = external_tools::load_settings()?;
    let audit_path = project.logs_dir().join("external-tools.jsonl");
    let executable = external_tools::find_ready_tool(
        ToolKind::Greaseweazle,
        settings.path(ToolKind::Greaseweazle),
        &audit_path,
    )?;
    let mut backend = ProcessGreaseweazleBackend::new(executable, audit_path)?;
    eprintln!(
        "Offline decode of disk {disk_number:03} raw capture #{capture_attempt:03}; no floppy drive accessed"
    );
    let result = flux_capture::decode(
        project,
        disk_number,
        capture_attempt,
        profile_override,
        &mut backend,
    )?;
    Ok(CliResponse {
        output: if json_output {
            json!({
                "disk": result.disk_number,
                "capture_attempt": result.capture_attempt,
                "decode_attempt": result.decode_attempt,
                "derived_image": result.image_path,
                "metadata": result.metadata_path,
                "bytes": result.bytes,
                "sha256": result.sha256,
                "sector_quality": "unverified",
                "physical_media_access": false
            })
            .to_string()
        } else {
            format!(
                "Offline decode {:03} capture #{:03}, decode #{:03}: {} bytes\nImage: {}\nSHA-256: {}\nSector quality is not yet verified; this image is not automatically promoted for extraction.",
                result.disk_number,
                result.capture_attempt,
                result.decode_attempt,
                result.bytes,
                result.image_path.display(),
                result.sha256
            )
        },
        exit_code: 3,
    })
}

fn infer_profile(project: &ProjectState, disk_number: u32) -> Result<GreaseweazleProfile, String> {
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk_number)?;
    attempts
        .iter()
        .rev()
        .find_map(|attempt| match attempt.total_sectors {
            2880 => Some(GreaseweazleProfile::Ibm1440),
            1440 => Some(GreaseweazleProfile::Ibm720),
            _ => None,
        })
        .ok_or_else(|| {
            format!(
                "Cannot infer disk {disk_number:03} format from saved USB evidence; pass --profile ibm.1440 or ibm.720"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn capture_gate_rejects_unconfirmed_media_before_tool_or_drive_access() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-flux-cli-gate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let error = capture(
            &project,
            1,
            Some(GreaseweazleProfile::Ibm1440),
            'A',
            3,
            false,
            true,
        )
        .unwrap_err();
        assert!(error.contains("--source-write-protected"));
        assert!(
            std::fs::read_dir(project.root().join("Flux"))
                .unwrap()
                .next()
                .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
