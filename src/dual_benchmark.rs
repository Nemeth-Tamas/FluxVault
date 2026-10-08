//! Dual-only, bounded telemetry replay. The custody journal remains authoritative;
//! timing logs never authorize a read or certify recovered customer data.
use crate::{benchmark, production, project::ProjectState};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

pub(crate) fn record(
    log: &mut benchmark::Session,
    kind: &str,
    mut data: Value,
) -> Result<(), String> {
    data.as_object_mut()
        .ok_or("Dual telemetry data must be an object")?
        .insert("elapsed_ms".into(), json!(log.elapsed_ms()));
    log.record(kind, data)
}

pub(crate) struct Live {
    started: Instant,
    saved: BTreeSet<u32>,
    fresh_saved: BTreeSet<u32>,
}
impl Live {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            saved: BTreeSet::new(),
            fresh_saved: BTreeSet::new(),
        }
    }
    pub(crate) fn saved(&mut self, disk: u32, fresh: bool) {
        self.saved.insert(disk);
        if fresh {
            self.fresh_saved.insert(disk);
        }
    }
    pub(crate) fn text(&self, state: &Value) -> String {
        pace_text(
            state,
            self.saved.len(),
            self.fresh_saved.len(),
            benchmark::milliseconds(self.started.elapsed()),
        )
    }
}

fn pace_text(state: &Value, saved: usize, fresh_saved: usize, elapsed_ms: u64) -> String {
    let pace = if elapsed_ms > 0 && saved > 0 {
        format!(
            "{:.1} saved labels/hour",
            saved as f64 * 3_600_000.0 / elapsed_ms as f64
        )
    } else {
        "warming up".into()
    };
    let remaining = state["remaining_unclaimed_fresh_labels"]
        .as_u64()
        .map(|n| n + state["pending_initial_reads"].as_u64().unwrap_or(0));
    let eta = match remaining {
        Some(0) => "fresh feeding complete".into(),
        Some(n) if fresh_saved >= 3 && elapsed_ms > 0 && state["paused"] != true => format!(
            "rough fresh-feed ETA {:.1} min for {n} labels",
            n as f64 * elapsed_ms as f64 / fresh_saved as f64 / 60_000.0
        ),
        Some(n) => format!(
            "{n} fresh labels remaining; ETA unavailable (need 3 fresh saved labels/unpaused feeding)"
        ),
        None => "no endpoint; ETA unavailable".into(),
    };
    format!(
        "PACE (this invocation, includes swaps/pauses): {pace} / {eta}. Recovery transfers and file tail are extra; not a completion promise."
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    schema_version: u32,
    session: String,
    sequence: u64,
    unix_ms: u64,
    kind: String,
    data: Value,
}

fn number(data: &Value, key: &str) -> Result<u64, String> {
    data[key]
        .as_u64()
        .ok_or_else(|| format!("Invalid dual telemetry {key}"))
}
fn station(data: &Value) -> Result<&str, String> {
    match data["station"].as_str() {
        Some(s @ ("USB" | "GW")) => Ok(s),
        _ => Err("Invalid dual telemetry station".into()),
    }
}
fn regular(path: &Path) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Dual telemetry reparse point refused".into());
        }
    }
    if !m.is_file() || m.len() > 8 * 1024 * 1024 {
        return Err("Dual telemetry is not a bounded regular file".into());
    }
    Ok(())
}

fn occupancy(intervals: &[(u64, u64, &str)]) -> (u64, u64) {
    let mut boundaries = Vec::new();
    for &(start, end, s) in intervals {
        boundaries.push((start, s, 1i32));
        boundaries.push((end, s, -1i32));
    }
    boundaries.sort_unstable();
    let mut active = [0i32; 2];
    let mut prev = 0;
    let (mut busy, mut both) = (0, 0);
    for (time, s, delta) in boundaries {
        if active[0] > 0 || active[1] > 0 {
            busy += time - prev;
        }
        if active[0] > 0 && active[1] > 0 {
            both += time - prev;
        }
        active[usize::from(s == "GW")] += delta;
        prev = time;
    }
    (busy, both)
}

/// Reads saved controls/logs only. No log directory is created by inspection.
pub fn report(project: &ProjectState) -> Result<Value, String> {
    let state = production::status(project)?;
    let dir = project.logs_dir().join("DualBenchmark");
    let mut paths = Vec::new();
    if dir.try_exists().map_err(|e| e.to_string())? {
        let parent = project
            .logs_dir()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let resolved = dir.canonicalize().map_err(|e| e.to_string())?;
        if resolved.parent() != Some(parent.as_path()) {
            return Err("Dual telemetry directory escapes Logs".into());
        }
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".fluxvault-dual-benchmark-") && name.ends_with(".jsonl") {
                regular(&entry.path())?;
                paths.push(entry.path());
                if paths.len() > 4096 {
                    return Err("Too many dual telemetry sessions".into());
                }
            }
        }
    }
    paths.sort();
    let mut sessions = Vec::new();
    let mut receipts: BTreeMap<(u32, String, u32), String> = BTreeMap::new();
    let mut timings = Vec::new();
    let mut warnings = Vec::new();
    let mut confirmations = 0;
    let mut removals = 0;
    let mut failures = 0;
    let mut finished = 0;
    let mut finished_elapsed = 0;
    let mut finished_feed = 0;
    let mut finished_labels = BTreeSet::new();
    let mut all_busy = 0;
    let mut all_overlap = 0;
    let mut total_lines = 0;
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy();
        let id = name
            .strip_prefix(".fluxvault-dual-benchmark-")
            .unwrap()
            .strip_suffix(".jsonl")
            .unwrap();
        let mut reader = BufReader::new(
            File::open(path)
                .map_err(|e| e.to_string())?
                .take(8 * 1024 * 1024 + 1),
        );
        let mut read_bytes = 0;
        let mut sequence = 0;
        let mut elapsed = 0;
        let mut ended = false;
        let mut feed = None;
        let mut labels = BTreeSet::new();
        let mut starts: BTreeMap<u64, (u64, u32, String)> = BTreeMap::new();
        let mut generations = BTreeSet::new();
        let mut intervals = Vec::new();
        let mut paused_at = None;
        let mut pause_ms = 0;
        let mut truncated = false;
        let mut processing_exit = None;
        loop {
            let mut bytes = Vec::new();
            let count = reader
                .by_ref()
                .take(65_537)
                .read_until(b'\n', &mut bytes)
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            read_bytes += count;
            if read_bytes > 8 * 1024 * 1024 {
                return Err("Dual telemetry grew beyond its bound".into());
            }
            if count > 65_536 {
                return Err("Oversized dual telemetry line".into());
            }
            if !bytes.ends_with(b"\n") {
                truncated = true;
                break;
            }
            total_lines += 1;
            if total_lines > 1_000_000 {
                return Err("Dual telemetry event limit exceeded".into());
            }
            let e: Event = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid committed dual telemetry: {e}"))?;
            if e.schema_version != 1
                || e.session != id
                || e.sequence != sequence
                || e.unix_ms == 0
                || ended
                || (sequence == 0 && e.kind != "session_started")
            {
                return Err("Invalid dual telemetry schema/session/sequence/finish".into());
            }
            let time = if sequence == 0 {
                0
            } else {
                number(&e.data, "elapsed_ms")?
            };
            if time < elapsed || time > 366 * 24 * 3_600_000 {
                return Err("Dual telemetry elapsed time went backwards".into());
            }
            elapsed = time;
            match e.kind.as_str() {
                "session_started" if sequence == 0 => {
                    if e.data["configuration"]["mode"] != "dual" {
                        return Err("Not a dual telemetry session".into());
                    }
                    let root = e.data["configuration"]["project_root"]
                        .as_str()
                        .ok_or("Missing dual project binding")?;
                    if Path::new(root).canonicalize().map_err(|e| e.to_string())?
                        != project.root().canonicalize().map_err(|e| e.to_string())?
                    {
                        return Err("Dual telemetry belongs to another project".into());
                    }
                    if e.data["configuration"]["paused"] == true {
                        paused_at = Some(0);
                    }
                }
                "dual_read_started" => {
                    let generation = number(&e.data, "generation")?;
                    let disk =
                        u32::try_from(number(&e.data, "disk")?).map_err(|e| e.to_string())?;
                    let s = station(&e.data)?.to_owned();
                    if disk == 0
                        || disk == u32::MAX
                        || feed.is_some()
                        || generation == 0
                        || !generations.insert(generation)
                        || starts.values().any(|(_, _, old)| *old == s)
                        || starts.insert(generation, (time, disk, s)).is_some()
                    {
                        return Err("Duplicated/invalid dual reader ticket".into());
                    }
                    confirmations += 1;
                }
                "dual_receipt_saved" | "dual_read_failed" => {
                    let generation = number(&e.data, "generation")?;
                    let (start, disk, s) = starts
                        .remove(&generation)
                        .ok_or("Dual result without matching start")?;
                    if number(&e.data, "disk")? != disk as u64 || station(&e.data)? != s {
                        return Err("Dual result ticket mismatch".into());
                    }
                    intervals.push((start, time, s.clone()));
                    if e.kind == "dual_read_failed" {
                        failures += 1;
                    } else {
                        let attempt = u32::try_from(number(&e.data, "attempt")?)
                            .map_err(|e| e.to_string())?;
                        let sha = e.data["image_sha256"]
                            .as_str()
                            .ok_or("Missing dual image hash")?;
                        let key = if s == "USB" { "usb" } else { "gw" };
                        let sealed = &state["disks"][disk.to_string()][key];
                        if attempt == 0
                            || sha.len() != 64
                            || !sha.bytes().all(|b| b.is_ascii_hexdigit())
                            || sealed["attempt"] != attempt
                            || sealed["sha256"] != sha
                        {
                            return Err(
                                "Dual timing receipt does not match verified custody evidence"
                                    .into(),
                            );
                        }
                        if number(&e.data, "bad_sectors")?
                            != sealed["bad"]
                                .as_array()
                                .ok_or("Missing sealed bad-sector map")?
                                .len() as u64
                        {
                            return Err(
                                "Dual timing bad-sector count differs from sealed receipt".into()
                            );
                        }
                        if number(&e.data, "read_decode_ms")? > time - start {
                            return Err("Dual reader duration exceeds recorded interval".into());
                        }
                        let receipt = (disk, s.clone(), attempt);
                        if receipts.insert(receipt, sha.into()).is_none() {
                            timings.push(e.data.clone());
                        }
                        labels.insert(disk);
                    }
                }
                "dual_removal_confirmed" => {
                    number(&e.data, "disk")?;
                    station(&e.data)?;
                    removals += 1;
                }
                "dual_pause" => {
                    if paused_at.is_none() {
                        paused_at = Some(time);
                    }
                }
                "dual_resume" => {
                    if let Some(start) = paused_at.take() {
                        pause_ms += time - start;
                    }
                }
                "dual_feeding_finished" => {
                    if feed.replace(time).is_some() {
                        return Err("Duplicate dual feed finish".into());
                    }
                }
                "dual_session_finished" => {
                    if feed.is_none() || !starts.is_empty() {
                        return Err("Incomplete readers/feed in finished dual session".into());
                    }
                    finished += 1;
                    finished_elapsed += time;
                    finished_feed += feed.unwrap();
                    ended = true;
                    if !e.data["processing_exit_code"].is_null() {
                        let code = number(&e.data, "processing_exit_code")?;
                        if !matches!(code, 0 | 2 | 3) {
                            return Err("Invalid dual processing exit code".into());
                        }
                        processing_exit = Some(code);
                    }
                }
                _ => {
                    return Err(format!(
                        "Unknown/misplaced dual telemetry event: {}",
                        e.kind
                    ));
                }
            }
            sequence += 1;
        }
        if sequence == 0 {
            return Err("Empty dual telemetry session".into());
        }
        if ended {
            finished_labels.extend(labels.iter().copied());
        }
        if let Some(start) = paused_at {
            pause_ms += elapsed - start;
        }
        if truncated {
            warnings.push(format!("{id}: incomplete trailing event ignored"));
        }
        if !ended {
            warnings.push(format!(
                "{id}: unfinished invocation; total wall clock unknown"
            ));
        }
        let borrowed = intervals
            .iter()
            .map(|(a, b, s)| (*a, *b, s.as_str()))
            .collect::<Vec<_>>();
        let (busy, overlap) = occupancy(&borrowed);
        all_busy += busy;
        all_overlap += overlap;
        sessions.push(json!({"session":id,"finished":ended,"truncated_tail":truncated,
            "last_durable_elapsed_ms":elapsed,"session_elapsed_ms":if ended {Some(elapsed)} else {None},
            "feeding_elapsed_ms":feed,"paused_observed_ms":pause_ms,"saved_unique_labels":labels.len(),
            "final_processing_exit_code":processing_exit,
            "reader_busy_union_ms":busy,"both_readers_overlap_ms":overlap,"unclosed_reader_tickets":starts.len()}));
    }
    let verified_count = state["disks"].as_object().map_or(0, |ds| {
        ds.values()
            .filter(|d| d["usb"].is_object() || d["gw"].is_object())
            .count()
    });
    let timed_labels = receipts.keys().map(|(d, _, _)| *d).collect::<BTreeSet<_>>();
    if verified_count > timed_labels.len() {
        warnings.push("Some verified saved disks predate timing logs or were committed before a telemetry write; missing timings are not invented".into());
    }
    Ok(
        json!({"schema_version":1,"measurement_scope":"recorded_dual_scan_invocations",
        "customer_delivery_certified":false,"physical_media_access":false,"sessions":sessions,
        "finished_sessions":finished,"incomplete_sessions":paths.len()-finished,
        "verified_saved_unique_labels":verified_count,"timed_saved_unique_labels":timed_labels.len(),
        "unique_timed_receipts":receipts.len(),"numbered_read_confirmations":confirmations,
        "explicit_removal_confirmations":removals,"reader_failures":failures,
        "finished_session_elapsed_ms":finished_elapsed,"finished_feeding_elapsed_ms":finished_feed,
        "finished_invocations_timed_saved_unique_labels":finished_labels.len(),
        "finished_invocations_saved_labels_per_hour":if finished_feed>0 && !finished_labels.is_empty(){Some(finished_labels.len() as f64*3_600_000.0/finished_feed as f64)} else {None},
        "recorded_reader_busy_union_ms":all_busy,"recorded_both_readers_overlap_ms":all_overlap,
        "timings":timings,"warnings":warnings,"custody":state,
        "note":"Invocation wall times include swaps/pauses; incomplete durations are lower bounds. Reader intervals include decode/publication; confirmation commands are not measured physical touches. Old DualScan reports are not synthesized into timing logs."}),
    )
}

/// Immutable workstation snapshots; these are excluded from customer archives.
pub fn export(project: &ProjectState, report: &Value) -> Result<(PathBuf, PathBuf), String> {
    let dir = benchmark::dual_output_directory(project)?;
    let stamp = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
        std::process::id()
    );
    let json_path = dir.join(format!("DualBenchmark-{stamp}.json"));
    let csv_path = dir.join(format!("DualBenchmark-{stamp}.csv"));
    let mut csv = String::from(
        "Disk,Station,Attempt,ReaderInvocationMilliseconds,BadSectors,ImageSHA256\r\n",
    );
    for row in report["timings"]
        .as_array()
        .ok_or("Missing dual timing rows")?
    {
        // These fields were validated by replay, not arbitrary customer names.
        let sha = row["image_sha256"]
            .as_str()
            .ok_or("Missing timing image hash")?;
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid timing hash for CSV".into());
        }
        csv.push_str(&format!(
            "{:03},{},{},{},{},{}\r\n",
            number(row, "disk")?,
            station(row)?,
            number(row, "attempt")?,
            number(row, "read_decode_ms")?,
            number(row, "bad_sectors")?,
            sha
        ));
    }
    for (path, bytes) in [
        (
            &json_path,
            serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?,
        ),
        (&csv_path, csv.into_bytes()),
    ] {
        let partial = path.with_extension("partial");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        crate::flux_recovery::publish_image_no_replace(&partial, path)?;
    }
    Ok((json_path, csv_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> ProjectState {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        ProjectState::create_without_session(std::env::temp_dir().join(format!(
                "fv-dual-benchmark-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )))
        .unwrap()
    }
    fn log(p: &ProjectState) -> benchmark::Session {
        benchmark::Session::start_dual(
            p,
            json!({"mode":"dual","project_root":p.root(),"paused":false}),
        )
        .unwrap()
    }
    #[test]
    fn empty_inspection_creates_no_controls_or_logs_and_exports_are_immutable() {
        let p = fixture();
        let result = report(&p).unwrap();
        assert_eq!(result["verified_saved_unique_labels"], 0);
        assert_eq!(result["finished_sessions"], 0);
        assert!(!p.root().join(".fluxvault-production.json").exists());
        assert!(!p.logs_dir().join("DualBenchmark").exists());
        let (first, csv) = export(&p, &result).unwrap();
        let bytes = fs::read(&first).unwrap();
        let (second, _) = export(&p, &result).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first).unwrap(), bytes);
        assert_eq!(fs::read_to_string(csv).unwrap().lines().count(), 1);
        fs::remove_dir_all(p.root()).unwrap();
    }
    #[test]
    fn truncated_tail_retains_prior_events_and_marks_total_duration_unknown() {
        let p = fixture();
        let mut log = log(&p);
        record(&mut log, "dual_pause", json!({})).unwrap();
        let path = log.path().to_path_buf();
        drop(log);
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{\"partial");
        fs::write(&path, &bytes).unwrap();
        let result = report(&p).unwrap();
        assert_eq!(result["incomplete_sessions"], 1);
        assert_eq!(result["sessions"][0]["truncated_tail"], true);
        assert!(result["sessions"][0]["session_elapsed_ms"].is_null());
        assert_eq!(result["finished_session_elapsed_ms"], 0);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(p.root()).unwrap();
    }
    #[test]
    fn malformed_committed_records_and_out_of_order_elapsed_are_not_softened() {
        let p = fixture();
        let mut log = log(&p);
        record(&mut log, "dual_pause", json!({})).unwrap();
        let path = log.path().to_path_buf();
        drop(log);
        let original = fs::read_to_string(&path).unwrap();
        let mut events: Vec<Value> = original
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        events[1]["data"]["elapsed_ms"] = json!(u64::MAX);
        let edited = events
            .iter()
            .map(|e| format!("{}\n", e))
            .collect::<String>();
        fs::write(&path, &edited).unwrap();
        assert!(report(&p).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        fs::write(&path, format!("{original}invalid committed record\n")).unwrap();
        assert!(report(&p).unwrap_err().contains("Invalid committed"));
        assert!(!p.reports_dir().join("DualBenchmark").exists());
        fs::remove_dir_all(p.root()).unwrap();
    }
    #[test]
    fn foreign_project_bindings_and_oversized_lines_are_refused() {
        let p = fixture();
        let other = fixture();
        let mut log =
            benchmark::Session::start_dual(&p, json!({"mode":"dual","project_root":other.root()}))
                .unwrap();
        record(&mut log, "dual_feeding_finished", json!({})).unwrap();
        let path = log.path().to_path_buf();
        drop(log);
        assert!(report(&p).unwrap_err().contains("another project"));
        fs::write(&path, vec![b'x'; 65_537]).unwrap();
        assert!(report(&p).unwrap_err().contains("Oversized"));
        fs::remove_dir_all(p.root()).unwrap();
        fs::remove_dir_all(other.root()).unwrap();
    }
    #[test]
    fn zero_read_finished_sessions_are_not_counted_as_scanned_labels() {
        let p = fixture();
        let mut log = log(&p);
        record(&mut log, "dual_feeding_finished", json!({})).unwrap();
        record(&mut log, "dual_session_finished", json!({})).unwrap();
        let result = report(&p).unwrap();
        assert_eq!(result["finished_sessions"], 1);
        assert_eq!(result["timed_saved_unique_labels"], 0);
        assert!(result["finished_invocations_saved_labels_per_hour"].is_null());
        drop(log);
        fs::remove_dir_all(p.root()).unwrap();
    }
    #[test]
    fn oversized_event_is_refused_before_append_and_does_not_poison_the_sequence() {
        let p = fixture();
        let mut log = log(&p);
        let original = fs::read(log.path()).unwrap();
        assert!(
            record(&mut log, "dual_pause", json!({"detail":"x".repeat(70_000)}))
                .unwrap_err()
                .contains("Oversized")
        );
        assert_eq!(fs::read(log.path()).unwrap(), original);
        record(&mut log, "dual_feeding_finished", json!({})).unwrap();
        record(&mut log, "dual_session_finished", json!({})).unwrap();
        assert_eq!(report(&p).unwrap()["finished_sessions"], 1);
        drop(log);
        fs::remove_dir_all(p.root()).unwrap();
    }
    #[test]
    fn reader_intervals_use_union_and_intersection_not_sum_as_wall_clock() {
        assert_eq!(
            occupancy(&[(0, 100, "USB"), (50, 150, "GW"), (100, 160, "USB")]),
            (160, 100)
        );
        assert_eq!(occupancy(&[(0, 100, "USB"), (100, 200, "GW")]), (200, 0));
        assert_eq!(occupancy(&[]), (0, 0));
    }
    #[test]
    fn fresh_feed_eta_requires_samples_endpoint_and_unpaused_feeding() {
        let mut state =
            json!({"remaining_unclaimed_fresh_labels":6,"pending_initial_reads":1,"paused":false});
        assert!(pace_text(&state, 2, 2, 120_000).contains("ETA unavailable"));
        assert!(pace_text(&state, 3, 3, 180_000).contains("ETA 7.0 min for 7 labels"));
        state["paused"] = json!(true);
        assert!(pace_text(&state, 3, 3, 180_000).contains("ETA unavailable"));
        assert!(pace_text(&json!({}), 3, 3, 180_000).contains("no endpoint"));
    }
    #[test]
    fn old_transfers_do_not_supply_fresh_eta_samples_or_double_count_labels() {
        let mut live = Live::new();
        live.saved(1, true);
        live.saved(1, false);
        live.saved(2, false);
        live.saved(3, false);
        assert_eq!(live.saved.len(), 3);
        assert_eq!(live.fresh_saved.len(), 1);
        let state =
            json!({"remaining_unclaimed_fresh_labels":4,"pending_initial_reads":0,"paused":false});
        assert!(pace_text(&state, 3, 1, 180_000).contains("ETA unavailable"));
        assert!(pace_text(&state, 5, 3, 180_000).contains("ETA 4.0 min"));
    }
}
