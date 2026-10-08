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

#[test]
fn forced_decode_exit_reuses_completed_flux_without_reading_media_again() {
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
    run.kill();
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
