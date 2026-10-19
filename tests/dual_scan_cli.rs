//! Real CLI/process lifecycle with mock GW only. Never sends a USB read command.
use fluxvault::{imaging, project::ProjectState};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn fast_handoff_flag_guards_and_preview_never_open_devices_or_write_scan_state() {
    let f = Fixture::new();
    let before = fs::read(f.project.root().join("project.json")).unwrap();
    for args in [
        vec!["scan", "--usb", "--usb-fast-handoff"],
        vec!["scan", "--usb-fast-handoff"],
        vec!["acquire", "--usb-fast-handoff"],
        vec!["scan", "--double", "--usb-handoff-retries", "1"],
        vec![
            "scan",
            "--double",
            "--usb-fast-handoff",
            "--usb-handoff-retries",
            "3",
        ],
        vec![
            "production",
            "start",
            "--usb-fast-handoff",
            "--last-disk",
            "1",
        ],
    ] {
        let out = f.command().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
    for command in ["scan", "start"] {
        let out = f
            .command()
            .args([
                command,
                "--double",
                "--plan",
                "--usb-fast-handoff",
                "--usb-handoff-retries",
                "1",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["requested_usb_fast_handoff_retries"], 1);
    }
    assert_eq!(
        fs::read(f.project.root().join("project.json")).unwrap(),
        before
    );
    for name in [
        ".fluxvault-production.json",
        ".fluxvault-production-workflow.json",
        ".fluxvault-dual-settings.json",
        ".fluxvault-run-control.json",
    ] {
        assert!(!f.project.root().join(name).exists(), "{name}");
    }
    assert_eq!(fs::read_dir(f.project.images_dir()).unwrap().count(), 0);
}

struct Fixture {
    root: PathBuf,
    project: ProjectState,
    appdata: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-dual-cli-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        // Clock precision is not exclusive admission: parallel test cases must
        // never share journals, settings, device reservations or source images.
        fs::create_dir(&root).unwrap();
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        let appdata = root.join("appdata");
        fs::create_dir_all(appdata.join("FluxVault")).unwrap();
        fs::write(
            appdata.join("FluxVault/settings.json"),
            serde_json::to_vec(&json!({
            "greaseweazle_path":env!("CARGO_BIN_EXE_mock_gw")}))
            .unwrap(),
        )
        .unwrap();
        Self {
            root,
            project,
            appdata,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
        command
            .env("APPDATA", &self.appdata)
            .current_dir(self.project.root());
        command
    }
    fn start(&self, last: bool) -> Running {
        self.start_with(last, &[], false)
    }
    fn start_with(&self, last: bool, extra: &[&str], fail_host: bool) -> Running {
        let mut command = self.command();
        command.args([
            "scan",
            "--double",
            "--write-blocker-verified",
            "--acquisition-only",
            "--json",
            "--color",
            "never",
            "--sound",
            "on",
        ]);
        if last {
            command.args(["--last-disk", "3"]);
        }
        command.args(extra);
        if fail_host {
            command.env("MOCK_GW_FAIL_CAPTURE_DECODE", "1");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        Running {
            child,
            out: Some(thread::spawn(move || {
                let mut s = String::new();
                stdout.read_to_string(&mut s).unwrap();
                s
            })),
            err: Some(thread::spawn(move || {
                let mut s = String::new();
                stderr.read_to_string(&mut s).unwrap();
                s
            })),
        }
    }
    fn wait(&self, disk: u32, phase: &str) {
        let start = Instant::now();
        loop {
            let path = self.project.root().join(".fluxvault-production.json");
            if let Ok(bytes) = fs::read(&path)
                && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
                && v["disks"][disk.to_string()]["phase"] == phase
            {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out waiting for {disk} {phase} in {}; error: {}; journal: {}",
                path.display(),
                fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .map_or(Value::Null, |v| v["disks"][disk.to_string()]["error"]
                        .clone()),
                fs::read_to_string(&path)
                    .unwrap_or_else(|e| e.to_string())
                    .chars()
                    .take(2048)
                    .collect::<String>()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Running {
    child: Child,
    out: Option<thread::JoinHandle<String>>,
    err: Option<thread::JoinHandle<String>>,
}
impl Running {
    fn send(&mut self, text: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{text}").unwrap();
        self.child.stdin.as_mut().unwrap().flush().unwrap();
    }
    fn finish(mut self) -> (i32, Value, String) {
        self.child.stdin.take();
        let start = Instant::now();
        let code = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status.code().unwrap_or(-1);
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "dual CLI did not drain/exit"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let out = self.out.take().unwrap().join().unwrap();
        let err = self.err.take().unwrap().join().unwrap();
        assert!(!out.contains('\x07') && !err.contains('\x07'));
        let value =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}; stderr={err}"));
        (code, value, err)
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn seed_usb_partial(project: &ProjectState, disk: u32, bad: &[u64]) {
    use sha2::{Digest, Sha256};
    let mut image = vec![0xe5; 2880 * 512];
    for lba in bad {
        image[*lba as usize * 512..(*lba as usize + 1) * 512].fill(0);
    }
    let sha = format!("{:x}", Sha256::digest(&image));
    let stem = format!("{disk:03}_attempt_001");
    fs::write(project.images_dir().join(format!("{stem}.img")), image).unwrap();
    let log = project.logs_dir().join(format!("{stem}.log"));
    let mut text = format!(
        "BEGIN | disk={disk} | attempt=1 | source=windows-raw-sector\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\n"
    );
    for lba in bad {
        text.push_str(&format!("BAD_SECTOR | LBA={lba}\n"));
    }
    text.push_str(&format!(
        "END | status=PARTIAL | bytes=1474560 | sha256={sha}\n"
    ));
    fs::write(&log, text).unwrap();
    fs::write(project.images_dir().join(format!("{stem}.json")), serde_json::to_vec(&json!({
        "fluxvault_version":"fixture", "status":"PARTIAL", "disk_number":disk,"attempt_number":1,
        "source_backend":"windows-raw-sector","source_device":"mock only","image_file":format!("{stem}.img"),"log_file":log,
        "timestamp_unix_ms":1,"geometry":{"cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":1474560,"format_guess":"synthetic"},
        "sector_retries":0,"total_sectors":2880,"bytes_written":1474560,"retry_recovered_sectors":0,"bad_sector_count":bad.len(),
        "bad_sectors":bad.iter().map(|l|json!({"lba":l,"cylinder":l/36,"head":(l%36)/18,"sector":l%18+1})).collect::<Vec<_>>(),"sha256":sha
    })).unwrap()).unwrap();
}

fn seed_usb_zero_readable_handoff(project: &ProjectState, disk: u32) {
    seed_usb_partial(project, disk, &(0..2880).collect::<Vec<_>>());
    let stem = format!("{disk:03}_attempt_001");
    let path = project.images_dir().join(format!("{stem}.json"));
    let mut meta: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta["usb_fast_handoff"] = json!({"schema":1,"retries":0,"total_sectors":2880,
        "attempted_sectors":1,"first_failed_lba":0});
    fs::write(&path, serde_json::to_vec(&meta).unwrap()).unwrap();
    fs::write(project.logs_dir().join(format!("{stem}.log")), format!(
        "BEGIN | disk={disk} | attempt=1 | source=windows-raw-sector\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\nRETRY_POLICY | retries=0\nUSB_HANDOFF_POLICY | retries=0\nSECTOR_READ_FAILED | LBA=0 | pass=initial_forward\nBAD_SECTOR | LBA=0\nUNREAD_RANGE | first=1 | end_exclusive=2880 | reason=usb_fast_handoff\nEND | status=PARTIAL | bytes=1474560 | sha256={}\n", meta["sha256"].as_str().unwrap()
    )).unwrap();
}

#[test]
fn zero_readable_usb_requires_explicit_label_assertion_and_resumes_saved_gw_without_host() {
    use fluxvault::production::{Coordinator, Station};
    let f = Fixture::new();
    let mut c = Coordinator::open(f.project.clone(), Some(3), false).unwrap();
    let t = c.claim(Station::Usb, 1).unwrap();
    c.confirm(&t, "1", true).unwrap();
    seed_usb_zero_readable_handoff(&f.project, 1);
    c.complete(&t, 1).unwrap();
    c.removed(&t, true).unwrap();
    drop(c);
    let source_paths = [
        f.project.images_dir().join("001_attempt_001.img"),
        f.project.images_dir().join("001_attempt_001.json"),
        f.project.logs_dir().join("001_attempt_001.log"),
    ];
    let original = source_paths.each_ref().map(|p| fs::read(p).unwrap());
    let mut run = f.start_with(
        true,
        &["--usb-fast-handoff", "--usb-handoff-retries", "0"],
        false,
    );
    run.send("g1");
    f.wait(1, "interrupted");
    assert!(!f.project.images_dir().join("001_attempt_002.img").exists());
    run.send("g confirm 2"); // Wrong held label must not grant an assertion.
    run.send("g confirm 1");
    let job = f.project.root().join("Flux/Recovery/001_job.json");
    let start = Instant::now();
    let approved = loop {
        let value: Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
        if value["read_conflicts"]["operator_custody_only"] == true {
            break value;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(approved["read_conflicts"]["agreeing_sectors"], 0);
    assert_eq!(approved["read_conflicts"]["conflicts"], json!([]));
    run.send("g confirm 1"); // Idempotent: keep original proof, not a new timestamp.
    run.send("QUIT");
    let (code, _, console) = run.finish();
    assert_eq!(code, 3, "{console}");
    assert!(console.contains("PHYSICAL LABEL CONFIRMED / IDENTITY NOT BYTE-VERIFIED"));
    let approved_again: Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
    assert_eq!(approved["read_conflicts"], approved_again["read_conflicts"]);
    let stages = approved["stages"].clone();
    // Startup mock health checks remain available, but capture/decode commands
    // fail. Publishing must reuse the approved saved candidate, not read again.
    let mut resumed = f.start_with(false, &[], true);
    resumed.send("g1");
    f.wait(1, "saved");
    resumed.send("g out");
    resumed.send("QUIT");
    let (code, value, console) = resumed.finish();
    assert_eq!(code, 3, "{console}"); // Identity attention must survive completion.
    assert_eq!(value["state"]["disks"]["1"]["phase"], "partial");
    assert_eq!(value["state"]["usb_recovery_queue"], json!([]));
    assert!(console.contains("IDENTITY NOT BYTE-VERIFIED / operator label assertion only"));
    let final_job: Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
    assert_eq!(final_job["stages"], stages);
    let attempts = imaging::load_attempts_for_disk(&f.project.images_dir(), 1).unwrap();
    assert_eq!(attempts.len(), 2);
    let saved = attempts.iter().find(|a| a.attempt_number == 2).unwrap();
    assert!(saved.attention_required);
    assert!(saved.bad_sectors.is_empty());
    let meta_bytes = fs::read(&saved.metadata_path).unwrap();
    let mut meta: Value = serde_json::from_slice(&meta_bytes).unwrap();
    assert_eq!(meta["read_conflicts"], approved["read_conflicts"]);
    let benchmark = f
        .command()
        .args(["production", "benchmark", "--json"])
        .output()
        .unwrap();
    assert!(
        benchmark.status.success(),
        "{}",
        String::from_utf8_lossy(&benchmark.stdout)
    );
    let audit = f.command().args(["audit", "--json"]).output().unwrap();
    assert_ne!(
        audit.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&audit.stdout)
    );
    let audit_summary: Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(audit_summary["verified"], 0);
    let audit_document: Value = serde_json::from_slice(
        &fs::read(f.project.reports_dir().join("EvidenceAudit.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        audit_document["disks"][0]["evidence_status"],
        "OPERATOR_CUSTODY_ONLY"
    );
    assert!(
        audit_document["disks"][0]["issue"]
            .as_str()
            .unwrap()
            .contains("NOT byte-verified")
    );
    for (path, bytes) in source_paths.iter().zip(original) {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    // Removing attention from metadata cannot turn this into a certified match.
    meta["read_conflicts"]
        .as_object_mut()
        .unwrap()
        .remove("operator_custody_only");
    fs::write(&saved.metadata_path, serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(imaging::load_attempts_for_disk(&f.project.images_dir(), 1).is_err());
    fs::write(&saved.metadata_path, meta_bytes).unwrap();
    let journal = f.project.root().join(".fluxvault-production.json");
    let journal_bytes = fs::read(&journal).unwrap();
    let mut state: Value = serde_json::from_slice(&journal_bytes).unwrap();
    state["disks"]["1"]["gw"]
        .as_object_mut()
        .unwrap()
        .remove("identity_not_byte_verified");
    state["disks"]["1"]["phase"] = json!("complete");
    fs::write(&journal, serde_json::to_vec(&state).unwrap()).unwrap();
    let refused = f
        .command()
        .args(["production", "status", "--json"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    fs::write(journal, journal_bytes).unwrap();
}

#[test]
fn no_comparison_wrong_label_release_preserves_rejected_evidence_and_unblocks_station() {
    use fluxvault::production::{Coordinator, Station};
    let f = Fixture::new();
    let mut c = Coordinator::open(f.project.clone(), Some(3), false).unwrap();
    let t = c.claim(Station::Usb, 1).unwrap();
    c.confirm(&t, "1", true).unwrap();
    seed_usb_zero_readable_handoff(&f.project, 1);
    c.complete(&t, 1).unwrap();
    c.removed(&t, true).unwrap();
    drop(c);
    let mut run = f.start_with(
        true,
        &["--usb-fast-handoff", "--usb-handoff-retries", "0"],
        false,
    );
    run.send("g1");
    f.wait(1, "interrupted");
    let job = f.project.root().join("Flux/Recovery/001_job.json");
    let bytes = fs::read(&job).unwrap();
    let before: Value = serde_json::from_slice(&bytes).unwrap();
    let source = fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap();
    run.send("g release 2"); // Wrong label and OUT cannot discard an interrupted job.
    run.send("g out");
    run.send("g release 1");
    f.wait(1, "await_gw");
    let after: Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
    assert_eq!(after["stages"], json!([]));
    assert_eq!(
        after["rejected_capture_attempts"].as_array().unwrap().len(),
        before["stages"].as_array().unwrap().len()
    );
    let recovery = f.project.root().join("Flux/Recovery");
    let archived = fs::read_dir(recovery)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("001_identity_rejected_")
        })
        .unwrap();
    assert_eq!(fs::read(archived).unwrap(), bytes);
    run.send("g2");
    f.wait(2, "saved");
    run.send("g out");
    run.send("QUIT");
    let (code, value, console) = run.finish();
    assert_eq!(code, 3, "{value}\n{console}");
    assert_eq!(value["state"]["usb_recovery_queue"], json!([1]));
    assert_eq!(value["state"]["disks"]["2"]["phase"], "complete");
    assert!(console.contains("GW IDENTITY RELEASED / NO READ STARTED"));
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap(),
        source
    );
    // Saved rejected captures and decoded files remain on disk; their capture
    // identities must not reappear in a later recovery stage automatically.
    assert_eq!(after["read_conflicts"], Value::Null);
    let status = f
        .command()
        .args(["production", "status", "--json"])
        .output()
        .unwrap();
    assert!(status.status.success());
}

#[test]
fn real_cli_reports_ranked_queue_and_audits_explicit_override_with_mock_gw() {
    use fluxvault::production::{Coordinator, Station};
    let f = Fixture::new();
    let mut c = Coordinator::open(f.project.clone(), Some(3), false).unwrap();
    for (disk, bad) in [(1, &[1, 2, 3][..]), (2, &[1][..])] {
        let t = c.claim(Station::Usb, disk).unwrap();
        c.confirm(&t, &disk.to_string(), true).unwrap();
        seed_usb_partial(&f.project, disk, bad);
        c.complete(&t, 1).unwrap();
        c.removed(&t, true).unwrap();
    }
    drop(c);
    let before = fs::read(f.project.root().join(".fluxvault-production.json")).unwrap();
    let output = f
        .command()
        .args(["production", "queue", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let queue: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(queue["recommended_gw_disk"], 2);
    assert_eq!(queue["physical_media_access"], false);
    assert_eq!(
        fs::read(f.project.root().join(".fluxvault-production.json")).unwrap(),
        before
    );
    assert_eq!(
        imaging::load_project_statistics(&f.project.images_dir())
            .unwrap()
            .total_attempts,
        2
    );
    let mut run = f.start(true);
    run.send("g1"); // Lower-ranked label is valid; no actual USB/hardware invocation.
    f.wait(1, "saved");
    run.send("QUIT");
    let (code, value, console) = run.finish();
    assert_eq!(code, 3, "{console}"); // Disk 002 is still queued.
    assert_eq!(value["state"]["recommended_gw_disk"], 2);
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 2)
            .unwrap()
            .len(),
        1
    );
    let mut start_event = None;
    for entry in fs::read_dir(f.project.logs_dir().join("DualBenchmark")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        for line in fs::read_to_string(path).unwrap().lines() {
            let event: Value = serde_json::from_str(line).unwrap();
            if event["kind"] == "dual_read_started" {
                start_event = Some(event);
            }
        }
    }
    let event = start_event.expect("selection telemetry missing");
    assert_eq!(event["data"]["disk"], 1);
    assert_eq!(event["data"]["recommended_gw_disk"], 2);
    assert_eq!(event["data"]["selected_recovery_priority"]["disk"], 1);
    assert_eq!(
        event["data"]["selected_recovery_priority"]["availability"],
        "ready"
    );
    assert!(
        event["data"]["selected_recovery_priority"]["reason"]
            .as_str()
            .unwrap()
            .contains("not recovery success")
    );
}

#[test]
fn dual_cli_mock_gw_publishes_once_packs_and_holds_usb_across_projects() {
    let f = Fixture::new();
    let mut run = f.start(true);
    run.send("");
    run.send("g3");
    run.send("g1");
    f.wait(1, "saved");
    let other = ProjectState::create_without_session(f.root.join("other")).unwrap();
    let refused = f
        .command()
        .args([
            "acquire",
            "--drive",
            "A:",
            "--disk",
            "1",
            "--write-blocker-verified",
            "--json",
            "--project",
        ])
        .arg(other.root())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("USB drive is reserved"));
    assert!(fs::read_dir(other.images_dir()).unwrap().next().is_none());
    run.send("g out");
    run.send("QUIT");
    let (code, value, stderr) = run.finish();
    assert_eq!(code, 0, "{value}\n{stderr}");
    assert_eq!(value["completed_this_session"], 1);
    assert_eq!(value["state"]["disks"]["1"]["phase"], "complete");
    assert_eq!(value["source_media_access"], "read_only");
    assert_eq!(value["report_schema"], 3);
    assert_eq!(value["dual_benchmark"]["finished_sessions"], 1);
    assert_eq!(value["dual_benchmark"]["timed_saved_unique_labels"], 1);
    assert_eq!(value["dual_benchmark"]["numbered_read_confirmations"], 1);
    assert!(PathBuf::from(value["dual_telemetry"].as_str().unwrap()).is_file());
    assert!(
        PathBuf::from(
            value["dual_benchmark_export"]["receipts_csv"]
                .as_str()
                .unwrap()
        )
        .is_file()
    );
    assert!(
        value["session_elapsed_ms"].as_u64().unwrap()
            >= value["feeding_elapsed_ms"].as_u64().unwrap()
    );
    assert!(value["session_elapsed_ms"].as_u64().unwrap() > 0);
    assert!(stderr.contains("NO NEW READ") && stderr.contains("GW / OK SAVED 001"));
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
    let status = f
        .command()
        .args(["production", "status", "--json"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let state: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(state["coordinator_owner_active"], false);
    assert_eq!(state["usb_transfer_pending"], json!([]));
    let status = f.command().args(["production", "status"]).output().unwrap();
    assert!(status.status.success());
    let text = String::from_utf8(status.stdout).unwrap();
    assert!(text.contains("saved state; not a live reader probe"));
    assert!(text.contains("INSERT fresh 002 in USB"));
    assert!(!text.contains('\x1b'));
    let result = f
        .command()
        .args(["production", "benchmark", "--json"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let benchmark: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(benchmark["benchmark"]["unique_timed_receipts"], 1);
    assert_eq!(benchmark["physical_media_access"], false);
    // Timing records cannot relabel or certify a different receipt, even when
    // their JSON/session sequence is otherwise valid. A refusal preserves the
    // log and creates no new exported snapshot.
    let path = PathBuf::from(value["dual_telemetry"].as_str().unwrap());
    let original = fs::read_to_string(&path).unwrap();
    let exports = f.project.reports_dir().join("DualBenchmark");
    let count = fs::read_dir(&exports).unwrap().count();
    for (field, changed) in [
        ("disk", json!(2)),
        ("station", json!("USB")),
        ("generation", json!(999)),
        ("attempt", json!(2)),
        ("bad_sectors", json!(1)),
        ("image_sha256", json!("0".repeat(64))),
        ("read_decode_ms", json!(u64::MAX)),
    ] {
        let mut events: Vec<Value> = original
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        let saved = events
            .iter_mut()
            .find(|e| e["kind"] == "dual_receipt_saved")
            .unwrap();
        saved["data"][field] = changed;
        let edited = events.iter().map(|e| format!("{e}\n")).collect::<String>();
        fs::write(&path, &edited).unwrap();
        let refused = f
            .command()
            .args(["production", "benchmark", "--json"])
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(2), "accepted changed {field}");
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        assert_eq!(fs::read_dir(&exports).unwrap().count(), count);
    }
    fs::write(path, original).unwrap();
}

#[test]
fn actual_process_exit_after_saved_receipt_keeps_removal_and_next_label_without_reread() {
    let f = Fixture::new();
    let mut run = f.start(true);
    run.send("g1");
    f.wait(1, "saved");
    let image = fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap();
    run.child.kill().unwrap();
    run.child.wait().unwrap();
    drop(run);
    let mut resumed = f.start(false);
    resumed.send("g2");
    f.wait(2, "saved");
    resumed.send("g out");
    resumed.send("QUIT");
    let (code, value, stderr) = resumed.finish();
    assert_eq!(code, 0, "{value}\n{stderr}");
    assert_eq!(value["completed_this_session"], 1);
    assert_eq!(value["state"]["disks"]["1"]["phase"], "complete");
    assert_eq!(value["state"]["disks"]["2"]["phase"], "complete");
    assert_eq!(value["dual_benchmark"]["verified_saved_unique_labels"], 2);
    assert_eq!(value["dual_benchmark"]["finished_sessions"], 1);
    assert_eq!(value["dual_benchmark"]["incomplete_sessions"], 1);
    // Killing after the custody commit may precede the timing write. Replay
    // keeps that gap explicit instead of assuming a timing exists for 001.
    assert!(
        value["dual_benchmark"]["timed_saved_unique_labels"]
            .as_u64()
            .unwrap()
            >= 1
    );
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap(),
        image
    );
    assert_eq!(
        ProjectState::open_without_session(f.project.root().into())
            .unwrap()
            .current_disk_number(),
        3
    );
}

#[test]
fn pause_during_mock_read_survives_process_exit_and_resume_alone_does_not_reread() {
    let f = Fixture::new();
    let mut run = f.start(true);
    run.send("g1");
    run.send("PAUSE");
    f.wait(1, "saved");
    let control = f.project.root().join(".fluxvault-production.json");
    let start = Instant::now();
    loop {
        let v: Value = serde_json::from_slice(&fs::read(&control).unwrap()).unwrap();
        if v["paused"] == true {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(10));
    }
    let image = fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap();
    run.child.kill().unwrap();
    run.child.wait().unwrap();
    drop(run);

    let mut blocked = f.start(false);
    blocked.send("g2");
    blocked.send("QUIT");
    let (code, value, stderr) = blocked.finish();
    assert_eq!(code, 3, "{value}\n{stderr}");
    assert_eq!(value["state"]["paused"], true);
    assert_eq!(value["completed_this_session"], 0);
    assert_eq!(value["state"]["disks"]["1"]["phase"], "saved");
    assert!(stderr.contains("No custody changed and no read started"));
    assert!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 2)
            .unwrap()
            .is_empty()
    );

    let mut enabled = f.start(false);
    enabled.send("RESUME");
    enabled.send("STATUS");
    enabled.send("QUIT");
    let (code, value, stderr) = enabled.finish();
    assert_eq!(code, 3, "{value}\n{stderr}");
    assert_eq!(value["state"]["paused"], false);
    assert_eq!(value["completed_this_session"], 0);
    assert!(stderr.contains("FEEDING RESUMED / NO AUTOMATIC READ"));
    assert_eq!(
        fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap(),
        image
    );

    let mut continued = f.start(false);
    continued.send("g2");
    f.wait(2, "saved");
    continued.send("g out");
    continued.send("QUIT");
    let (code, value, stderr) = continued.finish();
    assert_eq!(code, 0, "{value}\n{stderr}");
    assert_eq!(value["completed_this_session"], 1);
    assert_eq!(value["state"]["disks"]["1"]["phase"], "complete");
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn cli_no_verify_and_missing_usb_assertion_fail_before_owner_or_production_state() {
    let f = Fixture::new();
    for flags in [
        vec!["scan", "--double", "--no-verify", "--json"],
        vec!["scan", "--double", "--json"],
    ] {
        let output = f
            .command()
            .args(flags)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(
            !f.project.root().join(".fluxvault-production.json").exists(),
            "invalid command created journal in {}; stdout: {}; stderr: {}",
            f.project.root().display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!f.project.root().join(".fluxvault-processing.lock").exists());
    }
}
