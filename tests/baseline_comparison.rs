//! Offline comparison: no tools, no board, no physical drive.
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-baseline-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn project(&self) -> PathBuf {
        let root = self.0.join("project");
        fluxvault::project::ProjectState::create_without_session(root.clone()).unwrap();
        fs::write(root.join("Images/001.bin"), [0u8; 512]).unwrap();
        fs::create_dir(root.join("Extracted/001")).unwrap();
        root
    }
    fn zip(&self, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = self.0.join("reference.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn invoke(project: &Path, reference: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .current_dir(project)
        .args([
            "benchmark",
            "compare",
            "--baseline",
            reference.to_str().unwrap(),
            "--json",
        ])
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn deleted_is_opt_in_unscanned_is_excluded_and_sources_are_immutable() {
    let fixture = Fixture::new();
    let root = fixture.project();
    fs::write(root.join("Extracted/001/live file.doc"), b"live").unwrap();
    let list: Vec<u8> = "-- -- 4 ----- ---- . $Noname 01\\$Root\\live file.doc\r\n-- -- 7 ----- ---- x deleted.jpg\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let archive = fixture.zip(&[
        ("Images/001.bin", &[0u8; 512]),
        ("Extracted/001/$Noname 01/$Root/live file.doc", b"live"),
        ("Extracted/001/deleted.jpg", b"deleted"),
        ("Extracted/001/filelist.txt", &list),
        ("Extracted/002/unscanned.doc", b"other"),
        ("Converted/001/derivative.pdf", b"not a source"),
    ]);
    let before = Sha256::digest(fs::read(&archive).unwrap());
    let result = invoke(&root, &archive, &[]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let comparison = &value["comparison"];
    assert_eq!(comparison["reference_payloads"], 1);
    assert_eq!(comparison["matched_payloads"], 1);
    assert_eq!(comparison["missing_payloads"], 0);
    assert_eq!(
        comparison["excluded_reference_files"]["deleted_out_of_scope"],
        1
    );
    assert_eq!(
        comparison["unscanned_reference_disks"],
        serde_json::json!([2])
    );
    assert_eq!(comparison["disks"][0]["same_image_bytes"], true);
    assert_eq!(comparison["customer_delivery_certified"], false);
    let first_report = value["summary"].as_str().unwrap();
    let first_bytes = fs::read(first_report).unwrap();
    let result = invoke(&root, &archive, &["--include-deleted"]);
    assert_eq!(result.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["comparison"]["reference_payloads"], 2);
    assert_eq!(value["comparison"]["missing_payloads"], 1);
    assert_eq!(value["comparison"]["include_deleted"], true);
    assert_ne!(first_report, value["summary"].as_str().unwrap());
    assert_eq!(fs::read(first_report).unwrap(), first_bytes);
    assert_eq!(Sha256::digest(fs::read(&archive).unwrap()), before);
    assert_eq!(
        fs::read(root.join("Extracted/001/live file.doc")).unwrap(),
        b"live"
    );
    assert_eq!(fs::read(root.join("Images/001.bin")).unwrap(), [0u8; 512]);
}

#[test]
fn unsafe_zip_is_refused_without_reports() {
    let fixture = Fixture::new();
    let root = fixture.project();
    let archive = fixture.zip(&[("Extracted/001/../bad.doc", b"bad")]);
    assert_eq!(invoke(&root, &archive, &[]).status.code(), Some(2));
    assert_eq!(fs::read_dir(root.join("Reports")).unwrap().count(), 0);
}

#[test]
fn reference_mutation_and_case_duplicate_members_refuse_publication() {
    let fixture = Fixture::new();
    let root = fixture.project();
    let archive = fixture.zip(&[
        ("Extracted/001/a.doc", b"same"),
        ("Extracted/001/A.doc", b"same"),
    ]);
    assert_eq!(invoke(&root, &archive, &[]).status.code(), Some(2));
    let archive = fixture.zip(&[("Extracted/001/a.doc", b"same")]);
    let project = fluxvault::project::ProjectState::open_without_session(root.clone()).unwrap();
    let error = fluxvault::baseline::run(&project, &archive, false, &|stage| {
        if stage.starts_with("Comparing recovered") {
            fs::OpenOptions::new()
                .append(true)
                .open(&archive)
                .unwrap()
                .write_all(b"changed")
                .unwrap();
        }
    })
    .unwrap_err();
    assert!(error.contains("archive changed"), "{error}");
    assert_eq!(fs::read_dir(root.join("Reports")).unwrap().count(), 0);
}

#[test]
fn missing_classification_and_size_mismatch_do_not_hide_missing_payloads() {
    let fixture = Fixture::new();
    let root = fixture.project();
    let archive = fixture.zip(&[
        ("Extracted/001/deleted.doc", b"actually nine"),
        (
            "Extracted/001/filelist.txt",
            b"-- -- 4 ----- ---- x deleted.doc\r\n",
        ),
        ("Extracted/001/unknown.jpg", b"unknown"),
    ]);
    let output = invoke(&root, &archive, &[]);
    assert_eq!(output.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["comparison"]["reference_payloads"], 2);
    assert_eq!(value["comparison"]["missing_payloads"], 2);
}

#[test]
fn changed_renamed_and_duplicate_bytes_remain_distinct_and_flag_is_scoped() {
    let fixture = Fixture::new();
    let root = fixture.project();
    fs::write(root.join("Extracted/001/renamed.doc"), b"same").unwrap();
    fs::write(root.join("Extracted/001/changed.doc"), b"new").unwrap();
    let archive = fixture.zip(&[
        ("Extracted/001/original.doc", b"same"),
        ("Extracted/001/duplicate.doc", b"same"),
        ("Extracted/001/changed.doc", b"old"),
    ]);
    let output = invoke(&root, &archive, &[]);
    assert_eq!(output.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["comparison"]["matched_payloads"], 1);
    assert_eq!(value["comparison"]["missing_payloads"], 1);
    assert_eq!(value["comparison"]["changed_payloads"], 1);
    let invalid = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .current_dir(&root)
        .args(["process", "--include-deleted", "--json"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}

#[test]
fn preserved_managed_generation_is_verified_not_scored_as_missing() {
    let fixture = Fixture::new();
    let root = fixture.project();
    let output = root.join("Extracted/001/legacy_native_v2");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("live.doc"), b"live").unwrap();
    let image_sha = format!("{:x}", Sha256::digest([0u8; 512]));
    let marker = serde_json::json!({"schema_version":1,"source_image":"001.bin","source_sha256":image_sha,"extracted_unix_ms":0,"file_count":1,"total_bytes":4});
    let inventory = serde_json::json!({"schema_version":1,"source_image":"001.bin","source_sha256":image_sha,"files":[{"relative_path":"live.doc","bytes":4,"modified_unix_ms":null,"attributes":"","sha256":format!("{:x}", Sha256::digest(b"live"))}]});
    fs::write(
        output.join(".fluxvault-extraction.json"),
        marker.to_string(),
    )
    .unwrap();
    fs::write(
        output.join(".fluxvault-inventory.json"),
        inventory.to_string(),
    )
    .unwrap();
    let archive = fixture.zip(&[("Extracted/001/live.doc", b"live")]);
    let result = invoke(&root, &archive, &[]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["comparison"]["matched_payloads"], 1);
    assert_eq!(
        value["comparison"]["disks"][0]["extraction_binding_verified"],
        true
    );
    fs::write(output.join("live.doc"), b"tampered").unwrap();
    assert_eq!(invoke(&root, &archive, &[]).status.code(), Some(2));
}

#[test]
#[cfg(windows)]
fn powershell_launcher_collects_offline_results_without_scanning() {
    let fixture = Fixture::new();
    let root = fixture.project();
    fs::write(root.join("Extracted/001/live.doc"), b"live").unwrap();
    let archive = fixture.zip(&[("Extracted/001/live.doc", b"live")]);
    let result = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/start-pilot.ps1"))
        .args([
            "-CollectOnly",
            "-Project",
            root.to_str().unwrap(),
            "-Executable",
            env!("CARGO_BIN_EXE_fluxvault"),
            "-Baseline",
            archive.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let reports: Vec<_> = fs::read_dir(root.join("Reports"))
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("TestSummary-")
                && e.path().extension().is_some_and(|s| s == "json")
        })
        .collect();
    assert_eq!(reports.len(), 1);
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(reports[0].path()).unwrap()).unwrap();
    assert_eq!(value["collected_only"], true);
    assert_eq!(value["scan_exit"], serde_json::Value::Null);
    assert_eq!(value["comparison"]["comparison"]["matched_payloads"], 1);
    assert!(!root.join(".fluxvault-gw-scan.json").exists());
    assert!(!root.join("Logs/external-tools.jsonl").exists());
}
