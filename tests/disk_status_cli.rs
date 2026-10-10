//! Real executable inspection/annotation contracts; synthetic evidence only.
use fluxvault::{
    production::{Coordinator, Station},
    project::ProjectState,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(ProjectState);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-disk-status-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Self(ProjectState::create_without_session(root).unwrap())
    }
    fn image(&self) {
        let bytes = vec![b'X'; 2048];
        let image = self.0.images_dir().join("001_attempt_001.img");
        let log = self.0.logs_dir().join("001_attempt_001.log");
        let sha = format!("{:x}", Sha256::digest(&bytes));
        fs::write(&image, &bytes).unwrap();
        fs::write(&log,format!("BEGIN | disk=1 | attempt=1\nGEOMETRY | cylinders=1 | heads=2 | sectors_per_track=2 | bytes_per_sector=512 | total_sectors=4 | total_bytes=2048\nEND | status=OK | bytes=2048 | sha256={sha}\n")).unwrap();
        fs::write(self.0.images_dir().join("001_attempt_001.json"),serde_json::to_vec(&json!({"disk_number":1,"attempt_number":1,"status":"OK","image_file":image,"log_file":log,
            "fluxvault_version":"fixture","source_device":"mock","timestamp_unix_ms":1,"sector_retries":0,"retry_recovered_sectors":0,
            "geometry":{"cylinders":1,"heads":2,"sectors_per_track":2,"bytes_per_sector":512,"total_bytes":2048,"format_guess":"fixture"},
            "bytes_written":2048,"total_sectors":4,"bad_sector_count":0,"bad_sectors":[],"sha256":sha})).unwrap()).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_fluxvault"))
            .args(args)
            .current_dir(self.0.root())
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(self.0.root()).unwrap();
    }
}

fn inventory(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), files)
            } else {
                files.push((
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    if entry.file_name().to_string_lossy().ends_with(".lock") {
                        entry.metadata().unwrap().len().to_le_bytes().to_vec()
                    } else {
                        fs::read(entry.path()).unwrap()
                    },
                ));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort();
    files
}

#[test]
fn empty_dashboard_is_machine_readable_and_creates_no_controls_or_fake_eta() {
    let fixture = Fixture::new();
    let before = inventory(fixture.0.root());
    let output = fixture.run(&["status", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["operations"]["schema_version"], 1);
    assert_eq!(value["operations"]["physical_media_access"], false);
    assert_eq!(value["operations"]["stations"], json!([]));
    assert!(value["operations"]["timing"]["rough_feed_eta_minutes"].is_null());
    assert_eq!(value["operations"]["run"]["active"], false);
    assert!(output.stderr.is_empty());
    let human = fixture.run(&["status"]);
    assert!(human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("ETA: unavailable"));
    assert_eq!(inventory(fixture.0.root()), before);
}

#[test]
fn persisted_read_never_becomes_live_reader_proof_or_a_removal_instruction() {
    let fixture = Fixture::new();
    let mut coordinator = Coordinator::open(fixture.0.clone(), Some(20), false).unwrap();
    let ticket = coordinator.claim(Station::Usb, 1).unwrap();
    coordinator.confirm(&ticket, "1", true).unwrap();
    let output = fixture.run(&["status", "--json"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let station = &value["operations"]["stations"][0];
    assert_eq!(station["station"], "usb");
    assert_eq!(station["disk"], 1);
    assert!(station["reader_active"].is_null());
    assert!(
        station["next_action"]
            .as_str()
            .unwrap()
            .contains("DO NOT REMOVE")
    );
    drop(coordinator);
    let before = inventory(fixture.0.root());
    let output = fixture.run(&["status", "--json"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["operations"]["run"]["active"], false);
    assert!(value["operations"]["stations"][0]["reader_active"].is_null());
    assert_eq!(inventory(fixture.0.root()), before);
}

#[test]
fn single_pending_station_is_bounded_and_never_uses_cursor_as_saved_evidence() {
    let fixture = Fixture::new();
    let path = fixture.0.root().join(".fluxvault-gw-scan.json");
    let journal = json!({"schema_version":1,"profile":"ibm.1440","drive":"B",
        "policy":fluxvault::flux_recovery::RecoveryPolicy::default(),"pending":{"disk":1,"result":null},"completed":[],"last_disk":20});
    fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let before = inventory(fixture.0.root());
    let output = fixture.run(&["status", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["operations"]["stations"][0]["disk"], 1);
    assert_eq!(
        value["operations"]["stations"][0]["recorded_phase"],
        "pending"
    );
    assert!(
        value["operations"]["stations"][0]["next_action"]
            .as_str()
            .unwrap()
            .contains("DO NOT REMOVE")
    );
    assert_eq!(inventory(fixture.0.root()), before);
    fs::write(&path, vec![b' '; 8 * 1024 * 1024 + 1]).unwrap();
    let output = fixture.run(&["status", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Oversized")
    );
}

#[test]
fn notes_and_lifecycle_preserve_evidence_cursor_and_old_notes_and_are_optional() {
    let fixture = Fixture::new();
    fixture.image();
    let image = fs::read(fixture.0.images_dir().join("001_attempt_001.img")).unwrap();
    let control = fs::read(fixture.0.project_file()).unwrap();
    for text in ["Folded sleeve; leave original intact", "Updated note", ""] {
        let output = fixture.run(&["disk", "note", "1", text, "--json"]);
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["note"]["text"], text);
    }
    let dir = fixture.0.reports_dir().join("DiskNotes");
    assert_eq!(
        fs::read_dir(&dir)
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".history.json"))
            .count(),
        2
    );
    let before = inventory(fixture.0.root());
    let output = fixture.run(&["disk", "show", "1", "--details", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["lifecycle"]["label"], "001");
    assert_eq!(value["lifecycle"]["preferred_image"]["attempt"], 1);
    assert_eq!(value["lifecycle"]["extraction"]["state"], "missing");
    assert_eq!(value["lifecycle"]["recovery"]["action"], "complete");
    assert!(value["lifecycle"]["audit"]["record"].is_null());
    assert_eq!(value["lifecycle"]["physical_media_access"], false);
    assert!(output.stderr.is_empty());
    assert_eq!(inventory(fixture.0.root()), before);
    assert_eq!(fs::read(fixture.0.project_file()).unwrap(), control);
    assert_eq!(
        fs::read(fixture.0.images_dir().join("001_attempt_001.img")).unwrap(),
        image
    );
}

#[test]
fn invalid_notes_owner_conflicts_and_foreign_reports_refuse_without_overwrite() {
    let fixture = Fixture::new();
    fixture.image();
    let large = "x".repeat(4097);
    for args in [
        vec!["disk", "note", "0", "note", "--json"],
        vec!["disk", "note", "1", &large, "--json"],
        vec!["disk", "note", "1", "note\u{1b}", "--json"],
    ] {
        let before = inventory(fixture.0.root());
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(inventory(fixture.0.root()), before);
    }
    let coordinator = Coordinator::open(fixture.0.clone(), Some(20), false).unwrap();
    let before = inventory(fixture.0.root());
    let output = fixture.run(&["disk", "note", "1", "note", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(inventory(fixture.0.root()), before);
    drop(coordinator);
    fs::write(
        fixture.0.reports_dir().join("EvidenceAudit.json"),
        serde_json::to_vec(&json!({"schema_version":1,"project":"foreign","disks":[]})).unwrap(),
    )
    .unwrap();
    let before = inventory(fixture.0.root());
    let output = fixture.run(&["disk", "show", "1", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(inventory(fixture.0.root()), before);
}

#[test]
fn historical_audit_is_marked_stale_after_preferred_hash_changes() {
    let fixture = Fixture::new();
    fixture.image();
    fs::write(fixture.0.reports_dir().join("EvidenceAudit.json"),serde_json::to_vec(&json!({"schema_version":1,"project":fixture.0.name(),"disks":[{"disk":"001","attempt":1,"image_sha256":"0".repeat(64),"evidence_status":"IMAGE_FILES_CONVERSIONS_VERIFIED"}]})).unwrap()).unwrap();
    let output = fixture.run(&["disk", "show", "1", "--json"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["lifecycle"]["audit"]["matches_preferred_image"],
        false
    );
    assert_eq!(value["lifecycle"]["audit"]["historical_report_only"], true);
    assert_eq!(
        value["lifecycle"]["audit"]["fresh_integrity_audit_performed"],
        false
    );
}
