//! Bounded saved-evidence case review. No preferred image, source, extraction,
//! report generation or physical device is changed by this inspector.
use crate::{document_salvage, evidence_impact, fat12, project::ProjectState, sector_inspection};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const WARNING: &str = "Investigation of recorded evidence, not a byte winner, repaired original or completeness certificate. Missing/disputed input is excluded from text decoding. Legacy Word6 results map bytes only; they do not infer character encoding, formatting or meaning. Unknown layouts, allocation and ownership remain explicit. No physical media or external tool is accessed; no project files are written.";
const MAX_FILES: usize = 256;
const MAX_REPORT: usize = 64 * 1024 * 1024;
#[cfg(test)]
#[path = "recovery_investigation_tests.rs"]
mod tests;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn inspect(project: &ProjectState, disk: u32, attempt: Option<u32>) -> Result<Value, String> {
    let snapshot = sector_inspection::load_snapshot(project, disk, attempt)?;
    let impact = evidence_impact::inspect_snapshot(&snapshot, disk, None)?;
    let number = impact["attempt"]
        .as_u64()
        .ok_or("Missing inspected attempt")? as u32;
    let bad = snapshot.metadata()["bad_sectors"]
        .as_array()
        .ok_or("Missing sector map")?
        .iter()
        .map(|b| b["lba"].as_u64().ok_or("Invalid map"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let confirmation: Option<crate::read_conflicts::Confirmation> = serde_json::from_value(
        snapshot
            .metadata()
            .get("read_conflicts")
            .cloned()
            .unwrap_or(Value::Null),
    )
    .map_err(|e| e.to_string())?;
    let disputed = confirmation
        .as_ref()
        .map(|c| c.conflicts.iter().map(|c| c.lba).collect())
        .unwrap_or_default();
    let excluded = bad.union(&disputed).copied().collect::<BTreeSet<_>>();
    let analysis = if snapshot.map_known {
        fat12::analyze(&snapshot.bytes, &bad.iter().copied().collect::<Vec<_>>())
    } else {
        Err("Complete acquisition map unavailable".into())
    };
    let mut files = Vec::new();
    if let Ok(a) = &analysis {
        for (record, incomplete) in a
            .recovered_files
            .iter()
            .map(|r| (r, None))
            .chain(a.unrecovered_files.iter().map(|u| (&u.record, Some(u))))
        {
            let payload = record
                .data_lbas
                .iter()
                .filter(|l| excluded.contains(l))
                .copied()
                .collect::<BTreeSet<_>>();
            let metadata = record
                .metadata_lbas
                .iter()
                .filter(|l| excluded.contains(l))
                .copied()
                .collect::<BTreeSet<_>>();
            if incomplete.is_none() && payload.is_empty() && metadata.is_empty() {
                continue;
            }
            if files.len() >= MAX_FILES {
                return Err(
                    "Investigation exceeds 256 affected file records; no truncated clean result"
                        .into(),
                );
            }
            crate::cancellation::check()?;
            let safe = incomplete.is_none_or(|u| fat12::partial_file_safe(u, a));
            let name = record.path.to_ascii_lowercase();
            let header = record
                .data_lbas
                .first()
                .filter(|l| !excluded.contains(l))
                .and_then(|l| usize::try_from(*l).ok()?.checked_mul(512))
                .and_then(|at| snapshot.bytes.get(at..at + 8));
            let compound =
                header.is_some_and(|b| b == [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
            let rtf = header.is_some_and(|b| b.starts_with(b"{\\rtf1"));
            let mut word = Value::Null;
            let mut prefix = Value::Null;
            if safe && (compound || name.ends_with(".doc") || name.ends_with(".dot")) {
                word = document_salvage::inspect_record(&snapshot.bytes, record, &bad, &disputed);
            }
            if safe && (rtf || name.ends_with(".rtf")) {
                prefix = crate::rtf_salvage::inspect_record(&snapshot.bytes, record, &excluded);
            }
            let classification = if record.bytes == 0 {
                "zero_length_entry_no_payload_current_chain_refused"
            } else if record.path.replace('\\', "/").split('/').any(|p| {
                p.eq_ignore_ascii_case("System Volume Information")
                    || p.eq_ignore_ascii_case("$RECYCLE.BIN")
            }) {
                "windows_metadata_not_customer_payload"
            } else if !safe {
                "unsafe_or_ambiguous_parent_mapping"
            } else if word["word_text"]["state"] == "mapped_forensic_text" {
                if word["word_text"]["missing_positions"] == 0 {
                    "main_text_survives_original_container_still_incomplete"
                } else {
                    "partial_main_text_with_exact_gaps"
                }
            } else if prefix["state"] == "mapped_forensic_prefix" {
                "readable_rtf_prefix_unknown_remainder"
            } else if word["compound"]["legacy_word6_main_text_map"]["state"]
                == "mapped_bytes_not_decoded_text"
            {
                "legacy_word6_byte_map_not_decoded_or_repaired"
            } else if !word["compound"]["legacy_word6_main_text_map"].is_null() {
                "legacy_word6_byte_mapping_refused"
            } else if !payload.is_empty() {
                "mapped_raw_fragments_format_repair_unavailable"
            } else {
                "allocation_or_format_unresolved_raw_evidence_retained"
            };
            files.push(json!({"path":record.path,"directory_entry_offset":record.directory_entry_offset,"bytes":record.bytes,
                "classification":classification,"parent_mapping_eligible":safe,"observed_complete_payload_sha256":if record.sha256.is_empty(){None}else{Some(&record.sha256)},
                "incomplete_chain":incomplete.is_some(),"chain_reason":incomplete.map(|u|&u.reason),"unreadable_ranges":incomplete.map(|u|&u.unreadable_ranges),
                "unmapped_tail_bytes":incomplete.map_or(0,|u|u.unmapped_tail_bytes),"problem_payload_lbas":payload,"problem_metadata_lbas":metadata,
                "word":word,"rtf":prefix,"whole_original_repaired":false}));
        }
    }
    let disputes = if let Some(c) = &confirmation {
        crate::read_conflicts::verify(
            project,
            c,
            &snapshot.bytes,
            &bad.iter().copied().collect::<Vec<_>>(),
        )?;
        let usb = sector_inspection::load_snapshot(project, disk, Some(c.usb_attempt))?;
        if usb.metadata()["sha256"] != c.usb_image_sha256 {
            return Err("USB comparison image differs from confirmation".into());
        }
        let usb_bad = usb.metadata()["bad_sectors"]
            .as_array()
            .ok_or("Missing USB map")?
            .iter()
            .map(|b| b["lba"].as_u64().ok_or("Invalid USB LBA"))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut rows = Vec::new();
        for conflict in &c.conflicts {
            let at = conflict.lba as usize * 512;
            let first = &usb.bytes[at..at + 512];
            let second = &snapshot.bytes[at..at + 512];
            if hash(first) != conflict.usb_sha256 || hash(second) != conflict.gw_sha256 {
                return Err("Disputed sector bytes differ from their recorded versions".into());
            }
            let deltas = first
                .iter()
                .zip(second)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(offset, (a, b))| json!({"sector_offset":offset,"usb_byte":a,"gw_byte":b}))
                .collect::<Vec<_>>();
            rows.push(json!({"lba":conflict.lba,"usb_sha256":conflict.usb_sha256,"gw_sha256":conflict.gw_sha256,"changed_bytes":deltas.len(),"differences":deltas,
                "fat_observations":analysis.as_ref().ok().map(|a|fat_observations(a,&snapshot.bytes,&usb.bytes,&bad,&usb_bad,conflict.lba))}));
        }
        for file in &mut files {
            let legacy = &file["word"]["compound"]["legacy_word6_main_text_map"];
            if legacy["state"] == "mapped_bytes_not_decoded_text" {
                let ranges = legacy["pieces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|p| p["source_extents"].as_array().unwrap())
                    .map(|e| {
                        (
                            e["source_byte_offset"].as_u64().unwrap() as usize,
                            e["bytes"].as_u64().unwrap() as usize,
                        )
                    })
                    .collect::<Vec<_>>();
                let comparison =
                    compare_ranges(&snapshot.bytes, &usb.bytes, &bad, &usb_bad, &ranges)?;
                let overlaps = ranges
                    .iter()
                    .any(|(at, n)| (*at..at + n).any(|b| disputed.contains(&((b / 512) as u64))));
                file["legacy_main_byte_comparison"] = json!({"scope":"selected_GW_file_mapping_applied_to_readable_USB_physical_positions_not_independent_USB_filesystem_mapping",
                    "comparison":comparison,"disputed_sectors_intersect_mapped_main_ranges":overlaps,"selected_main_unavailable_bytes":legacy["unavailable_or_disputed_main_text_bytes"],
                    "text_encoding_verified":false,"original_content_winner":null});
            }
        }
        usb.unchanged()?;
        json!({"usb_attempt":c.usb_attempt,"usb_image_sha256":c.usb_image_sha256,"gw_image_sha256":c.gw_image_sha256,"agreeing_readable_sectors":c.agreeing_sectors,"sector_versions":rows,"original_content_winner":null})
    } else {
        Value::Null
    };
    snapshot.unchanged()?;
    let result = json!({"schema_version":1,"mode":"saved_recovery_investigation","disk":disk,"attempt":number,"source_image":snapshot.image_path(),"recorded_source_image":impact["source_image"],"source_image_sha256":impact["source_image_sha256"],
        "physical_media_access":false,"files_written":0,"customer_delivery_certified":false,"warning":WARNING,"attention_required":impact["attention_required"],
        "impact":impact,"reviewed_files":files,"dispute_review":disputes,"filesystem_error":analysis.err()});
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_REPORT
    {
        return Err("Investigation report size ceiling".into());
    }
    Ok(result)
}

fn compare_ranges(
    gw: &[u8],
    usb: &[u8],
    gw_bad: &BTreeSet<u64>,
    usb_bad: &BTreeSet<u64>,
    ranges: &[(usize, usize)],
) -> Result<Value, String> {
    let (mut agree, mut differ, mut unknown) = (0usize, 0usize, 0usize);
    let mut total = 0usize;
    let mut a = Sha256::new();
    let mut b = Sha256::new();
    for (at, n) in ranges {
        total = total.checked_add(*n).ok_or("Cross-reader work overflow")?;
        if total > 500_000 {
            return Err("Cross-reader range comparison work ceiling".into());
        }
        let end = at.checked_add(*n).ok_or("Cross-reader range overflow")?;
        let first = gw
            .get(*at..end)
            .ok_or("GW comparison range outside image")?;
        let second = usb
            .get(*at..end)
            .ok_or("USB comparison range outside image")?;
        for (i, (x, y)) in first.iter().zip(second).enumerate() {
            let lba = ((*at + i) / 512) as u64;
            if gw_bad.contains(&lba) || usb_bad.contains(&lba) {
                unknown += 1;
            } else if x == y {
                agree += 1;
            } else {
                differ += 1;
            }
        }
        a.update(first);
        b.update(second);
    }
    Ok(
        json!({"agreeing_observed_bytes":agree,"different_observed_bytes":differ,"unavailable_bytes":unknown,
        "gw_ordered_bytes_sha256":if unknown==0{Some(format!("{:x}",a.finalize()))}else{None},
        "usb_ordered_bytes_sha256":if unknown==0{Some(format!("{:x}",b.finalize()))}else{None},"full_original_certified":false}),
    )
}

fn fat_observations(
    a: &fat12::Analysis,
    gw: &[u8],
    usb: &[u8],
    gw_bad: &BTreeSet<u64>,
    usb_bad: &BTreeSet<u64>,
    lba: u64,
) -> Value {
    let l = &a.layout;
    if lba < (l.reserved_sectors as u64) || lba >= l.root_start as u64 {
        return Value::Null;
    }
    let copy = (lba as usize - l.reserved_sectors) / l.sectors_per_fat;
    let within = (lba as usize - l.reserved_sectors) % l.sectors_per_fat;
    let mut entries = Vec::new();
    for cluster in 2..l.clusters + 2 {
        let off = cluster * 3 / 2;
        if off / 512 != within && (off + 1) / 512 != within {
            continue;
        }
        let read = |image: &[u8], bad: &BTreeSet<u64>, fat: usize| -> Option<u16> {
            let at = (l.reserved_sectors + fat * l.sectors_per_fat) * 512 + off;
            if bad.contains(&((at / 512) as u64)) || bad.contains(&(((at + 1) / 512) as u64)) {
                return None;
            }
            let value = u16::from_le_bytes(image.get(at..at + 2)?.try_into().ok()?);
            Some(if cluster % 2 == 0 {
                value & 0xfff
            } else {
                value >> 4
            })
        };
        let usb_value = read(usb, usb_bad, copy);
        let gw_value = read(gw, gw_bad, copy);
        if usb_value != gw_value {
            entries.push(json!({"cluster":cluster,"usb_value":usb_value,"gw_value":gw_value,
                "gw_copy_values":(0..l.fat_copies).map(|c|read(gw,gw_bad,c)).collect::<Vec<_>>(),
                "usb_copy_values":(0..l.fat_copies).map(|c|read(usb,usb_bad,c)).collect::<Vec<_>>(),
                "known_allocation_claim":a.owned_clusters.contains(&(cluster as u16)),"known_deleted_exclusion":a.deleted_clusters_excluded.contains(&(cluster as u16))}));
        }
    }
    json!({"fat_copy":copy,"sector_in_copy":within,"different_declared_cluster_entries":entries,"winner_selected":false,"zero_value_is_observation_not_original_allocation_certificate":true})
}

pub fn inspect_all(
    project: &ProjectState,
    mut progress: impl FnMut(usize, usize, u32),
) -> Result<Value, String> {
    let before = evidence_impact::inventory(project)?;
    let impact = evidence_impact::inspect_all(project, &mut progress)?;
    let candidates = impact["disks"]
        .as_array()
        .ok_or("Missing batch inventory")?
        .iter()
        .filter(|d| {
            d["counts"]["known_files_with_problem_dependencies"]
                .as_u64()
                .unwrap_or(0)
                > 0
                || d["counts"]["incomplete_or_ambiguous_files"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0
        })
        .collect::<Vec<_>>();
    let mut disks = Vec::new();
    let mut bytes = 0;
    let mut reviewed = 0;
    for (index, candidate) in candidates.iter().enumerate() {
        let disk = candidate["disk"].as_u64().ok_or("Missing disk")? as u32;
        progress(index + 1, candidates.len(), disk);
        let result = inspect(
            project,
            disk,
            Some(candidate["attempt"].as_u64().ok_or("Missing attempt")? as u32),
        )?;
        if result["source_image_sha256"] != candidate["source_image_sha256"]
            || result["impact"]["snapshot_binding"] != candidate["snapshot_binding"]
        {
            return Err("Batch selection changed during investigation".into());
        }
        reviewed += result["reviewed_files"].as_array().unwrap().len();
        bytes += serde_json::to_vec(&result)
            .map_err(|e| e.to_string())?
            .len();
        if bytes > MAX_REPORT {
            return Err("Batch investigation exceeds 64 MiB".into());
        }
        disks.push(result);
    }
    let changed = evidence_impact::inventory(project)? != before;
    let result = json!({"schema_version":1,"mode":"batch_recovery_investigation","project":project.root(),"physical_media_access":false,"files_written":0,"customer_delivery_certified":false,"warning":WARNING,
        "inventory_changed":changed || impact["inventory_changed"]==true,"attention_required":impact["attention_required"]==true || changed,
        "counts":{"saved_labels":impact["counts"]["saved_labels"],"refused_disks":impact["counts"]["refused_disks"],"reviewed_disks":disks.len(),"reviewed_file_records":reviewed,
            "known_files_with_problem_dependencies":impact["counts"]["inspected_image_totals"]["known_files_with_problem_dependencies"],"incomplete_or_ambiguous_files":impact["counts"]["inspected_image_totals"]["incomplete_or_ambiguous_files"]},
        "impact":impact,"disks":disks});
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_REPORT
    {
        return Err("Combined batch investigation exceeds 64 MiB".into());
    }
    Ok(result)
}

pub fn render(value: &Value) -> String {
    render_inner(value, true)
}

fn render_inner(value: &Value, header: bool) -> String {
    let mut out = if header {
        format!("SAVED RECOVERY INVESTIGATION\n{WARNING}\n")
    } else {
        String::new()
    };
    if value["mode"] == "batch_recovery_investigation" {
        out.push_str(&format!("{} labels / {} reviewed disks / {} file records; {} affected dependencies / {} incomplete chains\n",value["counts"]["saved_labels"],value["counts"]["reviewed_disks"],value["counts"]["reviewed_file_records"],value["counts"]["known_files_with_problem_dependencies"],value["counts"]["incomplete_or_ambiguous_files"]));
        for disk in value["disks"].as_array().into_iter().flatten() {
            out.push_str(&render_inner(disk, false));
        }
        if value["inventory_changed"] == true {
            out.push_str("ATTENTION: inventory changed; rerun when idle\n");
        }
    } else {
        out.push_str(&format!(
            "\nDisk {:03} / attempt {} / image SHA-256 {}\n",
            value["disk"].as_u64().unwrap_or(0),
            value["attempt"],
            value["source_image_sha256"].as_str().unwrap_or("?")
        ));
        for file in value["reviewed_files"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "{} | {}\n",
                file["path"].as_str().unwrap_or("?"),
                file["classification"].as_str().unwrap_or("?")
            ));
            let word = &file["word"]["word_text"];
            if word["state"] == "mapped_forensic_text" {
                out.push_str(&format!(
                    "  Word main text: {} readable / {} declared; {} missing positions\n",
                    word["readable_positions"], word["main_positions"], word["missing_positions"]
                ));
            }
            let rtf = &file["rtf"];
            if rtf["state"] == "mapped_forensic_prefix" {
                out.push_str(&format!(
                    "  RTF prefix: {} readable positions; remainder unknown\n",
                    rtf["readable_utf16_positions"]
                ));
            }
            let comparison = &file["legacy_main_byte_comparison"];
            if !comparison.is_null() {
                out.push_str(&format!("  Word6 ordered main-byte observations: {} agree / {} differ / {} unavailable; no encoding or meaning inferred\n",comparison["comparison"]["agreeing_observed_bytes"],comparison["comparison"]["different_observed_bytes"],comparison["comparison"]["unavailable_bytes"]));
                out.push_str(&format!(
                    "  Disputed sectors intersect mapped main ranges: {}\n",
                    comparison["disputed_sectors_intersect_mapped_main_ranges"]
                ));
            }
            if let Some(reason) = file["chain_reason"].as_str() {
                out.push_str(&format!("  Chain: {reason}\n"));
            }
        }
        for sector in value["dispute_review"]["sector_versions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "  DISPUTED LBA {}: {} different bytes; both versions retained, no winner\n",
                sector["lba"], sector["changed_bytes"]
            ));
        }
        if !value["filesystem_error"].is_null() {
            out.push_str(&format!("  UNKNOWN: {}\n", value["filesystem_error"]));
        }
    }
    out
}
