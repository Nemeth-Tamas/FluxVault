//! Local, append-only pilot telemetry. No customer file contents are collected.
use crate::{flux_recovery::RecoveryResult, project::ProjectState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct Event {
    schema_version: u32,
    session: String,
    sequence: u64,
    unix_ms: u64,
    kind: String,
    data: Value,
}

pub(crate) struct Session {
    file: File,
    path: PathBuf,
    id: String,
    sequence: u64,
    started: Instant,
    max_record_bytes: Option<usize>,
}

fn directory_named(project: &ProjectState, base: &str, name: &str) -> Result<PathBuf, String> {
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let root_text = root.to_string_lossy().to_ascii_uppercase();
    if ["A:\\", "B:\\", "\\\\?\\A:\\", "\\\\?\\B:\\"]
        .iter()
        .any(|p| root_text.starts_with(p))
    {
        return Err("Refusing benchmark output on a floppy drive letter".to_owned());
    }
    // Validate the parent before creating anything: no output through a redirected Logs/Reports.
    let parent = root.join(base).canonicalize().map_err(|e| e.to_string())?;
    if parent.parent() != Some(root.as_path()) {
        return Err("Benchmark output directory escapes the project".to_owned());
    }
    let child = parent.join(name);
    if child.exists()
        && !fs::symlink_metadata(&child)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_dir()
    {
        return Err("Unsafe benchmark directory".to_owned());
    }
    fs::create_dir_all(&child).map_err(|e| e.to_string())?;
    let resolved = child.canonicalize().map_err(|e| e.to_string())?;
    if resolved.parent() != Some(parent.as_path()) {
        return Err("Benchmark directory escapes its parent".to_owned());
    }
    Ok(resolved)
}

fn directory(project: &ProjectState, base: &str) -> Result<PathBuf, String> {
    directory_named(project, base, "Benchmark")
}

pub(crate) fn dual_output_directory(project: &ProjectState) -> Result<PathBuf, String> {
    directory_named(project, "Reports", "DualBenchmark")
}

fn nonce() -> Result<String, String> {
    Ok(format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
        std::process::id()
    ))
}

impl Session {
    pub(crate) fn start(project: &ProjectState, configuration: Value) -> Result<Self, String> {
        Self::start_in(project, configuration, "Benchmark", ".fluxvault-benchmark-")
    }

    pub(crate) fn start_dual(project: &ProjectState, configuration: Value) -> Result<Self, String> {
        Self::start_in(
            project,
            configuration,
            "DualBenchmark",
            ".fluxvault-dual-benchmark-",
        )
    }

    fn start_in(
        project: &ProjectState,
        configuration: Value,
        folder: &str,
        prefix: &str,
    ) -> Result<Self, String> {
        if folder == "DualBenchmark"
            && serde_json::to_vec(&configuration)
                .map_err(|e| e.to_string())?
                .len()
                > 48_000
        {
            return Err("Oversized dual benchmark configuration".into());
        }
        let dir = directory_named(project, "Logs", folder)?;
        let id = nonce()?;
        let path = dir.join(format!("{prefix}{id}.jsonl"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let executable_sha256 = std::env::current_exe()
            .ok()
            .and_then(|p| fs::read(p).ok())
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        let mut session = Self {
            file,
            path,
            id,
            sequence: 0,
            started: Instant::now(),
            max_record_bytes: (folder == "DualBenchmark").then_some(65_536),
        };
        session.record(
            "session_started",
            json!({"app_version":env!("CARGO_PKG_VERSION"),
            "executable_sha256":executable_sha256,"os":std::env::consts::OS,
            "architecture":std::env::consts::ARCH,"configuration":configuration}),
        )?;
        Ok(session)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn elapsed_ms(&self) -> u64 {
        milliseconds(self.started.elapsed())
    }

    pub(crate) fn record(&mut self, kind: &str, data: Value) -> Result<(), String> {
        let event = Event {
            schema_version: 1,
            session: self.id.clone(),
            sequence: self.sequence,
            unix_ms: crate::external_tools::current_unix_ms(),
            kind: kind.to_owned(),
            data,
        };
        let mut bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if self
            .max_record_bytes
            .is_some_and(|limit| bytes.len() > limit)
        {
            return Err("Oversized dual benchmark event; no bytes appended".into());
        }
        self.file
            .write_all(&bytes)
            .and_then(|_| self.file.sync_data())
            .map_err(|e| format!("Cannot persist pilot telemetry: {e}"))?;
        self.sequence += 1;
        Ok(())
    }

    pub(crate) fn finish(&mut self, scanned: usize, next_disk: u32) -> Result<(), String> {
        self.record(
            "session_finished",
            json!({"elapsed_ms":milliseconds(self.started.elapsed()),
            "scanned":scanned,"next_disk":next_disk}),
        )
    }
}

pub(crate) fn milliseconds(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn outcome(result: &RecoveryResult) -> Value {
    json!({"disk":result.disk,"status":result.status,"stop_reason":result.stop_reason,
        "missing_sectors":if result.format_exception.is_some(){None}else{Some(result.missing_lbas.len())},"conflicting_sectors":if result.format_exception.is_some(){None}else{Some(result.conflicting_lbas.len())},
        "missing_lbas":result.missing_lbas,"conflicting_lbas":result.conflicting_lbas,
        "corroborated_sectors":result.corroborated_sectors,"single_capture_sectors":result.single_capture_sectors,
        "capture_attempts":result.capture_attempts,"physical_reads_this_run":result.physical_reads_this_run,
        "reused_job":result.resumed,"image_sha256":if result.format_exception.is_some(){None}else{Some(&result.image_sha256)},
        "evidence_sha256":result.format_exception.as_ref().map(|e|e.source_sha256.as_str()).unwrap_or(&result.image_sha256),
        "format_exception":result.format_exception,"provenance_sha256":result.provenance_sha256})
}

#[derive(Debug, Serialize)]
pub struct BenchmarkReport {
    pub schema_version: u32,
    pub measurement_scope: &'static str,
    pub customer_delivery_certified: bool,
    pub sessions: usize,
    pub finished_sessions: usize,
    pub incomplete_sessions: usize,
    pub unique_committed_disks: usize,
    pub recorded_numbering_commits: usize,
    pub confirmed_insertions: usize,
    pub reported_physical_reads: u64,
    pub recovery_errors: usize,
    pub downstream_errors: usize,
    pub downstream_attention: usize,
    pub recovery_seconds: f64,
    pub failed_recovery_seconds: f64,
    pub operator_wait_seconds: f64,
    pub downstream_seconds: f64,
    pub finished_session_seconds: f64,
    pub timed_physical_jobs: usize,
    pub mean_recovery_seconds: Option<f64>,
    pub median_recovery_seconds: Option<f64>,
    pub p95_recovery_seconds: Option<f64>,
    pub projected_136_feed_hours: Option<f64>,
    pub status_counts: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
    pub disks: Vec<Value>,
    pub session_configurations: Vec<Value>,
    pub downstream_runs: Vec<Value>,
    pub recorded_failures: Vec<Value>,
}

fn number(data: &Value, field: &str) -> Result<u64, String> {
    data[field]
        .as_u64()
        .ok_or_else(|| format!("Invalid benchmark field: {field}"))
}

fn disk_number(data: &Value) -> Result<u64, String> {
    let disk = number(data, "disk")?;
    if disk == 0 || disk > u32::MAX as u64 {
        return Err("Invalid benchmark disk number".to_owned());
    }
    Ok(disk)
}

/// Snapshot saved telemetry only. This never opens a physical drive or invokes a tool.
pub fn report(project: &ProjectState) -> Result<BenchmarkReport, String> {
    let logs = project.logs_dir().join("Benchmark");
    let mut paths = Vec::new();
    if logs.exists() {
        let safe = directory(project, "Logs")?;
        for entry in fs::read_dir(safe).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".fluxvault-benchmark-") && name.ends_with(".jsonl") {
                if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    return Err("Unsafe benchmark event file".to_owned());
                }
                paths.push(entry.path());
            }
        }
    }
    paths.sort();
    let mut result = BenchmarkReport {
        schema_version: 1,
        measurement_scope: "recorded_greaseweazle_scan_sessions",
        customer_delivery_certified: false,
        sessions: 0,
        finished_sessions: 0,
        incomplete_sessions: 0,
        unique_committed_disks: 0,
        recorded_numbering_commits: 0,
        confirmed_insertions: 0,
        reported_physical_reads: 0,
        recovery_errors: 0,
        downstream_errors: 0,
        downstream_attention: 0,
        recovery_seconds: 0.0,
        failed_recovery_seconds: 0.0,
        operator_wait_seconds: 0.0,
        downstream_seconds: 0.0,
        finished_session_seconds: 0.0,
        timed_physical_jobs: 0,
        mean_recovery_seconds: None,
        median_recovery_seconds: None,
        p95_recovery_seconds: None,
        projected_136_feed_hours: None,
        status_counts: BTreeMap::new(),
        warnings: Vec::new(),
        disks: Vec::new(),
        session_configurations: Vec::new(),
        downstream_runs: Vec::new(),
        recorded_failures: Vec::new(),
    };
    let mut disks: BTreeMap<u64, Value> = BTreeMap::new();
    let mut timings: BTreeMap<(u64, String), Value> = BTreeMap::new();
    let mut durations = Vec::new();
    let mut feeding_ms = 0u64;
    let mut identities = BTreeSet::new();
    for path in paths {
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
            return Err("Benchmark session exceeds the 16 MiB reader bound".to_owned());
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |p| p + 1);
        if complete < bytes.len() {
            result.warnings.push(format!(
                "Uncommitted trailing record ignored: {}",
                path.display()
            ));
        }
        let text = std::str::from_utf8(&bytes[..complete]).map_err(|e| e.to_string())?;
        let mut session = None;
        let mut finished = false;
        let mut waits = BTreeMap::new();
        result.sessions += 1;
        for (sequence, line) in text.lines().enumerate() {
            let event: Event = serde_json::from_str(line).map_err(|e| {
                format!(
                    "Invalid benchmark record {}:{}: {e}",
                    path.display(),
                    sequence + 1
                )
            })?;
            if event.schema_version != 1
                || event.sequence != sequence as u64
                || finished
                || (sequence == 0 && event.kind != "session_started")
                || (sequence > 0 && event.kind == "session_started")
                || session.as_ref().is_some_and(|id| id != &event.session)
            {
                return Err(
                    "Benchmark schema, session identity or event sequence is invalid".to_owned(),
                );
            }
            if sequence == 0 && !identities.insert(event.session.clone()) {
                return Err("Duplicated benchmark session identity".to_owned());
            }
            session = Some(event.session.clone());
            let data = &event.data;
            match event.kind.as_str() {
                "session_started" => result.session_configurations.push(
                    json!({"session":event.session,"started_unix_ms":event.unix_ms,"data":data}),
                ),
                "read_confirmed" => {
                    let disk = disk_number(data)?;
                    let wait = number(data, "operator_wait_ms")?;
                    result.confirmed_insertions += 1;
                    result.operator_wait_seconds += wait as f64 / 1000.0;
                    waits.insert(disk, wait);
                }
                "recovery_finished" => {
                    let disk = disk_number(data)?;
                    let elapsed = number(data, "elapsed_ms")?;
                    let reads = number(&data["outcome"], "physical_reads_this_run")?;
                    let verification_ms = number(data, "verification_ms")?;
                    result.recovery_seconds += elapsed as f64 / 1000.0;
                    result.reported_physical_reads += reads;
                    let wait = waits.get(&disk).copied().unwrap_or(0);
                    let hash = data["outcome"]
                        .get("evidence_sha256")
                        .unwrap_or(&data["outcome"]["image_sha256"])
                        .as_str()
                        .ok_or("Missing image hash")?
                        .to_owned();
                    let key = (disk, hash);
                    if reads > 0 || !timings.contains_key(&key) {
                        timings.insert(
                            key,
                            json!({"recovery_ms":elapsed,"operator_wait_ms":wait,
                        "verification_ms":data["verification_ms"],"session":event.session}),
                        );
                    }
                    if reads > 0 {
                        durations.push(elapsed);
                        feeding_ms = feeding_ms.saturating_add(elapsed).saturating_add(wait);
                        feeding_ms = feeding_ms.saturating_add(verification_ms);
                    }
                }
                "disk_committed" => {
                    let disk = disk_number(data)?;
                    if disk_number(&data["outcome"])? != disk {
                        return Err("Benchmark outcome disk identity disagrees".to_owned());
                    }
                    let hash = data["outcome"]
                        .get("evidence_sha256")
                        .unwrap_or(&data["outcome"]["image_sha256"])
                        .as_str()
                        .ok_or("Missing image hash")?
                        .to_owned();
                    result.recorded_numbering_commits += 1;
                    // Multiple resumed commits never inflate unique disk/yield counts.
                    disks.insert(
                        disk,
                        json!({"disk":disk,"outcome":data["outcome"],
                        "committed_unix_ms":event.unix_ms,"timing":timings.get(&(disk, hash)),
                        "numbering_resumed":data["numbering_resumed"]}),
                    );
                }
                "recovery_failed" => {
                    if data["cancelled"] != true {
                        result.recovery_errors += 1;
                    }
                    result.failed_recovery_seconds += number(data, "elapsed_ms")? as f64 / 1000.0;
                    result
                        .recorded_failures
                        .push(json!({"session":event.session,"data":data}));
                }
                "downstream_finished" => {
                    result.downstream_seconds += number(data, "elapsed_ms")? as f64 / 1000.0;
                    match number(data, "exit_code")? {
                        0 => {}
                        3 => result.downstream_attention += 1,
                        _ => result.downstream_errors += 1,
                    }
                    result
                        .downstream_runs
                        .push(json!({"session":event.session,"data":data}));
                }
                "session_finished" => {
                    finished = true;
                    result.finished_session_seconds += number(data, "elapsed_ms")? as f64 / 1000.0;
                    result.finished_sessions += 1;
                }
                "downstream_started" => {}
                _ => return Err(format!("Unknown benchmark event: {}", event.kind)),
            }
        }
        if !finished {
            result.incomplete_sessions += 1;
        }
    }
    durations.sort_unstable();
    result.timed_physical_jobs = durations.len();
    if !durations.is_empty() {
        result.mean_recovery_seconds = Some(
            durations.iter().map(|v| *v as f64).sum::<f64>() / durations.len() as f64 / 1000.0,
        );
        result.median_recovery_seconds = Some(if durations.len() % 2 == 0 {
            (durations[durations.len() / 2 - 1] as f64 + durations[durations.len() / 2] as f64)
                / 2000.0
        } else {
            durations[durations.len() / 2] as f64 / 1000.0
        });
        let p95 = (durations.len() * 95).div_ceil(100).saturating_sub(1);
        result.p95_recovery_seconds = Some(durations[p95] as f64 / 1000.0);
        result.projected_136_feed_hours =
            Some(feeding_ms as f64 / durations.len() as f64 * 136.0 / 3_600_000.0);
    }
    for row in disks.values() {
        let status = row["outcome"]["status"]
            .as_str()
            .ok_or("Missing disk status")?
            .to_owned();
        if !matches!(
            status.as_str(),
            "acquired" | "partial" | "unrecoverable_within_policy" | "raw_format_exception"
        ) {
            return Err("Unknown benchmark terminal status".to_owned());
        }
        *result.status_counts.entry(status).or_default() += 1;
    }
    result.unique_committed_disks = disks.len();
    result.disks = disks.into_values().collect();
    Ok(result)
}

pub(crate) fn export(
    project: &ProjectState,
    report: &BenchmarkReport,
) -> Result<(PathBuf, PathBuf), String> {
    let dir = directory(project, "Reports")?;
    let stem = format!("Benchmark-{}", nonce()?);
    let json_path = dir.join(format!("{stem}.json"));
    let csv_path = dir.join(format!("{stem}.csv"));
    let mut csv = String::from(
        "Disk,Status,MissingSectors,ConflictingSectors,RecoverySeconds,OperatorWaitSeconds,ImageSHA256\r\n",
    );
    for row in &report.disks {
        let seconds = |field: &str| {
            row["timing"][field]
                .as_u64()
                .map(|v| format!("{:.3}", v as f64 / 1000.0))
                .unwrap_or_default()
        };
        let fields = [
            format!("{:03}", row["disk"].as_u64().unwrap_or(0)),
            row["outcome"]["status"].as_str().unwrap_or("").to_owned(),
            row["outcome"]["missing_sectors"].to_string(),
            row["outcome"]["conflicting_sectors"].to_string(),
            seconds("recovery_ms"),
            seconds("operator_wait_ms"),
            row["outcome"]["image_sha256"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        ];
        csv.push_str(
            &fields
                .iter()
                .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
    }
    for (path, bytes) in [
        (
            &json_path,
            serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?,
        ),
        (&csv_path, csv.into_bytes()),
    ] {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok((json_path, csv_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(ProjectState);
    impl Fixture {
        fn new() -> Self {
            Self(
                ProjectState::create_without_session(
                    std::env::temp_dir()
                        .join(format!("fluxvault-benchmark-test-{}", nonce().unwrap())),
                )
                .unwrap(),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(self.0.root()).unwrap();
        }
    }
    fn disk(disk: u32, partial: bool, reads: u64) -> Value {
        json!({"disk":disk,"status":if partial {"partial"} else {"acquired"},
            "missing_sectors":if partial {1} else {0},"conflicting_sectors":0,
            "physical_reads_this_run":reads,"image_sha256":format!("hash-{disk}"),"provenance_sha256":"provenance"})
    }
    fn commit(session: &mut Session, value: Value, resumed: bool) {
        session
            .record(
                "disk_committed",
                json!({"disk":value["disk"],"outcome":value,"numbering_resumed":resumed}),
            )
            .unwrap();
    }

    #[test]
    fn known_timings_yield_and_resumed_numbering_are_aggregated_without_double_counting() {
        let fixture = Fixture::new();
        let mut first =
            Session::start(&fixture.0, json!({"profile":"ibm.1440","drive":"B"})).unwrap();
        for (number, wait, elapsed, reads) in [(1, 1000, 10000, 1), (2, 2000, 30000, 3)] {
            let value = disk(number, number == 2, reads);
            first
                .record(
                    "read_confirmed",
                    json!({"disk":number,"operator_wait_ms":wait}),
                )
                .unwrap();
            first
                .record(
                    "recovery_finished",
                    json!({"disk":number,"elapsed_ms":elapsed,
                "verification_ms":250,"outcome":value}),
                )
                .unwrap();
            commit(&mut first, value, false);
        }
        first.record("downstream_started", json!({})).unwrap();
        first
            .record(
                "downstream_finished",
                json!({"elapsed_ms":7000,"exit_code":3}),
            )
            .unwrap();
        first
            .record("session_finished", json!({"elapsed_ms":50000}))
            .unwrap();
        drop(first);
        let mut second = Session::start(&fixture.0, json!({})).unwrap();
        commit(&mut second, disk(2, true, 0), true);
        second
            .record("session_finished", json!({"elapsed_ms":1000}))
            .unwrap();
        drop(second);
        let measured = report(&fixture.0).unwrap();
        assert_eq!(measured.unique_committed_disks, 2);
        assert_eq!(measured.recorded_numbering_commits, 3);
        assert_eq!(measured.confirmed_insertions, 2);
        assert_eq!(measured.reported_physical_reads, 4);
        assert_eq!(measured.timed_physical_jobs, 2);
        assert_eq!(measured.mean_recovery_seconds, Some(20.0));
        assert_eq!(measured.median_recovery_seconds, Some(20.0));
        assert_eq!(measured.p95_recovery_seconds, Some(30.0));
        assert_eq!(measured.operator_wait_seconds, 3.0);
        assert_eq!(measured.downstream_seconds, 7.0);
        assert_eq!(measured.finished_session_seconds, 51.0);
        assert_eq!(measured.downstream_errors, 0);
        assert_eq!(measured.downstream_attention, 1);
        assert_eq!(measured.status_counts["acquired"], 1);
        assert_eq!(measured.status_counts["partial"], 1);
        assert_eq!(measured.disks[1]["timing"]["recovery_ms"], 30000);
        let expected_hours = 21750.0 * 136.0 / 3600000.0;
        assert!((measured.projected_136_feed_hours.unwrap() - expected_hours).abs() < 0.000001);
        let (first_json, csv) = export(&fixture.0, &measured).unwrap();
        let first_bytes = fs::read(&first_json).unwrap();
        let (second_json, _) = export(&fixture.0, &measured).unwrap();
        assert_ne!(first_json, second_json);
        assert_eq!(fs::read(first_json).unwrap(), first_bytes);
        assert_eq!(fs::read_to_string(csv).unwrap().lines().count(), 3);
    }

    #[test]
    fn interrupted_and_truncated_events_remain_visible_without_fake_completion() {
        let fixture = Fixture::new();
        let mut session = Session::start(&fixture.0, json!({})).unwrap();
        session
            .record(
                "recovery_failed",
                json!({"disk":7,"phase":"acquisition","elapsed_ms":4000,"error":"disconnected"}),
            )
            .unwrap();
        let path = session.path().to_owned();
        drop(session);
        let mut file = OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(b"{\"schema_version\":").unwrap();
        drop(file);
        let measured = report(&fixture.0).unwrap();
        assert_eq!(measured.sessions, 1);
        assert_eq!(measured.incomplete_sessions, 1);
        assert_eq!(measured.finished_sessions, 0);
        assert_eq!(measured.recovery_errors, 1);
        assert_eq!(measured.failed_recovery_seconds, 4.0);
        assert_eq!(measured.unique_committed_disks, 0);
        assert_eq!(measured.projected_136_feed_hours, None);
        assert_eq!(measured.warnings.len(), 1);
    }

    #[test]
    fn downstream_operation_failure_is_not_hidden_as_partial_attention() {
        let fixture = Fixture::new();
        let mut session = Session::start(&fixture.0, json!({})).unwrap();
        for code in [0, 3, 2] {
            session
                .record(
                    "downstream_finished",
                    json!({"elapsed_ms":1000,"exit_code":code}),
                )
                .unwrap();
        }
        drop(session);
        let measured = report(&fixture.0).unwrap();
        assert_eq!(measured.downstream_errors, 1);
        assert_eq!(measured.downstream_attention, 1);
        assert_eq!(measured.downstream_seconds, 3.0);
    }

    #[test]
    fn malformed_committed_records_and_duplicate_sessions_are_refused() {
        let fixture = Fixture::new();
        let session = Session::start(&fixture.0, json!({})).unwrap();
        let path = session.path().to_owned();
        drop(session);
        let original = fs::read(&path).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"not-json\n").unwrap();
        drop(file);
        assert!(
            report(&fixture.0)
                .unwrap_err()
                .contains("Invalid benchmark record")
        );
        fs::write(&path, &original).unwrap();
        fs::write(
            path.parent()
                .unwrap()
                .join(".fluxvault-benchmark-duplicate.jsonl"),
            original,
        )
        .unwrap();
        assert!(
            report(&fixture.0)
                .unwrap_err()
                .contains("Duplicated benchmark session")
        );
    }

    #[test]
    fn oversized_session_and_invalid_disk_or_status_are_refused() {
        let fixture = Fixture::new();
        let mut session = Session::start(&fixture.0, json!({})).unwrap();
        commit(&mut session, disk(0, false, 0), false);
        let path = session.path().to_owned();
        drop(session);
        assert!(report(&fixture.0).unwrap_err().contains("disk number"));
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(16 * 1024 * 1024 + 1)
            .unwrap();
        assert!(report(&fixture.0).unwrap_err().contains("reader bound"));
    }

    #[test]
    fn an_empty_project_has_no_invented_throughput_or_yield() {
        let fixture = Fixture::new();
        let measured = report(&fixture.0).unwrap();
        assert_eq!(measured.sessions, 0);
        assert_eq!(measured.unique_committed_disks, 0);
        assert_eq!(measured.p95_recovery_seconds, None);
        assert_eq!(measured.projected_136_feed_hours, None);
        assert!(!measured.customer_delivery_certified);
    }
}
