//! Windows process lifecycle tests. Only mock GW / disposable workstation files.
#![cfg(windows)]
use fluxvault::{imaging, project::ProjectState};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

struct ProcessHandle(HANDLE);
impl ProcessHandle {
    fn open(pid: u32) -> Self {
        Self(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid).unwrap() })
    }
    fn exited(&self) {
        assert_eq!(unsafe { WaitForSingleObject(self.0, 5000) }, WAIT_OBJECT_0);
    }
    fn alive(&self) {
        assert_eq!(unsafe { WaitForSingleObject(self.0, 0) }, WAIT_TIMEOUT);
    }
}
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Running(Child);
impl Running {
    fn send(&mut self, text: &str) {
        let stdin = self.0.stdin.as_mut().unwrap();
        writeln!(stdin, "{text}").unwrap();
        stdin.flush().unwrap();
    }
    fn kill(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
    fn finish(&mut self) -> i32 {
        self.0.stdin.take();
        self.wait_stopped()
    }
    fn wait_stopped(&mut self) -> i32 {
        let begun = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status.code().unwrap_or(-1);
            }
            assert!(
                begun.elapsed() < Duration::from_secs(30),
                "controller failed to exit/drain"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    root: PathBuf,
    project: ProjectState,
    appdata: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-job-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        let appdata = root.join("appdata");
        fs::create_dir_all(appdata.join("FluxVault")).unwrap();
        fs::write(
            appdata.join("FluxVault/settings.json"),
            json!({"greaseweazle_path":env!("CARGO_BIN_EXE_mock_gw")}).to_string(),
        )
        .unwrap();
        Self {
            root,
            project,
            appdata,
        }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
        cmd.current_dir(self.project.root())
            .env("APPDATA", &self.appdata);
        cmd
    }
    fn scan(&self, dual: bool, stage: Option<&str>) -> Running {
        let mut cmd = self.command();
        cmd.args([
            "scan",
            "--acquisition-only",
            "--last-disk",
            "1",
            "--capture-storage",
            "raw",
            "--color",
            "never",
        ]);
        if dual {
            // Dual has automatic packing and refuses expert retention overrides.
            cmd = self.command();
            cmd.args([
                "scan",
                "--double",
                "--write-blocker-verified",
                "--acquisition-only",
                "--last-disk",
                "1",
                "--color",
                "never",
            ]);
        }
        if let Some(stage) = stage {
            cmd.env(stage, "1")
                .env("MOCK_GW_TREE_ROOT", self.root.join("tree"));
        }
        Running(
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    fn tree(&self) -> (ProcessHandle, ProcessHandle) {
        let v = wait_json(&self.root.join("tree/tree.json"));
        (
            ProcessHandle::open(v["leader"].as_u64().unwrap() as u32),
            ProcessHandle::open(v["worker"].as_u64().unwrap() as u32),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn wait_json(path: &Path) -> Value {
    let begun = Instant::now();
    loop {
        if let Ok(b) = fs::read(path)
            && let Ok(v) = serde_json::from_slice(&b)
        {
            return v;
        }
        assert!(
            begun.elapsed() < Duration::from_secs(20),
            "missing event: {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn partials(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut result = vec![];
    for e in fs::read_dir(root).unwrap().flatten() {
        if e.file_type().unwrap().is_dir() {
            result.extend(partials(&e.path()));
        } else if e.file_name().to_string_lossy().contains(".partial.") {
            result.push((e.path(), fs::read(e.path()).unwrap()));
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

fn interrupted_capture(dual: bool) {
    let f = Fixture::new();
    let outside = f.root.join("unrelated");
    fs::create_dir(&outside).unwrap();
    let mut unrelated = Running(
        Command::new(env!("CARGO_BIN_EXE_mock_gw"))
            .arg("mock-worker")
            .env("MOCK_GW_TREE_ROOT", outside)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let independent = ProcessHandle::open(unrelated.0.id());
    let mut run = f.scan(dual, Some("MOCK_GW_READ_TREE"));
    run.send(if dual { "g1" } else { "001" });
    let (leader, worker) = f.tree();
    leader.alive();
    worker.alive();
    let saved = partials(f.project.root());
    assert!(
        saved
            .iter()
            .any(|(p, _)| p.extension().is_some_and(|x| x == "scp"))
    );
    assert!(
        saved
            .iter()
            .any(|(p, _)| p.extension().is_some_and(|x| x == "json"))
    );
    run.kill();
    leader.exited();
    worker.exited();
    independent.alive();
    let beat = fs::read(f.root.join("tree/worker.beat")).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(fs::read(f.root.join("tree/worker.beat")).unwrap(), beat);
    assert!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .is_empty()
    );
    for (path, bytes) in &saved {
        assert_eq!(fs::read(path).unwrap(), *bytes);
    }
    let mut resumed = f.scan(dual, None);
    resumed.send(if dual { "g1" } else { "001" });
    if dual {
        let begun = Instant::now();
        loop {
            let v = wait_json(&f.project.root().join(".fluxvault-production.json"));
            if v["disks"]["1"]["phase"] == "saved" {
                break;
            }
            assert!(begun.elapsed() < Duration::from_secs(20));
            thread::sleep(Duration::from_millis(10));
        }
        resumed.send("g out\nQUIT");
    }
    assert_eq!(resumed.finish(), 0);
    let attempts = imaging::load_attempts_for_disk(&f.project.images_dir(), 1).unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].bad_sectors.is_empty());
    for (path, bytes) in saved {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    let mut completed = f.scan(dual, None);
    completed.send("QUIT");
    assert_eq!(completed.finish(), 0);
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
    unrelated.kill();
}
#[test]
fn forced_single_controller_exit_stops_host_descendants_preserves_partial_and_resumes_once() {
    interrupted_capture(false);
}
#[test]
fn forced_dual_controller_exit_stops_host_descendants_preserves_custody_and_resumes_once() {
    interrupted_capture(true);
}

fn cooperative_capture_stop(dual: bool, typed: bool, after_quit: bool) {
    let f = Fixture::new();
    let mut run = f.scan(dual, Some("MOCK_GW_READ_TREE"));
    run.send(if dual { "g1" } else { "001" });
    let (leader, worker) = f.tree();
    leader.alive();
    worker.alive();
    if typed {
        if after_quit {
            run.send("QUIT");
            thread::sleep(Duration::from_millis(100));
            leader.alive();
        }
        run.send("STOP");
    } else {
        let output = f.command().args(["stop", "--json"]).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let v: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(v["stop_requested"], true);
        assert_eq!(v["physical_media_access"], false);
    }
    assert_eq!(
        run.wait_stopped(),
        130,
        "stop must be distinguished from success/fatal failure"
    );
    leader.exited();
    worker.exited();
    let saved = partials(f.project.root());
    assert!(
        saved
            .iter()
            .any(|(p, _)| p.extension().is_some_and(|e| e == "scp"))
    );
    assert!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ProjectState::open_without_session(f.project.root().into())
            .unwrap()
            .current_disk_number(),
        1
    );
    let status = f
        .command()
        .args(["run", "status", "--json"])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["active"], false);
    assert!(!f.command().arg("stop").output().unwrap().status.success());
    let mut resumed = f.scan(dual, None);
    resumed.send(if dual { "g1" } else { "001" });
    if dual {
        let start = Instant::now();
        while wait_json(&f.project.root().join(".fluxvault-production.json"))["disks"]["1"]["phase"]
            != "saved"
        {
            assert!(start.elapsed() < Duration::from_secs(20));
            thread::sleep(Duration::from_millis(10));
        }
        resumed.send("g out\nQUIT");
    }
    assert_eq!(resumed.finish(), 0);
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
    if dual {
        assert_eq!(
            fluxvault::dual_benchmark::report(&f.project).unwrap()["reader_failures"],
            0
        );
    } else {
        assert_eq!(
            fluxvault::benchmark::report(&f.project)
                .unwrap()
                .recovery_errors,
            0
        );
    }
    for (path, bytes) in saved {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
#[test]
fn separate_console_stop_cancels_single_capture_tree_and_resumes_once() {
    cooperative_capture_stop(false, false, false);
}
#[test]
fn typed_stop_cancels_single_capture_tree_and_resumes_once() {
    cooperative_capture_stop(false, true, false);
}
#[test]
fn typed_stop_cancels_dual_capture_tree_preserves_custody_and_resumes_once() {
    cooperative_capture_stop(true, true, false);
}
#[test]
fn separate_console_stop_cancels_dual_capture_tree_and_resumes_once() {
    cooperative_capture_stop(true, false, false);
}
#[test]
fn typed_stop_after_dual_quit_still_cancels_active_read() {
    cooperative_capture_stop(true, true, true);
}

#[test]
fn stop_while_waiting_at_prompt_needs_no_stdin_eof_and_stale_request_cannot_stop_resume() {
    let f = Fixture::new();
    let mut run = f.scan(false, None);
    wait_json(&f.project.root().join(".fluxvault-run-control.json"));
    assert!(f.command().arg("stop").output().unwrap().status.success());
    assert_eq!(run.wait_stopped(), 130);
    assert!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .is_empty()
    );
    let mut resumed = f.scan(false, None);
    resumed.send("001");
    assert_eq!(resumed.finish(), 0);
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn forced_decode_exit_reuses_completed_flux_without_reading_media_again() {
    interrupted_decode(false);
}
#[test]
fn cooperative_decode_stop_reuses_completed_flux_without_reading_media_again() {
    interrupted_decode(true);
}

#[test]
fn cooperative_office_stop_retains_partial_and_retries_saved_issue_without_publication() {
    let f = Fixture::new();
    fs::write(
        f.appdata.join("FluxVault/settings.json"),
        json!({"libreoffice_path":env!("CARGO_BIN_EXE_mock_gw")}).to_string(),
    )
    .unwrap();
    let source = f.project.root().join("Extracted/001/sample.rtf");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    let bytes = b"{\\rtf1\\ansi Disposable stop/resume fixture}";
    fs::write(&source, bytes).unwrap();
    let mut running = Running(
        f.command()
            .args(["conversion", "run", "--conversion-workers", "1", "--json"])
            .env("MOCK_GW_OFFICE_TREE", "1")
            .env("MOCK_GW_TREE_ROOT", f.root.join("tree"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let (leader, worker) = f.tree();
    assert!(f.command().arg("stop").output().unwrap().status.success());
    assert_eq!(running.wait_stopped(), 130);
    leader.exited();
    worker.exited();
    assert_eq!(fs::read(&source).unwrap(), bytes);
    let target = f
        .project
        .root()
        .join("Converted/001/sample [from RTF].docx");
    assert!(!target.exists(), "interrupted output must not be promoted");
    let issues = f
        .command()
        .args(["conversion", "issues", "--json"])
        .output()
        .unwrap();
    assert_eq!(issues.status.code(), Some(3));
    let state: Value = serde_json::from_slice(&issues.stdout).unwrap();
    assert_eq!(state["issues"].as_array().unwrap().len(), 1);
    let detail = state["issues"][0]["modern_detail"].as_str().unwrap();
    assert!(detail.contains("[FV_STOPPED]"), "{detail}");
    let temporary = PathBuf::from(
        detail
            .split("temporary conversion evidence retained at ")
            .nth(1)
            .unwrap(),
    );
    let partial = temporary.join("out/sample.docx");
    assert_eq!(
        fs::read(&partial).unwrap(),
        b"interrupted mock capture evidence"
    );
    let resumed = f
        .command()
        .args(["conversion", "retry", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        resumed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(target.is_file());
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(
        fs::read(&partial).unwrap(),
        b"interrupted mock capture evidence"
    );
    let issues = f
        .command()
        .args(["conversion", "issues", "--json"])
        .output()
        .unwrap();
    assert_eq!(issues.status.code(), Some(0));
    let state: Value = serde_json::from_slice(&issues.stdout).unwrap();
    assert!(state["issues"].as_array().unwrap().is_empty());
    fs::remove_dir_all(temporary).unwrap(); // Only this fixture's own retained scratch.
}
fn interrupted_finalization(cooperative: bool) {
    let f = Fixture::new();
    let rtf = b"{\\rtf1\\ansi Disposable end-to-end FluxVault document}";
    let mut image = vec![0; 2880 * 512];
    image[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    image[3..11].copy_from_slice(b"FVTEST  ");
    image[11..13].copy_from_slice(&512u16.to_le_bytes());
    image[13] = 1;
    image[14..16].copy_from_slice(&1u16.to_le_bytes());
    image[16] = 2;
    image[17..19].copy_from_slice(&224u16.to_le_bytes());
    image[19..21].copy_from_slice(&2880u16.to_le_bytes());
    image[21] = 0xf0;
    image[22..24].copy_from_slice(&9u16.to_le_bytes());
    image[24..26].copy_from_slice(&18u16.to_le_bytes());
    image[26..28].copy_from_slice(&2u16.to_le_bytes());
    image[38] = 0x29;
    image[43..54].copy_from_slice(b"FV FIXTURE ");
    image[54..62].copy_from_slice(b"FAT12   ");
    image[510..512].copy_from_slice(&[0x55, 0xaa]);
    for at in [512, 10 * 512] {
        image[at..at + 5].copy_from_slice(&[0xf0, 0xff, 0xff, 0xff, 0x0f]);
    }
    let entry = 19 * 512;
    image[entry..entry + 11].copy_from_slice(b"NOTE    RTF");
    image[entry + 11] = 0x20;
    image[entry + 26..entry + 28].copy_from_slice(&2u16.to_le_bytes());
    image[entry + 28..entry + 32].copy_from_slice(&(rtf.len() as u32).to_le_bytes());
    image[33 * 512..33 * 512 + rtf.len()].copy_from_slice(rtf);
    let fixture = f.root.join("disposable-fat12.img");
    fs::write(&fixture, &image).unwrap();

    let mut acquisition = f
        .command()
        .args([
            "scan",
            "--last-disk",
            "1",
            "--acquisition-only",
            "--capture-storage",
            "raw",
            "--json",
        ])
        .env("MOCK_GW_IMAGE", &fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    acquisition.stdin.take().unwrap().write_all(b"1\n").unwrap();
    let acquired = acquisition.wait_with_output().unwrap();
    assert!(
        acquired.status.success(),
        "{}",
        String::from_utf8_lossy(&acquired.stderr)
    );
    fs::write(
        f.appdata.join("FluxVault/settings.json"),
        json!({
            "greaseweazle_path":env!("CARGO_BIN_EXE_mock_gw"),
            "seven_zip_path":env!("CARGO_BIN_EXE_mock_gw"),
            "libreoffice_path":env!("CARGO_BIN_EXE_mock_gw")
        })
        .to_string(),
    )
    .unwrap();
    let delivery = f.root.join("delivery");
    fs::create_dir(&delivery).unwrap();
    let image_path = f.project.images_dir().join("001_attempt_001.img");
    let original = fs::read(&image_path).unwrap();
    let mut run = Running(
        f.command()
            .args([
                "finalize",
                "--destination",
                delivery.to_str().unwrap(),
                "--allow-attention",
                "--conversion-workers",
                "1",
                "--json",
            ])
            .env("MOCK_GW_OFFICE_TREE", "1")
            .env("MOCK_GW_SEVENZIP_PROBE", "1")
            .env("MOCK_GW_TREE_ROOT", f.root.join("tree"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let (leader, worker) = f.tree();
    let active = f
        .command()
        .args(["finalize", "status", "--json"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&active.stdout).unwrap();
    assert_eq!(value["active"], true);
    assert_eq!(value["record"]["phase"], "processing");
    assert_eq!(value["record"]["options"]["workers"], 1);
    assert_eq!(value["record"]["options"]["allow_attention"], true);
    let competing = f.command().args(["process", "--json"]).output().unwrap();
    assert_eq!(competing.status.code(), Some(2));
    if cooperative {
        assert!(f.command().arg("stop").output().unwrap().status.success());
        assert_eq!(run.wait_stopped(), 130);
    } else {
        run.kill();
    }
    leader.exited();
    worker.exited();
    assert_eq!(fs::read(&image_path).unwrap(), original);
    assert_eq!(fs::read_dir(&delivery).unwrap().count(), 0);
    let stopped = f
        .command()
        .args(["finalize", "status", "--json"])
        .output()
        .unwrap();
    let stopped: Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(stopped["active"], false);
    assert_eq!(
        stopped["record"]["phase"],
        if cooperative {
            "interrupted"
        } else {
            "processing"
        }
    );
    let resumed = f
        .command()
        .args(["finalize", "resume", "--json"])
        .env("MOCK_GW_SEVENZIP_PROBE", "1")
        .output()
        .unwrap();
    assert!(
        matches!(resumed.status.code(), Some(0 | 3)),
        "{}\n{}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed: Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert!(
        resumed["package_status"]
            .as_str()
            .unwrap()
            .starts_with("verified_archival_zip")
    );
    assert_eq!(resumed["customer_delivery_certified"], false);
    let archive = Path::new(resumed["package"]["zip"].as_str().unwrap());
    assert!(archive.is_file());
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    assert!(zip.by_name("Reports/FinalReportLatest.json").is_ok());
    drop(zip);
    assert_eq!(fs::read(&image_path).unwrap(), original);
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn finalize_stop_cancels_office_tree_and_saved_settings_resume_to_archive() {
    interrupted_finalization(true);
}

#[test]
fn forced_finalize_exit_preserves_receipt_and_rechecks_on_resume_without_media() {
    interrupted_finalization(false);
}
fn interrupted_decode(cooperative: bool) {
    let f = Fixture::new();
    let mut run = f.scan(false, Some("MOCK_GW_CONVERT_TREE"));
    run.send("001");
    let (leader, worker) = f.tree();
    leader.alive();
    worker.alive();
    let capture = f.project.root().join("Flux/001_attempt_001.scp");
    let capture_bytes = fs::read(&capture).unwrap();
    let saved = partials(f.project.root());
    assert!(
        saved
            .iter()
            .any(|(p, _)| p.extension().is_some_and(|x| x == "img"))
    );
    if cooperative {
        assert!(f.command().arg("stop").output().unwrap().status.success());
        assert_eq!(run.wait_stopped(), 130);
    } else {
        run.kill();
    }
    leader.exited();
    worker.exited();
    assert!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .is_empty()
    );
    let audit = f.project.logs_dir().join("external-tools.jsonl");
    let read_count = |p: &Path| {
        fs::read_to_string(p)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|v| v["arguments"][0] == "read")
            .count()
    };
    assert_eq!(read_count(&audit), 1);
    let mut resumed = f.scan(false, None);
    resumed.send("001");
    assert_eq!(resumed.finish(), 0);
    assert_eq!(
        read_count(&audit),
        1,
        "saved flux must resume offline, not reread media"
    );
    assert_eq!(fs::read(&capture).unwrap(), capture_bytes);
    assert!(!f.project.root().join("Flux/001_attempt_002.scp").exists());
    let attempts = imaging::load_attempts_for_disk(&f.project.images_dir(), 1).unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].bad_sectors.is_empty());
    for (path, bytes) in saved {
        if cooperative && path.extension().is_some_and(|e| e == "json") {
            continue;
        }
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn exited_host_with_descendant_held_pipes_finishes_and_records_supervision() {
    let f = Fixture::new();
    let begun = Instant::now();
    let out = f
        .command()
        .args(["greaseweazle", "info", "--json"])
        .env("MOCK_GW_INFO_TREE", "1")
        .env("MOCK_GW_TREE_ROOT", f.root.join("tree"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(begun.elapsed() < Duration::from_secs(10));
    let audit = f.project.logs_dir().join("external-tools.jsonl");
    let entries = fs::read_to_string(audit).unwrap();
    assert!(entries.contains("windows_controller_and_operation_jobs"));
    let tree = wait_json(&f.root.join("tree/tree.json"));
    // If the OS has already removed the PID, it cannot still hold our output pipe.
    for pid in [
        tree["leader"].as_u64().unwrap(),
        tree["worker"].as_u64().unwrap(),
    ] {
        if let Ok(h) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid as u32) } {
            ProcessHandle(h).exited();
        }
    }
}

#[test]
fn bounded_timeout_stops_the_entire_operation_tree_without_waiting_for_controller_exit() {
    use fluxvault::greaseweazle::{
        GreaseweazleBackend, GreaseweazleCommand, GreaseweazleProfile, ProcessGreaseweazleBackend,
    };
    let f = Fixture::new();
    let mut backend = ProcessGreaseweazleBackend::new(
        PathBuf::from(env!("CARGO_BIN_EXE_mock_gw")),
        f.root.join("audit.jsonl"),
    )
    .unwrap()
    .with_timeout(Duration::from_secs(2))
    .with_env("MOCK_GW_READ_TREE", "1")
    .with_env("MOCK_GW_TREE_ROOT", f.root.join("tree").to_string_lossy());
    let result = backend
        .execute(
            &GreaseweazleCommand::raw_flux_read(
                GreaseweazleProfile::Ibm1440,
                'B',
                1,
                &f.root.join("test.partial.scp"),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(result.timed_out && !result.success);
    let tree = wait_json(&f.root.join("tree/tree.json"));
    for key in ["leader", "worker"] {
        if let Ok(h) = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE,
                false,
                tree[key].as_u64().unwrap() as u32,
            )
        } {
            ProcessHandle(h).exited();
        }
    }
    assert_eq!(
        fs::read(f.root.join("test.partial.scp")).unwrap(),
        b"interrupted mock capture evidence"
    );
    // Controller is still alive: the timeout killed only its operation job.
    assert!(
        backend
            .execute(&GreaseweazleCommand::info())
            .unwrap()
            .success
    );
}
