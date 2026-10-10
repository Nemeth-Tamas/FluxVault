//! Explicit same-label confirmation of MINOR cross-reader disagreements.
//! This retains observations, not repaired bytes or independent CRC proof.
use crate::project::ProjectState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Read, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Conflict {
    pub lba: u64,
    pub usb_sha256: String,
    pub gw_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Confirmation {
    pub schema: u32,
    pub disk: u32,
    pub usb_attempt: u32,
    pub ticket_generation: u64,
    pub confirmed_unix_ms: u64,
    pub assertion: String,
    pub usb_image_sha256: String,
    pub usb_metadata_sha256: String,
    pub usb_log_sha256: String,
    pub gw_image_sha256: String,
    pub gw_bad_lbas: Vec<u64>,
    pub agreeing_sectors: usize,
    pub conflicts: Vec<Conflict>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let info = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if info.file_attributes() & 0x400 != 0 {
            return Err("Read-conflict evidence is redirected".into());
        }
    }
    if !info.is_file() || info.len() > limit as u64 {
        return Err("Unsafe read-conflict evidence".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Read-conflict evidence exceeds bound".into());
    }
    Ok(bytes)
}
fn map(lbas: &[u64], sectors: usize) -> Result<BTreeSet<u64>, String> {
    let result: BTreeSet<_> = lbas.iter().copied().collect();
    if result.len() != lbas.len() || result.iter().any(|n| *n >= sectors as u64) {
        return Err("Invalid conflict source map".into());
    }
    Ok(result)
}

pub(crate) fn build(
    project: &ProjectState,
    disk: u32,
    usb_attempt: u32,
    generation: u64,
    confirmed_unix_ms: u64,
    gw: &[u8],
    bad: &[u64],
) -> Result<Confirmation, String> {
    let name = format!("{disk:03}_attempt_{usb_attempt:03}.json");
    let meta_path = crate::recovery_plan::resolve_image_path(&project.images_dir(), &name)?;
    let metadata = read(&meta_path, 1048576)?;
    let value: serde_json::Value = serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
    if value["source_backend"] != "windows-raw-sector"
        || value["disk_number"] != disk
        || value["attempt_number"] != usb_attempt
        || disk == 0
        || generation == 0
    {
        return Err("Confirmation requires its original USB acquisition identity".into());
    }
    let image = crate::recovery_plan::resolve_image_path(
        &project.images_dir(),
        value["image_file"].as_str().ok_or("Missing USB image")?,
    )?;
    let usb = read(&image, crate::fat12::MAX_IMAGE_BYTES)?;
    if usb.len() != gw.len()
        || !usb.len().is_multiple_of(512)
        || usb.is_empty()
        || value["sha256"] != hash(&usb)
        || value["total_sectors"] != usb.len() / 512
    {
        return Err("Changed USB source or GW geometry; confirmation refused".into());
    }
    let sectors = usb.len() / 512;
    let usb_bad = value["bad_sectors"]
        .as_array()
        .ok_or("Missing USB map")?
        .iter()
        .map(|b| b["lba"].as_u64().ok_or("Invalid USB map"))
        .collect::<Result<Vec<_>, _>>()?;
    let usb_bad = map(&usb_bad, sectors)?;
    let gw_bad = map(bad, sectors)?;
    let mut agreeing = 0;
    let mut conflicts = Vec::new();
    for lba in 0..sectors as u64 {
        if usb_bad.contains(&lba) || gw_bad.contains(&lba) {
            continue;
        }
        let range = lba as usize * 512..(lba as usize + 1) * 512;
        if usb[range.clone()] == gw[range.clone()] {
            agreeing += 1;
        } else {
            conflicts.push(Conflict {
                lba,
                usb_sha256: hash(&usb[range.clone()]),
                gw_sha256: hash(&gw[range]),
            });
        }
    }
    // An explicit operator assertion is not permission to accept arbitrary
    // wrong-disk data. Keep broad disagreements and sparse overlap blocked.
    if conflicts.is_empty()
        || conflicts.len() > 32
        || agreeing < 128
        || conflicts.len() * 100 > agreeing + conflicts.len()
    {
        return Err(format!(
            "Same-disk confirmation requires minor disagreement: {agreeing} matching / {} conflicting sectors; at least 128 matches, at most 32 conflicts and 1% disagreement",
            conflicts.len()
        ));
    }
    let log_name = Path::new(value["log_file"].as_str().ok_or("Missing USB log")?)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid USB log")?;
    let log_path = crate::recovery_plan::resolve_image_path(&project.logs_dir(), log_name)?;
    let log = read(&log_path, 8 * 1048576)?;
    // Recheck the source snapshots before returning an approval.
    if read(&meta_path, 1048576)? != metadata
        || read(&image, crate::fat12::MAX_IMAGE_BYTES)? != usb
        || read(&log_path, 8 * 1048576)? != log
    {
        return Err("USB evidence changed during confirmation".into());
    }
    Ok(Confirmation {
        schema: 1,
        disk,
        usb_attempt,
        ticket_generation: generation,
        confirmed_unix_ms,
        assertion: "operator_confirms_same_physical_disk_not_content_certification".into(),
        usb_image_sha256: hash(&usb),
        usb_metadata_sha256: hash(&metadata),
        usb_log_sha256: hash(&log),
        gw_image_sha256: hash(gw),
        gw_bad_lbas: gw_bad.into_iter().collect(),
        agreeing_sectors: agreeing,
        conflicts,
    })
}

pub(crate) fn verify(
    project: &ProjectState,
    approval: &Confirmation,
    gw: &[u8],
    bad: &[u64],
) -> Result<(), String> {
    let replay = build(
        project,
        approval.disk,
        approval.usb_attempt,
        approval.ticket_generation,
        approval.confirmed_unix_ms,
        gw,
        bad,
    )?;
    if &replay != approval {
        return Err(
            "Same-disk confirmation changed or does not bind these source bytes/maps".into(),
        );
    }
    Ok(())
}

/// Cheap immutable publication binding for catalog loading: removing an
/// attention field must not turn a confirmed-conflict image into a clean one.
pub(crate) fn verify_metadata_binding(
    images: &Path,
    value: &serde_json::Value,
) -> Result<(), String> {
    let recorded: Option<Confirmation> = serde_json::from_value(
        value
            .get("read_conflicts")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|e| e.to_string())?;
    if value["source_backend"] != "greaseweazle-derived" {
        return if recorded.is_some() {
            Err("Conflict confirmation was relabelled as another backend".into())
        } else {
            Ok(())
        };
    }
    let Some(proof_path) = value["flux_provenance"].as_str() else {
        return if recorded.is_some() {
            Err("Conflict confirmation lacks immutable publication proof".into())
        } else {
            Ok(())
        };
    };
    let root = images
        .canonicalize()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("Missing Images parent")?
        .to_owned();
    let directory = root.join("Flux/Recovery");
    let proof_path = crate::recovery_plan::resolve_image_path(&directory, proof_path)?;
    let bytes = read(&proof_path, 4 * 1048576)?;
    if value["flux_provenance_sha256"] != hash(&bytes) {
        return Err("Immutable flux publication hash changed".into());
    }
    let proof: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let approval: Option<Confirmation> = serde_json::from_value(
        proof
            .get("read_conflicts")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|e| e.to_string())?;
    if approval != recorded {
        return Err("Conflict attention field differs from immutable publication".into());
    }
    if let Some(approval) = approval {
        if value["disk_number"] != approval.disk || value["sha256"] != approval.gw_image_sha256 {
            return Err("Read-conflict publication identity changed".into());
        }
        let project = ProjectState::open_without_session(root)?;
        let image = crate::recovery_plan::resolve_image_path(
            images,
            value["image_file"].as_str().ok_or("Missing GW image")?,
        )?;
        let gw = read(&image, crate::fat12::MAX_IMAGE_BYTES)?;
        let bad = value["bad_sectors"]
            .as_array()
            .ok_or("Missing GW map")?
            .iter()
            .map(|b| b["lba"].as_u64().ok_or("Invalid GW map"))
            .collect::<Result<Vec<_>, _>>()?;
        verify(&project, &approval, &gw, &bad)?;
    }
    Ok(())
}
