//! Greaseweazle CLI commands. Physical capture is read-only; decoding is image-only.

use serde_json::json;

use crate::{
    external_tools::{self, ToolKind},
    flux_capture::{self, CaptureRequest},
    greaseweazle::{
        GreaseweazleBackend, GreaseweazleCommand, GreaseweazleDeviceStatus, GreaseweazleProfile,
        ProcessGreaseweazleBackend, classify_info_output,
    },
    imaging,
    project::ProjectState,
};

use super::{CliResponse, media_reservation::GreaseweazleReservation};

pub(super) struct RecoveryOptions {
    pub disk: u32,
    pub profile: GreaseweazleProfile,
    pub automatic_format: bool,
    pub drive: char,
    pub protected: bool,
    pub policy: crate::flux_recovery::RecoveryPolicy,
    pub acquisition_only: bool,
    pub json_output: bool,
}

pub(super) fn process_saved(project: &ProjectState) -> Result<CliResponse, String> {
    process_saved_with_workers(project, crate::conversion_run::DEFAULT_CONVERSION_WORKERS)
}

pub(super) fn process_saved_with_workers(
    project: &ProjectState,
    workers: usize,
) -> Result<CliResponse, String> {
    super::run(
        &[
            "process".to_owned(),
            "--project".to_owned(),
            project.root().display().to_string(),
            "--json".to_owned(),
            "--conversion-workers".to_owned(),
            workers.to_string(),
        ],
        project.root(),
    )
}

pub(super) fn recover(
    project: &ProjectState,
    options: RecoveryOptions,
) -> Result<CliResponse, String> {
    if !options.protected {
        return Err(
            "Recovery requires --source-write-protected after checking the physical tab".to_owned(),
        );
    }
    options.policy.validate()?;
    let reservation = GreaseweazleReservation::acquire()?;
    recover_reserved(project, options, &reservation)
}

pub(super) fn recover_reserved(
    project: &ProjectState,
    options: RecoveryOptions,
    _reservation: &GreaseweazleReservation,
) -> Result<CliResponse, String> {
    let RecoveryOptions {
        disk,
        profile,
        automatic_format,
        drive,
        protected,
        policy,
        acquisition_only,
        json_output,
    } = options;
    if !protected {
        return Err(
            "Recovery requires --source-write-protected after checking the physical tab".to_owned(),
        );
    }
    policy.validate()?;
    let settings = external_tools::load_settings()?;
    let audit = project.logs_dir().join("external-tools.jsonl");
    let executable = external_tools::find_ready_tool(
        ToolKind::Greaseweazle,
        settings.path(ToolKind::Greaseweazle),
        &audit,
    )?;
    let mut backend =
        ProcessGreaseweazleBackend::new(executable, audit)?.with_stream_to_stderr(false);
    eprintln!("READ ONLY: automatic recovery of disk {disk:03} on Greaseweazle drive {drive}");
    let result = if automatic_format {
        crate::flux_recovery::recover_auto(project, disk, drive, policy, &mut backend, &|s| {
            eprintln!("{s}")
        })?
    } else {
        crate::flux_recovery::recover(project, disk, profile, drive, policy, &mut backend, &|s| {
            eprintln!("{s}")
        })?
    };
    let mut attention = result.status != "acquired";
    let processing = if acquisition_only || result.format_exception.is_some() {
        json!({"skipped":true})
    } else {
        eprintln!("Acquisition saved. Processing project files and reports...");
        match process_saved(project) {
            Ok(response) => {
                attention |= response.exit_code != 0;
                serde_json::from_str(&response.output)
                    .unwrap_or_else(|_| json!({"detail":response.output}))
            }
            Err(error) => {
                attention = true;
                json!({"error":error,"acquisition_preserved":true})
            }
        }
    };
    let processing_summary = if result.format_exception.is_some() {
        "Skipped: raw-only exception; no supported geometry/image claimed. Sector counts are unknown; format trials are preserved in the report.".into()
    } else if acquisition_only {
        "Skipped (--acquisition-only).".to_owned()
    } else if let Some(error) = processing.get("error").and_then(|v| v.as_str()) {
        format!("Needs attention: {error}. Acquisition evidence is preserved.")
    } else {
        format!(
            "Extracted disks: {}; converted OK: {}; conversion failures: {}; recovery queue: {}; evidence attention: {}.\nWorkbook: {}",
            processing["extracted"],
            processing["converted_ok"],
            processing["converted_failed"],
            processing["recovery_queue"],
            processing["evidence_attention"],
            processing["workbook"]
                .as_str()
                .unwrap_or("See project Reports folder")
        )
    };
    Ok(CliResponse {
        output: if json_output {
            json!({"recovery":result,"processing":processing,"source_media_access":"read_only","customer_delivery_certified":false}).to_string()
        } else {
            format!(
                "Disk {disk:03}: {} ({}). Missing sectors: {}; conflicts: {}.\nPhysical reads this run: {}; corroborated sectors: {}; single-capture sectors: {}.\nImage: {}\nProvenance: {}\nDownstream: {}\nPhysical work finished; you may remove disk {disk:03}. Not customer-delivery certification.",
                result.status,
                result.stop_reason,
                if result.format_exception.is_some() {
                    "unknown".into()
                } else {
                    result.missing_lbas.len().to_string()
                },
                if result.format_exception.is_some() {
                    "unknown".into()
                } else {
                    result.conflicting_lbas.len().to_string()
                },
                result.physical_reads_this_run,
                result.corroborated_sectors,
                result.single_capture_sectors,
                result.image.display(),
                result.provenance.display(),
                processing_summary
            )
        },
        exit_code: if attention { 3 } else { 0 },
    })
}

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
    let _reservation = GreaseweazleReservation::acquire()?;
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
    verify_device_for_capture(&mut backend)?;
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

fn verify_device_for_capture(backend: &mut impl GreaseweazleBackend) -> Result<(), String> {
    let info = backend.execute(&GreaseweazleCommand::info())?;
    match (info.success, classify_info_output(&info.output_text())) {
        (true, GreaseweazleDeviceStatus::Connected) => Ok(()),
        (true, GreaseweazleDeviceStatus::NotFound) => {
            Err("No Greaseweazle board was found; no capture attempt was started".to_owned())
        }
        _ => Err(format!(
            "Greaseweazle board status could not be verified; no capture attempt was started: {} {}",
            info.stdout, info.stderr
        )),
    }
}

pub(super) fn decode(
    project: &ProjectState,
    disk_number: u32,
    capture_attempt: Option<u32>,
    profile_override: Option<GreaseweazleProfile>,
    json_output: bool,
) -> Result<CliResponse, String> {
    let profile_override = match profile_override {
        Some(profile) => Some(profile),
        None => crate::flux_recovery::completed_profile(project, disk_number)?
            .as_deref()
            .map(GreaseweazleProfile::parse)
            .transpose()?,
    };
    let capture_attempt = match capture_attempt {
        Some(attempt) => attempt,
        None => flux_capture::latest_capture_attempt(project, disk_number)?,
    };
    let settings = external_tools::load_settings()?;
    let audit_path = project.logs_dir().join("external-tools.jsonl");
    let executable =
        external_tools::find_offline_greaseweazle(settings.path(ToolKind::Greaseweazle))?;
    let mut backend = ProcessGreaseweazleBackend::new_offline(executable, audit_path)?;
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
                "gw_reported_found_sectors": result.reported_sectors.map(|(found, _)| found),
                "gw_reported_total_sectors": result.reported_sectors.map(|(_, total)| total),
                "gw_reported_bad_lbas": result.gw_bad_lbas,
                "sector_quality": "unverified",
                "physical_media_access": false
            })
            .to_string()
        } else {
            format!(
                "Offline decode {:03} capture #{:03}, decode #{:03}: {} bytes\nImage: {}\nSHA-256: {}\nGreaseweazle reported sectors: {}\nConservative bad-LBA map: {}\nSector quality is not yet verified; this image is not automatically promoted for extraction.",
                result.disk_number,
                result.capture_attempt,
                result.decode_attempt,
                result.bytes,
                result.image_path.display(),
                result.sha256,
                result
                    .reported_sectors
                    .map(|(found, total)| format!("{found}/{total}"))
                    .unwrap_or_else(|| "unavailable".to_owned()),
                result
                    .gw_bad_lbas
                    .as_ref()
                    .map(|bad| format!("{} missing sector(s)", bad.len()))
                    .unwrap_or_else(|| "unavailable/incomplete grid".to_owned())
            )
        },
        exit_code: 3,
    })
}

pub(super) fn status(
    project: &ProjectState,
    disk_number: u32,
    json_output: bool,
) -> Result<CliResponse, String> {
    let status = flux_capture::inspect_disk(project, disk_number)?;
    let output = if json_output {
        json!({
            "disk": status.disk_number,
            "captures": status.captures,
            "decodes": status.decodes,
            "evidence_healthy": status.evidence_healthy,
            "preferred_profile": status.preferred_profile,
            "attention_required": status.attention_required,
            "physical_media_access": false
        })
        .to_string()
    } else {
        let mut lines = vec![format!(
            "Greaseweazle disk {disk_number:03}: evidence {}, sector quality not yet certified",
            if status.evidence_healthy {
                "intact"
            } else {
                "NEEDS ATTENTION"
            }
        )];
        for capture in &status.captures {
            lines.push(format!(
                "  Raw #{:03}: {} | hash {} | {}",
                capture.attempt,
                capture.status,
                if capture.hash_matches {
                    "OK"
                } else {
                    "MISSING/CHANGED"
                },
                capture.metadata.display()
            ));
        }
        for decode in &status.decodes {
            lines.push(format!(
                "  Decode of raw #{:03}, pass #{:03} ({}): {}, output hash {}, source hash {}, gw sectors {}, bad-LBA map {}{}",
                decode.capture_attempt,
                decode.decode_attempt,
                decode.profile,
                decode.sector_quality,
                if decode.output_hash_matches { "OK" } else { "MISSING/CHANGED" },
                if decode.source_hash_matches { "OK" } else { "MISSING/CHANGED" },
                match (decode.gw_reported_found_sectors, decode.gw_reported_total_sectors) {
                    (Some(found), Some(total)) => format!("{found}/{total}"),
                    _ => "unavailable".to_owned(),
                },
                decode
                    .gw_bad_lbas
                    .as_ref()
                    .map(|bad| format!("{} missing", bad.len()))
                    .unwrap_or_else(|| "unknown".to_owned()),
                decode
                    .detail
                    .as_ref()
                    .map(|detail| format!("; {detail}"))
                    .unwrap_or_default()
            ));
        }
        lines.join("\n")
    };
    Ok(CliResponse {
        output,
        exit_code: 3,
    })
}

pub(super) fn compare(
    project: &ProjectState,
    disk_number: u32,
    json_output: bool,
) -> Result<CliResponse, String> {
    let comparison = flux_capture::compare_with_usb(project, disk_number)?;
    let attention = !comparison.unresolved_lbas.is_empty()
        || !comparison.conflicting_good_lbas.is_empty()
        || !comparison.candidate_flux_donor_lbas.is_empty();
    let output = if json_output {
        serde_json::to_string(&comparison).map_err(|error| error.to_string())?
    } else {
        format!(
            "Disk {disk_number:03}: USB attempt #{:03} vs Greaseweazle capture #{:03}/decode #{:03}\nMatching sectors reported good by both: {}\nUSB-bad, Greaseweazle-reported good (donor candidates only): {:?}\nUSB-good, Greaseweazle-reported bad: {:?}\nBad in both: {:?}\nConflicting bytes in sectors reported good by both: {:?}\nNo files or media changed. Greaseweazle's sector map is vendor-reported evidence; donor bytes are not yet certified or merged.",
            comparison.usb_attempt,
            comparison.capture_attempt,
            comparison.decode_attempt,
            comparison.matching_good_sectors,
            comparison.candidate_flux_donor_lbas,
            comparison.usb_only_good_lbas,
            comparison.unresolved_lbas,
            comparison.conflicting_good_lbas
        )
    };
    Ok(CliResponse {
        output,
        exit_code: if attention { 3 } else { 0 },
    })
}

pub(super) fn consensus(
    project: &ProjectState,
    disk_number: u32,
    json_output: bool,
) -> Result<CliResponse, String> {
    let result = flux_capture::compare_flux_captures(project, disk_number)?;
    let attention = !result.conflicting_reported_good_lbas.is_empty()
        || !result.both_reported_bad_lbas.is_empty()
        || !result.older_only_reported_good_lbas.is_empty()
        || !result.newer_only_reported_good_lbas.is_empty();
    let output = if json_output {
        serde_json::to_string(&result).map_err(|error| error.to_string())?
    } else {
        format!(
            "Disk {disk_number:03}: raw captures #{:03} and #{:03} (distinct physical reads)\nMatching bytes in sectors Greaseweazle reported good on both: {}\nConflicting good-sector LBAs: {:?}\nGood only on older capture: {:?}\nGood only on newer capture: {:?}\nBad in both: {:?}\nNo images were merged or certified; no floppy drive was accessed.",
            result.older_capture_attempt,
            result.newer_capture_attempt,
            result.matching_reported_good_lbas.len(),
            result.conflicting_reported_good_lbas,
            result.older_only_reported_good_lbas,
            result.newer_only_reported_good_lbas,
            result.both_reported_bad_lbas
        )
    };
    Ok(CliResponse {
        output,
        exit_code: if attention { 3 } else { 0 },
    })
}

pub(super) fn plan(
    project: &ProjectState,
    disk_number: u32,
    json_output: bool,
) -> Result<CliResponse, String> {
    let plan = flux_capture::plan_flux_recovery(project, disk_number)?;
    let output = if json_output {
        serde_json::to_string(&plan).map_err(|error| error.to_string())?
    } else {
        format!(
            "Disk {disk_number:03}: USB #{:03}, raw captures #{:03}/#{:03}\nMatching USB/dual-flux control sectors: {}\nCorroborated donor candidates (two raw captures agree): {:?}\nOnly one flux capture reported good: {:?}\nStill unresolved: {:?}\nUSB versus flux byte conflicts: {:?}\nFlux versus flux byte conflicts: {:?}\nNo image was changed or promoted; these are evidence-ranked candidates, not certified recovery.",
            plan.usb_attempt,
            plan.older_capture_attempt,
            plan.newer_capture_attempt,
            plan.matching_control_sectors,
            plan.corroborated_donor_lbas,
            plan.single_flux_read_lbas,
            plan.unresolved_lbas,
            plan.usb_flux_conflict_lbas,
            plan.flux_flux_conflict_lbas
        )
    };
    Ok(CliResponse {
        output,
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
    use crate::greaseweazle::MockGreaseweazleBackend;
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

    #[test]
    fn unknown_board_preflight_never_issues_a_read_command() {
        let mut backend = MockGreaseweazleBackend::default();
        assert!(
            verify_device_for_capture(&mut backend)
                .unwrap_err()
                .contains("could not be verified")
        );
        assert_eq!(backend.commands().len(), 1);
        assert_eq!(backend.commands()[0].arguments(), &["info".to_owned()]);
    }
}
