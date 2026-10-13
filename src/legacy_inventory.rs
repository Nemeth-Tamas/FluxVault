//! Reproduce historical script inventory/counts from the same ZIP, read-only.
//! This is preservation/accounting, not new recovery-yield parity.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek},
    path::Path,
};
fn hash_reader(mut r: impl Read, limit: u64) -> Result<(u64, String), String> {
    let mut h = Sha256::new();
    let mut n = 0u64;
    let mut b = [0; 65536];
    loop {
        crate::cancellation::check()?;
        let c = r.read(&mut b).map_err(|e| e.to_string())?;
        if c == 0 {
            break;
        }
        n += c as u64;
        if n > limit {
            return Err("Legacy inventory byte bound exceeded".into());
        }
        h.update(&b[..c]);
    }
    Ok((n, format!("{:x}", h.finalize())))
}
fn safe(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 4096
        || name.contains('\\')
        || name
            .chars()
            .any(|c| c.is_control() || ":*?\"<>|".contains(c))
        || name
            .trim_end_matches('/')
            .split('/')
            .any(|s| s.is_empty() || matches!(s, "." | ".."))
    {
        return Err("Unsafe historical archive member".into());
    }
    Ok(())
}
fn report<T: Read + Seek>(
    archive: &mut zip::ZipArchive<T>,
    name: &str,
) -> Result<Vec<BTreeMap<String, String>>, String> {
    let file = archive
        .by_name(name)
        .map_err(|e| format!("Required legacy report {name}: {e}"))?;
    if file.size() > 8 * 1024 * 1024 {
        return Err("Legacy report exceeds 8 MiB".into());
    }
    let mut text = String::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 8 * 1024 * 1024 {
        return Err("Legacy report grew beyond bound".into());
    }
    let mut rows = crate::audit::parse_csv(text.trim_start_matches('\u{feff}'))?.into_iter();
    let header = rows.next().ok_or("Empty legacy report")?;
    if header.is_empty()
        || header.len() > 64
        || header.iter().collect::<BTreeSet<_>>().len() != header.len()
    {
        return Err("Invalid legacy report header".into());
    }
    rows.map(|r| {
        if r.len() != header.len() {
            Err("Legacy row width mismatch".into())
        } else {
            Ok(header.iter().cloned().zip(r).collect())
        }
    })
    .collect()
}
pub fn inspect(path: &Path) -> Result<Value, String> {
    crate::sector_inspection::regular(path, false)?;
    crate::safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)?;
    let (archive_bytes, archive_sha) = hash_reader(
        File::open(path).map_err(|e| e.to_string())?,
        4 * 1024 * 1024 * 1024,
    )?;
    let mut zip = zip::ZipArchive::new(File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if zip.len() > 100_000 {
        return Err("Too many legacy members".into());
    }
    let mut inventory = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut total = 0;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        safe(&name)?;
        if !folded.insert(name.to_lowercase())
            || entry
                .unix_mode()
                .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err("Duplicate/colliding or special ZIP member refused".into());
        }
        if entry.is_dir() {
            continue;
        }
        let expected = entry.size();
        let (bytes, sha) = hash_reader(entry, 64 * 1024 * 1024)?;
        if bytes != expected {
            return Err("Legacy member size changed".into());
        }
        total += bytes;
        if total > 4 * 1024 * 1024 * 1024 {
            return Err("Legacy expanded archive exceeds 4 GiB".into());
        }
        inventory.insert(name, (bytes, sha));
    }
    let master = report(&mut zip, "Reports/MasterFileList.csv")?;
    let extraction = report(&mut zip, "Reports/ExtractionSummary.csv")?;
    let conversion = report(&mut zip, "Reports/ConversionSummary.csv")?;
    let index = report(&mut zip, "Reports/ArchiveIndex.csv")?;
    let mut discrepancies = Vec::new();
    let mut seen = BTreeSet::new();
    for (number, row) in master.iter().enumerate() {
        let label = row
            .get("Floppy")
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|n| *n > 0)
            .ok_or("Invalid historical manifest label")?;
        let file = row
            .get("Path")
            .ok_or("Missing historical manifest path")?
            .replace('\\', "/");
        let path = format!("Extracted/{label:03}/{file}");
        safe(&path)?;
        if !seen.insert(path.clone()) {
            discrepancies
                .push(json!({"row":number+1,"path":path,"reason":"duplicate_master_record"}));
        }
        let declared = row
            .get("SizeBytes")
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or("Invalid historical source size")?;
        match inventory.get(&path){
            Some((size,_)) if *size==declared=>{},
            Some((size,_))=>discrepancies.push(json!({"row":number+1,"path":path,"reason":"recorded_size_differs","declared":declared,"actual":size})),
            None=>discrepancies.push(json!({"row":number+1,"path":path,"reason":"master_record_member_missing"})),
        }
    }
    let counts = |rows: &Vec<BTreeMap<String, String>>, key: &str| -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for r in rows {
            *out.entry(r.get(key).cloned().unwrap_or_else(|| "unknown".into()))
                .or_default() += 1;
        }
        out
    };
    let mut output_checks = BTreeMap::<String, usize>::new();
    for row in &conversion {
        for (kind, path_key, ok_key) in [
            ("modern", "ModernPath", "ModernOK"),
            ("pdf", "PDFPath", "PDFOK"),
        ] {
            let Some(relative) = row.get(path_key).filter(|s| !s.is_empty()) else {
                continue;
            };
            let path = format!("Converted/{}", relative.replace('\\', "/"));
            safe(&path)?;
            let exists = inventory.contains_key(&path);
            let claimed = row
                .get(ok_key)
                .is_some_and(|s| s.eq_ignore_ascii_case("true"));
            *output_checks
                .entry(format!(
                    "{kind}_{}",
                    if exists {
                        "present_crc_checked"
                    } else {
                        "missing"
                    }
                ))
                .or_default() += 1;
            if claimed && !exists {
                discrepancies.push(json!({"path":path,"reason":"reported_success_output_missing"}));
            }
        }
    }
    let historical = crate::legacy_reconciliation::reconcile(
        &mut zip,
        &inventory,
        &index,
        &extraction,
        &conversion,
    )?;
    let (after_bytes, after_sha) = hash_reader(
        File::open(path).map_err(|e| e.to_string())?,
        4 * 1024 * 1024 * 1024,
    )?;
    if after_sha != archive_sha || after_bytes != archive_bytes {
        return Err("Historical archive changed during inspection".into());
    }
    let extracted = inventory
        .keys()
        .filter(|p| p.starts_with("Extracted/"))
        .count();
    Ok(
        json!({"schema_version":1,"physical_media_access":false,"archive_modified":false,"archive_bytes":archive_bytes,"archive_sha256":archive_sha,
        "crc_and_sha256_checked_members":inventory.len(),"expanded_member_bytes":total,"source_records":master.len(),"extracted_members":extracted,
        "source_record_count_matches_members":master.len()==extracted,"extraction_rows":extraction.len(),"extraction_statuses":counts(&extraction,"Status"),
        "conversion_rows":conversion.len(),"conversion_statuses":counts(&conversion,"Status"),"conversion_outputs":output_checks,
        "archive_index_rows":index.len(),"archive_index_statuses":counts(&index,"Status"),"discrepancies":discrepancies,
        "historical_reconciliation":historical,
        "note":"Reproduces the exact archived script/DMDE inventory and original aggregate classifications, with per-label script/CSV status precedence and parsed log differences. All archived bytes are streamed/CRC/SHA checked, not extracted or rewritten. Source records include tool/metadata/deleted/carved scope as originally inventoried; these totals are not fresh automated recovery yield or customer certification. An absent original workbook is explicitly unavailable."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_member_guard_refuses_paths_and_preserves_unicode() {
        for p in ["../x", "/x", "a//b", "a:ads", "a\\b"] {
            assert!(safe(p).is_err());
        }
        assert!(safe("Extracted/007/levél.doc").is_ok());
    }
    #[test]
    fn csv_inventory_supports_quoted_newlines_but_refuses_duplicate_headers() {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        for (content, ok) in [
            ("A,B\n1,\"quoted, value\"\n", true),
            ("A,A\n1,2\n", false),
            ("A,B\n1\n", false),
        ] {
            let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
            w.start_file("report.csv", SimpleFileOptions::default())
                .unwrap();
            w.write_all(content.as_bytes()).unwrap();
            let b = w.finish().unwrap();
            let mut z = zip::ZipArchive::new(Cursor::new(b.into_inner())).unwrap();
            assert_eq!(report(&mut z, "report.csv").is_ok(), ok);
        }
    }
}
