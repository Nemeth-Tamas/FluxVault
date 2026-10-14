//! Executable saved-only snapshot generation and separate baseline accounting.
use fluxvault::{imaging, project::ProjectState};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fv-snapshot-cli-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn project(&self) -> ProjectState {
        ProjectState::create_without_session(self.0.join("project")).unwrap()
    }
    fn zip(&self, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = self.0.join("reference.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, bytes) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn command(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn image() -> Vec<u8> {
    let mut b = vec![0; 2880 * 512];
    b[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    b[3..11].copy_from_slice(b"FVTEST  ");
    b[11..13].copy_from_slice(&512u16.to_le_bytes());
    b[13] = 1;
    b[14..16].copy_from_slice(&1u16.to_le_bytes());
    b[16] = 2;
    b[17..19].copy_from_slice(&224u16.to_le_bytes());
    b[19..21].copy_from_slice(&2880u16.to_le_bytes());
    b[21] = 0xf0;
    b[22..24].copy_from_slice(&9u16.to_le_bytes());
    b[24..26].copy_from_slice(&18u16.to_le_bytes());
    b[26..28].copy_from_slice(&2u16.to_le_bytes());
    b[510..512].copy_from_slice(&[0x55, 0xaa]);
    for at in [512, 10 * 512] {
        b[at..at + 5].copy_from_slice(&[0xf0, 0xff, 0xff, 0xff, 0x0f]);
    }
    let at = 19 * 512;
    b[at..at + 11].copy_from_slice(b"NOTE    TXT");
    b[at + 11] = 0x20;
    b[at + 26..at + 28].copy_from_slice(&2u16.to_le_bytes());
    b[at + 28..at + 32].copy_from_slice(&10u32.to_le_bytes());
    b[33 * 512..33 * 512 + 10].copy_from_slice(b"candidate!");
    b
}
#[test]
fn executable_preview_publication_and_comparison_do_not_promote_or_double_count_candidates() {
    let f = Fixture::new();
    let p = f.project();
    let bytes = image();
    fs::write(p.images_dir().join("001.bin"), &bytes).unwrap();
    let preview = command(p.root(), &["recovery", "snapshot", "1", "--plan", "--json"]);
    assert_eq!(preview.status.code(), Some(0));
    assert_eq!(
        json(&preview)["report"]["files"].as_array().unwrap().len(),
        1
    );
    assert!(!p.recovery_dir().join("SnapshotCandidates").exists());
    let saved = command(p.root(), &["recovery", "snapshot", "1", "--json"]);
    assert_eq!(saved.status.code(), Some(3));
    let output = json(&saved);
    assert_eq!(output["report"]["physical_readability"], "unknown");
    assert!(!p.extracted_dir().join("001").exists());
    let reused = command(p.root(), &["recovery", "snapshot", "1", "--json"]);
    assert_eq!(json(&reused)["reused"], true);
    let reference = f.zip(&[
        ("Images/001.bin", &bytes),
        ("Extracted/001/$Root/renamed.txt", b"candidate!"),
        ("Extracted/001/$Root/duplicate.txt", b"candidate!"),
    ]);
    let compared = command(
        p.root(),
        &[
            "benchmark",
            "compare",
            "--baseline",
            reference.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(compared.status.code(), Some(3));
    let v = json(&compared);
    let c = &v["comparison"];
    assert_eq!(c["matched_payloads"], 0);
    assert_eq!(c["missing_payloads"], 2);
    assert_eq!(
        c["snapshot_candidates"][0]["additional_exact_reference_matches"],
        1
    );
    assert_eq!(
        c["snapshot_candidates"][0]["normal_payload_totals_changed"],
        false
    );
    let candidate = PathBuf::from(output["output_directory"].as_str().unwrap())
        .join(output["report"]["files"][0]["path"].as_str().unwrap());
    fs::write(&candidate, b"tampered!").unwrap();
    let refused = command(
        p.root(),
        &[
            "benchmark",
            "compare",
            "--baseline",
            reference.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!refused.status.success());
    assert!(json(&refused)["error"].is_object());
    assert_eq!(fs::read(p.images_dir().join("001.bin")).unwrap(), bytes);
    assert_eq!(
        imaging::load_attempts_for_disk(&p.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn automatic_extraction_retains_recovery_attention_but_materializes_snapshot_candidates() {
    automatic_fixture(false);
}

#[test]
fn automatic_extraction_routes_extent_contradiction_to_separate_candidates_not_seven_zip() {
    automatic_fixture(true);
}

fn automatic_fixture(overclaimed_extent: bool) {
    let f = Fixture::new();
    let p = f.project();
    let bytes = image();
    fs::write(p.images_dir().join("001.bin"), &bytes).unwrap();
    let log = if overclaimed_extent {
        use sha2::{Digest, Sha256};
        let text = format!(
            "BEGIN floppy=001\nGEOMETRY cylinders=80 heads=2 sectors_per_track=18 bytes_per_sector=512 total_sectors=2881 total_bytes=1475072\nSHA256 {:x}\nEND status=OK bad_sectors=0 recovered_after_retry=0 track_fallbacks=0 duration_seconds=1.0\n",
            Sha256::digest(&bytes)
        );
        fs::write(p.logs_dir().join("001.log"), &text).unwrap();
        assert!(imaging::load_attempts_for_disk(&p.images_dir(), 1).unwrap()[0].attention_required);
        Some(text)
    } else {
        None
    };
    let request = fluxvault::batch_extraction::BatchExtractionRequest {
        seven_zip_executable: PathBuf::from(env!("CARGO_BIN_EXE_mock_gw")),
        images_directory: p.images_dir(),
        logs_directory: p.logs_dir(),
        extracted_root: p.extracted_dir(),
        recovery_root: p.recovery_dir(),
        reports_directory: p.reports_dir(),
        command_audit_path: p.logs_dir().join("external-tools.jsonl"),
    };
    let events = fluxvault::batch_extraction::spawn_batch_extraction(request);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let result = loop {
        let event = events
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
        if let fluxvault::batch_extraction::BatchExtractionEvent::Finished(result) = event {
            break (*result).unwrap();
        }
    };
    assert_eq!(result.total_disks, 1);
    assert_eq!(result.extracted_disks, 0);
    assert_eq!(result.recovery_disks, 1);
    assert!(
        fs::read_to_string(&result.summary_path)
            .unwrap()
            .contains("snapshot byte candidates saved separately")
    );
    assert!(p.recovery_dir().join("SnapshotCandidates/001").is_dir());
    assert!(!p.extracted_dir().join("001").exists());
    assert_eq!(fs::read(p.images_dir().join("001.bin")).unwrap(), bytes);
    if let Some(log) = log {
        assert_eq!(
            fs::read_to_string(p.logs_dir().join("001.log")).unwrap(),
            log
        );
    }
    assert!(!p.logs_dir().join("external-tools.jsonl").exists());
}
