//! Saved FAT12 dependency inspection. Never extracts files or opens a drive.
//! A dependency is not proof of corruption, identity, or document semantics.
use crate::{fat12, project::ProjectState, sector_inspection};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const WARNING: &str = "Saved evidence only. Mapped dependencies are not proof of original content, independent flux CRC, or customer completeness. A disputed readable sector keeps its observed bytes; it is not silently zeroed or repaired. Unknown directory entries and unmapped tails cannot be attributed to invented files. Deleted, carved, manual and converted files are outside this live FAT-chain map.";
const MAX_RELATIONS: usize = 65_536;

fn troubles(snapshot: &sector_inspection::Snapshot) -> Result<BTreeMap<u64, Value>, String> {
    let mut result = BTreeMap::new();
    for b in snapshot.metadata()["bad_sectors"]
        .as_array()
        .ok_or("Missing saved sector map")?
    {
        let lba = b["lba"].as_u64().ok_or("Invalid saved LBA")?;
        result.insert(
            lba,
            json!({"lba":lba,"kind":"unreadable_or_within_reader_conflicting","versions":null}),
        );
    }
    if let Some(value) = snapshot
        .metadata()
        .get("read_conflicts")
        .filter(|v| !v.is_null())
    {
        let confirmation: crate::read_conflicts::Confirmation =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        for conflict in confirmation.conflicts {
            result.insert(conflict.lba, json!({"lba":conflict.lba,"kind":"confirmed_cross_reader_conflict","versions":conflict}));
        }
    }
    Ok(result)
}

fn role(layout: Option<&fat12::Layout>, lba: u64) -> Value {
    let Some(l) = layout else {
        return json!({"region":"unknown_layout"});
    };
    let n = lba as usize;
    if n == 0 {
        json!({"region":"boot"})
    } else if n < l.reserved_sectors {
        json!({"region":"reserved"})
    } else if n < l.root_start {
        json!({"region":"fat","copy":(n-l.reserved_sectors)/l.sectors_per_fat,"sector_in_copy":(n-l.reserved_sectors)%l.sectors_per_fat})
    } else if n < l.data_start {
        json!({"region":"root_directory"})
    } else if n < l.data_start + l.clusters * l.sectors_per_cluster {
        json!({"region":"data","cluster":2+(n-l.data_start)/l.sectors_per_cluster})
    } else {
        json!({"region":"trailing_volume_sectors"})
    }
}

fn origin(snapshot: &sector_inspection::Snapshot, lba: u64) -> Value {
    // An offline source reference describes an upstream sector, not necessarily
    // a physical read; retain the transformations and do not invent a pass.
    json!({"saved_attempt":snapshot.metadata()["attempt_number"],
        "source_backend":snapshot.metadata()["source_backend"],
        "source_device":snapshot.metadata()["source_device"],
        "offline_source":snapshot.origins.as_ref().map(|a| &a[lba as usize]),
        "flux_sector":snapshot.flux.as_ref().map(|a| &a["sectors"][lba as usize]),
        "physical_pass_detail":if snapshot.flux.is_some() {"recorded_flux_sector_and_stage_settings"} else if snapshot.origins.is_some() {"follow_offline_source_attempt_and_lba"} else {"not_resolved_per_sector_by_this_report"}})
}

fn records(
    analysis: &fat12::Analysis,
) -> Vec<(&fat12::FileRecord, Option<&fat12::UnrecoveredFile>)> {
    analysis
        .recovered_files
        .iter()
        .map(|f| (f, None))
        .chain(
            analysis
                .unrecovered_files
                .iter()
                .map(|f| (&f.record, Some(f))),
        )
        .collect()
}

fn base(snapshot: &sector_inspection::Snapshot, disk: u32) -> Value {
    json!({"schema_version":1,"disk":disk,"attempt":snapshot.metadata()["attempt_number"],
        "source_image":snapshot.metadata()["image_file"],"source_image_sha256":snapshot.metadata()["sha256"],
        "source_backend":snapshot.metadata()["source_backend"],"map_verified":snapshot.map_known,
        "physical_media_access":false,"files_written":0,"customer_delivery_certified":false,
        "independent_flux_crc_verified":false,"warning":WARNING,
        "flux_stages":snapshot.flux.as_ref().map(|v| &v["stages"]),
        "flux_policy":snapshot.flux.as_ref().map(|v| &v["policy"])})
}

fn normalize(path: &str) -> Result<String, String> {
    let p = path.replace('\\', "/");
    if p.is_empty()
        || p.len() > 4096
        || p.chars().any(|c| c.is_control() || ":*?\"<>|".contains(c))
        || p.split('/')
            .any(|s| s.is_empty() || matches!(s, "." | ".."))
    {
        return Err("Use a relative live FAT file path, not a drive path or traversal".into());
    }
    Ok(p.to_lowercase())
}

/// File selection is exact apart from slash style and Unicode lowercasing. No
/// wildcard, basename guess, recursive host lookup, or extraction is performed.
pub fn inspect(
    project: &ProjectState,
    disk: u32,
    attempt: Option<u32>,
    file: Option<&str>,
) -> Result<Value, String> {
    let requested = file.map(normalize).transpose()?;
    let snapshot = sector_inspection::load_snapshot(project, disk, attempt)?;
    let trouble = troubles(&snapshot)?;
    let analysis = if !snapshot.map_known {
        Err("Saved acquisition map lacks a complete matching log; file attribution refused".into())
    } else if snapshot.metadata()["geometry"]["bytes_per_sector"] != 512 {
        Err("Live FAT dependency mapping requires 512-byte sectors".into())
    } else {
        let bad = snapshot.metadata()["bad_sectors"]
            .as_array()
            .ok_or("Missing map")?
            .iter()
            .map(|b| b["lba"].as_u64().ok_or("Invalid LBA"))
            .collect::<Result<Vec<_>, _>>()?;
        fat12::analyze(&snapshot.bytes, &bad)
    };
    let mut result = base(&snapshot, disk);
    result["mode"] = json!(if requested.is_some() {
        "file_trace"
    } else {
        "sector_impact"
    });
    result["filesystem_error"] = json!(analysis.as_ref().err());
    let attention = !trouble.is_empty()
        || !snapshot.map_known
        || snapshot.metadata()["status"] == "DERIVED"
        || analysis.is_err();
    result["attention_required"] = json!(attention);
    if let Some(requested) = requested {
        let analysis = analysis
            .as_ref()
            .map_err(|e| format!("Cannot trace live file: {e}"))?;
        let matches = records(analysis)
            .into_iter()
            .filter(|(f, _)| normalize(&f.path).ok().as_ref() == Some(&requested))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(if matches.is_empty() {"No uniquely mapped live FAT file matches this path; use recovery impact N to inspect directory gaps"} else {"Ambiguous live FAT file path; trace refused"}.into());
        }
        let (f, incomplete) = matches[0];
        let data = f.data_lbas.iter().enumerate().map(|(i,lba)|json!({"lba":lba,"file_offset":i*512,
            "bytes":f.bytes.saturating_sub(i*512).min(512),"problem":trouble.get(lba),"origin":origin(&snapshot,*lba)})).collect::<Vec<_>>();
        let metadata = f
            .metadata_lbas
            .iter()
            .map(|lba| {
                json!({"lba":lba,"region":role(Some(&analysis.layout),*lba),
            "problem":trouble.get(lba),"origin":origin(&snapshot,*lba)})
            })
            .collect::<Vec<_>>();
        let affected = f
            .data_lbas
            .iter()
            .chain(&f.metadata_lbas)
            .any(|l| trouble.contains_key(l));
        result["file"] = json!({"path":f.path,"bytes":f.bytes,"payload_sha256":if f.sha256.is_empty() {None} else {Some(&f.sha256)},
            "state":if incomplete.is_some() {"incomplete_or_ambiguous_chain"} else if affected {"observed_complete_payload_with_disputed_dependencies"} else {"mapped_complete_saved_payload"},
            "directory_entry_offset":f.directory_entry_offset,"data":data,"metadata_dependencies":metadata,
            "fat_links":f.fat_links,"name_method":f.name_method,"layout_evidence":analysis.layout_evidence,
            "reason":incomplete.map(|u|&u.reason),"unreadable_ranges":incomplete.map(|u|&u.unreadable_ranges),
            "unmapped_tail_bytes":incomplete.map_or(0,|u|u.unmapped_tail_bytes)});
        result["attention_required"] = json!(
            incomplete.is_some()
                || affected
                || analysis.layout_evidence.warning.is_some()
                || snapshot.metadata()["status"] == "DERIVED"
        );
    } else {
        let mut edges: BTreeMap<u64, Vec<Value>> = BTreeMap::new();
        let mut relations = 0;
        let mut limited = false;
        let mut affected_files = 0;
        if let Ok(a) = &analysis {
            for (f, u) in records(a) {
                let mut affected = false;
                for (i, lba) in f.data_lbas.iter().enumerate() {
                    if trouble.contains_key(lba) {
                        affected = true;
                        if relations < MAX_RELATIONS {
                            edges.entry(*lba).or_default().push(json!({"path":f.path,"dependency":"payload","file_offset":i*512,"bytes":f.bytes.saturating_sub(i*512).min(512),"incomplete":u.is_some()}));
                            relations += 1;
                        } else {
                            limited = true
                        }
                    }
                }
                for lba in &f.metadata_lbas {
                    if trouble.contains_key(lba) {
                        affected = true;
                        if relations < MAX_RELATIONS {
                            edges.entry(*lba).or_default().push(json!({"path":f.path,"dependency":"name_layout_or_allocation_metadata","incomplete":u.is_some()}));
                            relations += 1;
                        } else {
                            limited = true
                        }
                    }
                }
                affected_files += usize::from(affected);
            }
        }
        let sectors = trouble.iter().map(|(lba,problem)|{
            let references=edges.remove(lba).unwrap_or_default();
            json!({"lba":lba,"problem":problem,"region":role(analysis.as_ref().ok().map(|a|&a.layout),*lba),
                "origin":origin(&snapshot,*lba),"file_dependencies":references,
                "attribution":if !references.is_empty() {"known_live_file_dependencies"} else {"no_known_file_dependency_not_proof_of_unused_or_harmless"}})
        }).collect::<Vec<_>>();
        result["sectors"] = json!(sectors);
        result["counts"] = json!({"problem_sectors":trouble.len(),"known_files_with_problem_dependencies":affected_files,
            "mapped_complete_payloads":analysis.as_ref().map_or(0,|a|a.recovered_files.len()),
            "incomplete_or_ambiguous_files":analysis.as_ref().map_or(0,|a|a.unrecovered_files.len()),
            "recorded_dependencies":relations});
        result["dependencies_truncated"] = json!(limited);
        result["layout_evidence"] = json!(analysis.as_ref().ok().map(|a| &a.layout_evidence));
        result["directory_gaps"] = json!(analysis.as_ref().ok().map(|a| &a.directory_gaps));
        result["skipped_entries"] = json!(analysis.as_ref().ok().map(|a| &a.skipped));
        result["unmapped_tails"]=json!(analysis.as_ref().ok().into_iter().flat_map(|a|&a.unrecovered_files)
            .filter(|u|u.unmapped_tail_bytes>0).map(|u|json!({"path":u.record.path,"bytes":u.unmapped_tail_bytes,"reason":u.reason})).collect::<Vec<_>>());
        result["attention_required"] = json!(
            attention
                || limited
                || analysis.as_ref().is_ok_and(|a| !a.skipped.is_empty()
                    || !a.directory_gaps.is_empty()
                    || a.layout_evidence.warning.is_some())
        );
    }
    snapshot.unchanged()?;
    Ok(result)
}

pub fn render(value: &Value) -> String {
    let mut out = format!(
        "Disk {:03} / saved attempt {:03} / {}\nImage: {}\nSHA-256: {}\n{}\n",
        value["disk"].as_u64().unwrap_or(0),
        value["attempt"].as_u64().unwrap_or(0),
        value["mode"].as_str().unwrap_or("?"),
        value["source_image"].as_str().unwrap_or("?"),
        value["source_image_sha256"].as_str().unwrap_or("?"),
        WARNING
    );
    if let Some(error) = value["filesystem_error"].as_str() {
        out.push_str(&format!("LAYOUT UNKNOWN: {error}\n"));
    }
    if let Some(warning) = value["layout_evidence"]["warning"]
        .as_str()
        .or_else(|| value["file"]["layout_evidence"]["warning"].as_str())
    {
        out.push_str(&format!("LAYOUT HYPOTHESIS: {warning}\n"));
    }
    if value["mode"] == "file_trace" {
        let f = &value["file"];
        out.push_str(&format!(
            "\n{} | {} bytes | {}\nPayload hash: {}\nUnmapped tail: {} bytes\n",
            f["path"].as_str().unwrap_or("?"),
            f["bytes"],
            f["state"].as_str().unwrap_or("?"),
            f["payload_sha256"],
            f["unmapped_tail_bytes"]
        ));
        if let Some(reason) = f["reason"].as_str() {
            out.push_str(&format!("Incomplete/ambiguous: {reason}\n"));
        }
        if !f["unreadable_ranges"].is_null() {
            out.push_str(&format!("Mapped holes: {}\n", f["unreadable_ranges"]));
        }
        for row in f["data"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "Offset {} + {} bytes -> LBA {}{}\n  Origin: {}\n",
                row["file_offset"],
                row["bytes"],
                row["lba"],
                if row["problem"].is_null() {
                    ""
                } else {
                    " / ATTENTION"
                },
                row["origin"]
            ));
        }
        out.push_str(&format!(
            "Metadata dependencies: {}\n",
            f["metadata_dependencies"]
        ));
    } else {
        out.push_str(&format!(
            "\n{} problem sectors; {} known files depend on them.\n",
            value["counts"]["problem_sectors"],
            value["counts"]["known_files_with_problem_dependencies"]
        ));
        for s in value["sectors"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "LBA {} / {} / {}\n",
                s["lba"],
                s["problem"]["kind"].as_str().unwrap_or("?"),
                s["region"]
            ));
            for dep in s["file_dependencies"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "  {}: {}{}\n",
                    dep["path"].as_str().unwrap_or("?"),
                    dep["dependency"].as_str().unwrap_or("?"),
                    dep["file_offset"]
                        .as_u64()
                        .map_or(String::new(), |n| format!(" at file byte {n}"))
                ));
            }
            if s["file_dependencies"].as_array().is_none_or(Vec::is_empty) {
                out.push_str("  No known file dependency; NOT proof this sector is harmless.\n");
            }
        }
        if value["dependencies_truncated"] == true {
            out.push_str("WARNING: dependency work ceiling reached; listing is not exhaustive.\n");
        }
        let gaps = value["directory_gaps"].as_array().map_or(0, Vec::len);
        let skipped = value["skipped_entries"].as_array().map_or(0, Vec::len);
        let tails = value["unmapped_tails"].as_array().map_or(0, Vec::len);
        out.push_str(&format!("Directory gaps: {gaps}; skipped entries: {skipped}; unmapped tails: {tails}. Use --json for details.\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{fs, path::Path};

    struct Fixture(ProjectState);
    impl Fixture {
        fn new() -> Self {
            Self(
                ProjectState::create_without_session(std::env::temp_dir().join(format!(
                        "fv-impact-{}-{}",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos()
                    )))
                .unwrap(),
            )
        }
        fn save(&self, attempt: u32, original: &[u8], bad: &[u64]) {
            let mut bytes = original.to_vec();
            for lba in bad {
                bytes[*lba as usize * 512..(*lba as usize + 1) * 512].fill(0);
            }
            let sha = format!("{:x}", Sha256::digest(&bytes));
            let stem = format!("001_attempt_{attempt:03}");
            let log = self.0.logs_dir().join(format!("{stem}.log"));
            let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
            fs::write(&log,format!("BEGIN | disk=1 | attempt={attempt}\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\n{}END | status={status} | bytes=1474560 | sha256={sha}\n",bad.iter().map(|l|format!("BAD_SECTOR | LBA={l}\n")).collect::<String>())).unwrap();
            fs::write(self.0.images_dir().join(format!("{stem}.img")), bytes).unwrap();
            fs::write(self.0.images_dir().join(format!("{stem}.json")),serde_json::to_vec(&json!({"disk_number":1,"attempt_number":attempt,"status":status,
                "source_backend":"synthetic-test","source_device":"none","timestamp_unix_ms":attempt,
                "image_file":format!("{stem}.img"),"log_file":log,"sha256":sha,
                "total_sectors":2880,"bytes_written":1474560,"bad_sector_count":bad.len(),"bad_sectors":bad.iter().map(|l|json!({"lba":l})).collect::<Vec<_>>(),
                "geometry":{"cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":1474560}})).unwrap()).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(self.0.root()).unwrap();
        }
    }
    fn image() -> Vec<u8> {
        let mut bytes = fat12::tests::image();
        fat12::tests::file(&mut bytes, 19 * 512, b"CHAIN   TXT", 2, &vec![b'A'; 1100]);
        for copy in 0..2 {
            fat12::tests::set_fat(&mut bytes, copy, 2, 5);
            fat12::tests::set_fat(&mut bytes, copy, 5, 3);
            fat12::tests::set_fat(&mut bytes, copy, 3, 0xfff);
        }
        bytes[33 * 512..34 * 512].fill(b'A');
        bytes[36 * 512..37 * 512].fill(b'B');
        bytes[34 * 512..35 * 512].fill(b'C');
        bytes
    }
    fn tree(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut rows = Vec::new();
        for e in fs::read_dir(root).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                rows.extend(tree(&e.path()))
            } else {
                rows.push((e.path(), fs::read(e.path()).unwrap()));
            }
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }
    #[test]
    fn fragmented_payload_order_eof_and_metadata_are_exact_without_writes() {
        let f = Fixture::new();
        f.save(1, &image(), &[]);
        let before = tree(f.0.root());
        let r = inspect(&f.0, 1, None, Some("chain.txt")).unwrap();
        assert_eq!(r["file"]["state"], "mapped_complete_saved_payload");
        assert_eq!(r["attention_required"], false);
        let data = r["file"]["data"].as_array().unwrap();
        assert_eq!(
            data.iter()
                .map(|r| r["lba"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [33, 36, 34]
        );
        assert_eq!(data[2]["file_offset"], 1024);
        assert_eq!(data[2]["bytes"], 76);
        let expected = [vec![b'A'; 512], vec![b'B'; 512], vec![b'C'; 76]].concat();
        assert_eq!(
            r["file"]["payload_sha256"],
            format!("{:x}", Sha256::digest(expected))
        );
        assert!(
            r["file"]["metadata_dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["lba"] == 19)
        );
        assert_eq!(
            r["file"]["data"][0]["origin"]["physical_pass_detail"],
            "not_resolved_per_sector_by_this_report"
        );
        assert!(render(&r).contains("Offset 1024 + 76 bytes -> LBA 34"));
        assert_eq!(tree(f.0.root()), before);
    }
    #[test]
    fn unreadable_payload_has_exact_hole_and_impact_offset_not_a_fake_hash() {
        let f = Fixture::new();
        f.save(1, &image(), &[36]);
        let r = inspect(&f.0, 1, None, Some("CHAIN.TXT")).unwrap();
        assert_eq!(r["file"]["state"], "incomplete_or_ambiguous_chain");
        assert!(r["file"]["payload_sha256"].is_null());
        assert_eq!(r["file"]["unreadable_ranges"][0]["file_offset"], 512);
        assert_eq!(r["file"]["unreadable_ranges"][0]["bytes"], 512);
        assert_eq!(r["attention_required"], true);
        let r = inspect(&f.0, 1, None, None).unwrap();
        assert_eq!(r["sectors"][0]["file_dependencies"][0]["path"], "CHAIN.TXT");
        assert_eq!(r["sectors"][0]["file_dependencies"][0]["file_offset"], 512);
        assert_eq!(r["counts"]["known_files_with_problem_dependencies"], 1);
        assert!(render(&r).contains("at file byte 512"));
    }
    #[test]
    fn missing_directory_and_unclaimed_sector_never_mean_harmless() {
        let f = Fixture::new();
        f.save(1, &image(), &[19, 1000]);
        let r = inspect(&f.0, 1, None, None).unwrap();
        assert_eq!(r["sectors"][0]["region"]["region"], "root_directory");
        assert_eq!(
            r["sectors"][0]["attribution"],
            "no_known_file_dependency_not_proof_of_unused_or_harmless"
        );
        assert_eq!(r["directory_gaps"], json!([19]));
        assert!(inspect(&f.0, 1, None, Some("CHAIN.TXT")).is_err());
        assert!(render(&r).contains("NOT proof this sector is harmless"));
    }
    #[test]
    fn unlocated_tail_remains_unmapped_not_guessed_contiguous() {
        let f = Fixture::new();
        let mut bytes = image();
        for c in 0..2 {
            fat12::tests::set_fat(&mut bytes, c, 2, 0);
        }
        f.save(1, &bytes, &[]);
        let r = inspect(&f.0, 1, None, Some("CHAIN.TXT")).unwrap();
        assert_eq!(r["file"]["unmapped_tail_bytes"], 588);
        assert_eq!(r["file"]["data"].as_array().unwrap().len(), 1);
        let r = inspect(&f.0, 1, None, None).unwrap();
        assert_eq!(r["unmapped_tails"][0]["bytes"], 588);
        assert_eq!(r["attention_required"], true);
    }
    #[test]
    fn unknown_logs_changed_images_and_unbound_conflicts_are_refused_or_unknown() {
        let f = Fixture::new();
        f.save(1, &image(), &[36]);
        let meta = f.0.images_dir().join("001_attempt_001.json");
        fs::write(f.0.logs_dir().join("001_attempt_001.log"), "unknown note").unwrap();
        let r = inspect(&f.0, 1, None, None).unwrap();
        assert_eq!(r["map_verified"], false);
        assert_eq!(r["sectors"][0]["region"]["region"], "unknown_layout");
        assert!(
            inspect(&f.0, 1, None, Some("CHAIN.TXT"))
                .unwrap_err()
                .contains("matching log")
        );
        f.save(1, &image(), &[]);
        let mut v: Value = serde_json::from_slice(&fs::read(&meta).unwrap()).unwrap();
        v["read_conflicts"] = json!({"fake":true});
        fs::write(&meta, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(inspect(&f.0, 1, None, None).is_err());
        f.save(1, &image(), &[]);
        fs::write(
            f.0.images_dir().join("001_attempt_001.img"),
            vec![0; 2880 * 512],
        )
        .unwrap();
        assert!(
            inspect(&f.0, 1, None, None)
                .unwrap_err()
                .contains("SHA-256")
        );
    }
    #[test]
    fn explicit_attempt_and_cli_contracts_use_only_saved_evidence() {
        let f = Fixture::new();
        f.save(1, &image(), &[]);
        f.save(2, &image(), &[36]);
        assert_eq!(inspect(&f.0, 1, None, None).unwrap()["attempt"], 1);
        let before = tree(f.0.root());
        let run = |args: &[&str]| {
            crate::cli::run(
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                f.0.root(),
            )
        };
        let r = run(&["recovery", "impact", "1", "--attempt", "2", "--json"]).unwrap();
        assert_eq!(r.exit_code, 3);
        assert_eq!(
            serde_json::from_str::<Value>(&r.output).unwrap()["files_written"],
            0
        );
        assert_eq!(
            run(&["recovery", "trace", "1", "CHAIN.TXT", "--json"])
                .unwrap()
                .exit_code,
            0
        );
        for args in [
            vec!["recovery", "impact", "1", "--lba", "0"],
            vec!["recovery", "impact", "1", "--include-deleted"],
            vec!["recovery", "trace", "1", "CHAIN.TXT", "--drive", "A:"],
        ] {
            assert!(run(&args).is_err());
        }
        for path in [
            "../CHAIN.TXT",
            "A:\\CHAIN.TXT",
            "\\\\.\\PhysicalDrive0",
            "*.TXT",
            "/CHAIN.TXT",
            "CHAIN.TXT/",
            "CHAIN.TXT\n",
        ] {
            assert!(inspect(&f.0, 1, None, Some(path)).is_err());
        }
        assert_eq!(normalize("folder\\file.txt").unwrap(), "folder/file.txt");
        assert!(inspect(&f.0, 0, None, None).is_err());
        assert!(inspect(&f.0, 1, Some(0), None).is_err());
        assert_eq!(tree(f.0.root()), before);
    }
    #[test]
    fn snapshot_change_is_detected_before_diagnostic_publication() {
        let f = Fixture::new();
        f.save(1, &image(), &[]);
        let snapshot = sector_inspection::load_snapshot(&f.0, 1, None).unwrap();
        fs::write(f.0.logs_dir().join("001_attempt_001.log"), "changed").unwrap();
        assert!(
            snapshot
                .unchanged()
                .unwrap_err()
                .contains("changed during inspection")
        );
    }
}
