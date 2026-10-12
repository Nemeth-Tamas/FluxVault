//! Saved-only independent CRC gate for committed GW sector images. A report is
//! not a certification of physical disk identity, filesystem or file semantics.
use crate::{flux_capture, project::ProjectState};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
};

pub fn inspect(project: &ProjectState, disk: u32) -> Result<Value, String> {
    crate::processing::validate_workspace(project)?;
    let _owner = crate::project_work::reserve(project.root())?;
    inspect_owned(project, disk)
}
pub(crate) fn inspect_owned(project: &ProjectState, disk: u32) -> Result<Value, String> {
    if disk == 0 {
        return Err("CRC verification requires a positive disk number".into());
    }
    let largest = crate::flux_diagnostics::bounded_inventory(project, disk)?;
    let _budget = crate::resource_budget::background(
        crate::resource_budget::Kind::Recovery,
        256 * crate::resource_budget::MIB,
        &[(
            project.root(),
            largest.saturating_add(16 * crate::resource_budget::MIB),
        )],
        &|s| eprintln!("{s}"),
    )?;
    let binding = crate::flux_recovery::diagnostic_binding(project, disk)?
        .ok_or("No committed GW recovery journal")?;
    let profile = binding["profile"]
        .as_str()
        .ok_or("Missing bound GW profile")?;
    let cell = match profile {
        "ibm.1440" => 1000,
        "ibm.720" => 2000,
        _ => return Err("Independent CRC gate supports IBM HD/DD only".into()),
    };
    let result = &binding["result"];
    if !result["format_exception"].is_null() || !binding["sectors"].is_array() {
        return Err("Raw-only/uncommitted recovery has no sector CRC gate".into());
    }
    let path = std::path::PathBuf::from(
        result["image"]
            .as_str()
            .ok_or("Missing committed GW image")?,
    );
    crate::sector_inspection::regular(&path, false)?;
    let count = if cell == 1000 { 2880 } else { 1440 };
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() != count as u64 * 512 {
        return Err("Committed GW image extent changed".into());
    }
    let image = fs::read(&path).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(&image));
    if result["image_sha256"] != hash {
        return Err("Committed GW image hash changed".into());
    }
    let status = flux_capture::inspect_disk(project, disk)?;
    let selected: BTreeSet<u32> = binding["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            s["capture_attempt"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Invalid stage capture identity")
        })
        .collect::<Result<_, _>>()?;
    let mut payloads = BTreeMap::<(u32, usize), BTreeSet<String>>::new();
    let mut raw = Vec::new();
    let mut remaining = 2 * 1024 * 1024 * 1024u64;
    for attempt in &selected {
        let capture = status
            .captures
            .iter()
            .find(|c| c.attempt == *attempt && c.hash_matches)
            .ok_or("Missing/unverified selected CRC source")?;
        let bytes = capture.bytes.ok_or("Missing CRC capture size")?;
        if bytes > remaining {
            return Err("CRC gate raw input budget exceeded; no partial certification".into());
        }
        remaining -= bytes;
        let logical = capture.raw_flux.as_ref().ok_or("Missing CRC source")?;
        let digest = capture
            .sha256
            .as_deref()
            .ok_or("Missing CRC source SHA-256")?;
        let source = crate::flux_archive::open_source(logical, bytes, digest)?;
        let mut file = File::open(&source.path).map_err(|e| e.to_string())?;
        let inspection = crate::mfm_evidence::inspect(&mut file, bytes, cell)?;
        for track in &inspection.tracks {
            for row in &track.observations {
                if row.id_crc_valid
                    && row.data_crc_valid == Some(true)
                    && row.clock_violations == 0
                    && !row.deleted_data_mark
                    && usize::from(row.id[0]) == track.cylinder
                    && usize::from(row.id[1]) == track.head
                    && row.id[3] == 2
                    && row.id[2] > 0
                    && row.id[2] as usize <= count / 160
                {
                    let lba = (track.cylinder * 2 + track.head) * (count / 160)
                        + usize::from(row.id[2])
                        - 1;
                    if lba < count {
                        payloads
                            .entry((*attempt, lba))
                            .or_default()
                            .insert(row.payload_sha256.clone().unwrap());
                    }
                }
            }
        }
        raw.push(json!({"capture_attempt":attempt,"source_sha256":digest,"source_bytes":bytes,
            "inspection_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&inspection).map_err(|e|e.to_string())?)),
            "tracks":inspection.tracks.len(),"engine":inspection.engine}));
    }
    let mut validated = Vec::new();
    let mut unconfirmed = Vec::new();
    let mut conflicts = Vec::new();
    let mut unresolved = Vec::new();
    let mut origins = Vec::new();
    for row in binding["sectors"].as_array().unwrap() {
        let lba = row["lba"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < count)
            .ok_or("Invalid committed CRC LBA")?;
        if ["unreadable", "conflict"].contains(&row["confidence"].as_str().unwrap_or("")) {
            unresolved.push(lba);
            continue;
        }
        let digest = format!("{:x}", Sha256::digest(&image[lba * 512..(lba + 1) * 512]));
        let claimed = row["capture_attempts"]
            .as_array()
            .ok_or("Missing sector capture origins")?;
        let mut matching = Vec::new();
        let mut alternatives = BTreeSet::new();
        for attempt in &selected {
            if let Some(hashes) = payloads.get(&(*attempt, lba)) {
                alternatives.extend(hashes.iter().cloned());
                if hashes.contains(&digest) && claimed.contains(&json!(attempt)) {
                    matching.push(*attempt);
                }
            }
        }
        if alternatives.iter().any(|h| h != &digest) {
            conflicts.push(lba);
        } else if matching.is_empty() {
            unconfirmed.push(lba);
        } else {
            validated.push(lba);
            origins.push(json!({"lba":lba,"payload_sha256":digest,"crc_valid_supporting_capture_attempts":matching}));
        }
    }
    if validated.len() + unconfirmed.len() + conflicts.len() + unresolved.len() != count {
        return Err("Committed CRC map is not complete".into());
    }
    // Under the writer reservation, revalidate the raw/packed identities and
    // sealed journal/derived bytes; stale proof is never returned as a pass.
    for capture in status
        .captures
        .iter()
        .filter(|c| selected.contains(&c.attempt))
    {
        crate::flux_archive::verify(
            capture.raw_flux.as_ref().unwrap(),
            capture.bytes.unwrap(),
            capture.sha256.as_ref().unwrap(),
        )?;
    }
    if crate::flux_recovery::diagnostic_binding(project, disk)?.as_ref() != Some(&binding)
        || crate::conversion::sha256_file(&path)? != hash
    {
        return Err("CRC source binding changed during inspection".into());
    }
    let eligible = unconfirmed.is_empty() && conflicts.is_empty();
    Ok(
        json!({"schema_version":1,"disk":disk,"profile":profile,"engine":"fluxvault-ibm-mfm-crc-v1",
        "image":path,"image_sha256":hash,"journal_sha256":binding["journal_sha256"],"raw_sources":raw,
        "validated_lbas":validated,"unconfirmed_lbas":unconfirmed,"conflicting_crc_valid_lbas":conflicts,
        "unresolved_lbas":unresolved,"sector_origins":origins,
        "all_reported_good_bytes_independently_validated":eligible,
        "complete_sector_image":eligible && unresolved.is_empty(),"attention_required":!eligible || !unresolved.is_empty(),
        "physical_media_access":false,"host_tools_invoked":false,"project_modified":false,
        "note":"Independent ID/data CRC and ordinary MFM clock proof, bound to exact immutable raw hashes and committed sector origins. Missing/conflicting bytes remain placeholders. A pass is not proof of physical label identity, file semantics, magnetic weakness or complete customer recovery; USB-only bytes have no flux CRC proof. No filtering, guessing, winner selection or automatic promotion."}),
    )
}
pub fn render(report: &Value) -> String {
    if report["scope"] == "saved_batch_crc" {
        return format!(
            "Saved batch independent IBM MFM gate\n{} disks checked; {} CRC byte passes; {} partial passes; {} unconfirmed/conflicting/refused; {} without committed GW evidence.\nSaved media only; original captures, selection and delivery unchanged.",
            report["disk_count"],
            report["complete_passes"],
            report["partial_passes"],
            report["attention_or_refused"],
            report["not_applicable"]
        );
    }
    format!(
        "Disk {:03} / independent saved IBM MFM verification\n{} validated sectors; {} unconfirmed; {} conflicting; {} unresolved.\nReported-good byte gate: {}. Complete sector image: {}.\n{}",
        report["disk"].as_u64().unwrap_or(0),
        report["validated_lbas"].as_array().map_or(0, Vec::len),
        report["unconfirmed_lbas"].as_array().map_or(0, Vec::len),
        report["conflicting_crc_valid_lbas"]
            .as_array()
            .map_or(0, Vec::len),
        report["unresolved_lbas"].as_array().map_or(0, Vec::len),
        report["all_reported_good_bytes_independently_validated"],
        report["complete_sector_image"],
        report["note"].as_str().unwrap_or("")
    )
}
pub fn inspect_all(project: &ProjectState, stage: impl Fn(u32)) -> Result<Value, String> {
    crate::processing::validate_workspace(project)?;
    let _owner = crate::project_work::reserve(project.root())?;
    let stats = crate::imaging::load_project_statistics(&project.images_dir())?;
    if stats.disk_count > 4096 {
        return Err("CRC batch exceeds supported label bound".into());
    }
    let mut rows = Vec::new();
    let (mut complete, mut partial, mut attention, mut absent) = (0, 0, 0, 0);
    for disk in stats.disks {
        crate::cancellation::check()?;
        let journal = project
            .root()
            .join(format!("Flux/Recovery/{:03}_job.json", disk.disk_number));
        if !journal.try_exists().map_err(|e| e.to_string())? {
            absent += 1;
            rows.push(json!({"disk":disk.disk_number,"status":"not_applicable","reason":"no_committed_GW_journal_USB_bytes_have_no_flux_CRC_proof"}));
            continue;
        }
        stage(disk.disk_number);
        match inspect_owned(project, disk.disk_number) {
            Ok(v) => {
                if v["complete_sector_image"] == true {
                    complete += 1;
                } else if v["all_reported_good_bytes_independently_validated"] == true {
                    partial += 1;
                } else {
                    attention += 1;
                }
                rows.push(v);
            }
            Err(e) => {
                attention += 1;
                rows.push(json!({"disk":disk.disk_number,"status":"refused","reason":e}));
            }
        }
    }
    Ok(
        json!({"schema_version":1,"scope":"saved_batch_crc","disk_count":rows.len(),"complete_passes":complete,"partial_passes":partial,"attention_or_refused":attention,"not_applicable":absent,
        "attention_required":partial>0||attention>0,"physical_media_access":false,"project_modified":false,"host_tools_invoked":false,"disks":rows,
        "note":"Full/partial CRC byte gates are not customer completeness or semantic recovery rates. Not-applicable USB-only rows are not failed CRCs. Refusals/unconfirmed evidence are never silently counted as passes."}),
    )
}
#[cfg(test)]
#[path = "flux_crc_tests.rs"]
mod tests;
