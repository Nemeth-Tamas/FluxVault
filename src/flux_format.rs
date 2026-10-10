//! Conservative IBM format selection from immutable whole-disk captures.
//! No physical read is performed here. Other geometries remain unsupported.

use crate::{
    flux_capture,
    greaseweazle::{GreaseweazleBackend, GreaseweazleProfile},
    project::ProjectState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    io::Write,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub profile: String,
    pub decode_attempt: Option<u32>,
    pub image_sha256: Option<String>,
    pub good_sectors: usize,
    pub total_sectors: usize,
    pub boot_geometry_matches: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FormatDecision {
    pub schema_version: u32,
    pub disk: u32,
    pub capture_attempt: u32,
    pub source_sha256: String,
    pub selected_profile: Option<String>,
    pub reason: String,
    pub candidates: Vec<Candidate>,
    pub physical_media_access: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_profile_hint: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_profile_hint_error: Option<String>,
}

// This is only an ordering hint, never a format/CRC certificate. Flux/BPB
// validation and competing interpretations still make the final decision.
fn decode_priority(
    project: &ProjectState,
    disk: u32,
) -> Result<([GreaseweazleProfile; 2], Option<serde_json::Value>), String> {
    crate::processing::validate_workspace(project)?;
    let all = crate::imaging::load_attempts_for_disk(&project.images_dir(), disk)?;
    let mut usb = Vec::new();
    for a in all {
        if !matches!(a.status.as_str(), "OK" | "PARTIAL") || !matches!(a.total_sectors, 1440 | 2880)
        {
            continue;
        }
        if !a.legacy_image {
            let metadata = bounded_read(&a.metadata_path, 1024 * 1024)?;
            let value: serde_json::Value =
                serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
            if value["source_backend"] != "windows-raw-sector" {
                continue;
            }
        }
        usb.push(a);
    }
    let default = [GreaseweazleProfile::Ibm1440, GreaseweazleProfile::Ibm720];
    let Some(a) = crate::imaging::best_attempt(&usb) else {
        return Ok((default, None));
    };
    let path = crate::recovery_plan::resolve_image_path(&project.images_dir(), &a.image_file)?;
    crate::safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() != a.total_sectors as u64 * 512 {
        return Err("USB geometry hint image extent changed".into());
    }
    let bytes = bounded_read(&path, a.total_sectors as u64 * 512)?;
    if format!("{:x}", Sha256::digest(&bytes)) != a.sha256 {
        return Err("USB geometry hint image hash changed".into());
    }
    crate::fat12_recovery::validate_sector_evidence(a, a.total_sectors, &a.sha256)?;
    let order = if a.total_sectors == 1440 {
        [default[1], default[0]]
    } else {
        default
    };
    Ok((
        order,
        Some(
            serde_json::json!({"attempt":a.attempt_number,"image_sha256":a.sha256,"total_sectors":a.total_sectors,"first_profile":order[0].argument(),"scope":"Verified saved USB size/map guides decode order only; flux/BPB decides format"}),
        ),
    ))
}

fn bounded_read(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, String> {
    crate::cancellation::check()?;
    crate::safety::workstation_path(path)?;
    crate::safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)?;
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > limit {
        return Err("USB hint requires bounded regular evidence".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("USB hint refuses reparse evidence".into());
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("USB hint evidence grew beyond bounds".into());
    }
    crate::cancellation::check()?;
    Ok(bytes)
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
    let (profiles, initial_profile_hint, initial_profile_hint_error) =
        match decode_priority(project, disk) {
            Ok((order, hint)) => (order, hint, None),
            Err(error) => (
                [GreaseweazleProfile::Ibm1440, GreaseweazleProfile::Ibm720],
                None,
                Some(error),
            ),
        };
    for profile in profiles {
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
        // A coherent almost-complete first profile needs no second invocation.
        let confident_first = candidate.error.is_none()
            && candidate.boot_geometry_matches
            && candidate.good_sectors * 100 >= candidate.total_sectors * 98;
        candidates.push(candidate);
        if confident_first {
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
        initial_profile_hint,
        initial_profile_hint_error,
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
    fn usb_fixture(spt: usize) -> ProjectState {
        let p = ProjectState::create_without_session(std::env::temp_dir().join(format!(
                "fv-format-hint-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
        .unwrap();
        let count = 160 * spt;
        let bytes = vec![0x42; count * 512];
        let sha = format!("{:x}", Sha256::digest(&bytes));
        let log = p.logs_dir().join("001_attempt_001.log");
        fs::write(p.images_dir().join("001_attempt_001.img"), bytes).unwrap();
        fs::write(&log,format!("BEGIN | disk=1 | attempt=1\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track={spt} | bytes_per_sector=512 | total_sectors={count} | total_bytes={}\nEND | status=OK | bytes={} | sha256={sha}\n",count*512,count*512)).unwrap();
        let metadata = serde_json::json!({"fluxvault_version":"fixture","status":"OK","disk_number":1,"attempt_number":1,"source_backend":"windows-raw-sector","source_device":"fixture","image_file":"001_attempt_001.img","log_file":log,"timestamp_unix_ms":1,"geometry":{"cylinders":80,"heads":2,"sectors_per_track":spt,"bytes_per_sector":512,"total_bytes":count*512,"format_guess":"fixture"},"sector_retries":0,"total_sectors":count,"bytes_written":count*512,"retry_recovered_sectors":0,"bad_sector_count":0,"bad_sectors":[],"sha256":sha});
        fs::write(
            p.images_dir().join("001_attempt_001.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        p
    }
    #[test]
    fn verified_usb_size_guides_first_decode_but_not_final_format() {
        for (spt, expected) in [
            (9, GreaseweazleProfile::Ibm720),
            (18, GreaseweazleProfile::Ibm1440),
        ] {
            let p = usb_fixture(spt);
            let (order, hint) = decode_priority(&p, 1).unwrap();
            assert_eq!(order[0], expected);
            assert_ne!(order[0], order[1]);
            assert_eq!(hint.unwrap()["first_profile"], expected.argument());
            assert_eq!(
                select(&[
                    c(order[0].argument(), 0, 1440, false),
                    c(order[1].argument(), 2880, 2880, true)
                ])
                .0
                .as_deref(),
                Some(order[1].argument())
            );
            fs::write(p.images_dir().join("001_attempt_001.img"), b"changed").unwrap();
            assert!(decode_priority(&p, 1).is_err());
        }
    }
    #[test]
    fn absent_usb_uses_normal_order_and_old_decisions_remain_compatible() {
        let p = usb_fixture(9);
        let (order, hint) = decode_priority(&p, 2).unwrap();
        assert_eq!(order[0], GreaseweazleProfile::Ibm1440);
        assert!(hint.is_none());
        let old = serde_json::json!({"schema_version":1,"disk":1,"capture_attempt":1,"source_sha256":"hash","selected_profile":null,"reason":"unknown","candidates":[],"physical_media_access":false});
        let decision: FormatDecision = serde_json::from_value(old).unwrap();
        assert!(decision.initial_profile_hint.is_none());
    }
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
