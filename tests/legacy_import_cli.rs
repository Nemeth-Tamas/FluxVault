//! Saved ZIPs only: importing must never require tools, hardware or a profile.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-legacy-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn archive(&self) -> PathBuf {
        let path = self.0.join("legacy.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, bytes) in [
            ("Images/001.bin", b"saved image".as_slice()),
            ("Extracted/001/original.doc", b"legacy original".as_slice()),
            ("Converted/001/original.pdf", b"legacy PDF".as_slice()),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        path
    }
    fn invoke(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_fluxvault"))
            .current_dir(&self.0)
            .args(args)
            .env("APPDATA", self.0.join("isolated-settings"))
            .env("LOCALAPPDATA", self.0.join("isolated-local"))
            .output()
            .unwrap()
    }
    fn target(&self) -> PathBuf {
        self.0.join("Restored")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn value(output: &Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn sha(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    format!("{:x}", digest.finalize())
}

#[test]
fn offline_cli_plan_import_status_and_repeat_are_guarded() {
    let f = Fixture::new();
    let archive = f.archive();
    let before = sha(&archive);
    let plan = f.invoke(&[
        "project",
        "import",
        "--source",
        "legacy.zip",
        "--destination",
        "Restored",
        "--plan",
        "--json",
    ]);
    assert_eq!(value(&plan, 0)["images"], 1);
    assert!(!f.target().exists());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    let imported = f.invoke(&[
        "project",
        "import",
        "--source",
        "legacy.zip",
        "--destination",
        "Restored",
        "--json",
    ]);
    let result = value(&imported, 3);
    assert_eq!(result["published"], true);
    assert_eq!(result["customer_delivery_certified"], false);
    assert_eq!(sha(&archive), before);
    let project = f.target();
    let project = project.to_str().unwrap();
    let shown = f.invoke(&["project", "show", "--project", project, "--json"]);
    value(&shown, 0);
    let disk = f.invoke(&[
        "disk",
        "show",
        "1",
        "--project",
        project,
        "--details",
        "--json",
    ]);
    value(&disk, 0);
    let control = f.invoke(&["run", "status", "--project", project, "--json"]);
    assert_eq!(value(&control, 0)["active"], false);
    let metadata = fs::read(f.target().join("project.json")).unwrap();
    let repeat = f.invoke(&[
        "project",
        "import",
        "--source",
        "legacy.zip",
        "--destination",
        "Restored",
        "--json",
    ]);
    assert_eq!(repeat.status.code(), Some(2));
    assert_eq!(fs::read(f.target().join("project.json")).unwrap(), metadata);
    assert_eq!(
        fs::read(f.target().join("Extracted/001/original.doc")).unwrap(),
        b"legacy original"
    );
    assert!(!f.0.join("isolated-settings").exists());
    assert!(!f.0.join("isolated-local").exists());
}

#[test]
fn acquisition_flags_duplicate_options_and_missing_arguments_are_rejected() {
    let f = Fixture::new();
    let archive = f.archive();
    let before = sha(&archive);
    for extra in [
        vec!["--no-verify"],
        vec!["--drive", "A:"],
        vec!["--project", "A:"],
        vec!["--source", "legacy.zip"],
        vec!["--plan", "--plan"],
    ] {
        let mut args = vec![
            "project",
            "import",
            "--source",
            "legacy.zip",
            "--destination",
            "Restored",
            "--plan",
            "--json",
        ];
        args.extend(extra);
        let output = f.invoke(&args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for args in [
        vec!["project", "import"],
        vec!["project", "import", "--source", "legacy.zip"],
        vec![
            "project",
            "import",
            "--source",
            "A:",
            "--destination",
            "Restored",
        ],
    ] {
        assert_eq!(f.invoke(&args).status.code(), Some(2));
    }
    assert_eq!(sha(&archive), before);
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
}

#[test]
#[ignore = "Set FV_LEGACY_SOURCE to the original script ZIP; copies only into an isolated temporary fixture"]
fn original_136_script_zip_import_preserves_every_payload_and_short_image() {
    let f = Fixture::new();
    let source = PathBuf::from(std::env::var_os("FV_LEGACY_SOURCE").expect("FV_LEGACY_SOURCE"));
    let before = sha(&source);
    let output = f.invoke(&[
        "project",
        "import",
        "--source",
        source.to_str().unwrap(),
        "--destination",
        "Restored",
        "--json",
    ]);
    let result = value(&output, 3);
    assert_eq!(result["images"], 136);
    assert_eq!(result["next_disk"], 137);
    assert_eq!(result["member_payloads_verified"], true);
    let report: Value =
        serde_json::from_slice(&fs::read(f.target().join("Reports/LegacyImport.json")).unwrap())
            .unwrap();
    let disks = report["disks"].as_array().unwrap();
    let dd = disks.iter().find(|d| d["disk"] == 9).unwrap();
    assert_eq!(dd["image_bytes"], 737280);
    let cropped = disks.iter().find(|d| d["disk"] == 133).unwrap();
    assert_eq!(cropped["image_bytes"], 417792);
    assert_eq!(cropped["attention_required"], true);
    let known = disks.iter().find(|d| d["disk"] == 7).unwrap();
    assert_eq!(known["legacy_status"], "OK");
    assert_eq!(known["recorded_log_sha256_matches"], true);
    let files = report["files"].as_array().unwrap();
    for member in files {
        let path = f.target().join(member["path"].as_str().unwrap());
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            member["bytes"].as_u64().unwrap()
        );
        assert_eq!(sha(&path), member["sha256"].as_str().unwrap());
    }
    let mut original = zip::ZipArchive::new(fs::File::open(&source).unwrap()).unwrap();
    let mut copied = 0usize;
    for i in 0..original.len() {
        let mut entry = original.by_index(i).unwrap();
        if entry.is_dir() {
            continue;
        }
        let path = entry.name().replace('\\', "/");
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = entry.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
        }
        assert_eq!(
            sha(&f.target().join(&path)),
            format!("{:x}", digest.finalize()),
            "{path}"
        );
        copied += 1;
    }
    assert_eq!(files.len(), copied);
    assert_eq!(sha(&source), before);
    println!(
        "136-image import: {} files / {} bytes / statuses={} / discrepancies={} / disk133extent={}",
        files.len(),
        result["source_bytes"],
        result["legacy_status_counts"],
        result["historical_claim_discrepancies"],
        cropped["reported_extent_matches_saved_image"]
    );
    println!(
        "Archive-index historical statuses={}; log-hash mismatches={}; extent mismatches={}; index-hash mismatches={}",
        result["archive_index_status_counts"],
        disks
            .iter()
            .filter(|d| d["recorded_log_sha256_matches"] == false)
            .count(),
        disks
            .iter()
            .filter(|d| d["reported_extent_matches_saved_image"] == false)
            .count(),
        disks
            .iter()
            .flat_map(|d| d["archive_index_claims"].as_array().unwrap())
            .filter(|c| c["image_sha256_matches"] == false)
            .count()
    );
}
