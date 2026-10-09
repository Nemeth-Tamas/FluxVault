//! Saved-only physical-flux measurements and vendor-reported decode diagnostics.
//! Never launches a tool or changes source, recovery selection, custody or numbering.
use crate::{flux_capture, greaseweazle::GreaseweazleProfile, project::ProjectState};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_CAPTURES: usize = 64;
const MAX_DECODES: usize = 128;
const RAW_SCAN_BUDGET: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct DiagnosticResult {
    pub report: PathBuf,
    pub tracks_csv: PathBuf,
    pub sectors_csv: PathBuf,
    pub note: PathBuf,
    pub report_sha256: String,
    pub disk: u32,
    pub captures: usize,
    pub decodes: usize,
    pub attention: bool,
    pub physical_media_access: bool,
}

fn read_control(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let info = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !info.file_type().is_file() || info.len() > limit {
        return Err(format!(
            "Unsafe/oversized diagnostic control: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Diagnostic control grew beyond its bound".into());
    }
    Ok(bytes)
}
fn bounded_inventory(project: &ProjectState, disk: u32) -> Result<u64, String> {
    let flux = flux_capture::project_flux_dir(project)?;
    let mut captures = 0;
    let mut decodes = 0;
    let mut largest = 0;
    for (dir, prefix, kind) in [
        (&flux, format!("{disk:03}_attempt_"), true),
        (&flux.join("Derived"), format!("{disk:03}_flux_"), false),
    ] {
        if !dir.try_exists().map_err(|e| e.to_string())? {
            continue;
        }
        let canonical = dir.canonicalize().map_err(|e| e.to_string())?;
        if canonical != flux && canonical.parent() != Some(flux.as_path()) {
            return Err("Diagnostic directory escapes Flux".into());
        }
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || !name.ends_with(".json") {
                continue;
            }
            let bytes = read_control(&entry.path(), 256 * 1024)?;
            if kind {
                captures += 1;
                let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if let Some(size) = value["bytes"].as_u64() {
                    if size > crate::scp_diagnostics::LIMIT {
                        return Err("Capture exceeds diagnostic size bound".into());
                    }
                    largest = largest.max(size);
                }
            } else {
                decodes += 1;
                let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if value["bytes"].as_u64().is_some_and(|n| n > 2880 * 512) {
                    return Err("Decode exceeds supported diagnostic geometry".into());
                }
            }
            if captures > MAX_CAPTURES || decodes > MAX_DECODES {
                return Err("Disk exceeds bounded diagnostic capture/decode history".into());
            }
        }
    }
    Ok(largest)
}
fn confined_directory(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(name);
    if path.try_exists().map_err(|e| e.to_string())? {
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_dir()
        {
            return Err("Unsafe diagnostic output directory".into());
        }
    } else {
        fs::create_dir(&path).map_err(|e| e.to_string())?;
    }
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if canonical.parent() != Some(root) {
        return Err("Diagnostic output directory escapes its parent".into());
    }
    Ok(canonical)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    if sha(path)? != format!("{:x}", Sha256::digest(bytes)) {
        return Err("Diagnostic export failed read-back verification".into());
    }
    Ok(())
}
fn sha(path: &Path) -> Result<String, String> {
    crate::conversion::sha256_file(path)
}
fn selected(decode: &flux_capture::DecodeInspection, cylinder: usize) -> bool {
    decode
        .capture_settings
        .as_ref()
        .and_then(|s| s.cylinders.as_ref())
        .is_none_or(|list| list.contains(&(cylinder as u32)))
}
#[derive(Default)]
struct History {
    first_good: Vec<Option<[u8; 32]>>,
    seen_bad: BTreeSet<usize>,
    previous_good: BTreeSet<usize>,
}

pub fn diagnose(project: &ProjectState, disk: u32) -> Result<DiagnosticResult, String> {
    if disk == 0 {
        return Err("Diagnostics require a positive disk number".into());
    }
    crate::processing::validate_workspace(project)?;
    let largest = bounded_inventory(project, disk)?;
    let _budget = crate::resource_budget::background(
        crate::resource_budget::Kind::Recovery,
        256 * crate::resource_budget::MIB,
        &[(
            project.root(),
            largest.saturating_add(16 * crate::resource_budget::MIB),
        )],
        &|s| eprintln!("{s}"),
    )?;
    let _owner = crate::project_work::reserve(project.root())?;
    if bounded_inventory(project, disk)? > largest {
        return Err("Diagnostic inventory grew during resource admission; retry inspection".into());
    }
    let status = flux_capture::inspect_disk(project, disk)?;
    let binding = crate::flux_recovery::diagnostic_binding(project, disk)?;
    let mut issues = Vec::new();
    let mut raw = Vec::new();
    let mut scan_remaining = RAW_SCAN_BUDGET;
    let mut sources = Vec::<(PathBuf, String)>::new();
    let mut tracks_csv = String::from(
        "kind,capture,decode,profile,cylinder,head,revolution,reported_good,reported_unavailable,unobserved,transitions,index_duration_ns,rpm\n",
    );
    for capture in &status.captures {
        let control = read_control(&capture.metadata, 256 * 1024)?;
        sources.push((
            capture.metadata.clone(),
            format!("{:x}", Sha256::digest(&control)),
        ));
        if !capture.hash_matches {
            if !capture
                .metadata
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".partial.json"))
            {
                return Err("Completed capture binding changed; diagnostics refused".into());
            }
            issues.push(format!(
                "Capture {} is {}; partial/failed evidence was not promoted",
                capture.attempt, capture.status
            ));
            raw.push(json!({"attempt":capture.attempt,"status":capture.status,"measured":false}));
            continue;
        }
        let size = capture.bytes.ok_or("Capture size missing")?;
        if size > scan_remaining {
            issues.push(format!("Capture {} exceeds remaining raw measurement budget; hashes/decodes remain inspected",capture.attempt));
            raw.push(json!({"attempt":capture.attempt,"sha256":capture.sha256,"measured":false,"reason":"raw_measurement_budget"}));
            continue;
        }
        scan_remaining -= size;
        let logical = capture
            .raw_flux
            .as_ref()
            .ok_or("Capture has no logical raw path")?;
        let source = crate::flux_archive::open_source(
            logical,
            size,
            capture.sha256.as_deref().ok_or("Capture hash missing")?,
        )?;
        let mut file = File::open(&source.path).map_err(|e| e.to_string())?;
        match crate::scp_diagnostics::inspect(&mut file, size) {
            Ok(measurement) => {
                if !measurement.checksum_matches {
                    issues.push(format!("Capture {} SCP checksum unavailable/mismatched; acquisition SHA-256 still matches",capture.attempt));
                }
                for track in &measurement.tracks {
                    if !track.captured {
                        continue;
                    }
                    for rev in &track.revolutions {
                        tracks_csv.push_str(&format!(
                            "raw,{},{},,{},{},{},,,,{},{},{}\n",
                            capture.attempt,
                            "",
                            track.cylinder,
                            track.head,
                            rev.number,
                            rev.transitions,
                            rev.index_duration_ns,
                            rev.rpm.map(|v| format!("{v:.3}")).unwrap_or_default()
                        ));
                    }
                }
                raw.push(json!({"attempt":capture.attempt,"sha256":capture.sha256,"storage":capture.storage,"measured":true,"measurement":measurement}));
            }
            Err(reason) => {
                issues.push(format!(
                    "Capture {} raw layout not measured: {reason}",
                    capture.attempt
                ));
                raw.push(json!({"attempt":capture.attempt,"sha256":capture.sha256,"measured":false,"reason":reason}));
            }
        }
    }
    let mut histories = BTreeMap::<String, History>::new();
    let mut passes = Vec::new();
    for decode in &status.decodes {
        let metadata = read_control(&decode.metadata, 256 * 1024)?;
        sources.push((
            decode.metadata.clone(),
            format!("{:x}", Sha256::digest(&metadata)),
        ));
        if !decode.output_hash_matches || !decode.source_hash_matches {
            if !decode
                .metadata
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".partial.json"))
            {
                return Err("Completed decode binding changed; diagnostics refused".into());
            }
            issues.push(format!(
                "Capture {} decode {} incomplete/failed; excluded",
                decode.capture_attempt, decode.decode_attempt
            ));
            continue;
        }
        sources.push((decode.image.clone(), decode.output_sha256.clone()));
        let profile = GreaseweazleProfile::parse(&decode.profile)?;
        let count = profile.expected_sector_image_bytes() as usize / 512;
        let spt = count / 160;
        if decode.gw_bad_lbas.is_none() {
            issues.push(format!(
                "Capture {} decode {} lacks a complete sector map",
                decode.capture_attempt, decode.decode_attempt
            ));
            continue;
        }
        let (bytes, bad) = flux_capture::verified_decode(project, decode, count)?;
        let history = histories
            .entry(decode.profile.clone())
            .or_insert_with(|| History {
                first_good: vec![None; count],
                ..Default::default()
            });
        let mut observed = BTreeSet::new();
        let mut good = BTreeSet::new();
        let mut newly_readable = Vec::new();
        let mut lost = Vec::new();
        let mut conflicts = Vec::new();
        let mut track_rows = Vec::new();
        for cylinder in 0..80 {
            for head in 0..2 {
                let start = (cylinder * 2 + head) * spt;
                let covered = selected(decode, cylinder);
                let mut missing = Vec::new();
                let mut found = 0;
                for lba in start..start + spt {
                    if !covered {
                        continue;
                    }
                    observed.insert(lba);
                    if bad.contains(&(lba as u64)) {
                        missing.push(lba);
                        history.seen_bad.insert(lba);
                        if history.previous_good.contains(&lba) {
                            lost.push(lba);
                        }
                        continue;
                    }
                    found += 1;
                    good.insert(lba);
                    let digest: [u8; 32] =
                        Sha256::digest(&bytes[lba * 512..(lba + 1) * 512]).into();
                    if let Some(previous) = history.first_good[lba] {
                        if previous != digest {
                            conflicts.push(lba);
                        }
                    } else {
                        if history.seen_bad.contains(&lba) {
                            newly_readable.push(lba);
                        }
                        history.first_good[lba] = Some(digest);
                    }
                }
                tracks_csv.push_str(&format!(
                    "decode,{},{},{},{},{},,{},{},{},,,\n",
                    decode.capture_attempt,
                    decode.decode_attempt,
                    decode.profile,
                    cylinder,
                    head,
                    found,
                    missing.len(),
                    if covered { 0 } else { spt }
                ));
                track_rows.push(json!({"cylinder":cylinder,"head":head,"reported_good":found,"reported_unavailable_lbas":missing,"unobserved_sectors":if covered{0}else{spt}}));
            }
        }
        history.previous_good = good.clone();
        passes.push(json!({"capture_attempt":decode.capture_attempt,"decode_attempt":decode.decode_attempt,"profile":decode.profile,"image_sha256":decode.output_sha256,"reported_good":good.len(),"reported_unavailable":observed.len()-good.len(),"unobserved":count-observed.len(),"newly_readable_after_previous_unavailable_lbas":newly_readable,"newly_unavailable_vs_previous_observed_pass_lbas":lost,"disagreeing_reported_good_lbas":conflicts,"tracks":track_rows}));
        if !conflicts.is_empty() {
            issues.push(format!(
                "Capture {} decode {} disagrees with earlier reported-good sector bytes at {} LBAs",
                decode.capture_attempt,
                decode.decode_attempt,
                conflicts.len()
            ));
        }
    }
    let mut sectors_csv = String::from(
        "lba,cylinder,head,sector,saved_confidence,capture_attempts,decode_attempts,distinct_raw_hashes\n",
    );
    let mut final_summary = Value::Null;
    if let Some(b) = &binding {
        if let Some(result) = b.get("result").filter(|v| v.is_object()) {
            for (path_key, hash_key) in [
                ("image", "image_sha256"),
                ("provenance", "provenance_sha256"),
            ] {
                if let Some(path) = result[path_key].as_str().filter(|s| !s.is_empty()) {
                    sources.push((
                        PathBuf::from(path),
                        result[hash_key]
                            .as_str()
                            .ok_or("Diagnostic result binding missing")?
                            .into(),
                    ));
                }
            }
        }
        if let Some(rows) = b["sectors"].as_array() {
            let count = rows.len();
            if ![1440, 2880].contains(&count) {
                return Err("Unexpected committed diagnostic geometry".into());
            }
            let spt = count / 160;
            let mut weak = Vec::new();
            for row in rows {
                let lba = row["lba"].as_u64().ok_or("Invalid replayed LBA")? as usize;
                let attempts = row["capture_attempts"]
                    .as_array()
                    .ok_or("Invalid replayed capture list")?;
                let mut distinct = BTreeSet::new();
                for attempt in attempts {
                    let n = attempt.as_u64().ok_or("Invalid capture identity")?;
                    let hash = status
                        .captures
                        .iter()
                        .find(|c| u64::from(c.attempt) == n && c.hash_matches)
                        .and_then(|c| c.sha256.as_ref())
                        .ok_or("Missing provenance capture")?;
                    distinct.insert(hash);
                }
                let confidence = row["confidence"]
                    .as_str()
                    .ok_or("Invalid replayed confidence")?;
                if confidence == "corroborated" && distinct.len() < 2 {
                    weak.push(lba);
                }
                let ids = |key: &str| {
                    row[key]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(";")
                };
                sectors_csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{}\n",
                    lba,
                    lba / (2 * spt),
                    (lba / spt) % 2,
                    lba % spt + 1,
                    confidence,
                    ids("capture_attempts"),
                    ids("decode_attempts"),
                    distinct.len()
                ));
            }
            if !weak.is_empty() {
                issues.push(format!("{} saved corroboration labels use fewer than two distinct raw hashes; not independent evidence",weak.len()));
            }
            let result = &b["result"];
            final_summary = json!({"status":result["status"],"profile":b["profile"],"image":result["image"],"image_sha256":result["image_sha256"],"provenance_sha256":result["provenance_sha256"],"missing_lbas":result["missing_lbas"],"conflicting_lbas":result["conflicting_lbas"],"repeated_hash_corroboration_lbas":weak});
        }
        let journal = PathBuf::from(b["journal"].as_str().ok_or("Missing diagnostic journal")?);
        sources.push((
            journal,
            b["journal_sha256"]
                .as_str()
                .ok_or("Missing diagnostic journal hash")?
                .into(),
        ));
    }
    // Recheck every used immutable image/control and logical raw identity before
    // publishing. New captures not in this snapshot are not silently incorporated.
    for (path, digest) in &sources {
        if sha(path)? != *digest {
            return Err("Diagnostic source changed during inspection; no report committed".into());
        }
    }
    for capture in status.captures.iter().filter(|c| c.hash_matches) {
        crate::flux_archive::verify(
            capture.raw_flux.as_ref().unwrap(),
            capture.bytes.unwrap(),
            capture.sha256.as_ref().unwrap(),
        )?;
    }
    let attention = !issues.is_empty()
        || final_summary.is_null()
        || binding
            .as_ref()
            .and_then(|b| b["result"]["status"].as_str())
            .is_some_and(|s| s != "acquired");
    let mut summary = String::new();
    if final_summary.is_null() {
        summary.push_str("No committed final sector recovery; standalone/partial decodes are not a final image.\n");
    } else {
        summary.push_str(&format!("Committed recovery: {} / {}; missing LBAs: {}; conflicting LBAs: {}.\nImage SHA-256: {}\n", final_summary["status"],final_summary["profile"],final_summary["missing_lbas"],final_summary["conflicting_lbas"],final_summary["image_sha256"]));
    }
    for pass in &passes {
        summary.push_str(&format!("Capture {} / decode {} / {}: {} reported good, {} unavailable, {} unobserved.\n  Newly readable: {}; newly unavailable: {}; disagreeing reported-good bytes: {}.\n",pass["capture_attempt"],pass["decode_attempt"],pass["profile"],pass["reported_good"],pass["reported_unavailable"],pass["unobserved"],pass["newly_readable_after_previous_unavailable_lbas"],pass["newly_unavailable_vs_previous_observed_pass_lbas"],pass["disagreeing_reported_good_lbas"]));
    }
    let note = format!(
        "FluxVault saved-flux diagnostics: disk {disk:03}\n{} captures; {} decode records.\n\n{summary}\n{}\n\nThis is a measurement snapshot, not a new recovery or repaired original. Raw RPM/pulse metrics do not prove magnetic weakness, alignment or valid CRC. Decode maps are Greaseweazle-reported: unavailable combines missing/bad CRC; duplicate IDs and per-revolution CRC are unknown. Outside targeted cylinders is unobserved, not a newly failed sector. Repeated decodes/revolutions are not independent physical captures. Final sectors are replayed from the committed job, never inferred from zero filler. No physical media accessed; no host tool launched; default recovery/delivery selection unchanged.\n",
        status.captures.len(),
        status.decodes.len(),
        if issues.is_empty() {
            "No additional diagnostic issues.".into()
        } else {
            issues.join("\n")
        }
    );
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let reports = confined_directory(&root, "Reports")?;
    let out = confined_directory(&reports, "FluxDiagnostics")?;
    let stem = format!(
        "{disk:03}-{}-{}-{}",
        crate::external_tools::current_unix_ms(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let tracks = out.join(format!("{stem}-tracks.csv"));
    let sectors = out.join(format!("{stem}-sectors.csv"));
    let notes = out.join(format!("{stem}-notes.txt"));
    write_new(&tracks, tracks_csv.as_bytes())?;
    write_new(&sectors, sectors_csv.as_bytes())?;
    write_new(&notes, note.as_bytes())?;
    let value = json!({"schema_version":1,"engine":"saved-flux-diagnostics-v1","disk":disk,"physical_media_access":false,"host_tools_invoked":false,"recovery_selection_changed":false,"attention":attention,"captures":raw,"decode_passes":passes,"committed_recovery":final_summary,"recovery_binding":binding,"issues":issues,"unknown_fields":["per_revolution_sector_CRC","bad_CRC_vs_missing","duplicate_or_unusual_sector_IDs","physical_weak_bits","drive_alignment"],"exports":{"tracks_csv":tracks,"tracks_sha256":sha(&tracks)?,"sectors_csv":sectors,"sectors_sha256":sha(&sectors)?,"note":notes,"note_sha256":sha(&notes)?},"sources":sources});
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Diagnostic report exceeds bounded output".into());
    }
    let partial = out.join(format!("{stem}.partial.json"));
    let report = out.join(format!("{stem}.json"));
    write_new(&partial, &bytes)?;
    crate::flux_recovery::publish_image_no_replace(&partial, &report)?;
    if sha(&report)? != format!("{:x}", Sha256::digest(&bytes)) {
        return Err("Published diagnostic report changed".into());
    }
    Ok(DiagnosticResult {
        report,
        tracks_csv: tracks,
        sectors_csv: sectors,
        note: notes,
        report_sha256: format!("{:x}", Sha256::digest(&bytes)),
        disk,
        captures: status.captures.len(),
        decodes: status.decodes.len(),
        attention,
        physical_media_access: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flux_capture::CaptureRequest;
    use crate::greaseweazle::{
        BackendMode, GreaseweazleBackend, GreaseweazleCommand, GreaseweazleExecution,
    };
    struct Backend {
        reads: u8,
        bad_first: bool,
        conflict: bool,
        current: Vec<usize>,
    }
    impl Default for Backend {
        fn default() -> Self {
            Self {
                reads: 0,
                bad_first: true,
                conflict: false,
                current: (0..80).collect(),
            }
        }
    }
    fn grid(spt: usize, cylinders: &[usize], bad: &[usize]) -> String {
        let tens = (0..80)
            .map(|c| {
                if c % 10 == 0 {
                    char::from_digit(c / 10, 10).unwrap()
                } else {
                    ' '
                }
            })
            .collect::<String>();
        let units = (0..80)
            .map(|c| char::from_digit(c % 10, 10).unwrap())
            .collect::<String>();
        let mut text = format!("Cyl-> {tens}\nH. S: {units}\n");
        let mut found = 0;
        for head in 0..2 {
            for sector in 0..spt {
                let cells = (0..80)
                    .map(|c| {
                        if !cylinders.contains(&c) {
                            ' '
                        } else if bad.contains(&((c * 2 + head) * spt + sector)) {
                            'X'
                        } else {
                            found += 1;
                            '.'
                        }
                    })
                    .collect::<String>();
                text.push_str(&format!("{head}.{sector:>2}: {cells}\n"));
            }
        }
        text.push_str(&format!(
            "Found {found} sectors of {} (99%)\n",
            cylinders.len() * 2 * spt
        ));
        text
    }
    fn raw(cylinders: &[usize], identity: u8) -> Vec<u8> {
        let template = crate::scp_diagnostics::tests::fixture();
        let mut data = template[..0x2b0].to_vec();
        data[3] = identity;
        data[10] = 0;
        data[7] = (*cylinders.iter().max().unwrap() * 2 + 1) as u8;
        data[16..0x2b0].fill(0);
        for cylinder in cylinders {
            for head in 0..2 {
                let track = cylinder * 2 + head;
                let offset = data.len() as u32;
                data[16 + track * 4..20 + track * 4].copy_from_slice(&offset.to_le_bytes());
                let start = data.len();
                data.extend_from_slice(&template[0x2b0..]);
                data[start + 3] = track as u8;
            }
        }
        let sum = data[16..]
            .iter()
            .fold(0u32, |n, b| n.wrapping_add(u32::from(*b)));
        data[12..16].copy_from_slice(&sum.to_le_bytes());
        data
    }
    impl GreaseweazleBackend for Backend {
        fn mode(&self) -> BackendMode {
            BackendMode::MockNoHardware
        }
        fn execute(
            &mut self,
            command: &GreaseweazleCommand,
        ) -> Result<GreaseweazleExecution, String> {
            let mut stdout = String::new();
            match command.subcommand() {
                "info" => {
                    stdout =
                        "Host Tools: 1.23\nDevice:\n  Model: Greaseweazle V4\n  Firmware: 1.23\n"
                            .into()
                }
                "read" => {
                    self.reads += 1;
                    self.current = command
                        .arguments()
                        .iter()
                        .find_map(|a| a.strip_prefix("--tracks=c="))
                        .map(|s| {
                            s.split(':')
                                .next()
                                .unwrap()
                                .split(',')
                                .flat_map(|item| {
                                    if let Some((a, b)) = item.split_once('-') {
                                        (a.parse::<usize>().unwrap()..=b.parse::<usize>().unwrap())
                                            .collect::<Vec<_>>()
                                    } else {
                                        vec![item.parse().unwrap()]
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_else(|| (0..80).collect());
                    fs::write(
                        command.arguments().last().unwrap(),
                        raw(&self.current, self.reads),
                    )
                    .unwrap();
                }
                "convert" => {
                    let spt = if command.arguments().iter().any(|a| a == "--format=ibm.1440") {
                        18
                    } else {
                        9
                    };
                    let bad = if self.bad_first && self.reads == 1 {
                        vec![26]
                    } else {
                        vec![]
                    };
                    let mut bytes = vec![0x33; 160 * spt * 512];
                    if self.conflict && self.reads > 1 {
                        bytes[26 * 512] = 0x55;
                    }
                    fs::write(command.arguments().last().unwrap(), bytes).unwrap();
                    stdout = grid(spt, &self.current, &bad);
                }
                _ => panic!("Forbidden/unexpected host command"),
            }
            Ok(GreaseweazleExecution {
                mode: BackendMode::MockNoHardware,
                command: command.clone(),
                success: true,
                exit_code: Some(0),
                stdout,
                stderr: String::new(),
                timed_out: false,
                host_version: Some("mock".into()),
                started_unix_ms: 0,
                duration_ms: 0,
            })
        }
    }
    fn fixture() -> (ProjectState, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "fv-diagnostics-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        (
            ProjectState::create_without_session(root.clone()).unwrap(),
            root,
        )
    }
    fn acquire(project: &ProjectState) -> crate::flux_recovery::RecoveryResult {
        crate::flux_recovery::recover(
            project,
            7,
            GreaseweazleProfile::Ibm720,
            'B',
            crate::flux_recovery::RecoveryPolicy::default(),
            &mut Backend::default(),
            &|_| {},
        )
        .unwrap()
    }
    fn value(result: &DiagnosticResult) -> Value {
        assert_eq!(sha(&result.report).unwrap(), result.report_sha256);
        serde_json::from_slice(&fs::read(&result.report).unwrap()).unwrap()
    }
    #[test]
    fn committed_replay_reports_new_sectors_not_unobserved_losses_and_exports_are_bound() {
        let (project, root) = fixture();
        let recovered = acquire(&project);
        assert_eq!(recovered.status, "acquired");
        let before = sha(&recovered.image).unwrap();
        let first = diagnose(&project, 7).unwrap();
        let report = value(&first);
        assert!(!first.attention, "{}", report["issues"]);
        assert_eq!(report["physical_media_access"], false);
        assert_eq!(report["host_tools_invoked"], false);
        assert_eq!(
            report["decode_passes"][1]["newly_readable_after_previous_unavailable_lbas"],
            json!([26])
        );
        assert_eq!(
            report["decode_passes"][1]["newly_unavailable_vs_previous_observed_pass_lbas"],
            json!([])
        );
        assert!(report["decode_passes"][1]["unobserved"].as_u64().unwrap() > 0);
        assert_eq!(
            fs::read_to_string(&first.sectors_csv)
                .unwrap()
                .lines()
                .count(),
            1441
        );
        assert_eq!(
            report["exports"]["tracks_sha256"].as_str(),
            Some(sha(&first.tracks_csv).unwrap().as_str())
        );
        let second = diagnose(&project, 7).unwrap();
        assert_ne!(first.report, second.report);
        assert_eq!(sha(&recovered.image).unwrap(), before);
        assert_eq!(project.current_disk_number(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn packed_measurements_match_raw_without_overwriting_sources() {
        let (project, root) = fixture();
        let recovered = acquire(&project);
        let before = diagnose(&project, 7).unwrap();
        let expected = value(&before)["captures"].clone();
        for n in &recovered.capture_attempts {
            crate::flux_archive::pack(&project, 7, *n, true).unwrap();
        }
        let after = diagnose(&project, 7).unwrap();
        let actual = value(&after)["captures"].clone();
        for (a, b) in expected
            .as_array()
            .unwrap()
            .iter()
            .zip(actual.as_array().unwrap())
        {
            assert_eq!(a["measurement"], b["measurement"]);
            assert_eq!(a["sha256"], b["sha256"]);
            assert_eq!(b["storage"], "packed");
        }
        assert!(!fs::read_dir(root.join("Flux")).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".fluxvault-storage-work-")
        }));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_decode_or_published_provenance_refuses_before_report() {
        let (project, root) = fixture();
        let recovered = acquire(&project);
        let status = flux_capture::inspect_disk(&project, 7).unwrap();
        let decode = &status.decodes[0];
        let bytes = fs::read(&decode.image).unwrap();
        fs::write(&decode.image, b"changed").unwrap();
        assert!(diagnose(&project, 7).is_err());
        assert!(!root.join("Reports/FluxDiagnostics").exists());
        fs::write(&decode.image, bytes).unwrap();
        let old = fs::read(&recovered.provenance).unwrap();
        fs::write(&recovered.provenance, b"changed").unwrap();
        assert!(diagnose(&project, 7).is_err());
        fs::write(&recovered.provenance, old).unwrap();
        assert!(diagnose(&project, 7).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn alternative_reported_good_conflicts_do_not_create_a_final_recovery() {
        let (project, root) = fixture();
        let mut backend = Backend {
            bad_first: false,
            conflict: true,
            ..Default::default()
        };
        for _ in 0..2 {
            let capture = flux_capture::capture(
                &project,
                CaptureRequest {
                    disk_number: 7,
                    profile: GreaseweazleProfile::Ibm720,
                    drive: 'B',
                    revolutions: 2,
                },
                &mut backend,
            )
            .unwrap();
            flux_capture::decode(&project, 7, capture.attempt_number, None, &mut backend).unwrap();
        }
        let report = value(&diagnose(&project, 7).unwrap());
        assert!(report["committed_recovery"].is_null());
        assert_eq!(
            report["decode_passes"][1]["disagreeing_reported_good_lbas"],
            json!([26])
        );
        assert_eq!(report["attention"], true);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn owner_and_oversized_control_refuse_without_reports() {
        let (project, root) = fixture();
        acquire(&project);
        let owner = crate::project_work::reserve(project.root()).unwrap();
        assert!(diagnose(&project, 7).is_err());
        drop(owner);
        let path = root.join("Flux/007_attempt_001.json");
        let old = fs::read(&path).unwrap();
        fs::write(&path, vec![b' '; 256 * 1024 + 1]).unwrap();
        assert!(diagnose(&project, 7).is_err());
        assert!(!root.join("Reports/FluxDiagnostics").exists());
        fs::write(&path, old).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cli_alias_and_json_are_offline_and_hardware_flags_are_rejected() {
        let (project, root) = fixture();
        acquire(&project);
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let output = crate::cli::run(&args(&["diagnose", "7", "--json"]), project.root()).unwrap();
        let report: Value = serde_json::from_str(&output.output).unwrap();
        assert_eq!(report["physical_media_access"], false);
        assert_eq!(output.exit_code, 0);
        assert!(
            crate::cli::run(&args(&["diagnose", "7", "--gw-drive", "B"]), project.root()).is_err()
        );
        assert!(crate::cli::run(&args(&["diagnose", "0"]), project.root()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn incomplete_and_missing_map_evidence_are_not_invented_as_good() {
        let (project, root) = fixture();
        let mut backend = Backend {
            bad_first: false,
            ..Default::default()
        };
        let capture = flux_capture::capture(
            &project,
            CaptureRequest {
                disk_number: 7,
                profile: GreaseweazleProfile::Ibm720,
                drive: 'B',
                revolutions: 2,
            },
            &mut backend,
        )
        .unwrap();
        let decode =
            flux_capture::decode(&project, 7, capture.attempt_number, None, &mut backend).unwrap();
        let mut metadata: Value =
            serde_json::from_slice(&fs::read(&decode.metadata_path).unwrap()).unwrap();
        metadata["gw_bad_lbas"] = Value::Null;
        fs::write(
            &decode.metadata_path,
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let report = value(&diagnose(&project, 7).unwrap());
        assert_eq!(report["decode_passes"], json!([]));
        assert!(report["committed_recovery"].is_null());
        assert_eq!(report["attention"], true);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_optional_stage_fields_replay_but_redirected_result_paths_refuse() {
        let (project, root) = fixture();
        let result = acquire(&project);
        let journal = root.join("Flux/Recovery/007_job.json");
        let mut job: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
        let mut provenance: Value =
            serde_json::from_slice(&fs::read(&result.provenance).unwrap()).unwrap();
        for value in [&mut job, &mut provenance] {
            for stage in value["stages"].as_array_mut().unwrap() {
                stage.as_object_mut().unwrap().remove("capture_elapsed_ms");
                stage
                    .as_object_mut()
                    .unwrap()
                    .remove("capture_started_unix_ms");
            }
        }
        fs::write(&result.provenance, serde_json::to_vec(&provenance).unwrap()).unwrap();
        job["result"]["provenance_sha256"] = json!(sha(&result.provenance).unwrap());
        fs::write(&journal, serde_json::to_vec(&job).unwrap()).unwrap();
        assert!(diagnose(&project, 7).is_ok());
        job["result"]["disk"] = json!(8);
        fs::write(&journal, serde_json::to_vec(&job).unwrap()).unwrap();
        assert!(
            diagnose(&project, 7)
                .unwrap_err()
                .contains("identity mismatch")
        );
        job["result"]["disk"] = json!(7);
        let outside = root.join("outside.img");
        fs::copy(&result.image, &outside).unwrap();
        job["result"]["image"] = json!(outside);
        fs::write(&journal, serde_json::to_vec(&job).unwrap()).unwrap();
        assert!(
            diagnose(&project, 7)
                .unwrap_err()
                .contains("outside its managed directory")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_partial_capture_is_listed_without_flux_or_sector_claims() {
        let (project, root) = fixture();
        acquire(&project);
        let path = root.join("Flux/007_attempt_001.json");
        let mut record: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        record["attempt_number"] = json!(99);
        record["status"] = json!("failed");
        record["flux_file"] = Value::Null;
        record["bytes"] = Value::Null;
        record["sha256"] = Value::Null;
        fs::write(
            root.join("Flux/007_attempt_099.partial.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let report = value(&diagnose(&project, 7).unwrap());
        assert_eq!(report["attention"], true);
        assert_eq!(report["captures"][2]["measured"], false);
        assert_eq!(report["captures"][2]["status"], "failed");
        assert_eq!(report["committed_recovery"]["missing_lbas"], json!([]));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn alternate_profile_decodes_of_one_capture_have_separate_histories() {
        let (project, root) = fixture();
        let mut backend = Backend {
            bad_first: false,
            ..Default::default()
        };
        let capture = flux_capture::capture(
            &project,
            CaptureRequest {
                disk_number: 7,
                profile: GreaseweazleProfile::Ibm720,
                drive: 'B',
                revolutions: 2,
            },
            &mut backend,
        )
        .unwrap();
        for profile in [GreaseweazleProfile::Ibm720, GreaseweazleProfile::Ibm1440] {
            flux_capture::decode(
                &project,
                7,
                capture.attempt_number,
                Some(profile),
                &mut backend,
            )
            .unwrap();
        }
        let report = value(&diagnose(&project, 7).unwrap());
        let passes = report["decode_passes"].as_array().unwrap();
        assert_eq!(passes.len(), 2);
        assert_ne!(passes[0]["profile"], passes[1]["profile"]);
        assert!(
            passes
                .iter()
                .all(|p| p["disagreeing_reported_good_lbas"] == json!([]))
        );
        assert_eq!(
            backend.reads, 1,
            "offline alternate decode must not read the disk again"
        );
        assert!(report["committed_recovery"].is_null());
        fs::remove_dir_all(root).unwrap();
    }
}
