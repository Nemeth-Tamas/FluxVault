//! Force exit between completed modern output and unfinished PDF; mock Office only.
use fluxvault::{
    conversion::{ConversionPlanningEvent, ConversionPlanningRequest, spawn_conversion_planning},
    project::ProjectState,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    project: ProjectState,
    app: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-office-resume-cli-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        fs::create_dir(project.extracted_dir().join("001")).unwrap();
        fs::write(
            project.extracted_dir().join("001/Notes.doc"),
            b"{\\rtf1 Source publication fixture}",
        )
        .unwrap();
        let app = root.join("appdata");
        fs::create_dir_all(app.join("FluxVault")).unwrap();
        fs::write(
            app.join("FluxVault/settings.json"),
            json!({"libreoffice_path":env!("CARGO_BIN_EXE_mock_gw")}).to_string(),
        )
        .unwrap();
        Self { root, project, app }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
        c.current_dir(self.project.root())
            .env("APPDATA", &self.app)
            .stdin(Stdio::null());
        c
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn completed_output_survives_real_controller_exit_before_batch_state_and_is_not_reconverted() {
    let f = Fixture::new();
    let events = spawn_conversion_planning(ConversionPlanningRequest {
        extracted_root: f.project.extracted_dir(),
        converted_root: f.project.converted_dir(),
        reports_directory: f.project.reports_dir(),
    });
    let plan = loop {
        if let ConversionPlanningEvent::Finished(r) =
            events.recv_timeout(Duration::from_secs(10)).unwrap()
        {
            break r.unwrap();
        }
    };
    let modern = &plan.jobs[0].modern_path;
    let tree = f.root.join("tree");
    fs::create_dir(&tree).unwrap();
    let mut child = f
        .command()
        .args(["conversion", "run", "--conversion-workers", "1", "--json"])
        .env("MOCK_GW_OFFICE_HANG_FORMAT", "pdf")
        .env("MOCK_GW_TREE_ROOT", &tree)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(25);
    while !(modern.exists() && tree.join("tree.json").exists()) {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Mock PDF did not start after modern output publication");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let bytes = fs::read(modern).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        !f.project
            .reports_dir()
            .join("ConversionState.json")
            .exists()
    );
    let resumed = f
        .command()
        .args(["conversion", "run", "--conversion-workers", "1", "--json"])
        .output()
        .unwrap();
    assert!(
        matches!(resumed.status.code(), Some(0 | 3)),
        "{} {}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    let v: Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert!(v["reused_outputs"].as_u64().is_some_and(|n| n >= 1), "{v}");
    assert_eq!(fs::read(modern).unwrap(), bytes);
    assert!(plan.jobs[0].pdf_path.exists());
    let mut audit = String::new();
    fs::File::open(f.project.logs_dir().join("external-tools.jsonl"))
        .unwrap()
        .read_to_string(&mut audit)
        .unwrap();
    let outputs = audit
        .lines()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .filter(|v| v["tool"] == "LibreOffice conversion")
        .collect::<Vec<_>>();
    assert_eq!(
        outputs
            .iter()
            .filter(|v| v["arguments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a.as_str().is_some_and(|s| s.starts_with("docx:"))))
            .count(),
        1
    );
    assert!(outputs.iter().any(|v| {
        v["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a.as_str().is_some_and(|s| s.starts_with("pdf:")))
    }));
}
