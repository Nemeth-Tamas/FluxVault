//! Actual in-process mock capture/decode -> USB/GW composite -> extraction/audit.
use super::*;
use crate::{
    flux_capture, flux_recovery,
    greaseweazle::{
        BackendMode, GreaseweazleBackend, GreaseweazleCommand, GreaseweazleExecution,
        GreaseweazleProfile,
    },
};

struct Fixture {
    project: ProjectState,
    original: Vec<u8>,
    result: flux_recovery::RecoveryResult,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(self.project.root()).unwrap();
    }
}
struct Mock {
    bytes: Vec<u8>,
    reads: usize,
}
impl GreaseweazleBackend for Mock {
    fn mode(&self) -> BackendMode {
        BackendMode::MockNoHardware
    }
    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String> {
        let stdout = match command.subcommand() {
            "info" => {
                "Host Tools: 1.23\nDevice:\n  Model: Greaseweazle V4\n  Firmware: 1.23\n".into()
            }
            "read" => {
                self.reads += 1;
                fs::write(
                    command.arguments().last().unwrap(),
                    format!("SCP mock {}", self.reads),
                )
                .unwrap();
                String::new()
            }
            "convert" => {
                fs::write(command.arguments().last().unwrap(), &self.bytes).unwrap();
                grid(if self.bytes[34 * 512..35 * 512].iter().all(|b| *b == 0) {
                    34
                } else {
                    usize::MAX
                })
            }
            _ => panic!("Unexpected command; no hardware or writes authorized"),
        };
        Ok(GreaseweazleExecution {
            mode: self.mode(),
            command: command.clone(),
            success: true,
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            host_version: Some("mock".into()),
            started_unix_ms: 0,
            duration_ms: 0,
        })
    }
}
fn grid(bad: usize) -> String {
    let tens: String = (0..80)
        .map(|c| {
            if c % 10 == 0 {
                char::from_digit(c / 10, 10).unwrap()
            } else {
                ' '
            }
        })
        .collect();
    let units: String = (0..80)
        .map(|c| char::from_digit(c % 10, 10).unwrap())
        .collect();
    let mut output = format!("Cyl-> {tens}\nH. S: {units}\n");
    for head in 0..2 {
        for sector in 0..18 {
            let cells: String = (0..80)
                .map(|c| {
                    if (c * 2 + head) * 18 + sector == bad {
                        'X'
                    } else {
                        '.'
                    }
                })
                .collect();
            output.push_str(&format!("{head}.{sector:>2}: {cells}\n"));
        }
    }
    output.push_str(&format!(
        "Found {} sectors of 2880 (99%)\n",
        if bad < 2880 { 2879 } else { 2880 }
    ));
    output
}
fn mixed() -> Fixture {
    let (project, original) = fixture(&[vec![33]]);
    // Initial fixture represents native USB acquisition, not a raw flux donor.
    let usb = project.images_dir().join("001_attempt_001.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&usb).unwrap()).unwrap();
    value["source_backend"] = json!("windows-raw-sector");
    fs::write(usb, serde_json::to_vec(&value).unwrap()).unwrap();
    let mut bytes = original.clone();
    bytes[34 * 512..35 * 512].fill(0);
    let mut backend = Mock { bytes, reads: 0 };
    let mut policy = flux_recovery::RecoveryPolicy::default();
    policy.passes.truncate(1);
    let result = flux_recovery::recover(
        &project,
        1,
        GreaseweazleProfile::Ibm1440,
        'B',
        policy,
        &mut backend,
        &|_| {},
    )
    .unwrap();
    assert_eq!(backend.reads, 1);
    assert_eq!(result.missing_lbas, vec![34]);
    assert_eq!(result.status, "partial");
    Fixture {
        project,
        original,
        result,
    }
}

#[test]
fn released_wrong_identity_recaptures_in_a_new_slot_and_never_reuses_wrong_bytes() {
    let (project, mut original) = fixture(&[vec![33]]);
    // The extraction fixture omits physical BPB geometry; auto-format tests
    // need a coherent 80-cylinder, 2-head, 18-sector synthetic boot record.
    original[24..26].copy_from_slice(&18u16.to_le_bytes());
    original[26..28].copy_from_slice(&2u16.to_le_bytes());
    let mut wrong = original.clone();
    wrong[35 * 512..36 * 512].fill(0xEE);
    wrong[34 * 512..35 * 512].fill(0);
    let mut backend = Mock {
        bytes: wrong,
        reads: 0,
    };
    let mut policy = flux_recovery::RecoveryPolicy::default();
    policy.passes.truncate(1);
    let error = flux_recovery::recover_auto_checked(
        &project,
        1,
        'B',
        policy.clone(),
        &mut backend,
        &|_| {},
        &|_, _| Err("USB/GW readable bytes disagree; wrong label".into()),
    )
    .unwrap_err();
    assert!(
        error.starts_with("USB/GW readable bytes disagree;"),
        "{error}"
    );
    assert_eq!(backend.reads, 1);
    let old_raw = fs::read(project.root().join("Flux/001_attempt_001.scp")).unwrap();
    flux_recovery::reject_identity_job(&project, 1, 99).unwrap();
    backend.bytes = original.clone();
    backend.bytes[34 * 512..35 * 512].fill(0);
    let result = flux_recovery::recover_auto_checked(
        &project,
        1,
        'B',
        policy,
        &mut backend,
        &|_| {},
        &|bytes, bad| {
            assert_eq!(bad, &[34]);
            assert_eq!(&bytes[35 * 512..36 * 512], &original[35 * 512..36 * 512]);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(backend.reads, 2);
    assert_eq!(result.physical_reads_this_run, 1);
    assert_eq!(result.capture_attempts, vec![2]);
    assert_eq!(
        fs::read(project.root().join("Flux/001_attempt_001.scp")).unwrap(),
        old_raw
    );
    let status = flux_capture::inspect_disk(&project, 1).unwrap();
    assert_eq!(status.captures.len(), 1);
    assert_eq!(status.captures[0].attempt, 2);
    assert!(status.decodes.iter().all(|d| d.capture_attempt == 2));
    flux_recovery::verify_completed_result(&project, &result).unwrap();
    let meta: Value =
        serde_json::from_slice(&fs::read(result.image.with_extension("json")).unwrap()).unwrap();
    flux_recovery::verify_catalog_metadata(&project.images_dir(), &meta).unwrap();
    // Completed catalog images cannot later be "released" as mistakes.
    assert!(flux_recovery::reject_identity_job(&project, 1, 100).is_err());
    fs::remove_dir_all(project.root()).unwrap();
}
#[test]
fn confirmed_minor_reader_conflicts_resume_offline_stay_attention_and_preserve_versions() {
    use crate::production::{Coordinator, Station};
    let (p, mut original) = fixture(&[]);
    original[24..26].copy_from_slice(&18u16.to_le_bytes());
    original[26..28].copy_from_slice(&2u16.to_le_bytes());
    let mut c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    let usb_ticket = c.claim(Station::Usb, 1).unwrap();
    c.confirm(&usb_ticket, "1", true).unwrap();
    let mut usb = original.clone();
    usb[33 * 512..34 * 512].fill(0);
    usb[17 * 512] = 0xAA;
    usb[773 * 512] = 0xBB;
    let sha = hash(&usb);
    let log = p.logs_dir().join("001_attempt_001.log");
    fs::write(&log, format!("BEGIN | disk=1 | attempt=1 | source=windows-raw-sector\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\nBAD_SECTOR | LBA=33\nEND | status=PARTIAL | bytes=1474560 | sha256={sha}\n")).unwrap();
    let meta = json!({"fluxvault_version":"test", "status":"PARTIAL", "disk_number":1, "attempt_number":1,
        "source_backend":"windows-raw-sector", "source_device":"synthetic only", "image_file":"001_attempt_001.img", "log_file":log,
        "timestamp_unix_ms":1, "geometry":{"cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":1474560,"format_guess":"ibm.1440"},
        "sector_retries":0,"total_sectors":2880,"bytes_written":1474560,"retry_recovered_sectors":0,"bad_sector_count":1,
        "bad_sectors":[{"lba":33,"cylinder":0,"head":1,"sector":16}],"sha256":sha});
    fs::write(p.images_dir().join("001_attempt_001.img"), &usb).unwrap();
    fs::write(
        p.images_dir().join("001_attempt_001.json"),
        serde_json::to_vec(&meta).unwrap(),
    )
    .unwrap();
    c.complete(&usb_ticket, 1).unwrap();
    c.removed(&usb_ticket, true).unwrap();
    let ticket = c.claim(Station::Greaseweazle, 1).unwrap();
    c.confirm(&ticket, "1", true).unwrap();
    let guard = c.transfer_guard(&ticket).unwrap().unwrap();
    let mut backend = Mock {
        bytes: original.clone(),
        reads: 0,
    };
    let error = flux_recovery::recover_auto_checked(
        &p,
        1,
        'B',
        Default::default(),
        &mut backend,
        &|_| {},
        &|bytes, bad| guard.check(bytes, bad),
    )
    .unwrap_err();
    assert!(
        error.starts_with("USB/GW readable bytes disagree;"),
        "{error}"
    );
    c.failed(&ticket, &error).unwrap();
    assert!(c.confirm_same_disk(Station::Usb, 1).is_err());
    assert!(c.confirm_same_disk(Station::Greaseweazle, 2).is_err());
    let approval = c.confirm_same_disk(Station::Greaseweazle, 1).unwrap();
    assert_eq!(approval["conflicts"].as_array().unwrap().len(), 2);
    let raw_before = fs::read(p.root().join("Flux/001_attempt_001.scp")).unwrap();
    drop(c);
    let mut c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    let ticket = c.claim(Station::Greaseweazle, 1).unwrap();
    c.confirm(&ticket, "1", true).unwrap();
    let guard = c.transfer_guard(&ticket).unwrap().unwrap();
    let result = flux_recovery::recover_auto_checked(
        &p,
        1,
        'B',
        Default::default(),
        &mut backend,
        &|_| {},
        &|bytes, bad| guard.check(bytes, bad),
    )
    .unwrap();
    assert_eq!(backend.reads, 1);
    assert_eq!(result.physical_reads_this_run, 0);
    assert!(result.missing_lbas.is_empty());
    c.complete(&ticket, 2).unwrap();
    assert_eq!(
        c.status()["disks"]["1"]["gw"]["confirmed_conflicts"],
        json!([17, 773])
    );
    c.removed(&ticket, true).unwrap();
    assert_eq!(c.status()["disks"]["1"]["phase"], "partial");
    assert_eq!(c.status()["usb_recovery_queue"], json!([]));
    assert!(c.confirm_same_disk(Station::Greaseweazle, 1).is_err());
    drop(c);
    drop(Coordinator::open(p.clone(), Some(1), false).unwrap());
    let attempts = imaging::load_attempts_for_disk(&p.images_dir(), 1).unwrap();
    let best = imaging::best_attempt(&attempts).unwrap();
    assert_eq!(best.attempt_number, 2);
    assert!(best.attention_required && best.bad_sectors.is_empty());
    assert_eq!(fs::read(&result.image).unwrap(), original);
    let managed = managed(&p, best);
    assert_eq!(managed.files, 2);
    let audit = crate::audit::run_audit(&p, &|_| {}).unwrap();
    assert_eq!(audit.verified_disks, 0);
    assert_eq!(audit.attention_disks, 1);
    assert_eq!(
        audit.document.disks[0].evidence_status,
        "CONFIRMED_CROSS_READER_CONFLICTS"
    );
    assert!(audit.document.disks[0].issue.contains("read_conflicts"));
    assert_eq!(result.read_conflict_lbas, vec![17, 773]);
    assert_eq!(result.status, "partial");
    assert_eq!(
        fs::read(p.images_dir().join("001_attempt_001.img")).unwrap(),
        usb
    );
    assert_eq!(
        fs::read(p.root().join("Flux/001_attempt_001.scp")).unwrap(),
        raw_before
    );
    let metadata_path = result.image.with_extension("json");
    let published = fs::read(&metadata_path).unwrap();
    let mut tampered: Value = serde_json::from_slice(&published).unwrap();
    tampered.as_object_mut().unwrap().remove("read_conflicts");
    fs::write(&metadata_path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert!(imaging::load_attempts_for_disk(&p.images_dir(), 1).is_err());
    fs::write(&metadata_path, published).unwrap();
    flux_recovery::verify_completed_result(&p, &result).unwrap();
    let approved: crate::read_conflicts::Confirmation = serde_json::from_value(approval).unwrap();
    let mut changed = original.clone();
    changed[773 * 512] ^= 1;
    assert!(crate::read_conflicts::verify(&p, &approved, &changed, &[]).is_err());
    assert!(crate::read_conflicts::verify(&p, &approved, &original, &[17]).is_err());
    let usb_metadata_path = p.images_dir().join("001_attempt_001.json");
    let usb_metadata = fs::read(&usb_metadata_path).unwrap();
    fs::write(
        &usb_metadata_path,
        [usb_metadata.clone(), b"\n".to_vec()].concat(),
    )
    .unwrap();
    assert!(crate::read_conflicts::verify(&p, &approved, &original, &[]).is_err());
    fs::write(&usb_metadata_path, usb_metadata).unwrap();
    // Broad mismatch cannot get the same-disk escape, even with an assertion.
    assert!(
        crate::read_conflicts::build(&p, 1, 1, 99, 1, &vec![0x42; original.len()], &[]).is_err()
    );
    fs::remove_dir_all(p.root()).unwrap();
}

fn metadata(f: &Fixture) -> (PathBuf, Value) {
    let path = f.result.image.with_extension("json");
    let value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    (path, value)
}
fn verify(f: &Fixture) -> Result<(), String> {
    flux_recovery::verify_catalog_metadata(&f.project.images_dir(), &metadata(f).1)
}

#[test]
fn complementary_usb_and_gw_catalog_sources_replay_through_processing_and_reuse() {
    let f = mixed();
    verify(&f).unwrap();
    let originals = [
        fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap(),
        fs::read(&f.result.image).unwrap(),
    ];
    let first = pipeline::run_pipeline(&request(&f.project), &|_| {}).unwrap();
    assert_eq!(first.published_recovery_images, 1);
    assert_eq!(files(&f.project), vec!["FIRST.TXT", "SECOND.TXT"]);
    let attempts = imaging::load_attempts_for_disk(&f.project.images_dir(), 1).unwrap();
    let best = imaging::best_attempt(&attempts).unwrap();
    assert_eq!(best.status, "DERIVED");
    assert_eq!(
        fs::read(f.project.images_dir().join(&best.image_file)).unwrap(),
        f.original
    );
    assert!(best.attention_required);
    let audit: Value = serde_json::from_slice(&fs::read(first.audit.json_path).unwrap()).unwrap();
    assert_eq!(audit["customer_delivery_certified"], false);
    let again = pipeline::run_pipeline(&request(&f.project), &|_| {}).unwrap();
    assert_eq!(again.reused_recovery_images, 1);
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        fs::read(f.project.images_dir().join("001_attempt_001.img")).unwrap(),
        originals[0]
    );
    assert_eq!(fs::read(&f.result.image).unwrap(), originals[1]);
    let raw = f.project.root().join("Flux/001_attempt_001.scp");
    let raw_bytes = fs::read(&raw).unwrap();
    fs::write(&raw, b"changed after DERIVED publication").unwrap();
    assert!(imaging::load_attempts_for_disk(&f.project.images_dir(), 1).is_err());
    fs::write(raw, raw_bytes).unwrap();
    assert_eq!(
        imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn packed_sources_and_historical_publications_do_not_depend_on_latest_job() {
    let f = mixed();
    crate::flux_archive::pack(&f.project, 1, 1, true).unwrap();
    // A later job may replace this mutable controller; the original proof owns its stages.
    fs::write(f.project.root().join("Flux/Recovery/001_job.json"), b"{}").unwrap();
    verify(&f).unwrap();
    let (attempts, composite) = composite(&f.project);
    publish(&f.project, 1, &attempts, Some(&composite), None).unwrap();
    assert!(!f.project.root().join("Flux/001_attempt_001.scp").exists());
    let best = imaging::load_attempts_for_disk(&f.project.images_dir(), 1).unwrap();
    assert_eq!(
        fs::read(
            f.project
                .images_dir()
                .join(&imaging::best_attempt(&best).unwrap().image_file)
        )
        .unwrap(),
        f.original
    );
}

#[test]
fn changed_raw_or_decode_donor_is_refused_before_native_or_derived_publication() {
    for raw in [true, false] {
        let f = mixed();
        let status = flux_capture::inspect_disk(&f.project, 1).unwrap();
        let target = if raw {
            status.captures[0].raw_flux.as_ref().unwrap()
        } else {
            &status.decodes[0].image
        };
        let original = fs::read(target).unwrap();
        let mut changed = original.clone();
        changed[0] ^= 1;
        fs::write(target, changed).unwrap();
        assert!(verify(&f).is_err());
        let attempts = imaging::load_attempts_for_disk(&f.project.images_dir(), 1).unwrap();
        let gw = attempts.iter().find(|a| a.attempt_number == 2).unwrap();
        assert!(
            fat12_recovery::recover_attempt(
                &f.project.images_dir(),
                &f.project.extracted_dir(),
                &f.project.recovery_dir(),
                1,
                gw,
                &|_| {}
            )
            .is_err()
        );
        let sources = attempts
            .iter()
            .map(|a| CompositeSource {
                attempt_number: a.attempt_number,
                image_path: f.project.images_dir().join(&a.image_file),
                expected_sha256: Some(a.sha256.clone()),
                total_sectors: a.total_sectors,
                bad_sectors: a.bad_sectors.clone(),
            })
            .collect();
        assert!(
            composite::run_composite(
                &CompositeRequest {
                    images_directory: f.project.images_dir(),
                    recovery_root: f.project.recovery_dir(),
                    disk_number: 1,
                    sources,
                },
                &|_| {}
            )
            .is_err()
        );
        assert!(!f.project.recovery_dir().join("001").exists());
        assert_eq!(
            imaging::load_attempts_for_disk(&f.project.images_dir(), 1)
                .unwrap()
                .len(),
            2
        );
        fs::write(target, original).unwrap();
        verify(&f).unwrap();
    }
}

#[test]
fn rewritten_provenance_map_stage_and_geometry_cannot_pass_hash_only_validation() {
    for fault in 0..10 {
        let f = mixed();
        let (path, mut meta) = metadata(&f);
        let mut proof: Value =
            serde_json::from_slice(&fs::read(&f.result.provenance).unwrap()).unwrap();
        match fault {
            0 => proof["sectors"][33]["confidence"] = json!("corroborated"),
            1 => proof["stages"][0]["capture_attempt"] = json!(99),
            2 => {
                let first = proof["stages"][0].clone();
                proof["stages"].as_array_mut().unwrap().push(first);
            }
            3 => proof["stages"][0]["settings"]["retries"] = json!(1),
            4 => proof["disk"] = json!(2),
            5 => proof["profile"] = json!("ibm.720"),
            6 => meta["bad_sectors"][0]["lba"] = json!(33),
            7 => meta["source_backend"] = json!("windows-raw-sector"),
            8 => meta["geometry"]["heads"] = json!(1),
            9 => meta["flux_provenance"] = json!("A:\\untrusted.json"),
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&proof).unwrap();
        fs::write(&f.result.provenance, &bytes).unwrap();
        meta["flux_provenance_sha256"] = json!(hash(&bytes));
        fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
        assert!(verify(&f).is_err(), "fault {fault} escaped replay");
    }
}

#[test]
fn missing_oversized_or_inconsistent_flux_proofs_are_not_preferred_or_inspected() {
    let f = mixed();
    let (path, meta) = metadata(&f);
    let original = fs::read(&f.result.provenance).unwrap();
    fs::write(&f.result.provenance, vec![0; 4 * 1024 * 1024 + 1]).unwrap();
    assert!(verify(&f).unwrap_err().contains("bound"));
    fs::remove_file(&f.result.provenance).unwrap();
    assert!(crate::preferred_image::set(&f.project, 1, Some(2)).is_err());
    fs::write(&f.result.provenance, original).unwrap();
    verify(&f).unwrap();
    let mut wrong = meta.clone();
    wrong["sha256"] = json!("00".repeat(32));
    fs::write(&path, serde_json::to_vec(&wrong).unwrap()).unwrap();
    assert!(verify(&f).is_err());
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    let attempts = imaging::load_attempts_for_disk(&f.project.images_dir(), 1).unwrap();
    let mut false_map = attempts
        .iter()
        .find(|a| a.attempt_number == 2)
        .unwrap()
        .clone();
    false_map.bad_sectors.clear();
    assert!(verify_attempt(&f.project.images_dir(), &false_map).is_err());
    assert!(
        verify_composite_source(
            &f.project.images_dir(),
            1,
            &CompositeSource {
                attempt_number: 2,
                image_path: f.result.image.clone(),
                expected_sha256: Some(f.result.image_sha256.clone()),
                total_sectors: 2880,
                bad_sectors: vec![],
            }
        )
        .is_err()
    );
    // The inspector now independently replays flux origins, not just the proof hash.
    let result = crate::sector_inspection::inspect(&f.project, 1, 33, 1, Some(2)).unwrap();
    assert_eq!(result["flux_lineage_replayed"], true);
    assert_eq!(result["independent_flux_crc_verified"], false);
    assert_eq!(
        result["sectors"][0]["recorded_flux_origin"]["confidence"],
        "single_capture_gw_reported_good"
    );
    let raw = f.project.root().join("Flux/001_attempt_001.scp");
    fs::write(raw, b"changed raw").unwrap();
    assert!(crate::sector_inspection::inspect(&f.project, 1, 33, 1, Some(2)).is_err());
}

#[test]
#[ignore = "requires FV_LINEAGE_SOURCE; verifies saved 059/066 originals read-only, no tools or media"]
fn saved_damaged_catalog_lineage_replays_without_changing_evidence() {
    let project = ProjectState::open_without_session(PathBuf::from(
        std::env::var_os("FV_LINEAGE_SOURCE").expect("Set FV_LINEAGE_SOURCE"),
    ))
    .unwrap();
    for disk in [59, 66] {
        let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk).unwrap();
        let mut checked = 0;
        for attempt in attempts {
            if attempt.metadata_path.as_os_str().is_empty() {
                continue;
            }
            let meta_bytes = fs::read(&attempt.metadata_path).unwrap();
            let meta: Value = serde_json::from_slice(&meta_bytes).unwrap();
            if meta["source_backend"] != "greaseweazle-derived" {
                continue;
            }
            let provenance = PathBuf::from(meta["flux_provenance"].as_str().unwrap());
            let proof_bytes = fs::read(&provenance).unwrap();
            let image = project.images_dir().join(&attempt.image_file);
            let image_bytes = fs::read(&image).unwrap();
            flux_recovery::verify_catalog_metadata(&project.images_dir(), &meta).unwrap();
            assert_eq!(fs::read(&attempt.metadata_path).unwrap(), meta_bytes);
            assert_eq!(fs::read(provenance).unwrap(), proof_bytes);
            assert_eq!(fs::read(image).unwrap(), image_bytes);
            checked += 1;
            eprintln!(
                "Saved disk {disk:03}, image attempt {:03}: flux lineage replay passed; {} unresolved; original image/metadata/proof unchanged",
                attempt.attempt_number,
                attempt.bad_sectors.len()
            );
        }
        assert!(checked > 0, "No GW catalog source for disk {disk}");
    }
}
