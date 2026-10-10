//! Bounded, read-only inspection of completed saved acquisition images.
use crate::{imaging::AttemptSummary, legacy_logs, project::ProjectState};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const CONTROL_LIMIT: u64 = 8 * 1024 * 1024;
const WARNING: &str = "Saved evidence only: readable bytes are not proof of physical label identity or customer completeness. Missing-sector bytes are placeholders, not recovered data. Recorded flux origins are hash-bound records, not an independent flux replay.";

#[derive(Clone, Deserialize)]
struct Geometry {
    cylinders: u64,
    heads: u32,
    sectors_per_track: u32,
    bytes_per_sector: u32,
    total_bytes: u64,
}
#[derive(Deserialize)]
struct Metadata {
    disk_number: u32,
    attempt_number: u32,
    status: String,
    image_file: String,
    #[serde(default)]
    log_file: String,
    #[serde(default)]
    source_backend: String,
    #[serde(default)]
    source_device: String,
    #[serde(default)]
    timestamp_unix_ms: u128,
    geometry: Geometry,
    total_sectors: usize,
    bytes_written: u64,
    bad_sector_count: usize,
    bad_sectors: Vec<Bad>,
    sha256: String,
}
#[derive(Deserialize)]
struct Bad {
    lba: u64,
}
struct Candidate {
    path: PathBuf,
    snapshot: Vec<u8>,
    value: Value,
    meta: Metadata,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn regular(path: &Path, directory: bool) -> Result<(), String> {
    crate::safety::workstation_path(path)?;
    let info = fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
    if info.file_type().is_symlink()
        || (directory && !info.is_dir())
        || (!directory && !info.is_file())
    {
        return Err("Sector inspection requires regular workstation files/directories".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if info.file_attributes() & 0x400 != 0 {
            return Err("Sector inspection refuses reparse points".into());
        }
    }
    crate::safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    regular(path, false)?;
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > limit {
        return Err("Sector inspection input exceeds its size limit".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Sector inspection input grew beyond its size limit".into());
    }
    Ok(bytes)
}
fn local_file(directory: &Path, recorded: &str) -> Result<PathBuf, String> {
    crate::safety::workstation_path(Path::new(recorded))?;
    // The shared resolver rejects foreign/device targets before querying them.
    let path = crate::recovery_plan::resolve_image_path(directory, recorded)?;
    regular(&path, false)?;
    Ok(path)
}
pub(crate) fn open_project(root: PathBuf) -> Result<ProjectState, String> {
    regular(&root, true)?;
    read(&root.join("project.json"), 1024 * 1024)?;
    ProjectState::open_without_session(root)
}
fn chs(g: &Geometry, lba: u64) -> Value {
    let per_cylinder = u64::from(g.heads) * u64::from(g.sectors_per_track);
    json!({"cylinder":lba / per_cylinder,"head":lba / u64::from(g.sectors_per_track) % u64::from(g.heads),"sector":lba % u64::from(g.sectors_per_track) + 1})
}
fn validate(c: &Candidate, disk: u32, attempt: u32) -> Result<(), String> {
    let m = &c.meta;
    let g = &m.geometry;
    let sectors = g
        .cylinders
        .checked_mul(u64::from(g.heads))
        .and_then(|n| n.checked_mul(u64::from(g.sectors_per_track)))
        .ok_or("Geometry overflow")?;
    let size = sectors
        .checked_mul(u64::from(g.bytes_per_sector))
        .ok_or("Geometry overflow")?;
    let bad: BTreeSet<_> = m.bad_sectors.iter().map(|b| b.lba).collect();
    if disk != m.disk_number
        || attempt == 0
        || attempt != m.attempt_number
        || !matches!(m.status.as_str(), "OK" | "PARTIAL" | "DERIVED")
        || !(1..=2).contains(&g.heads)
        || !(1..=36).contains(&g.sectors_per_track)
        || !matches!(g.bytes_per_sector, 128 | 256 | 512 | 1024 | 2048 | 4096)
        || sectors == 0
        || size > crate::fat12::MAX_IMAGE_BYTES as u64
        || sectors != m.total_sectors as u64
        || size != g.total_bytes
        || size != m.bytes_written
        || bad.len() != m.bad_sectors.len()
        || bad.len() != m.bad_sector_count
        || bad.iter().any(|l| *l >= sectors)
        || (m.status == "OK" && !bad.is_empty())
        || m.sha256.len() != 64
        || !m.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(
            "Saved acquisition identity, geometry, hash or sector map is inconsistent".into(),
        );
    }
    Ok(())
}

/// Inspect 1..=8 sectors of a completed native acquisition. No project state is written.
/// Attempt selection uses the same attention/bad-sector/newness ranking as recovery.
pub fn inspect(
    project: &ProjectState,
    disk: u32,
    lba: u64,
    count: usize,
    attempt: Option<u32>,
) -> Result<Value, String> {
    if disk == 0 || !(1..=8).contains(&count) || attempt == Some(0) {
        return Err("Use a positive disk/attempt and 1..8 sectors; LBA is zero-based".into());
    }
    regular(project.root(), true)?;
    let images = project.root().join("Images");
    regular(&images, true)?;
    let prefix = format!("{disk:03}_attempt_");
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, entry) in fs::read_dir(&images)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if index >= 100_000 {
            return Err("Images inventory exceeds the inspection limit".into());
        }
        let path = entry.map_err(|e| e.to_string())?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rest) = name
            .strip_prefix(&prefix)
            .and_then(|n| n.strip_suffix(".json"))
        else {
            continue;
        };
        if rest.ends_with(".partial") {
            continue;
        }
        let number = rest
            .parse::<u32>()
            .map_err(|_| "Invalid acquisition metadata filename")?;
        if !seen.insert(number) {
            return Err("Duplicate saved acquisition attempt numbers".into());
        }
        if candidates.len() >= 256 {
            return Err("Disk has too many attempts for bounded inspection".into());
        }
        let snapshot = read(&path, 1024 * 1024)?;
        let value = serde_json::from_slice(&snapshot).map_err(|e| e.to_string())?;
        let meta = serde_json::from_slice(&snapshot).map_err(|e| e.to_string())?;
        let candidate = Candidate {
            path,
            snapshot,
            value,
            meta,
        };
        validate(&candidate, disk, number)?;
        candidates.push(candidate);
    }
    let selected = if let Some(number) = attempt {
        candidates.iter().find(|c| c.meta.attempt_number == number)
    } else {
        if let Some(number)=crate::preferred_image::preferred_number(&images,disk)? {
            candidates.iter().find(|c| c.meta.attempt_number==number)
        } else {
            candidates.iter().min_by_key(|c|(c.meta.status!="OK"||!c.meta.bad_sectors.is_empty(),c.meta.bad_sectors.len(),std::cmp::Reverse(c.meta.attempt_number)))
        }
    }.ok_or("No completed native acquisition for this disk/attempt; legacy bare images are unsupported by this diagnostic")?;
    let m = &selected.meta;
    let end = lba
        .checked_add(count as u64)
        .filter(|n| *n <= m.total_sectors as u64)
        .ok_or("Sector range lies outside the saved image")?;
    let image = local_file(&images, &m.image_file)?;
    let bytes = read(&image, crate::fat12::MAX_IMAGE_BYTES as u64)?;
    if bytes.len() as u64 != m.bytes_written || !hash(&bytes).eq_ignore_ascii_case(&m.sha256) {
        return Err("Saved image size/SHA-256 does not match acquisition metadata".into());
    }
    crate::offline_images::verify_metadata(&images, &selected.path, &selected.value)?;
    let mut log_path = None;
    let mut log_snapshot = None;
    let mut map_known = false;
    if !m.log_file.is_empty() {
        let logs = project.root().join("Logs");
        regular(&logs, true)?;
        let recorded = m
            .log_file
            .strip_prefix("Logs/")
            .or_else(|| m.log_file.strip_prefix("Logs\\"))
            .unwrap_or(&m.log_file);
        let path = local_file(&logs, recorded)?;
        let raw = read(&path, CONTROL_LIMIT)?;
        let text = std::str::from_utf8(&raw).map_err(|_| "Acquisition log is not UTF-8")?;
        if let Ok(log) = legacy_logs::parse_archiver_log(text) {
            let mut bad = log.bad_sectors.clone();
            bad.sort_unstable();
            let mut expected: Vec<_> = m.bad_sectors.iter().map(|b| b.lba).collect();
            expected.sort_unstable();
            if log.disk_number.is_some_and(|n| n != disk)
                || log.attempt_number.is_some_and(|n| n != m.attempt_number)
                || log
                    .sha256
                    .as_ref()
                    .is_some_and(|h| !h.eq_ignore_ascii_case(&m.sha256))
                || log
                    .geometry
                    .total_sectors
                    .is_some_and(|n| n != m.total_sectors as u64)
                || log
                    .geometry
                    .bytes_per_sector
                    .is_some_and(|n| n != m.geometry.bytes_per_sector)
                || log.geometry.heads.is_some_and(|n| n != m.geometry.heads)
                || log
                    .geometry
                    .cylinders
                    .is_some_and(|n| n != m.geometry.cylinders)
                || log
                    .geometry
                    .sectors_per_track
                    .is_some_and(|n| n != m.geometry.sectors_per_track)
                || log
                    .geometry
                    .total_bytes
                    .is_some_and(|n| n != m.bytes_written)
                || log.bytes_written.is_some_and(|n| n != m.bytes_written)
                || bad != expected
                || (log.end_seen && log.status.label() != m.status)
            {
                return Err("Saved acquisition log contradicts metadata/image/sector map".into());
            }
            map_known = log.begin_seen
                && log.end_seen
                && log.disk_number == Some(disk)
                && log.attempt_number == Some(m.attempt_number)
                && log.sha256.is_some()
                && log.geometry.total_sectors == Some(m.total_sectors as u64)
                && log.geometry.bytes_per_sector == Some(m.geometry.bytes_per_sector);
        }
        log_snapshot = Some(raw);
        log_path = Some(path);
    }
    let summary = AttemptSummary {
        preferred: false,
        attempt_number: m.attempt_number,
        status: m.status.clone(),
        timestamp_unix_ms: m.timestamp_unix_ms,
        image_file: m.image_file.clone(),
        metadata_path: selected.path.clone(),
        log_file: m.log_file.clone(),
        parsed_log: None,
        parsed_dmde_log: None,
        legacy_image: false,
        attention_required: m.status != "OK",
        sha256: m.sha256.clone(),
        total_sectors: m.total_sectors,
        retry_recovered_sectors: 0,
        bad_sectors: m.bad_sectors.iter().map(|b| b.lba).collect(),
    };
    let origins = crate::offline_images::inspection_origins(&images, &summary, lba, end)?;
    let mut flux = None;
    let mut flux_snapshot = None;
    if let Some(recorded) = selected
        .value
        .get("flux_provenance")
        .and_then(Value::as_str)
    {
        let directory = project.root().join("Flux").join("Recovery");
        regular(&project.root().join("Flux"), true)?;
        regular(&directory, true)?;
        let path = local_file(&directory, recorded)?;
        let raw = read(&path, CONTROL_LIMIT)?;
        if selected.value["flux_provenance_sha256"] != hash(&raw) {
            return Err("Flux provenance hash changed".into());
        }
        let value: Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        let records = value["sectors"]
            .as_array()
            .ok_or("Missing flux sector provenance")?;
        if value["schema_version"] != 1
            || value["disk"] != disk
            || value["image_sha256"] != m.sha256
            || records.len() != m.total_sectors
            || records
                .iter()
                .enumerate()
                .any(|(i, v)| v["lba"] != i as u64)
        {
            return Err("Flux provenance identity/image/sector inventory differs".into());
        }
        flux = Some(value);
        flux_snapshot = Some((path, raw));
    } else if selected.value.get("flux_provenance_sha256").is_some() {
        return Err("Flux provenance binding is incomplete".into());
    }
    let mut attention = !map_known || m.status == "DERIVED";
    let sectors: Vec<_> = (lba..end).map(|n| {
        let bad = m.bad_sectors.iter().any(|b| b.lba == n);
        let state = if bad { "unreadable_or_conflicting" } else if !map_known { "unknown" } else if m.status == "DERIVED" { "derived" } else { "readable_saved_evidence" };
        attention |= bad;
        let start = n as usize * m.geometry.bytes_per_sector as usize;
        let payload = &bytes[start..start + m.geometry.bytes_per_sector as usize];
        let rows: Vec<_> = payload.chunks(16).enumerate().map(|(i,row)| json!({"offset":start + i*16,
            "hex":row.iter().map(|b|format!("{b:02X}")).collect::<Vec<_>>().join(" "),
            "ascii":row.iter().map(|b|if (32..=126).contains(b) {char::from(*b)} else {'.'}).collect::<String>()})).collect();
        json!({"lba":n,"chs":chs(&m.geometry,n),"status":state,"sha256":hash(payload),"rows":rows,
            "origin":origins.as_ref().map(|o| &o[(n-lba) as usize]),
            "recorded_flux_origin":flux.as_ref().map(|v| &v["sectors"][n as usize])})
    }).collect();
    // Controls and bytes used for this response are snapshots. Refuse a concurrent
    // changed record instead of pairing an old map with a new image.
    if read(&selected.path, 1024 * 1024)? != selected.snapshot
        || read(&image, crate::fat12::MAX_IMAGE_BYTES as u64)? != bytes
        || log_path
            .as_ref()
            .zip(log_snapshot.as_ref())
            .is_some_and(|(p, b)| read(p, CONTROL_LIMIT).as_ref() != Ok(b))
        || flux_snapshot
            .as_ref()
            .is_some_and(|(p, b)| read(p, CONTROL_LIMIT).as_ref() != Ok(b))
    {
        return Err(
            "Saved evidence changed during inspection; retry when processing is idle".into(),
        );
    }
    Ok(
        json!({"schema_version":1,"disk":disk,"attempt":m.attempt_number,"image":image,"metadata":selected.path,
        "log":log_path,"image_sha256":hash(&bytes),"source_backend":m.source_backend,"source_device":m.source_device,
        "map_verified":map_known,"attention_required":attention,"warning":WARNING,"sectors":sectors}),
    )
}

pub fn render(value: &Value) -> String {
    let mut output = format!(
        "Disk {:03} / saved attempt {:03}\nImage: {}\nSHA-256: {}\n{}\n",
        value["disk"].as_u64().unwrap_or(0),
        value["attempt"].as_u64().unwrap_or(0),
        value["image"],
        value["image_sha256"].as_str().unwrap_or("?"),
        WARNING
    );
    output.push_str(&format!(
        "Metadata: {}\nLog: {}\nSource: {} / {}\n",
        value["metadata"], value["log"], value["source_backend"], value["source_device"]
    ));
    for sector in value["sectors"].as_array().into_iter().flatten() {
        output.push_str(&format!(
            "\nLBA {} / C{} H{} S{} / {}\n",
            sector["lba"],
            sector["chs"]["cylinder"],
            sector["chs"]["head"],
            sector["chs"]["sector"],
            sector["status"].as_str().unwrap_or("unknown")
        ));
        if !sector["origin"].is_null() {
            output.push_str(&format!("Replayed saved origin: {}\n", sector["origin"]));
        }
        if !sector["recorded_flux_origin"].is_null() {
            output.push_str(&format!(
                "Recorded flux origin: {}\n",
                sector["recorded_flux_origin"]
            ));
        }
        for row in sector["rows"].as_array().into_iter().flatten() {
            output.push_str(&format!(
                "{:08X}  {:47}  |{}|\n",
                row["offset"].as_u64().unwrap_or(0),
                row["hex"].as_str().unwrap_or(""),
                row["ascii"].as_str().unwrap_or("")
            ));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        project: ProjectState,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(self.project.root()).unwrap();
        }
    }
    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "fv-sector-inspector-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let f = Fixture {
            project: ProjectState::create_without_session(root).unwrap(),
        };
        save(&f.project, 1, vec![], false);
        f
    }
    fn save(project: &ProjectState, attempt: u32, bad: Vec<u64>, unknown: bool) {
        let mut bytes: Vec<_> = (0..2048).map(|i| (i % 256) as u8).collect();
        for lba in &bad {
            bytes[*lba as usize * 512..(*lba as usize + 1) * 512].fill(0);
        }
        let sha = hash(&bytes);
        let stem = format!("001_attempt_{attempt:03}");
        let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
        let log = project.logs_dir().join(format!("{stem}.log"));
        let text = format!(
            "BEGIN | disk=1 | attempt={attempt}\nGEOMETRY | cylinders=1 | heads=2 | sectors_per_track=2 | bytes_per_sector=512 | total_sectors=4 | total_bytes=2048\n{}END | status={status} | bytes=2048 | sha256={sha}\n",
            bad.iter()
                .map(|n| format!("BAD_SECTOR | lba={n}\n"))
                .collect::<String>()
        );
        fs::write(
            &log,
            if unknown {
                "unrecognized operator note"
            } else {
                &text
            },
        )
        .unwrap();
        fs::write(project.images_dir().join(format!("{stem}.img")), bytes).unwrap();
        let value = json!({"disk_number":1,"attempt_number":attempt,"status":status,"image_file":format!("{stem}.img"),"log_file":log,
            "geometry":{"cylinders":1,"heads":2,"sectors_per_track":2,"bytes_per_sector":512,"total_bytes":2048},"total_sectors":4,
            "bytes_written":2048,"bad_sector_count":bad.len(),"bad_sectors":bad.iter().map(|lba|json!({"lba":lba})).collect::<Vec<_>>(),"sha256":sha});
        fs::write(
            project.images_dir().join(format!("{stem}.json")),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn mutate(f: &Fixture, change: impl FnOnce(&mut Value)) {
        let path = f.project.images_dir().join("001_attempt_001.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    #[test]
    fn bounded_hex_ascii_and_chs_are_exact_and_do_not_modify_evidence() {
        let f = fixture();
        let path = f.project.images_dir().join("001_attempt_001.img");
        let before = fs::read(&path).unwrap();
        let r = inspect(&f.project, 1, 1, 3, None).unwrap();
        assert_eq!(r["sectors"].as_array().unwrap().len(), 3);
        assert_eq!(
            r["sectors"][2]["chs"],
            json!({"cylinder":0,"head":1,"sector":2})
        );
        assert_eq!(r["sectors"][0]["rows"][0]["offset"], 512);
        assert_eq!(r["sectors"][0]["rows"][0]["ascii"], "................");
        assert_eq!(r["sectors"][0]["rows"][2]["ascii"], " !\"#$%&'()*+,-./");
        assert_eq!(r["attention_required"], false);
        assert!(render(&r).contains("00000200  00 01 02"));
        assert_eq!(fs::read(path).unwrap(), before);
    }
    #[test]
    fn best_attempt_and_explicit_attempt_preserve_bad_placeholder_labels() {
        let f = fixture();
        save(&f.project, 2, vec![3], false);
        assert_eq!(inspect(&f.project, 1, 3, 1, None).unwrap()["attempt"], 1);
        let r = inspect(&f.project, 1, 3, 1, Some(2)).unwrap();
        assert_eq!(r["attention_required"], true);
        assert_eq!(r["sectors"][0]["status"], "unreadable_or_conflicting");
        assert_eq!(
            r["sectors"][0]["rows"][0]["hex"],
            "00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00"
        );
        // Range-level attention: readable sector of a partial attempt is not
        // mislabeled bad, and success is explicitly not a disk certificate.
        assert_eq!(
            inspect(&f.project, 1, 0, 1, Some(2)).unwrap()["attention_required"],
            false
        );
    }
    #[test]
    fn unrecognized_or_absent_log_never_certifies_a_readable_map() {
        let f = fixture();
        save(&f.project, 1, vec![], true);
        assert_eq!(
            inspect(&f.project, 1, 0, 1, None).unwrap()["sectors"][0]["status"],
            "unknown"
        );
        mutate(&f, |v| v["log_file"] = json!(""));
        assert_eq!(
            inspect(&f.project, 1, 0, 1, None).unwrap()["attention_required"],
            true
        );
    }
    #[test]
    fn changed_bytes_and_contradictory_log_are_refused() {
        let f = fixture();
        fs::write(
            f.project.images_dir().join("001_attempt_001.img"),
            vec![0; 2048],
        )
        .unwrap();
        assert!(
            inspect(&f.project, 1, 0, 1, None)
                .unwrap_err()
                .contains("SHA-256")
        );
        save(&f.project, 1, vec![], false);
        let path = f.project.logs_dir().join("001_attempt_001.log");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("heads=2", "heads=1");
        fs::write(path, text).unwrap();
        assert!(
            inspect(&f.project, 1, 0, 1, None)
                .unwrap_err()
                .contains("contradicts")
        );
    }
    #[test]
    fn invalid_bounds_identity_geometry_and_duplicate_maps_are_refused() {
        let f = fixture();
        for (lba, count) in [(4, 1), (u64::MAX, 2), (0, 0), (0, 9)] {
            assert!(inspect(&f.project, 1, lba, count, None).is_err());
        }
        assert!(inspect(&f.project, 0, 0, 1, None).is_err());
        assert!(inspect(&f.project, 1, 0, 1, Some(0)).is_err());
        assert!(inspect(&f.project, 1, 0, 1, Some(9)).is_err());
        for (field, value) in [
            ("disk_number", json!(2)),
            ("attempt_number", json!(2)),
            ("bytes_written", json!(2049)),
            ("bad_sectors", json!([{"lba":1},{"lba":1}])),
            ("bad_sectors", json!([{"lba":4}])),
        ] {
            save(&f.project, 1, vec![], false);
            mutate(&f, |v| v[field] = value);
            assert!(inspect(&f.project, 1, 0, 1, None).is_err());
        }
        save(&f.project, 1, vec![], false);
        mutate(&f, |v| v["geometry"]["cylinders"] = json!(u64::MAX));
        assert!(inspect(&f.project, 1, 0, 1, None).is_err());
    }
    #[test]
    fn foreign_device_paths_duplicate_ids_and_oversized_controls_are_refused() {
        let f = fixture();
        for path in ["A:\\customer.img", "\\\\.\\PhysicalDrive0", "../other.img"] {
            save(&f.project, 1, vec![], false);
            mutate(&f, |v| v["image_file"] = json!(path));
            assert!(inspect(&f.project, 1, 0, 1, None).is_err());
        }
        save(&f.project, 1, vec![], false);
        fs::copy(
            f.project.images_dir().join("001_attempt_001.json"),
            f.project.images_dir().join("001_attempt_1.json"),
        )
        .unwrap();
        assert!(
            inspect(&f.project, 1, 0, 1, None)
                .unwrap_err()
                .contains("Duplicate")
        );
        fs::remove_file(f.project.images_dir().join("001_attempt_1.json")).unwrap();
        fs::write(
            f.project.images_dir().join("001_attempt_001.json"),
            vec![b' '; 1024 * 1024 + 1],
        )
        .unwrap();
        assert!(
            inspect(&f.project, 1, 0, 1, None)
                .unwrap_err()
                .contains("size limit")
        );
    }
    #[test]
    fn hash_bound_flux_origins_are_recorded_not_independently_replayed() {
        let f = fixture();
        let directory = f.project.root().join("Flux").join("Recovery");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("001_provenance.json");
        let sha = hash(&fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap());
        let bytes = serde_json::to_vec(&json!({"schema_version":1,"disk":1,"image_sha256":sha,"sectors":(0..4).map(|lba|json!({"lba":lba,"capture_attempts":[1,2],"confidence":"agreement"})).collect::<Vec<_>>()})).unwrap();
        fs::write(&path, &bytes).unwrap();
        mutate(&f, |v| {
            v["flux_provenance"] = json!(path);
            v["flux_provenance_sha256"] = json!(hash(&bytes));
        });
        let r = inspect(&f.project, 1, 2, 1, None).unwrap();
        assert_eq!(
            r["sectors"][0]["recorded_flux_origin"]["capture_attempts"],
            json!([1, 2])
        );
        assert!(
            r["warning"]
                .as_str()
                .unwrap()
                .contains("not an independent flux replay")
        );
        fs::write(path, b"changed").unwrap();
        assert!(
            inspect(&f.project, 1, 2, 1, None)
                .unwrap_err()
                .contains("provenance hash")
        );
    }
    #[test]
    fn regular_guard_refuses_directory_inputs_and_device_aliases_before_lookup() {
        let f = fixture();
        assert!(regular(f.project.root(), false).is_err());
        for p in ["A:", "B:\\missing", "\\\\.\\PhysicalDrive0"] {
            assert!(
                regular(Path::new(p), false)
                    .unwrap_err()
                    .contains("Workstation")
            );
        }
    }
}
