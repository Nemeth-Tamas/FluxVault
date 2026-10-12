//! Actual CLI mock host exercise: custom definitions stay unverified/isolated.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};
struct Fixture {
    root: PathBuf,
    project: PathBuf,
    app: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-expert-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let f = Self {
            project: root.join("project"),
            app: root.join("appdata"),
            root,
        };
        assert_eq!(
            f.call(&["init", f.project.to_str().unwrap()], false)
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            f.call(
                &[
                    "tools",
                    "set",
                    "greaseweazle",
                    env!("CARGO_BIN_EXE_mock_gw")
                ],
                false
            )
            .status
            .code(),
            Some(0)
        );
        f
    }
    fn call(&self, args: &[&str], missing: bool) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
        c.current_dir(&self.root)
            .args(args)
            .args(["--project", self.project.to_str().unwrap()])
            .env("APPDATA", &self.app);
        // init/tools don't need or accept an extra project switch for all commands.
        if args[0] == "init" || args[0] == "tools" {
            c = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
            c.current_dir(&self.root)
                .args(args)
                .env("APPDATA", &self.app);
        }
        if missing {
            c.env("MOCK_GW_DEVICE_NOT_FOUND", "1");
        } else {
            c.env_remove("MOCK_GW_DEVICE_NOT_FOUND");
        }
        c.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn value(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}; stderr {}", String::from_utf8_lossy(&o.stderr)))
}
#[test]
fn expert_capture_decode_binds_copied_definitions_hashes_and_never_promotes_catalog() {
    let f = Fixture::new();
    let defs = f.root.join("custom defs.cfg");
    fs::write(&defs, b"disk ibm.1440\n cyls 80\n heads 2\nend\n").unwrap();
    let captured = f.call(
        &[
            "greaseweazle",
            "expert",
            "capture",
            "7",
            "--format",
            "ibm.1440",
            "--diskdefs",
            defs.to_str().unwrap(),
            "--cylinders",
            "0-39",
            "--heads",
            "0",
            "--step",
            "2",
            "--source-write-protected",
            "--json",
        ],
        false,
    );
    assert_eq!(
        captured.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    let raw = value(&captured);
    let path = Path::new(raw["output"].as_str().unwrap());
    let before = fs::read(path).unwrap();
    assert_eq!(raw["automatic_donor_eligible"], false);
    assert!(!path.with_file_name("evidence.partial.scp").exists());
    let decoded = f.call(
        &[
            "greaseweazle",
            "expert",
            "decode",
            "7",
            "--format",
            "ibm.1440",
            "--input",
            path.to_str().unwrap(),
            "--expected-bytes",
            "1474560",
            "--json",
        ],
        true,
    );
    assert_eq!(
        decoded.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let image = value(&decoded);
    assert_eq!(image["output_bytes"], 1474560);
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read_dir(f.project.join("Images")).unwrap().count(), 0);
    let audit = fs::read_to_string(f.project.join("Logs/external-tools.jsonl")).unwrap();
    let last: Value = serde_json::from_str(audit.lines().last().unwrap()).unwrap();
    assert_eq!(last["arguments"][0], "convert");
    assert!(fs::read_dir(f.project.join("Flux/Expert")).unwrap().count() >= 2);
}
#[test]
fn expert_bad_size_missing_board_and_imported_definitions_preserve_failed_or_no_evidence() {
    let f = Fixture::new();
    let blocked = f.call(
        &[
            "greaseweazle",
            "expert",
            "capture",
            "1",
            "--format",
            "ibm.1440",
            "--source-write-protected",
            "--json",
        ],
        true,
    );
    assert_eq!(blocked.status.code(), Some(2));
    assert!(!f.project.join("Flux/Expert").exists());
    let defs = f.root.join("unsafe.cfg");
    fs::write(&defs, b"import 'outside.cfg'\n").unwrap();
    let rejected = f.call(
        &[
            "greaseweazle",
            "expert",
            "capture",
            "1",
            "--format",
            "ibm.1440",
            "--diskdefs",
            defs.to_str().unwrap(),
            "--source-write-protected",
            "--json",
        ],
        false,
    );
    assert_eq!(rejected.status.code(), Some(2));
    assert!(!f.project.join("Flux/Expert").exists());
    let captured = f.call(
        &[
            "greaseweazle",
            "expert",
            "capture",
            "1",
            "--format",
            "ibm.1440",
            "--source-write-protected",
            "--json",
        ],
        false,
    );
    let v = value(&captured);
    let fail = f.call(
        &[
            "greaseweazle",
            "expert",
            "decode",
            "1",
            "--format",
            "ibm.1440",
            "--input",
            v["output"].as_str().unwrap(),
            "--expected-bytes",
            "1",
            "--json",
        ],
        true,
    );
    assert_eq!(fail.status.code(), Some(2));
    let failed = fs::read_dir(f.project.join("Flux/Expert"))
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().contains("decode"))
        .unwrap()
        .path();
    assert!(failed.join("evidence.partial.img").exists());
    assert!(!failed.join("evidence.img").exists());
    let result: Value =
        serde_json::from_slice(&fs::read(failed.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["status"], "failed_partial_retained");
}
