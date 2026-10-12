//! Compact exact unavailable/disputed byte ranges, not guessed recovery claims.
use crate::{evidence_impact, project::ProjectState, sector_inspection};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Called under the existing project writer/snapshot gate. Only new immutable
/// report evidence is published; earlier damaged-range snapshots are retained.
pub(crate) fn publish_all(
    project: &ProjectState,
    stage: &impl Fn(&str),
) -> Result<Vec<std::path::PathBuf>, String> {
    use sha2::{Digest, Sha256};
    use std::{fs, io::Write};
    let stats = crate::imaging::load_project_statistics(&project.images_dir())?;
    let mut paths = Vec::new();
    for disk in stats.disks {
        crate::cancellation::check()?;
        if disk.best_bad_sectors == 0 && !disk.attention_required {
            continue;
        }
        let value = match inspect(project, disk.disk_number, None) {
            Ok(v) => v,
            Err(e) => {
                json!({"schema_version":1,"disk":disk.disk_number,"range_mapping_refused":e,"physical_media_access":false,"unknown_ranges_not_invented":true})
            }
        };
        let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
        let sha = format!("{:x}", Sha256::digest(&bytes));
        crate::sector_inspection::regular(&project.reports_dir(), true)?;
        let parent = project
            .reports_dir()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let root = project.root().canonicalize().map_err(|e| e.to_string())?;
        if parent.parent() != Some(root.as_path()) {
            return Err("Reports directory escapes the project; no range file created".into());
        }
        let dir = parent.join("RecoveryRanges");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        crate::sector_inspection::regular(&dir, true)?;
        if dir.canonicalize().map_err(|e| e.to_string())?.parent() != Some(parent.as_path()) {
            return Err("Range report directory escapes Reports".into());
        }
        let final_path = dir.join(format!("{:03}_{}.json", disk.disk_number, &sha[..24]));
        if final_path.try_exists().map_err(|e| e.to_string())? {
            crate::sector_inspection::regular(&final_path, false)?;
            if fs::read(&final_path).map_err(|e| e.to_string())? != bytes {
                return Err(
                    "Published recovery range report changed; earlier report preserved".into(),
                );
            }
        } else {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let part = dir.join(format!(".{}-{}-{nonce}.partial", sha, std::process::id()));
            let mut f = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&part)
                .map_err(|e| e.to_string())?;
            f.write_all(&bytes)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
            drop(f);
            fs::hard_link(&part, &final_path).map_err(|e| e.to_string())?;
            fs::remove_file(&part).map_err(|e| e.to_string())?;
        }
        stage(&format!(
            "1/5: Disk {:03}: exact unavailable/disputed ranges recorded (unknown allocation stays explicit)",
            disk.disk_number
        ));
        paths.push(final_path);
    }
    Ok(paths)
}

fn ranges(lbas: &BTreeSet<u64>) -> Vec<Value> {
    let mut rows = Vec::new();
    let mut run = None;
    for lba in lbas.iter().copied() {
        match run {
            Some((start, end)) if lba == end + 1 => run = Some((start, lba)),
            Some((start, end)) => {
                rows.push(json!({"first_lba":start,"last_lba_inclusive":end,"byte_offset":start*512,"byte_length":(end-start+1)*512}));
                run = Some((lba, lba));
            }
            None => run = Some((lba, lba)),
        }
    }
    if let Some((start, end)) = run {
        rows.push(json!({"first_lba":start,"last_lba_inclusive":end,"byte_offset":start*512,"byte_length":(end-start+1)*512}));
    }
    rows
}
pub fn inspect(project: &ProjectState, disk: u32, attempt: Option<u32>) -> Result<Value, String> {
    let snapshot = sector_inspection::load_snapshot(project, disk, attempt)?;
    let missing = snapshot.metadata()["bad_sectors"]
        .as_array()
        .ok_or("Missing bad-sector map")?
        .iter()
        .map(|b| b["lba"].as_u64().ok_or("Invalid bad LBA"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let disputed = snapshot.metadata()["read_conflicts"]["conflicts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| b["lba"].as_u64().ok_or("Invalid conflict LBA"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let impact = evidence_impact::inspect_snapshot(&snapshot, disk, None)?;
    snapshot.unchanged()?;
    Ok(
        json!({"schema_version":1,"disk":disk,"attempt":impact["attempt"],"source_sha256":snapshot.metadata()["sha256"],"snapshot_binding":snapshot.binding(),
        "physical_media_access":false,"project_modified":false,"map_known":snapshot.map_known,
        "missing_sectors":missing.len(),"disputed_sectors":disputed.len(),"attention_required":impact["attention_required"],
        "unavailable_selected_image_ranges":if snapshot.map_known {Some(ranges(&missing))}else{None},
        "disputed_observed_byte_ranges":ranges(&disputed),"filesystem_impact":impact,
        "note":"Offsets are exact for this 512-byte-sector image. Missing bytes remain placeholders, disputed bytes retain both versions; neither is certified customer content. Incomplete allocation tails and unknown directory ownership are reported separately in filesystem_impact. Unavailable here does not mean globally unrecoverable from other evidence."}),
    )
}
pub fn render(v: &Value) -> String {
    let describe = |key: &str| {
        v[key]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        format!(
                            "LBA {}..{} / image bytes {} + {}",
                            r["first_lba"],
                            r["last_lba_inclusive"],
                            r["byte_offset"],
                            r["byte_length"]
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|| "Unknown sector map".into())
    };
    format!(
        "Disk {} attempt {} / {} missing / {} disputed\nMissing image ranges:\n{}\nDisputed observed ranges:\n{}\n{}\n{}",
        v["disk"],
        v["attempt"],
        v["missing_sectors"],
        v["disputed_sectors"],
        describe("unavailable_selected_image_ranges"),
        describe("disputed_observed_byte_ranges"),
        evidence_impact::render(&v["filesystem_impact"]),
        v["note"].as_str().unwrap_or("")
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence_impact::tests::{Fixture, image, tree};
    #[test]
    fn one_tens_hundreds_and_empty_ranges_are_exact_clipped_and_source_preserving() {
        for bad in [
            vec![],
            vec![100],
            (100..130).collect(),
            (100..801).collect(),
        ] {
            let p = Fixture::new();
            let bytes = image();
            p.save(1, &bytes, &bad);
            let before = tree(p.0.root());
            let v = inspect(&p.0, 1, None).unwrap();
            assert_eq!(v["source_sha256"].as_str().unwrap().len(), 64);
            assert_eq!(v["missing_sectors"], bad.len());
            assert_eq!(
                v["unavailable_selected_image_ranges"]
                    .as_array()
                    .unwrap()
                    .len(),
                usize::from(!bad.is_empty())
            );
            if let Some(first) = bad.first() {
                assert_eq!(
                    v["unavailable_selected_image_ranges"][0]["byte_offset"],
                    first * 512
                );
                assert_eq!(
                    v["unavailable_selected_image_ranges"][0]["byte_length"],
                    bad.len() * 512
                );
            }
            assert_eq!(before, tree(p.0.root()));
        }
    }
    #[test]
    fn disjoint_ranges_never_swallow_readable_bytes() {
        let r = ranges(&[1, 2, 4, 7, 8, 9].into_iter().collect());
        assert_eq!(r.len(), 3);
        assert_eq!(r[0]["byte_length"], 1024);
        assert_eq!(r[1]["first_lba"], 4);
        assert_eq!(r[2]["byte_length"], 1536);
    }
    #[test]
    fn automatic_range_reports_are_immutable_reused_and_tamper_refused() {
        let p = Fixture::new();
        p.save(1, &image(), &[100]);
        let metadata = p.0.images_dir().join("001_attempt_001.json");
        let mut m: Value = serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
        m["fluxvault_version"] = json!("fixture");
        m["sector_retries"] = json!(0);
        m["retry_recovered_sectors"] = json!(0);
        m["geometry"]["format_guess"] = json!("fixture");
        m["bad_sectors"] = json!([{"lba":100,"cylinder":2,"head":1,"sector":11}]);
        std::fs::write(metadata, serde_json::to_vec(&m).unwrap()).unwrap();
        let first = publish_all(&p.0, &|_| {}).unwrap();
        assert_eq!(first.len(), 1);
        let content = std::fs::read(&first[0]).unwrap();
        assert_eq!(publish_all(&p.0, &|_| {}).unwrap(), first);
        assert_eq!(std::fs::read(&first[0]).unwrap(), content);
        std::fs::write(&first[0], b"edited operator file").unwrap();
        assert!(publish_all(&p.0, &|_| {}).is_err());
        assert_eq!(std::fs::read(&first[0]).unwrap(), b"edited operator file");
    }
}
