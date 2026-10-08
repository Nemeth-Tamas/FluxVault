//! Real CLI/process lifecycle with mock GW only. Never sends a USB read command.
use fluxvault::{imaging, project::ProjectState};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    project: ProjectState,
    appdata: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-dual-cli-{}-{}",
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
        let mut command = self.command();
        command.args([
            "scan",
            "--double",
            "--write-blocker-verified",
            "--acquisition-only",
            "--json",
            "--color",
            "never",
        ]);
        if last {
            command.args(["--last-disk", "3"]);
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
            if let Ok(bytes) = fs::read(path)
                && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
                && v["disks"][disk.to_string()]["phase"] == phase
            {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out waiting for {disk} {phase}"
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
    assert_eq!(value["report_schema"], 2);
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
        assert!(!f.project.root().join(".fluxvault-production.json").exists());
        assert!(!f.project.root().join(".fluxvault-processing.lock").exists());
    }
}
