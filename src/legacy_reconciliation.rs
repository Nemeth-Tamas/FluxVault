//! Replay the historical final-audit classification, never certify current data.
//! The old script prefers ArchiveIndex over ExtractionSummary over NO STATUS.
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Seek},
    path::PathBuf,
};
type Row = BTreeMap<String, String>;
type Inventory = BTreeMap<String, (u64, String)>;

fn label(row: &Row, key: &str) -> Result<u32, String> {
    row.get(key)
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0 && *n <= 4096)
        .ok_or_else(|| format!("Invalid historical {key} label"))
}
fn indexed<'a>(rows: &'a [Row], key: &str) -> Result<BTreeMap<u32, &'a Row>, String> {
    let mut values = BTreeMap::new();
    for row in rows {
        if values.insert(label(row, key)?, row).is_some() {
            return Err("Duplicate historical label; script last-row overwrite cannot establish a unique claim".into());
        }
    }
    Ok(values)
}
fn bytes<T: Read + Seek>(
    zip: &mut zip::ZipArchive<T>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let f = zip.by_name(name).map_err(|e| e.to_string())?;
    if f.size() > limit {
        return Err("Historical reconciliation member exceeds bound".into());
    }
    let mut b = Vec::new();
    f.take(limit + 1)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() as u64 > limit {
        return Err("Historical reconciliation member grew beyond bound".into());
    }
    Ok(b)
}
fn delivery(path: &str) -> bool {
    path.split('/').all(|c| {
        let c = c.to_ascii_lowercase();
        !c.starts_with("__")
            && c != "$root"
            && c != "$raw"
            && c != "[$raw files by signatures]"
            && c != "$noname"
            && !c
                .strip_prefix("$noname ")
                .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()))
    })
}
fn old_integrity<T: Read + Seek>(zip: &mut zip::ZipArchive<T>, name: &str) -> Result<bool, String> {
    let b = bytes(zip, name, 64 * 1024 * 1024)?;
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if ext == "pdf" {
        return Ok(b.len() >= 10
            && b.starts_with(b"%PDF-")
            && b[b.len().saturating_sub(8192)..]
                .windows(5)
                .any(|s| s == b"%%EOF"));
    }
    let main = match ext.as_str() {
        "docx" => "word/document.xml",
        "xlsx" => "xl/workbook.xml",
        "pptx" => "ppt/presentation.xml",
        _ => return Ok(true),
    };
    let Ok(mut inner) = zip::ZipArchive::new(Cursor::new(b)) else {
        return Ok(false);
    };
    Ok(inner.by_name("[Content_Types].xml").is_ok() && inner.by_name(main).is_ok())
}
fn state(
    image: bool,
    sources: usize,
    invalid: usize,
    failed: usize,
    partial: usize,
    imaging: &str,
) -> &'static str {
    if !image {
        "CHECK: IMAGE MISSING"
    } else if sources == 0 {
        "CHECK: NO RECOVERED FILES"
    } else if invalid > 0 {
        "CHECK: INVALID GENERATED FILE"
    } else if failed > 0 {
        "CHECK: CONVERSION FAILED"
    } else if partial > 0 {
        "PARTIAL: CONVERSION"
    } else if !imaging.eq_ignore_ascii_case("OK") {
        "PARTIAL: IMAGE READ"
    } else {
        "OK"
    }
}

pub(crate) fn reconcile<T: Read + Seek>(
    zip: &mut zip::ZipArchive<T>,
    inventory: &Inventory,
    index: &[Row],
    extraction: &[Row],
    conversion: &[Row],
) -> Result<Value, String> {
    let index = indexed(index, "FloppyNumber")?;
    let extraction = indexed(extraction, "Floppy")?;
    let mut labels: BTreeSet<u32> = index.keys().chain(extraction.keys()).copied().collect();
    let mut images = BTreeMap::new();
    let mut source_counts = BTreeMap::<u32, usize>::new();
    let mut invalid_counts = BTreeMap::<u32, usize>::new();
    let mut invalid_members = Vec::new();
    let mut jobs = BTreeMap::<u32, Vec<&Row>>::new();
    for row in conversion {
        let disk = label(row, "Floppy")?;
        labels.insert(disk);
        jobs.entry(disk).or_default().push(row);
    }
    for name in inventory.keys() {
        let parts = name.split('/').collect::<Vec<_>>();
        if parts.len() == 2 && parts[0] == "Images" {
            let stem = parts[1].rsplit_once('.').map_or(parts[1], |(s, _)| s);
            if stem.len() == 3 && stem.bytes().all(|b| b.is_ascii_digit()) {
                let disk = stem.parse::<u32>().map_err(|e| e.to_string())?;
                if disk == 0 || images.insert(disk, name.clone()).is_some() {
                    return Err("Ambiguous historical image identity".into());
                }
                labels.insert(disk);
            }
        } else if parts.len() >= 3 && parts[0] == "Extracted" {
            let disk = parts[1]
                .parse::<u32>()
                .map_err(|_| "Invalid recovered-folder label")?;
            if disk == 0 || disk > 4096 {
                return Err("Historical folder label exceeds bound".into());
            }
            labels.insert(disk);
            if !parts.last().unwrap().starts_with("__") {
                *source_counts.entry(disk).or_default() += 1;
            }
        }
    }
    if labels.len() > 4096 {
        return Err("Historical label union exceeds bound".into());
    }
    let generated = conversion
        .iter()
        .flat_map(|row| {
            [("ModernOK", "ModernPath"), ("PDFOK", "PDFPath")]
                .into_iter()
                .filter_map(|(ok, path)| {
                    row.get(ok)
                        .is_some_and(|s| s.eq_ignore_ascii_case("true"))
                        .then(|| row.get(path))
                        .flatten()
                        .filter(|p| !p.is_empty())
                        .map(|p| format!("Converted/{}", p.replace('\\', "/")).to_lowercase())
                })
        })
        .collect::<BTreeSet<_>>();
    // Preserve the old rule: only paths claimed generated successfully are
    // checked, not customer originals or failed leftover converter files.
    for name in inventory.keys().filter(|n| {
        n.starts_with("Converted/") && delivery(n) && generated.contains(&n.to_lowercase())
    }) {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if matches!(ext.as_str(), "docx" | "xlsx" | "pptx" | "pdf") && !old_integrity(zip, name)? {
            let disk = name
                .split('/')
                .nth(1)
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or("Invalid converted label")?;
            *invalid_counts.entry(disk).or_default() += 1;
            invalid_members.push(json!({"disk":disk,"path":name,"sha256":inventory[name].1,"rule":"old_script_zip_members_or_pdf_header_eof"}));
        }
    }
    let log_paths = inventory
        .keys()
        .filter(|n| n.starts_with("Logs/") && n.ends_with(".log"))
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut statuses = BTreeMap::<String, usize>::new();
    let mut parsed_statuses = BTreeMap::<String, usize>::new();
    let mut imaging_ok = 0;
    let mut fully_ok = 0;
    let mut without_integrity_ok = 0;
    let mut disagreements = Vec::new();
    for disk in labels {
        crate::cancellation::check()?;
        let (claim, claim_source) = if let Some(row) = index.get(&disk) {
            (*row, "Reports/ArchiveIndex.csv")
        } else if let Some(row) = extraction.get(&disk) {
            (*row, "Reports/ExtractionSummary.csv")
        } else {
            (&Row::new(), "no_recorded_status")
        };
        let imaging = claim.get("Status").map_or("NO STATUS", String::as_str);
        let sources = source_counts.get(&disk).copied().unwrap_or(0);
        let count = |wanted: &str| {
            jobs.get(&disk)
                .into_iter()
                .flatten()
                .filter(|r| {
                    r.get("Status")
                        .is_some_and(|s| s.eq_ignore_ascii_case(wanted))
                })
                .count()
        };
        let failed = count("FAILED");
        let partial = count("PARTIAL");
        let invalid = invalid_counts.get(&disk).copied().unwrap_or(0);
        let image = images.get(&disk);
        let audit = state(image.is_some(), sources, invalid, failed, partial, imaging);
        imaging_ok += usize::from(imaging.eq_ignore_ascii_case("OK"));
        fully_ok += usize::from(audit == "OK");
        without_integrity_ok +=
            usize::from(state(image.is_some(), sources, 0, failed, partial, imaging) == "OK");
        *statuses.entry(audit.into()).or_default() += 1;
        let mut log_value = json!({"status":"MISSING LOG","path":null});
        if let Some(path) = crate::legacy_logs::choose_primary_log(log_paths.iter(), disk) {
            let path = path.to_string_lossy().replace('\\', "/");
            let data = bytes(zip, &path, 8 * 1024 * 1024)?;
            let text = std::str::from_utf8(&data).map_err(|e| e.to_string())?;
            if let Ok(log) = crate::legacy_logs::parse_archiver_log(text) {
                log_value = json!({"path":path,"status":log.status.label(),"kind":"archiver","image_sha256_claim":log.sha256,
                    "image_sha256_matches":image.and_then(|p|inventory.get(p)).zip(log.sha256.as_ref()).map(|((_,h),s)|h.eq_ignore_ascii_case(s)),
                    "image_extent_matches":image.and_then(|p|inventory.get(p)).zip(log.bytes_written).map(|((n,_),s)|*n==s),"bad_sectors":log.bad_sectors.len()});
            } else if let Ok(log) = crate::dmde_logs::parse_dmde_log(text) {
                log_value = json!({"path":path,"status":log.status.label(),"kind":"dmde","bad_sectors":log.bad_sectors.len(),
                    "image_extent_matches":image.and_then(|p|inventory.get(p)).zip(log.sector_size).map(|((n,_),s)|log.highest_sector_exclusive.checked_mul(u64::from(s))==Some(*n))});
            } else {
                log_value["path"] = json!(path);
            }
        }
        *parsed_statuses
            .entry(log_value["status"].as_str().unwrap().into())
            .or_default() += 1;
        if !log_value["status"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(imaging)
        {
            disagreements.push(json!({"disk":disk,"historical_status":imaging,"source":claim_source,"parsed_log_status":log_value["status"]}));
        }
        rows.push(json!({"disk":disk,"image":image,"actual_image":image.and_then(|p|inventory.get(p)).map(|(bytes,sha)|json!({"bytes":bytes,"sha256":sha})),
            "historical_imaging_status":imaging,"status_claim_source":claim_source,"original_claim":claim,
            "index_image_sha256_matches":image.and_then(|p|inventory.get(p)).zip(claim.get("SHA256")).map(|((_,h),s)|h.eq_ignore_ascii_case(s)),
            "parsed_primary_log":log_value,"source_files_counted_by_old_rule":sources,"conversion_failed":failed,"conversion_partial":partial,
            "invalid_generated_by_old_rule":invalid,"historical_final_status":audit}));
    }
    let n = rows.len();
    Ok(
        json!({"schema_version":1,"classification_rule":"New-FloppyFinalAudit_v1.2: index first, extraction fallback; image/source/integrity/failed/partial/imaging precedence",
        "labels":n,"images_present":images.len(),"historical_imaging_ok":imaging_ok,"historical_imaging_not_ok":n-imaging_ok,
        "historical_fully_ok":fully_ok,"historical_attention":n-fully_ok,"historical_final_statuses":statuses,
        "without_generated_integrity_gate":{"fully_ok":without_integrity_ok,"attention":n-without_integrity_ok,
            "scope":"diagnostic counterfactual, not the actual v1.2 final-audit result or proof of the missing workbook's cause"},
        "invalid_generated_members":invalid_members,
        "independently_parsed_primary_log_statuses":parsed_statuses,"status_disagreements":disagreements,"disks":rows,
        "original_final_workbook_present":inventory.contains_key("Reports/FloppyFinalReport.xlsx"),
        "customer_delivery_certified":false,"limits":"Replayed historical classifications, not current acquisition or semantic recovery certificates. Exact old Office ZIP member-presence/PDF-envelope rules are reproduced, not full Office rendering. Index/hash/log discrepancies are retained. An absent original workbook cannot be inspected; its recorded summary can be compared to this reproducible script/CSV replay."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_replay_separates_csv_precedence_integrity_and_missing_logs() {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let mut inventory = Inventory::new();
        let mut members = Vec::new();
        for disk in 1..=5 {
            members.push((format!("Images/{disk:03}.img"), b"saved image".to_vec()));
            members.push((
                format!("Extracted/{disk:03}/original.doc"),
                b"source".to_vec(),
            ));
            members.push((
                format!("Extracted/{disk:03}/__tool.txt"),
                b"excluded".to_vec(),
            ));
        }
        members.push(("Converted/003/broken.docx".into(), b"not a zip".to_vec()));
        members.push((
            "Converted/004/leftover.pdf".into(),
            b"failed output".to_vec(),
        ));
        for (name, bytes) in members {
            writer
                .start_file(&name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&bytes).unwrap();
            inventory.insert(
                name,
                (bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes))),
            );
        }
        let row = |fields: &[(&str, &str)]| -> Row {
            fields
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect()
        };
        let index = vec![
            row(&[("FloppyNumber", "001"), ("Status", "OK")]),
            row(&[("FloppyNumber", "002"), ("Status", "PARTIAL")]),
        ];
        let extraction = vec![
            row(&[("Floppy", "001"), ("Status", "PARTIAL")]),
            row(&[("Floppy", "003"), ("Status", "OK")]),
            row(&[("Floppy", "004"), ("Status", "OK")]),
        ];
        let conversion = vec![
            row(&[
                ("Floppy", "003"),
                ("Status", "OK"),
                ("ModernOK", "True"),
                ("ModernPath", "003/broken.docx"),
            ]),
            row(&[
                ("Floppy", "004"),
                ("Status", "PARTIAL"),
                ("PDFOK", "False"),
                ("PDFPath", "004/leftover.pdf"),
            ]),
        ];
        let mut archive = zip::ZipArchive::new(writer.finish().unwrap()).unwrap();
        let replay = reconcile(&mut archive, &inventory, &index, &extraction, &conversion).unwrap();
        assert_eq!(replay["historical_imaging_ok"], 3);
        assert_eq!(replay["historical_fully_ok"], 1);
        assert_eq!(replay["without_generated_integrity_gate"]["fully_ok"], 2);
        assert_eq!(
            replay["independently_parsed_primary_log_statuses"]["MISSING LOG"],
            5
        );
        assert_eq!(
            replay["invalid_generated_members"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            replay["disks"][0]["status_claim_source"],
            "Reports/ArchiveIndex.csv"
        );
        assert_eq!(replay["disks"][0]["source_files_counted_by_old_rule"], 1);
        assert_eq!(replay["disks"][4]["historical_imaging_status"], "NO STATUS");
        assert_eq!(replay["original_final_workbook_present"], false);
        assert_eq!(replay["customer_delivery_certified"], false);
    }
    #[test]
    fn historical_precedence_and_attention_keep_failures_and_unknown_distinct() {
        assert_eq!(state(false, 0, 1, 1, 1, "OK"), "CHECK: IMAGE MISSING");
        assert_eq!(state(true, 0, 1, 1, 1, "OK"), "CHECK: NO RECOVERED FILES");
        assert_eq!(
            state(true, 1, 1, 1, 1, "OK"),
            "CHECK: INVALID GENERATED FILE"
        );
        assert_eq!(
            state(true, 1, 0, 1, 1, "PARTIAL"),
            "CHECK: CONVERSION FAILED"
        );
        assert_eq!(state(true, 1, 0, 0, 1, "PARTIAL"), "PARTIAL: CONVERSION");
        assert_eq!(
            state(true, 1, 0, 0, 0, "MANUALLY RECOVERED"),
            "PARTIAL: IMAGE READ"
        );
        assert_eq!(state(true, 1, 0, 0, 0, "OK"), "OK");
        assert!(!delivery("Converted/007/$Root/raw.docx"));
        assert!(delivery("Converted/007/file-$Root.docx"));
        let row: Row = [("FloppyNumber".into(), "007".into())].into();
        assert!(indexed(&[row.clone(), row], "FloppyNumber").is_err());
    }
}
