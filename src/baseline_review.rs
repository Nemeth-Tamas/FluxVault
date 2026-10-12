//! Reference candidates versus actual saved acquisition bytes and live chains.
//! Exact observations only: raw signatures do not imply live/deleted ownership.
use super::*;
use serde_json::{Value, json};
use std::io::Cursor;

fn read_member(z: &mut zip::ZipArchive<File>, name: &str, bound: u64) -> Result<Vec<u8>, String> {
    let mut member = z.by_name(name).map_err(|e| e.to_string())?;
    if member.size() > bound {
        return Err("Reference review member exceeds bound".into());
    }
    let mut bytes = Vec::new();
    member
        .by_ref()
        .take(bound + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > bound {
        return Err("Reference review member grew beyond bound".into());
    }
    Ok(bytes)
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn occurrences(haystack: &[u8], needle: &[u8]) -> Value {
    if needle.len() < 128 {
        return json!({"tested":false,"reason":"tiny_signature_stub_not_a_whole_file"});
    }
    let mut prefix = vec![0usize; needle.len()];
    let mut matched = 0;
    for n in 1..needle.len() {
        while matched > 0 && needle[n] != needle[matched] {
            matched = prefix[matched - 1];
        }
        if needle[n] == needle[matched] {
            matched += 1;
        }
        prefix[n] = matched;
    }
    let mut offsets = Vec::new();
    matched = 0;
    for (n, b) in haystack.iter().enumerate() {
        while matched > 0 && *b != needle[matched] {
            matched = prefix[matched - 1];
        }
        if *b == needle[matched] {
            matched += 1;
        }
        if matched == needle.len() {
            offsets.push(n + 1 - matched);
            if offsets.len() == 16 {
                return json!({"tested":true,"offsets":offsets,"further_occurrences_unknown":true});
            }
            matched = prefix[matched - 1];
        }
    }
    json!({"tested":true,"offsets":offsets,"further_occurrences_unknown":false})
}
fn streams(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut cfb = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let paths = cfb
        .walk()
        .filter(|e| e.is_stream())
        .map(|e| e.path().to_path_buf())
        .collect::<Vec<_>>();
    if paths.len() > 256 {
        return Err("Reference CFB exceeds 256 stream bound".into());
    }
    let mut result = BTreeMap::new();
    let mut total = 0u64;
    for path in paths {
        let mut data = Vec::new();
        cfb.open_stream(&path)
            .map_err(|e| e.to_string())?
            .take(MAX_FILE + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        total += data.len() as u64;
        if total > MAX_FILE {
            return Err("Reference CFB stream bytes exceed bound".into());
        }
        result.insert(path.to_string_lossy().replace('\\', "/"), data);
    }
    Ok(result)
}
fn cfb_comparison(old: &[u8], new: &[u8]) -> Value {
    let (a, b) = match (streams(old), streams(new)) {
        (Ok(a), Ok(b)) => (a, b),
        (a, b) => {
            return json!({"compared":false,"reference_error":a.err(),"current_error":b.err()});
        }
    };
    let keys = a.keys().chain(b.keys()).cloned().collect::<BTreeSet<_>>();
    let mut rows = Vec::new();
    let mut properties_only = true;
    let mut differences = 0;
    for key in keys {
        let x = a.get(&key);
        let y = b.get(&key);
        let same = x == y;
        differences += usize::from(!same);
        if !same
            && ![
                "/\u{5}SummaryInformation",
                "/\u{5}DocumentSummaryInformation",
            ]
            .contains(&key.as_str())
        {
            properties_only = false;
        }
        let changes = x.zip(y).map(|(x, y)| byte_differences(x, y));
        rows.push(json!({"stream":key,"same_bytes":same,"reference_bytes":x.map(Vec::len),"current_bytes":y.map(Vec::len),"reference_sha256":x.map(|v|sha(v)),"current_sha256":y.map(|v|sha(v)),"byte_changes":changes}));
    }
    json!({"compared":true,"streams":rows,"allocated_streams_equal":differences==0,"only_summary_property_streams_differ":properties_only && differences>0,
        "limits":"Compares allocated stream bytes only. Does not certify container slack, storage metadata, renderability, formatting, ownership or semantics; differing property streams do not establish who/what changed them."})
}
fn byte_differences(a: &[u8], b: &[u8]) -> Value {
    let mut ranges = Vec::new();
    let mut differing = 0;
    let mut n = 0;
    let count = a.len().max(b.len());
    let mut truncated = false;
    while n < count {
        if a.get(n) == b.get(n) {
            n += 1;
            continue;
        }
        let start = n;
        while n < count && a.get(n) != b.get(n) {
            differing += 1;
            n += 1;
        }
        if ranges.len() < 32 {
            ranges.push(json!({"offset":start,"bytes":n-start}));
        } else {
            truncated = true;
        }
    }
    json!({"different_bytes":differing,"ranges":ranges,"further_ranges_not_displayed":truncated})
}
fn mapped_occurrences(image: &[u8], payload: &[u8], bad: Option<&BTreeSet<u64>>) -> Value {
    let mut value = occurrences(image, payload);
    let offsets = value["offsets"].as_array().cloned().unwrap_or_default();
    let matches=offsets.iter().map(|v|{
        let start=v.as_u64().unwrap();let end=start+payload.len() as u64;
        let missing=bad.map(|b|b.range(start/512..end.div_ceil(512)).copied().collect::<Vec<_>>());
        json!({"byte_offset":start,"known_unreadable_or_disputed_lbas":missing,"map_available":bad.is_some()})
    }).collect::<Vec<_>>();
    value["mapped_matches"] = json!(matches);
    value
}
fn complete_payload(image: &[u8], r: &crate::fat12::FileRecord) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for lba in &r.data_lbas {
        let start = usize::try_from(*lba).ok()?.checked_mul(512)?;
        bytes.extend_from_slice(image.get(start..start + 512)?);
    }
    if bytes.len() < r.bytes {
        return None;
    }
    bytes.truncate(r.bytes);
    Some(bytes)
}
pub fn inspect(project: &ProjectState, baseline: &Path, disk: u32) -> Result<Value, String> {
    if disk == 0 {
        return Err("Reference review needs a positive disk label".into());
    }
    crate::processing::validate_workspace(project)?;
    crate::sector_inspection::regular(baseline, false)?;
    let _owner = crate::project_work::reserve(project.root())?;
    let snapshot = crate::sector_inspection::load_snapshot(project, disk, None)?;
    if !snapshot.map_known {
        return Err("Reference review requires a complete current acquisition map".into());
    }
    let mut bad = snapshot.metadata()["bad_sectors"]
        .as_array()
        .ok_or("Missing selected map")?
        .iter()
        .map(|b| b["lba"].as_u64().ok_or("Invalid selected LBA"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    for row in snapshot.metadata()["read_conflicts"]["conflicts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        bad.insert(row["lba"].as_u64().ok_or("Invalid disputed review LBA")?);
    }
    let current_analysis =
        crate::fat12::analyze(&snapshot.bytes, &bad.iter().copied().collect::<Vec<_>>());
    let archive_hash = hash_file(baseline, 4 * 1024 * 1024 * 1024)?.1;
    let mut archive = zip::ZipArchive::new(File::open(baseline).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("Reference review entry count exceeds bound".into());
    }
    let mut members = BTreeSet::new();
    let mut raw_names = Vec::new();
    let prefix = format!("Extracted/{disk:03}/");
    let mut image_name = None;
    let mut filelist = None;
    for n in 0..archive.len() {
        let entry = archive.by_index(n).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = safe_relative(entry.name())?;
        if !members.insert(name.to_lowercase())
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("Duplicate/linked reference member refused".into());
        }
        if [
            format!("Images/{disk:03}.bin"),
            format!("Images/{disk:03}.img"),
        ]
        .contains(&name)
            && image_name.replace(name.clone()).is_some()
        {
            return Err("Ambiguous reference image".into());
        }
        if name == format!("{prefix}filelist.txt") {
            filelist = Some(name.clone());
        }
        if name.starts_with(&prefix) {
            raw_names.push(name);
        }
    }
    let old_image_name = image_name.ok_or("Reference disk image missing")?;
    let old_image = read_member(
        &mut archive,
        &old_image_name,
        crate::fat12::MAX_IMAGE_BYTES as u64,
    )?;
    let old_image_hash = sha(&old_image);
    let old_log_name = format!("Logs/{disk:03}.log");
    let log = if members.contains(&old_log_name.to_lowercase()) {
        Some(read_member(&mut archive, &old_log_name, 8 * 1024 * 1024)?)
    } else {
        None
    };
    let parsed = log
        .as_ref()
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|s| {
            crate::legacy_logs::parse_archiver_log(s.trim_start_matches('\u{feff}')).ok()
        });
    let old_map_bound = parsed.as_ref().is_some_and(|l| {
        l.end_seen
            && matches!(
                l.status,
                crate::legacy_logs::ArchiverLogStatus::Ok
                    | crate::legacy_logs::ArchiverLogStatus::Partial
            )
            && l.sha256
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&old_image_hash))
            && l.bytes_written == Some(old_image.len() as u64)
    });
    let dmde = log
        .as_ref()
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|s| crate::dmde_logs::parse_dmde_log(s).ok());
    let dmde_extent_associated = dmde.as_ref().is_some_and(|l| {
        l.status != crate::dmde_logs::DmdeLogStatus::InProgress
            && l.sector_size == Some(512)
            && l.highest_sector_exclusive == old_image.len() as u64 / 512
            && l.copied_sectors + l.bad_sectors.len() == old_image.len() / 512
            && l.bad_sectors
                .iter()
                .all(|n| *n < old_image.len() as u64 / 512)
    });
    let old_bad = if old_map_bound {
        Some(
            parsed
                .as_ref()
                .unwrap()
                .bad_sectors
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
        )
    } else if dmde_extent_associated {
        Some(
            dmde.as_ref()
                .unwrap()
                .bad_sectors
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
        )
    } else {
        None
    };
    let old_analysis = old_bad.as_ref().and_then(|bad| {
        crate::fat12::analyze(&old_image, &bad.iter().copied().collect::<Vec<_>>()).ok()
    });
    let classifications = if let Some(name) = filelist {
        dmde_filelist(&read_member(&mut archive, &name, MAX_FILE)?)?
    } else {
        BTreeMap::new()
    };
    let current_blocks = snapshot
        .bytes
        .chunks_exact(512)
        .enumerate()
        .filter(|(n, _)| !bad.contains(&(*n as u64)))
        .map(|(_, b)| sha(b))
        .collect::<BTreeSet<_>>();
    let mut rows = Vec::new();
    let mut total = 0u64;
    for name in raw_names {
        crate::cancellation::check()?;
        let payload = read_member(&mut archive, &name, MAX_FILE)?;
        total += payload.len() as u64;
        if total > 128 * 1024 * 1024 {
            return Err("Reference review payload budget exceeded".into());
        }
        let relative = name.strip_prefix(&prefix).unwrap();
        let digest = sha(&payload);
        let category = category(relative, payload.len() as u64);
        let origin = reference_origin(relative);
        let normalized = crate::conversion::comparison_path(Path::new(relative))?
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        let state = classifications
            .get(&normalized)
            .filter(|(n, _)| *n == payload.len() as u64)
            .map(|(_, s)| *s)
            .unwrap_or("unknown");
        let live_matches = current_analysis
            .as_ref()
            .ok()
            .map(|a| {
                a.recovered_files
                    .iter()
                    .filter(|r| r.sha256 == digest && r.bytes == payload.len())
                    .map(|r| r.path.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let old_live_matches = old_analysis
            .as_ref()
            .map(|a| {
                a.recovered_files
                    .iter()
                    .filter(|r| r.sha256 == digest && r.bytes == payload.len())
                    .map(|r| r.path.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let current_named = current_analysis.as_ref().ok().and_then(|a| {
            a.recovered_files.iter().find(|r| {
                crate::conversion::comparison_path(Path::new(&r.path))
                    .ok()
                    .is_some_and(|p| {
                        p.to_string_lossy().replace('\\', "/").to_lowercase() == normalized
                    })
            })
        });
        let incomplete = current_analysis.as_ref().ok().and_then(|a| {
            a.unrecovered_files.iter().find(|r| {
                crate::conversion::comparison_path(Path::new(&r.record.path))
                    .ok()
                    .is_some_and(|p| {
                        p.to_string_lossy().replace('\\', "/").to_lowercase() == normalized
                    })
            })
        });
        let classified = if category == "recovery_tool_report" {
            "recovery_tool_report_not_customer_payload"
        } else if category != "payload" {
            "empty_temporary_or_metadata_not_default_customer_payload"
        } else if payload.len() < 128 && origin == "signature_carved" {
            "tiny_signature_stub_not_whole_file"
        } else if !live_matches.is_empty() {
            "byte_identical_current_reachable_file"
        } else if incomplete.is_some() {
            "known_current_live_chain_incomplete"
        } else if state == "deleted" {
            "historical_deleted_candidate_not_default_live_scope"
        } else if origin == "signature_carved" {
            "signature_candidate_ownership_and_full_recoverability_unproven"
        } else if current_named.is_some() {
            "same_named_live_file_different_source_bytes"
        } else {
            "historical_candidate_not_current_reachable_file_ownership_unproven"
        };
        let cfb = if let Some(current) = current_named.filter(|r| r.sha256 != digest) {
            complete_payload(&snapshot.bytes, current).map(|b| cfb_comparison(&payload, &b))
        } else {
            None
        };
        let block_count = payload.len() / 512;
        let absent_blocks = payload
            .chunks_exact(512)
            .filter(|b| !current_blocks.contains(&sha(b)))
            .count();
        rows.push(json!({"reference_member":name,"bytes":payload.len(),"sha256":digest,"origin":origin,"reference_state":state,"classification":classified,
            "current_reachable_exact_matches":live_matches,"reference_reachable_exact_matches":old_live_matches,
            "current_whole_contiguous_occurrences":if category=="recovery_tool_report"{Value::Null}else{mapped_occurrences(&snapshot.bytes,&payload,Some(&bad))},
            "contiguous_match_scope":"stored_image_bytes_can_include_placeholder_bytes_not_recovery_or_ownership_proof",
            "reference_whole_contiguous_occurrences":if category=="recovery_tool_report"{Value::Null}else{mapped_occurrences(&old_image,&payload,old_bad.as_ref())},
            "aligned_full_payload_blocks":block_count,"blocks_not_observed_at_readable_current_LBAs":absent_blocks,
            "block_observations_establish_file_allocation_or_ownership":false,
            "current_named_sha256":current_named.map(|r|&r.sha256),"cfb_stream_comparison":cfb,
            "current_incomplete_chain":incomplete,"customer_delivery_certified":false}));
    }
    snapshot.unchanged()?;
    if hash_file(baseline, 4 * 1024 * 1024 * 1024)?.1 != archive_hash {
        return Err("Reference archive changed during review".into());
    }
    Ok(
        json!({"schema_version":1,"disk":disk,"reference_archive_sha256":archive_hash,"reference_image_sha256":old_image_hash,
        "current_image_sha256":snapshot.metadata()["sha256"],"same_image_bytes":old_image_hash==snapshot.metadata()["sha256"].as_str().unwrap_or(""),
        "reference_acquisition_map_bound":old_map_bound,"reference_DMDE_map_extent_associated":dmde_extent_associated,
        "reference_map_status":if old_map_bound {"hash_and_extent_bound_archiver_log"}else if dmde_extent_associated {"complete_DMDE_extent_association_not_original_hash_certificate"}else{"unavailable_or_contradictory"},
        "reference_bad_lbas":old_bad,"reference_log_sha256":log.as_ref().map(|b|sha(b)),"reference_filesystem_analysis_available":old_analysis.is_some(),
        "current_filesystem_error":current_analysis.err(),"candidates":rows,"physical_media_access":false,"host_tools_invoked":false,"project_modified":false,"archive_modified":false,"attention_required":true,
        "limits":"Classifies observed bytes/known live chains, not historic ownership, semantic completeness or cause of differences. Reference logs whose hash/extent contradict the archived image are not trusted as acquisition maps. Contiguous/block matches can be remnants or duplicates, not file allocation proof. CRC-checked ZIP members are saved extracted evidence, not proof they are reproducible from either saved disk image. No guessed missing bytes, automatic deleted classification, source mutation or new recovery."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linear_search_handles_repeated_prefixes_overlap_and_tiny_stubs() {
        let needle = vec![b'a'; 128];
        let haystack = vec![b'a'; 140];
        let v = occurrences(&haystack, &needle);
        assert_eq!(v["offsets"].as_array().unwrap().len(), 13);
        assert_eq!(v["further_occurrences_unknown"], false);
        let v = occurrences(&[b'a'; 200], &needle);
        assert_eq!(v["offsets"].as_array().unwrap().len(), 16);
        assert_eq!(v["further_occurrences_unknown"], true);
        assert_eq!(occurrences(b"abcd", b"ab")["tested"], false);
    }
    #[test]
    fn malformed_or_noncompound_files_do_not_become_metadata_only_differences() {
        assert_eq!(
            cfb_comparison(b"not CFB", b"also not CFB")["compared"],
            false
        );
    }
}
