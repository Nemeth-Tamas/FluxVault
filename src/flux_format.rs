//! Conservative IBM format selection from immutable whole-disk captures.
//! No physical read is performed here. Other geometries remain unsupported.

use crate::{
    flux_capture,
    greaseweazle::{GreaseweazleBackend, GreaseweazleProfile},
    project::ProjectState,
};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub profile: String,
    pub decode_attempt: Option<u32>,
    pub image_sha256: Option<String>,
    pub good_sectors: usize,
    pub total_sectors: usize,
    pub boot_geometry_matches: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FormatDecision {
    pub schema_version: u32,
    pub disk: u32,
    pub capture_attempt: u32,
    pub source_sha256: String,
    pub selected_profile: Option<String>,
    pub reason: String,
    pub candidates: Vec<Candidate>,
    pub physical_media_access: bool,
}

/// Require coherent DOS geometry, not an arbitrary string or output image size.
fn boot_matches(bytes: &[u8], profile: GreaseweazleProfile, boot_readable: bool) -> bool {
    if !boot_readable || bytes.len() < 512 {
        return false;
    }
    let word = |offset| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
    let total = if word(19) != 0 {
        word(19)
    } else {
        u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize
    };
    matches!(bytes[0], 0xeb | 0xe9)
        && word(11) == 512
        && matches!(bytes[13], 1 | 2 | 4 | 8)
        && word(14) > 0
        && matches!(bytes[16], 1 | 2)
        && word(17) > 0
        && word(22) > 0
        && word(26) == 2
        && word(24) == profile.expected_sector_image_bytes() as usize / 512 / 160
        && total == profile.expected_sector_image_bytes() as usize / 512
        && bytes[510..512] == [0x55, 0xaa]
}

fn select(candidates: &[Candidate]) -> (Option<String>, String) {
    let coherent: Vec<_> = candidates
        .iter()
        .filter(|c| c.error.is_none() && c.boot_geometry_matches && c.good_sectors >= 32)
        .collect();
    if coherent.len() == 1 {
        return (
            Some(coherent[0].profile.clone()),
            "readable_boot_geometry_and_consistent_sector_map".to_owned(),
        );
    }
    if coherent.len() > 1 {
        return (None, "conflicting_readable_boot_geometries".to_owned());
    }
    let strong: Vec<_> = candidates
        .iter()
        .filter(|c| c.error.is_none() && c.good_sectors * 100 >= c.total_sectors * 80)
        .collect();
    if strong.len() == 1
        && candidates
            .iter()
            .filter(|c| c.profile != strong[0].profile)
            .all(|c| c.error.is_none() && c.good_sectors * 100 <= c.total_sectors * 5)
    {
        return (
            Some(strong[0].profile.clone()),
            "dominant_sector_map_without_readable_boot_geometry".to_owned(),
        );
    }
    (None, "ambiguous_or_insufficient_format_evidence".to_owned())
}

pub fn identify(
    project: &ProjectState,
    disk: u32,
    capture_attempt: u32,
    backend: &mut impl GreaseweazleBackend,
    progress: &impl Fn(&str),
) -> Result<(FormatDecision, PathBuf), String> {
    let inspected = flux_capture::inspect_disk(project, disk)?;
    let capture = inspected
        .captures
        .iter()
        .find(|c| c.attempt == capture_attempt && c.status == "complete" && c.hash_matches)
        .ok_or("Format identification requires a hash-verified complete capture")?;
    let source_sha256 = capture.sha256.clone().ok_or("Capture hash is missing")?;
    if capture
        .capture_settings
        .as_ref()
        .is_some_and(|s| s.cylinders.is_some())
    {
        return Err(
            "Format discovery requires a whole-disk capture, not targeted tracks".to_owned(),
        );
    }
    let mut candidates = Vec::new();
    for profile in [GreaseweazleProfile::Ibm1440, GreaseweazleProfile::Ibm720] {
        progress(&format!(
            "Identifying disk {disk:03}: offline {} decode (no extra physical read)",
            profile.argument()
        ));
        backend.set_operation_timeout(Duration::from_secs(60));
        let outcome = (|| {
            let status = flux_capture::inspect_disk(project, disk)?;
            let existing = status.decodes.iter().rev().find(|d| {
                d.capture_attempt == capture_attempt
                    && d.profile == profile.argument()
                    && d.sector_quality != "failed"
                    && d.sector_quality != "started"
            });
            let d = match existing {
                Some(d) => d.clone(),
                None => {
                    let result = flux_capture::decode(
                        project,
                        disk,
                        capture_attempt,
                        Some(profile),
                        backend,
                    )?;
                    flux_capture::inspect_disk(project, disk)?
                        .decodes
                        .into_iter()
                        .find(|d| {
                            d.capture_attempt == capture_attempt
                                && d.profile == profile.argument()
                                && d.decode_attempt == result.decode_attempt
                        })
                        .ok_or("New decode is missing")?
                }
            };
            if d.capture_settings
                .as_ref()
                .is_some_and(|s| s.cylinders.is_some())
            {
                return Err(
                    "Format discovery requires a whole-disk capture, not targeted tracks"
                        .to_owned(),
                );
            }
            let total = profile.expected_sector_image_bytes() as usize / 512;
            let (bytes, bad) = flux_capture::verified_decode(project, &d, total)?;
            Ok(Candidate {
                profile: profile.argument().to_owned(),
                decode_attempt: Some(d.decode_attempt),
                image_sha256: Some(d.output_sha256),
                good_sectors: total - bad.len(),
                total_sectors: total,
                boot_geometry_matches: boot_matches(&bytes, profile, !bad.contains(&0)),
                error: None,
            })
        })();
        let candidate = outcome.unwrap_or_else(|error| Candidate {
            profile: profile.argument().to_owned(),
            decode_attempt: None,
            image_sha256: None,
            good_sectors: 0,
            total_sectors: profile.expected_sector_image_bytes() as usize / 512,
            boot_geometry_matches: false,
            error: Some(error),
        });
        // A coherent almost-complete HD image needs no second decoder invocation.
        let confident_hd = candidate.error.is_none()
            && candidate.boot_geometry_matches
            && candidate.good_sectors * 100 >= candidate.total_sectors * 98;
        candidates.push(candidate);
        if confident_hd {
            break;
        }
    }
    let (selected_profile, reason) = select(&candidates);
    let decision = FormatDecision {
        schema_version: 1,
        disk,
        capture_attempt,
        source_sha256,
        selected_profile,
        reason,
        candidates,
        physical_media_access: false,
    };
    let flux = flux_capture::project_flux_dir(project)?;
    let directory = flux.join("Formats");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if directory.parent() != Some(flux.as_path()) {
        return Err("Format report directory escapes Flux".to_owned());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = directory.join(format!(
        "{disk:03}_capture_{capture_attempt:03}_{nonce}_{}.json",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&decision).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    Ok((decision, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(profile: &str, good: usize, total: usize, boot: bool) -> Candidate {
        Candidate {
            profile: profile.to_owned(),
            decode_attempt: Some(1),
            image_sha256: Some("hash".to_owned()),
            good_sectors: good,
            total_sectors: total,
            boot_geometry_matches: boot,
            error: None,
        }
    }
    #[test]
    fn selection_refuses_ambiguity_and_severe_unknown_media() {
        assert_eq!(
            select(&[c("ibm.1440", 2880, 2880, true)]).0.as_deref(),
            Some("ibm.1440")
        );
        assert_eq!(
            select(&[
                c("ibm.1440", 0, 2880, false),
                c("ibm.720", 1439, 1440, false)
            ])
            .0
            .as_deref(),
            Some("ibm.720")
        );
        assert!(
            select(&[
                c("ibm.1440", 2880, 2880, true),
                c("ibm.720", 1440, 1440, true)
            ])
            .0
            .is_none()
        );
        assert!(
            select(&[
                c("ibm.1440", 2600, 2880, false),
                c("ibm.720", 1400, 1440, false)
            ])
            .0
            .is_none()
        );
        assert!(
            select(&[
                c("ibm.1440", 100, 2880, false),
                c("ibm.720", 0, 1440, false)
            ])
            .0
            .is_none()
        );
        let mut failed = c("ibm.720", 0, 1440, false);
        failed.error = Some("timeout".to_owned());
        assert!(
            select(&[c("ibm.1440", 2600, 2880, false), failed])
                .0
                .is_none()
        );
    }
    #[test]
    fn boot_geometry_must_be_readable_and_match_physical_profile() {
        let mut b = [0u8; 512];
        b[0] = 0xeb;
        b[11..13].copy_from_slice(&512u16.to_le_bytes());
        b[13] = 2;
        b[14] = 1;
        b[16] = 2;
        b[17] = 112;
        b[19..21].copy_from_slice(&1440u16.to_le_bytes());
        b[22] = 3;
        b[24] = 9;
        b[26] = 2;
        b[510] = 0x55;
        b[511] = 0xaa;
        assert!(boot_matches(&b, GreaseweazleProfile::Ibm720, true));
        assert!(!boot_matches(&b, GreaseweazleProfile::Ibm1440, true));
        assert!(!boot_matches(&b, GreaseweazleProfile::Ibm720, false));
        b[510] = 0;
        assert!(!boot_matches(&b, GreaseweazleProfile::Ibm720, true));
    }
}
