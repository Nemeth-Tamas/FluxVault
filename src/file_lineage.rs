//! Uniform saved-file provenance. Observed source extents are verified against
//! bytes; transformations never imply document semantics or invented ownership.
use crate::{
    conversion_run, evidence_impact, extraction, imaging, project::ProjectState, sector_inspection,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};
const MAX_CONTROL: u64 = 8 * 1024 * 1024;
const MAX_PAYLOAD: u64 = 16 * 1024 * 1024;

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn image_binding(snapshot: &sector_inspection::Snapshot) -> Value {
    let mut binding = snapshot.binding();
    for key in [
        "disk_number",
        "attempt_number",
        "image_file",
        "sha256",
        "source_backend",
    ] {
        binding[key] = snapshot.metadata()[key].clone();
    }
    binding
}
fn relative(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path.replace('\\', "/"));
    if p.as_os_str().is_empty()
        || p.components().any(|c| !matches!(c, Component::Normal(_)))
        || path.contains(':')
        || path.chars().any(|c| matches!(c, '*' | '?' | '\0'))
    {
        return Err("File lineage requires an exact project-relative path; no drive, parent, wildcard or alternate stream".into());
    }
    Ok(p)
}
fn confined(project: &ProjectState, rel: &Path, directory: bool) -> Result<PathBuf, String> {
    let mut p = project.root().to_owned();
    sector_inspection::regular(&p, true)?;
    let parts = rel.components().collect::<Vec<_>>();
    for (i, part) in parts.iter().enumerate() {
        let Component::Normal(part) = part else {
            return Err("Unsafe lineage path".into());
        };
        p.push(part);
        sector_inspection::regular(
            &p,
            if i + 1 == parts.len() {
                directory
            } else {
                true
            },
        )?;
    }
    if !p
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(project.root().canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("Lineage path escapes project".into());
    }
    Ok(p)
}
fn read(project: &ProjectState, path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let rel = path
        .strip_prefix(project.root())
        .map_err(|_| "Foreign lineage control")?;
    confined(project, rel, false)?;
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > limit {
        return Err("File lineage input exceeds bound".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("File lineage input grew beyond bound".into());
    }
    Ok(bytes)
}
fn control(project: &ProjectState, path: &Path) -> Result<(Value, String), String> {
    let b = read(project, path, MAX_CONTROL)?;
    Ok((
        serde_json::from_slice(&b).map_err(|e| e.to_string())?,
        sha(&b),
    ))
}
fn source_relative(project: &ProjectState, path: &Path) -> Result<PathBuf, String> {
    sector_inspection::regular(path, false)?;
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    Ok(canonical
        .strip_prefix(project.root().canonicalize().map_err(|e| e.to_string())?)
        .map_err(|_| "Conversion source escapes project")?
        .to_owned())
}
fn extents(
    snapshot: &sector_inspection::Snapshot,
    values: &[crate::carving::Extent],
    payload: &[u8],
) -> Result<Value, String> {
    if !snapshot.map_known || snapshot.metadata()["geometry"]["bytes_per_sector"] != 512 {
        return Err("Lineage extents require a complete 512-byte acquisition map".into());
    }
    let mut rebuilt = Vec::new();
    let mut rows = Vec::new();
    let bad = snapshot.metadata()["bad_sectors"]
        .as_array()
        .ok_or("Missing lineage acquisition map")?
        .iter()
        .map(|v| v["lba"].as_u64().ok_or("Invalid lineage bad LBA"))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    for e in values {
        if e.file_offset != rebuilt.len()
            || e.bytes == 0
            || e.bytes > payload.len()
            || e.source_byte_offset
                .checked_add(e.bytes)
                .is_none_or(|n| n > snapshot.bytes.len())
        {
            return Err("Invalid lineage extent".into());
        }
        rebuilt.extend_from_slice(
            &snapshot.bytes[e.source_byte_offset..e.source_byte_offset + e.bytes],
        );
        let end = e.source_byte_offset + e.bytes;
        for lba in e.source_byte_offset / 512..end.div_ceil(512) {
            if bad.contains(&(lba as u64)) {
                return Err("Lineage extent crosses an unreadable placeholder".into());
            }
            let from = e.source_byte_offset.max(lba * 512);
            let to = end.min((lba + 1) * 512);
            rows.push(json!({"file_offset":e.file_offset+from-e.source_byte_offset,"bytes":to-from,"source_byte_offset":from,"lba":lba,"origin":evidence_impact::origin(snapshot,lba as u64)}));
            if rows.len() > 65536 {
                return Err("Lineage relation bound exceeded".into());
            }
        }
    }
    if rebuilt != payload {
        return Err("Source extents do not reproduce the saved file; no lineage claim".into());
    }
    Ok(json!(rows))
}

fn metadata_origins(snapshot: &sector_inspection::Snapshot, lbas: &[u64]) -> Result<Value, String> {
    if lbas.len() > 8192
        || lbas
            .iter()
            .any(|lba| *lba >= snapshot.bytes.len() as u64 / 512)
    {
        return Err("Native metadata lineage exceeds source sector bounds".into());
    }
    Ok(json!(
        lbas.iter()
            .map(|lba| { json!({"lba":lba,"origin":evidence_impact::origin(snapshot,*lba)}) })
            .collect::<Vec<_>>()
    ))
}

fn trace(project: &ProjectState, disk: u32, rel: &Path, depth: usize) -> Result<Value, String> {
    if depth > 4 {
        return Err("File lineage transformation depth exceeded".into());
    }
    let parts = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if parts.len() < 3
        || parts.len() > 64
        || !matches!(parts[0].as_str(), "Extracted" | "Converted" | "Recovery")
        || parts[1] != format!("{disk:03}")
    {
        return Err("File path does not belong to the requested disk's managed output".into());
    }
    let path = confined(project, rel, false)?;
    // Deleted-native generations live under Recovery, not Extracted. Follow
    // their verified managed marker instead of treating them as unbound text.
    let recovery_root = project.recovery_dir().join(format!("{disk:03}"));
    let mut native_recovery = false;
    if parts[0] == "Recovery" {
        for parent in path
            .ancestors()
            .skip(1)
            .take_while(|p| p.starts_with(&recovery_root))
        {
            if parent
                .join(".fluxvault-extraction.json")
                .try_exists()
                .map_err(|e| e.to_string())?
            {
                native_recovery = true;
                break;
            }
        }
    }
    let payload = read(project, &path, MAX_PAYLOAD)?;
    let digest = sha(&payload);
    let mut result = json!({"path":rel.to_string_lossy().replace('\\',"/"),"bytes":payload.len(),"sha256":digest,"kind":"unbound_saved_file","confidence":"unknown_source_mapping","source_byte_extents":null,"attention_required":true});
    if parts[0] == "Converted" {
        let file = conversion_run::snapshot_path(&project.reports_dir());
        let (_, seal) = control(project, &file)?;
        let saved = conversion_run::load_snapshot(&project.reports_dir(), project.root())?;
        let mut matches = Vec::new();
        for row in &saved.rows {
            for (target, output, kind) in [
                (&row.job.modern_path, &row.modern, "modern"),
                (&row.job.pdf_path, &row.pdf, "pdf"),
            ] {
                if target.try_exists().map_err(|e| e.to_string())?
                    && target.canonicalize().map_err(|e| e.to_string())?
                        == path.canonicalize().map_err(|e| e.to_string())?
                {
                    if row.job.floppy != format!("{disk:03}")
                        || !output.state.successful()
                        || output.output_sha256.as_deref() != Some(digest.as_str())
                    {
                        return Err(
                            "Conversion lineage output binding changed or was not successful"
                                .into(),
                        );
                    }
                    let source_rel = source_relative(project, &row.job.source_path)?;
                    let source = trace(project, disk, &source_rel, depth + 1)?;
                    if source["sha256"] != row.job.source_sha256 {
                        return Err("Conversion lineage source bytes changed".into());
                    }
                    matches.push(json!({"role":kind,"recorded_source_sha256":row.job.source_sha256,"duration_seconds":row.duration_seconds,"conversion_state":output.state,"source":source}));
                }
            }
        }
        if matches.len() != 1 {
            return Err("No unique recorded conversion binds this output".into());
        }
        if control(project, &file)?.1 != seal {
            return Err("Conversion lineage control changed".into());
        }
        result["kind"] = json!("recorded_conversion");
        result["confidence"] = json!("hash_bound_transformation_not_semantic_equivalence");
        result["conversion"] = matches.pop().unwrap();
        result["control_sha256"] = json!(seal);
    } else if parts[0] == "Recovery" && !native_recovery {
        let parent = path.parent().unwrap();
        let controls = ["fragments.json", "word-text.json", "rtf-text.json"]
            .into_iter()
            .map(|name| parent.join(name))
            .filter_map(|p| match p.try_exists() {
                Ok(true) => Some(Ok(p)),
                Ok(false) => None,
                Err(e) => Some(Err(e.to_string())),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if controls.len() > 1 {
            return Err("Ambiguous recovery lineage controls".into());
        }
        if let Some(control_path) = controls.first() {
            let (v, seal) = control(project, control_path)?;
            if v["schema_version"] != 1 || v["disk"] != disk {
                return Err("Recovery lineage identity mismatch".into());
            }
            let attempt = v["attempt"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Missing recovery attempt")?;
            let snapshot = sector_inspection::load_snapshot(project, disk, Some(attempt))?;
            if v["source_sha256"] != snapshot.metadata()["sha256"] {
                return Err("Recovery lineage source hash mismatch".into());
            }
            let name = path.file_name().unwrap().to_string_lossy();
            let mut matches = Vec::new();
            if let Some(files) = v["fragments"].as_array() {
                for f in files {
                    if f["path"] == name.as_ref() {
                        matches.push((f.clone(), "fragment", true));
                    }
                }
            }
            if let Some(documents) = v["documents"].as_array() {
                for doc in documents {
                    for collection in ["segments", "text_segments"] {
                        if let Some(segments) = doc[collection].as_array() {
                            for seg in segments {
                                if seg["path"] == name.as_ref() {
                                    let mut f = seg.clone();
                                    f["parent"] = doc["parent"].clone();
                                    if collection == "text_segments" {
                                        f["source_extents"] = doc["source_extents"].clone();
                                    }
                                    matches.push((f, "decoded_text", false));
                                }
                            }
                        }
                    }
                    for (key, hash_key) in [
                        ("text_path", "text_sha256"),
                        ("edition_path", "edition_sha256"),
                    ] {
                        if doc[key] == name.as_ref() {
                            matches.push((json!({"sha256":doc[hash_key],"parent":doc["parent"],"source_extents":doc["source_extents"],"segments":doc["segments"]}),if key=="edition_path"{"readable_edition"}else{"decoded_text"},false));
                        }
                    }
                    if doc["readable_edition"]["path"] == name.as_ref() {
                        matches.push((json!({"sha256":doc["readable_edition"]["sha256"],"parent":doc["parent"],"segments":doc["segments"]}),"readable_edition",false));
                    }
                }
            }
            if matches.len() != 1 {
                return Err("No unique recovery artifact binds this filename".into());
            }
            let (f, kind, raw) = matches.pop().unwrap();
            if f["sha256"] != digest {
                return Err("Recovery artifact hash binding changed".into());
            }
            result["kind"] = json!(kind);
            result["recovery_record"] = f.clone();
            result["control_sha256"] = json!(seal);
            result["source_image_binding"] = image_binding(&snapshot);
            if raw {
                let ranges: Vec<crate::carving::Extent> =
                    serde_json::from_value(f["source_extents"].clone())
                        .map_err(|e| e.to_string())?;
                result["source_byte_extents"] = extents(&snapshot, &ranges, &payload)?;
                result["confidence"] = json!("byte_replayed_incomplete_fragment_not_whole_file");
            } else {
                let mut range_sets = vec![f["source_extents"].clone()];
                range_sets.extend(
                    f["segments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|s| s["source_extents"].clone()),
                );
                let mut sources = Vec::new();
                let mut mapped_bytes = 0;
                for set in range_sets.into_iter().filter(|v| v.is_array()) {
                    let ranges: Vec<crate::carving::Extent> =
                        serde_json::from_value(set).map_err(|e| e.to_string())?;
                    let mut encoded = Vec::new();
                    for e in &ranges {
                        let end = e
                            .source_byte_offset
                            .checked_add(e.bytes)
                            .filter(|n| *n <= snapshot.bytes.len())
                            .ok_or("Invalid decoded-text source extent")?;
                        mapped_bytes += e.bytes;
                        if mapped_bytes > 8 * 1024 * 1024 {
                            return Err("Decoded lineage source work bound exceeded".into());
                        }
                        encoded.extend_from_slice(&snapshot.bytes[e.source_byte_offset..end]);
                    }
                    sources.push(json!({"encoded_source_sha256":sha(&encoded),"extent_origins":extents(&snapshot,&ranges,&encoded)?}));
                }
                result["encoded_source_relations"] = json!(sources);
                result["confidence"] = json!(
                    "hash_bound_report_declared_decoding_or_presentation_not_independently_redecoded"
                );
                result["decoded_character_to_input_byte_mapping"] = Value::Null;
            }
            snapshot.unchanged()?;
            if control(project, control_path)?.1 != seal {
                return Err("Recovery lineage report changed during inspection".into());
            }
        }
    } else if parts[0] == "Extracted" || native_recovery {
        let mut directory = path.parent().unwrap().to_owned();
        let disk_root = if native_recovery {
            recovery_root
        } else {
            project.extracted_dir().join(format!("{disk:03}"))
        };
        loop {
            let manual = directory.join("__fluxvault-manual-recovery.json");
            let marker = directory.join(".fluxvault-extraction.json");
            if manual.try_exists().map_err(|e| e.to_string())? {
                let (v, seal) = control(project, &manual)?;
                let inside = path
                    .strip_prefix(&directory)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let files = v["files"]
                    .as_array()
                    .ok_or("Manual lineage inventory missing")?;
                let matched = files
                    .iter()
                    .filter(|f| f["relative_path"] == inside)
                    .collect::<Vec<_>>();
                if v["schema_version"] != 1
                    || v["disk_number"] != disk
                    || matched.len() != 1
                    || matched[0]["sha256"] != digest
                    || matched[0]["bytes"] != payload.len() as u64
                {
                    return Err("Manual lineage manifest binding changed".into());
                }
                result["kind"] = json!("manual_import");
                result["confidence"] = json!("hash_bound_import_inventory_physical_pass_unknown");
                result["control_sha256"] = json!(seal);
                result["external_source"] = json!({"source_directory":v["source_directory"],"source_dmde_log":v["source_dmde_log"],"recorded_dmde_log_sha256":v["dmde_log_sha256"],"physical_image_binding":null,"sector_origins":null});
                if control(project, &manual)?.1 != seal {
                    return Err("Manual lineage control changed".into());
                }
                break;
            }
            if marker.try_exists().map_err(|e| e.to_string())? {
                let (v, seal) = control(project, &marker)?;
                control(project, &directory.join(".fluxvault-inventory.json"))?;
                if v.get("native_recovery_report_sha256")
                    .is_some_and(|h| !h.is_null())
                {
                    control(project, &directory.join(".fluxvault-fat12.json"))?;
                }
                let source = v["source_image"].as_str().ok_or("Missing lineage source")?;
                let source_name = Path::new(source)
                    .file_name()
                    .ok_or("Invalid lineage source image")?;
                let source_hash = v["source_sha256"]
                    .as_str()
                    .ok_or("Missing lineage source hash")?;
                extraction::verify_managed_extraction(&directory, source_hash)?;
                let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk)?;
                let selected = attempts
                    .iter()
                    .filter(|a| {
                        Path::new(&a.image_file).file_name() == Some(source_name)
                            && a.sha256.eq_ignore_ascii_case(source_hash)
                    })
                    .collect::<Vec<_>>();
                if selected.len() != 1 {
                    return Err("No unique saved image binds this extraction".into());
                }
                let snapshot = sector_inspection::load_snapshot(
                    project,
                    disk,
                    Some(selected[0].attempt_number),
                )?;
                result["source_image_binding"] = image_binding(&snapshot);
                let inside = path
                    .strip_prefix(&directory)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some(proof) = extraction::verified_native_report(&directory)? {
                    if proof.disk != disk || proof.attempt != selected[0].attempt_number {
                        return Err("Native lineage identity mismatch".into());
                    }
                    let chain = proof
                        .deleted
                        .as_ref()
                        .map_or_else(
                            || {
                                proof
                                    .analysis
                                    .as_ref()
                                    .map_or(&[][..], |a| a.recovered_files.as_slice())
                            },
                            |d| d.surviving_chain_files.as_slice(),
                        )
                        .iter()
                        .filter(|f| f.path == inside)
                        .collect::<Vec<_>>();
                    let carved = proof
                        .carving
                        .iter()
                        .flat_map(|a| &a.files)
                        .filter(|f| f.path == inside)
                        .collect::<Vec<_>>();
                    if chain.len() + carved.len() != 1 {
                        return Err("No unique native file proof".into());
                    }
                    if let Some(f) = chain.first() {
                        let ranges = f
                            .data_lbas
                            .iter()
                            .enumerate()
                            .filter_map(|(i, l)| {
                                let bytes = f.bytes.saturating_sub(i * 512).min(512);
                                (bytes > 0).then_some(crate::carving::Extent {
                                    file_offset: i * 512,
                                    source_byte_offset: *l as usize * 512,
                                    bytes,
                                })
                            })
                            .collect::<Vec<_>>();
                        result["source_byte_extents"] = extents(&snapshot, &ranges, &payload)?;
                        result["metadata_lbas"] = json!(f.metadata_lbas);
                        result["metadata_origins"] = metadata_origins(&snapshot, &f.metadata_lbas)?;
                        result["fat_links"] = json!(f.fat_links);
                        result["kind"] = json!(if proof.deleted.is_some() {
                            "deleted_surviving_chain_candidate"
                        } else {
                            "native_chain_recovery"
                        });
                    } else {
                        let f = carved[0];
                        result["source_byte_extents"] =
                            extents(&snapshot, &f.source_extents, &payload)?;
                        result["allocation_scope"] = json!(f.allocation_scope);
                        result["metadata_origins"] = metadata_origins(&snapshot, &f.metadata_lbas)?;
                        result["original_name_known"] = json!(f.original_name_known);
                        result["kind"] = json!(f.recovery_method);
                    }
                    result["confidence"] =
                        json!("byte_replayed_native_extents_ownership_limits_retained");
                } else {
                    // Tool extraction may bind the whole source without retaining
                    // file allocation. Re-analyse only a uniquely identical live
                    // file; failure remains a whole-image tool binding, not a guess.
                    result["kind"] = json!("tool_extraction");
                    result["confidence"] =
                        json!("hash_bound_inventory_and_source_image_allocation_unknown");
                    if let Ok(live) =
                        evidence_impact::inspect_snapshot(&snapshot, disk, Some(&inside))
                        && live["file"]["payload_sha256"] == digest
                    {
                        result["live_file_trace"] = live;
                    }
                }
                snapshot.unchanged()?;
                extraction::verify_managed_extraction(&directory, source_hash)?;
                if control(project, &marker)?.1 != seal {
                    return Err("Extraction lineage marker changed".into());
                }
                break;
            }
            if directory == disk_root {
                break;
            }
            directory = directory
                .parent()
                .ok_or("Lineage parent missing")?
                .to_owned();
            if !directory.starts_with(&disk_root) {
                return Err("Lineage lookup escaped disk".into());
            }
        }
    }
    if sha(&read(project, &path, MAX_PAYLOAD)?) != digest {
        return Err("Lineage payload changed during inspection".into());
    }
    Ok(result)
}

pub fn inspect(project: &ProjectState, disk: u32, path: &str) -> Result<Value, String> {
    if disk == 0 || disk == u32::MAX {
        return Err("Lineage needs a positive disk number".into());
    }
    crate::processing::validate_workspace(project)?;
    let _budget = crate::resource_budget::background(
        crate::resource_budget::Kind::Recovery,
        256 * crate::resource_budget::MIB,
        &[],
        &|m| eprintln!("[LINEAGE] {m}"),
    )?;
    let _owner = crate::project_work::reserve(project.root())?;
    let rel = relative(path)?;
    let mut node = trace(project, disk, &rel, 0)?;
    let original_node = node.clone();
    let mut donors = std::collections::BTreeMap::new();
    resolve_donors(project, disk, &mut node, &mut donors, 0)?;
    // Donor replay can outlast the initial file/control check. Rebuild the
    // complete un-enriched view before returning, not just the payload hash.
    if trace(project, disk, &rel, 0)? != original_node {
        return Err("Saved file/source/control lineage changed during donor replay".into());
    }
    for donor in donors.values() {
        donor.unchanged()?;
    }
    Ok(
        json!({"schema_version":1,"disk":disk,"file":node,"physical_media_access":false,"host_tools_invoked":false,"files_written":0,"attention_required":true,"customer_delivery_certified":false,
        "limits":"Exact saved file/image/control bindings and replayed native extents, not physical disk identity, original semantics or unrestricted provenance. Manual external allocation, unsupported/old unbound outputs and converted character-to-source-byte mapping remain unknown. Older USB logs can lack pass detail; implicit initial success is explicitly inferred, not a per-sector sensor event."}),
    )
}

fn resolve_donors(
    project: &ProjectState,
    disk: u32,
    v: &mut Value,
    cache: &mut std::collections::BTreeMap<u32, sector_inspection::Snapshot>,
    depth: usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err("Lineage JSON depth bound exceeded".into());
    }
    match v {
        Value::Object(map) => {
            if let Some(o) = map.get("offline_source").filter(|v| v.is_object()) {
                let attempt = o["attempt"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or("Invalid lineage donor attempt")?;
                let lba = o["lba"].as_u64().ok_or("Invalid lineage donor LBA")?;
                let expected = o["image_sha256"]
                    .as_str()
                    .ok_or("Missing lineage donor hash")?
                    .to_owned();
                if !cache.contains_key(&attempt) {
                    if cache.len() >= 8 {
                        return Err("Lineage donor view bound exceeded".into());
                    }
                    let donor = sector_inspection::load_snapshot(project, disk, Some(attempt))?;
                    if donor.origins.is_some() {
                        return Err(
                            "Lineage donor recipe did not collapse to an original acquisition"
                                .into(),
                        );
                    }
                    cache.insert(attempt, donor);
                }
                let donor = &cache[&attempt];
                if donor.metadata()["sha256"] != expected
                    || donor.metadata()["geometry"]["bytes_per_sector"] != 512
                    || lba >= donor.bytes.len() as u64 / 512
                {
                    return Err("Lineage donor source identity/range mismatch".into());
                }
                map.insert(
                    "physical_donor_origin".into(),
                    evidence_impact::origin(donor, lba),
                );
            }
            for value in map.values_mut() {
                resolve_donors(project, disk, value, cache, depth + 1)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                resolve_donors(project, disk, value, cache, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn render(value: &Value) -> String {
    format!(
        "Disk {:03}: {}\nKind: {} / {} bytes\nSHA-256: {}\nEvidence: {}\nUse --json for exact source/pass relations and transformation limits. Saved files only; no media, tool calls or writes.\n{}",
        value["disk"].as_u64().unwrap_or(0),
        value["file"]["path"].as_str().unwrap_or(""),
        value["file"]["kind"].as_str().unwrap_or("unknown"),
        value["file"]["bytes"],
        value["file"]["sha256"].as_str().unwrap_or(""),
        value["file"]["confidence"].as_str().unwrap_or("unknown"),
        value["limits"].as_str().unwrap_or("")
    )
}

#[cfg(test)]
#[path = "file_lineage_tests.rs"]
mod tests;
