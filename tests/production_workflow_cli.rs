//! Entire production controller exercised with mock GW/Office and disposable FAT.
//! No physical drive/device APIs are requested in these single-GW fixtures.
use fluxvault::project::ProjectState;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct Fixture {
    root: PathBuf,
    project: ProjectState,
    app: PathBuf,
    image: PathBuf,
}
impl Fixture {
    fn new(document: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-production-flow-cli-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        let app = root.join("appdata");
        fs::create_dir_all(app.join("FluxVault")).unwrap();
        fs::write(app.join("FluxVault/settings.json"),json!({"greaseweazle_path":env!("CARGO_BIN_EXE_mock_gw"),"seven_zip_path":env!("CARGO_BIN_EXE_mock_gw"),"libreoffice_path":env!("CARGO_BIN_EXE_mock_gw")}).to_string()).unwrap();
        let mut bytes = vec![0; 2880 * 512];
        bytes[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
        bytes[3..11].copy_from_slice(b"FVTEST  ");
        bytes[11..13].copy_from_slice(&512u16.to_le_bytes());
        bytes[13] = 1;
        bytes[14..16].copy_from_slice(&1u16.to_le_bytes());
        bytes[16] = 2;
        bytes[17..19].copy_from_slice(&224u16.to_le_bytes());
        bytes[19..21].copy_from_slice(&2880u16.to_le_bytes());
        bytes[21] = 0xf0;
        bytes[22..24].copy_from_slice(&9u16.to_le_bytes());
        bytes[24..26].copy_from_slice(&18u16.to_le_bytes());
        bytes[26..28].copy_from_slice(&2u16.to_le_bytes());
        bytes[38] = 0x29;
        bytes[43..54].copy_from_slice(b"FV FIXTURE ");
        bytes[54..62].copy_from_slice(b"FAT12   ");
        bytes[510..512].copy_from_slice(&[0x55, 0xaa]);
        for at in [512, 10 * 512] {
            bytes[at..at + 5].copy_from_slice(&[0xf0, 0xff, 0xff, 0xff, 0x0f]);
        }
        let payload: &[u8] = if document {
            b"{\\rtf1\\ansi Disposable production document}"
        } else {
            b"Exact production fixture payload"
        };
        let at = 19 * 512;
        bytes[at..at + 11].copy_from_slice(if document {
            b"NOTE    RTF"
        } else {
            b"NOTE    TXT"
        });
        bytes[at + 11] = 0x20;
        bytes[at + 26..at + 28].copy_from_slice(&2u16.to_le_bytes());
        bytes[at + 28..at + 32].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[33 * 512..33 * 512 + payload.len()].copy_from_slice(payload);
        let image = root.join("fixture.img");
        fs::write(&image, bytes).unwrap();
        Self {
            root,
            project,
            app,
            image,
        }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_fluxvault"));
        c.current_dir(self.project.root())
            .env("APPDATA", &self.app)
            .env("MOCK_GW_IMAGE", &self.image)
            .env("MOCK_GW_SEVENZIP_PROBE", "1");
        c
    }
    fn run(&self, args: &[&str], input: &[u8], env: &[(&str, &str)]) -> Output {
        let mut command = self.command();
        // Unknown-format decodes must not receive the HD-sized FAT fixture.
        if env
            .iter()
            .any(|(k, v)| *k == "MOCK_GW_MEDIA_FORMAT" && *v != "ibm.1440")
        {
            command.env_remove("MOCK_GW_IMAGE");
        }
        let mut child = command
            .args(args)
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }
    fn record(&self) -> Value {
        serde_json::from_slice(
            &fs::read(
                self.project
                    .root()
                    .join(".fluxvault-production-workflow.json"),
            )
            .unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn value(o: &Output) -> Value {
    assert!(
        matches!(o.status.code(), Some(0 | 3)),
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
#[test]
fn production_archives_at_endpoint_and_repeated_start_never_reads_again() {
    let f = Fixture::new(false);
    let out = f.run(
        &["production", "start", "--last-disk", "1", "--json"],
        b"1\n",
        &[],
    );
    let r = value(&out);
    assert!(r["finalization"]["package"].is_object());
    let package = PathBuf::from(r["finalization"]["package"]["zip"].as_str().unwrap());
    let before = fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap();
    assert_eq!(before, fs::read(&f.image).unwrap());
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&package).unwrap())),
        r["finalization"]["package"]["sha256"]
    );
    let mut archive = zip::ZipArchive::new(fs::File::open(&package).unwrap()).unwrap();
    for i in 0..archive.len() {
        let mut member = archive.by_index(i).unwrap();
        std::io::copy(&mut member, &mut std::io::sink()).unwrap();
    }
    assert!(r["customer_delivery_certified"] == false);
    let control: Value = serde_json::from_slice(
        &fs::read(f.project.root().join(".fluxvault-run-control.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(control["operation"], "production_start");
    let again = f.run(
        &["production", "start", "--json"],
        b"",
        &[("MOCK_GW_DEVICE_NOT_FOUND", "1")],
    );
    assert_eq!(value(&again)["already_finished"], true);
    assert_eq!(
        fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap(),
        before
    );
    assert!(!f.project.images_dir().join("001_attempt_002.img").exists());
    assert!(
        !f.record()["scan_args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "--no-verify")
    );
}
#[test]
fn early_quit_retains_feed_state_and_does_not_archive_then_resumes() {
    let f = Fixture::new(false);
    let a = f.run(
        &[
            "production",
            "start",
            "--last-disk",
            "2",
            "--no-verify",
            "--json",
        ],
        b"\nQUIT\n",
        &[],
    );
    assert_eq!(a.status.code(), Some(3));
    assert!(value(&a)["package"].is_null());
    assert_eq!(f.record()["phase"], "waiting_for_disks");
    let status = value(&f.run(&["production", "status", "--json"], b"", &[]));
    assert_eq!(status["workflow"]["record"]["phase"], "waiting_for_disks");
    assert_eq!(status["workflow"]["active"], false);
    let human = f.run(&["production", "status"], b"", &[]);
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("Production: 1..2 / waiting_for_disks (inactive)"));
    assert!(human.contains("Continue: fv production resume"));
    assert_eq!(
        fs::read_dir(f.root.join("project-Delivery"))
            .unwrap()
            .count(),
        0
    );
    assert!(
        !f.record()["scan_args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "--no-verify")
    );
    let b = f.run(&["production", "resume", "--json"], b"2\n", &[]);
    assert!(value(&b)["finalization"]["package"].is_object());
    assert_eq!(
        fs::read_dir(f.project.images_dir())
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|s| s == "img"))
            .count(),
        2
    );
}

#[test]
fn dual_production_uses_shared_gw_scheduler_and_finishes_without_a_usb_read() {
    let f = Fixture::new(false);
    // Only g1 is submitted: reserve the USB station, never open/read its media.
    let out = f.run(
        &[
            "production",
            "start",
            "--double",
            "--write-blocker-verified",
            "--last-disk",
            "1",
            "--json",
        ],
        b"g1\n",
        &[],
    );
    let r = value(&out);
    assert_eq!(r["workflow"]["dual"], true);
    assert!(r["finalization"]["package"].is_object());
    // This dual fixture uses native recovery fallback, whose completeness
    // warning must survive even though the acquisition has zero missing sectors.
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(r["workflow"]["phase"], "complete_attention");
    assert_eq!(r["workflow"]["acquisition_attention"], false);
    let audit: Value = serde_json::from_slice(
        &fs::read(f.project.reports_dir().join("EvidenceAudit.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(audit["disks"][0]["bad_sectors"], 0);
    assert_eq!(audit["disks"][0]["extracted_hashes_verified"], true);
    assert!(
        audit["disks"][0]["issue"]
            .as_str()
            .unwrap()
            .contains("native recovery")
    );
    let control: Value = serde_json::from_slice(
        &fs::read(f.project.root().join(".fluxvault-run-control.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(control["operation"], "production_start");
    assert_eq!(
        f.run(&["production", "resume", "--no-verify", "--json"], b"", &[])
            .status
            .code(),
        Some(2)
    );
    // Finished receipt does not need USB assertions or another board probe.
    assert_eq!(
        value(&f.run(
            &["production", "resume", "--json"],
            b"",
            &[("MOCK_GW_DEVICE_NOT_FOUND", "1")]
        ))["already_finished"],
        true
    );
}

#[test]
#[ignore = "requires FLUXVAULT_TEST_7Z and FLUXVAULT_TEST_LIBREOFFICE; real saved-file tools, mock capture only"]
fn production_with_real_saved_file_tools_archives_clean_and_partial_document_batches() {
    let sevenzip = std::env::var("FLUXVAULT_TEST_7Z").unwrap();
    let office = std::env::var("FLUXVAULT_TEST_LIBREOFFICE").unwrap();
    for partial in [false, true] {
        let f = Fixture::new(true);
        fs::write(f.app.join("FluxVault/settings.json"),json!({"greaseweazle_path":env!("CARGO_BIN_EXE_mock_gw"),"seven_zip_path":sevenzip,"libreoffice_path":office}).to_string()).unwrap();
        let env = if partial {
            vec![("MOCK_GW_BAD_LBAS", "1200")]
        } else {
            vec![]
        };
        let out = f.run(
            &["production", "start", "--last-disk", "1", "--json"],
            b"1\n",
            &env,
        );
        let r = value(&out);
        assert_eq!(out.status.code(), Some(if partial { 3 } else { 0 }));
        let path = r["finalization"]["package"]["zip"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(fs::read(path).unwrap())),
            r["finalization"]["package"]["sha256"]
        );
        let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        assert!(archive.by_name("Reports/FinalReportLatest.json").is_ok());
        let names: Vec<_> = (0..archive.len())
            .map(|n| archive.by_index(n).unwrap().name().to_owned())
            .collect();
        assert!(
            names
                .iter()
                .any(|s| s.starts_with("Converted/") && s.ends_with(".docx")),
            "actual DOCX missing: {names:?}"
        );
        assert!(
            names
                .iter()
                .any(|s| s.starts_with("Converted/") && s.ends_with(".pdf")),
            "actual PDF missing: {names:?}"
        );
        assert_eq!(
            fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap(),
            {
                let mut bytes = fs::read(&f.image).unwrap();
                if partial {
                    bytes[1200 * 512..1201 * 512].fill(0);
                }
                bytes
            }
        );
    }
}
#[test]
fn partial_acquisition_is_archived_with_attention_not_certified_clean() {
    let f = Fixture::new(false);
    let out = f.run(
        &["production", "start", "--last-disk", "1", "--json"],
        b"1\n",
        &[("MOCK_GW_BAD_LBAS", "1200")],
    );
    let r = value(&out);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(r["workflow"]["phase"], "complete_attention");
    assert_eq!(r["workflow"]["acquisition_attention"], true);
    assert!(r["finalization"]["package"].is_object());
    assert_eq!(r["customer_delivery_certified"], false);
}
#[test]
fn unsupported_format_archives_raw_evidence_with_explicit_zero_file_yield() {
    let f = Fixture::new(false);
    let out = f.run(
        &["production", "start", "--last-disk", "1", "--json"],
        b"1\n",
        &[("MOCK_GW_MEDIA_FORMAT", "nonstandard")],
    );
    let r = value(&out);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(r["workflow"]["phase"], "complete_attention");
    assert_eq!(r["finalization"]["processing"]["sector_images"], 0);
    let report = PathBuf::from(r["finalization"]["processing"]["report"].as_str().unwrap());
    let evidence: Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    assert_eq!(evidence["disks"][0]["disk"], 1);
    assert!(evidence["disks"][0]["format_exception"].is_object());
    let mut zip = zip::ZipArchive::new(
        fs::File::open(r["finalization"]["package"]["zip"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let names: Vec<_> = (0..zip.len())
        .map(|n| zip.by_index(n).unwrap().name().to_owned())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("Flux/") && (n.ends_with(".scp") || n.ends_with(".scp.zip")))
    );
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("Reports/RawOnlyEndpoint-"))
    );
    assert!(!f.project.images_dir().join("001_attempt_001.img").exists());
    let ordinary = f.run(
        &[
            "finalize",
            "--destination",
            f.root.join("project-Delivery").to_str().unwrap(),
            "--allow-attention",
            "--json",
        ],
        b"",
        &[],
    );
    assert_eq!(ordinary.status.code(), Some(2));
}
#[test]
fn invalid_modes_scope_and_destinations_fail_without_custody_or_capture() {
    let f = Fixture::new(false);
    for args in [
        vec!["production", "start", "--json"],
        vec![
            "production",
            "start",
            "--last-disk",
            "1",
            "--acquisition-only",
            "--json",
        ],
        vec![
            "production",
            "start",
            "--last-disk",
            "1",
            "--destination",
            "A:\\out",
            "--json",
        ],
        vec![
            "production",
            "start",
            "--last-disk",
            "1",
            "--destination",
            "Images",
            "--json",
        ],
        vec![
            "production",
            "start",
            "--double",
            "--no-verify",
            "--last-disk",
            "1",
            "--json",
        ],
        vec!["production", "resume", "--json"],
        vec![
            "production",
            "start",
            "--last-disk",
            "1",
            "--drive",
            "A:",
            "--json",
        ],
    ] {
        let r = f.run(&args, b"", &[]);
        assert_eq!(r.status.code(), Some(2));
        assert_eq!(
            serde_json::from_slice::<Value>(&r.stdout).unwrap()["error"]["code"],
            "operation_error"
        );
    }
    assert_eq!(fs::read_dir(f.project.images_dir()).unwrap().count(), 0);
    assert!(!f.project.root().join(".fluxvault-gw-scan.json").exists());
}
#[cfg(windows)]
#[test]
fn production_stop_during_capture_preserves_one_controller_and_resumes_same_label() {
    let f = Fixture::new(false);
    let tree = f.root.join("tree");
    let mut child = f
        .command()
        .args(["production", "start", "--last-disk", "1", "--json"])
        .env("MOCK_GW_READ_TREE", "1")
        .env("MOCK_GW_TREE_ROOT", &tree)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"1\n").unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    while !tree.join("tree.json").exists() {
        if Instant::now() > until {
            let _ = child.kill();
            panic!("capture did not start");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let control: Value = serde_json::from_slice(
        &fs::read(f.project.root().join(".fluxvault-run-control.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(control["operation"], "production_start");
    let stop = f.run(&["stop", "--json"], b"", &[]);
    assert!(stop.status.success());
    let until = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > until {
            let _ = child.kill();
            panic!("production failed to stop");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(130));
    assert_eq!(f.record()["phase"], "interrupted");
    assert_eq!(
        ProjectState::open_without_session(f.project.root().to_path_buf())
            .unwrap()
            .current_disk_number(),
        1
    );
    let resumed = f.run(&["production", "resume", "--json"], b"1\n", &[]);
    assert!(value(&resumed)["finalization"]["package"].is_object());
    assert!(f.project.images_dir().join("001_attempt_001.img").exists());
    assert!(!f.project.images_dir().join("001_attempt_002.img").exists());
}

#[cfg(windows)]
fn interrupted_finishing(cooperative: bool) {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let f = Fixture::new(true);
    let acquired = f.run(
        &["scan", "--last-disk", "1", "--acquisition-only", "--json"],
        b"1\n",
        &[],
    );
    value(&acquired);
    let image = f.project.images_dir().join("001_attempt_001.img");
    let before = fs::read(&image).unwrap();
    let destination = f.root.join("project-Delivery");
    fs::create_dir(&destination).unwrap();
    // Model a crash immediately after the feed endpoint is durably saved.
    fs::write(f.project.root().join(".fluxvault-production-workflow.json"), json!({
        "schema":1,"project":f.project.root().canonicalize().unwrap(),"first":1,"last":1,"dual":false,
        "destination":destination.canonicalize().unwrap(),"workers":1,"scan_args":[],"phase":"finishing",
        "error":null,"package":null,"acquisition_attention":false
    }).to_string()).unwrap();
    let tree = f.root.join("tree");
    let mut child = f
        .command()
        .args(["production", "resume", "--json"])
        .env("MOCK_GW_DEVICE_NOT_FOUND", "1")
        .env("MOCK_GW_OFFICE_TREE", "1")
        .env("MOCK_GW_TREE_ROOT", &tree)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    let processes: Value = loop {
        if let Ok(bytes) = fs::read(tree.join("tree.json"))
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            break value;
        }
        if Instant::now() > until {
            let _ = child.kill();
            let _ = child.wait();
            panic!("offline Office did not start");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let handles: Vec<_> = ["leader", "worker"]
        .iter()
        .map(|key| unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE,
                false,
                processes[*key].as_u64().unwrap() as u32,
            )
            .unwrap()
        })
        .collect();
    let control: Value = serde_json::from_slice(
        &fs::read(f.project.root().join(".fluxvault-run-control.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(control["operation"], "production_start");
    assert_eq!(
        f.run(&["process", "--json"], b"", &[]).status.code(),
        Some(2)
    );
    if cooperative {
        assert!(f.run(&["stop", "--json"], b"", &[]).status.success());
    } else {
        child.kill().unwrap();
    }
    let until = Instant::now() + Duration::from_secs(15);
    let exit = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > until {
            let _ = child.kill();
            let _ = child.wait();
            panic!("production finishing did not stop");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if cooperative {
        assert_eq!(exit.code(), Some(130));
        assert_eq!(f.record()["phase"], "interrupted");
    } else {
        assert_eq!(f.record()["phase"], "finishing");
    }
    for handle in handles {
        unsafe {
            assert_eq!(WaitForSingleObject(handle, 5000), WAIT_OBJECT_0);
            let _ = CloseHandle(handle);
        }
    }
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    let resumed = f.run(
        &["production", "resume", "--json"],
        b"",
        &[("MOCK_GW_DEVICE_NOT_FOUND", "1")],
    );
    let completed = value(&resumed);
    assert!(
        completed["scan"].is_null(),
        "complete saved endpoint must resume offline"
    );
    assert!(completed["finalization"]["package"].is_object());
    assert_eq!(fs::read(&image).unwrap(), before);
    assert!(!f.project.images_dir().join("001_attempt_002.img").exists());
}
#[cfg(windows)]
#[test]
fn production_stop_during_office_resumes_offline_with_saved_options() {
    interrupted_finishing(true);
}
#[cfg(windows)]
#[test]
fn forcibly_closed_production_resumes_offline_and_terminates_descendants() {
    interrupted_finishing(false);
}
