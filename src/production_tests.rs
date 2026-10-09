use super::*;
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

fn project() -> ProjectState {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    ProjectState::create_without_session(std::env::temp_dir().join(format!(
        "fv-production-{}-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed))))
    .unwrap()
}

fn evidence(
    project: &ProjectState,
    disk: u32,
    attempt: u32,
    station: Station,
    bad: &[u64],
    fill: u8,
) {
    // Disposable tiny synthetic acquisition; not a physical media benchmark.
    let mut bytes = vec![fill; 4 * 512];
    for l in bad {
        bytes[*l as usize * 512..(*l as usize + 1) * 512].fill(0);
    }
    let sha = digest(&bytes);
    let stem = format!("{disk:03}_attempt_{attempt:03}");
    let backend = match station {
        Station::Usb => "windows-raw-sector",
        Station::Greaseweazle => "greaseweazle-derived",
    };
    let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
    fs::write(project.images_dir().join(format!("{stem}.img")), bytes).unwrap();
    let log = project.logs_dir().join(format!("{stem}.log"));
    let mut text = format!(
        "BEGIN | disk={disk} | attempt={attempt} | source={backend}\nGEOMETRY | cylinders=2 | heads=1 | sectors_per_track=2 | bytes_per_sector=512 | total_sectors=4 | total_bytes=2048\n"
    );
    for l in bad {
        text.push_str(&format!("BAD_SECTOR | LBA={l}\n"));
    }
    text.push_str(&format!(
        "END | status={status} | bytes=2048 | sha256={sha}\n"
    ));
    fs::write(&log, text).unwrap();
    let value = json!({"fluxvault_version":"synthetic", "status":status,"disk_number":disk,"attempt_number":attempt,
        "source_backend":backend,"source_device":"synthetic only","image_file":format!("{stem}.img"),"log_file":log,
        "timestamp_unix_ms":attempt,"geometry":{"cylinders":2,"heads":1,"sectors_per_track":2,"bytes_per_sector":512,"total_bytes":2048,"format_guess":"synthetic"},
        "sector_retries":0,"total_sectors":4,"bytes_written":2048,"retry_recovered_sectors":0,"bad_sector_count":bad.len(),
        "bad_sectors":bad.iter().map(|l| json!({"lba":l,"cylinder":l/2,"head":0,"sector":l%2+1})).collect::<Vec<_>>(),"sha256":sha});
    fs::write(
        project.images_dir().join(format!("{stem}.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

fn start(c: &mut Coordinator, station: Station, number: u32) -> Ticket {
    let t = c.claim(station, number).unwrap();
    c.confirm(&t, &number.to_string(), true).unwrap();
    t
}

#[test]
fn usb_triage_is_sealed_durable_and_clean_results_never_enter_recovery_priority() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let usb = start(&mut c, Station::Usb, 1);
    evidence(&p, 1, 1, Station::Usb, &[], 17);
    c.complete(&usb, 1).unwrap();
    let clean = c.status()["disks"]["1"]["usb_triage"].clone();
    assert_eq!(clean["route"], "complete");
    assert_eq!(clean["saved_generation"], usb.generation);
    assert_eq!(
        clean["image_sha256"],
        c.status()["disks"]["1"]["usb"]["sha256"]
    );
    assert_eq!(c.status()["recommended_gw_disk"], Value::Null);
    c.removed(&usb, true).unwrap();
    let usb = start(&mut c, Station::Usb, 2);
    evidence(&p, 2, 1, Station::Usb, &[1], 17);
    c.complete(&usb, 1).unwrap();
    assert_eq!(
        c.status()["disks"]["2"]["usb_triage"]["route"],
        "move_to_greaseweazle"
    );
    assert_eq!(
        c.status()["recovery_priorities"][0]["availability"],
        "held_in_usb"
    );
    let before = fs::read(p.root().join(JOURNAL)).unwrap();
    assert_eq!(status(&p).unwrap()["recommended_gw_disk"], 2);
    assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), before);
    assert!(c.claim(Station::Greaseweazle, 2).is_err());
    c.removed(&usb, true).unwrap();
    let ranked = c.status()["recovery_priorities"].clone();
    drop(c);
    let c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    assert_eq!(c.status()["recovery_priorities"], ranked);
    assert_eq!(c.status()["disks"]["1"]["usb_triage"], clean);
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn queue_ranking_respects_custody_and_explicit_lower_rank_override() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(5), false).unwrap();
    for (number, bad) in [(1, &[1, 2, 3][..]), (2, &[2][..])] {
        let t = start(&mut c, Station::Usb, number);
        evidence(&p, number, 1, Station::Usb, bad, 17);
        c.complete(&t, 1).unwrap();
        c.removed(&t, true).unwrap();
    }
    let held = start(&mut c, Station::Usb, 3);
    evidence(&p, 3, 1, Station::Usb, &[0], 17);
    c.complete(&held, 1).unwrap();
    let state = c.status();
    assert_eq!(state["usb_recovery_queue"], json!([1, 2])); // Existing API unchanged.
    assert_eq!(state["recommended_gw_disk"], 2);
    assert_eq!(
        state["recovery_priorities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["disk"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![2, 1, 3]
    );
    let gw = start(&mut c, Station::Greaseweazle, 1); // Operator has this disk available.
    assert_eq!(c.status()["recommended_gw_disk"], 2);
    assert_eq!(
        c.status()["recovery_priorities"].as_array().unwrap().len(),
        2
    );
    c.failed(&gw, "synthetic interruption").unwrap();
    assert!(
        !c.status()["recovery_priorities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["disk"] == 1)
    );
    c.removed(&held, true).unwrap();
    assert_eq!(c.status()["recommended_gw_disk"], 3); // Ready boot loss outranks other one-sector loss.
    c.set_paused(true).unwrap();
    assert!(c.claim(Station::Greaseweazle, 3).is_err());
    drop(c);
    let c = Coordinator::open(p.clone(), Some(5), false).unwrap();
    assert_eq!(c.status()["recommended_gw_disk"], 3);
    assert_eq!(c.status()["disks"]["1"]["phase"], "interrupted");
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn legacy_queue_age_is_unknown_and_malformed_triage_is_refused() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let usb = start(&mut c, Station::Usb, 1);
    evidence(&p, 1, 1, Station::Usb, &[1], 17);
    c.complete(&usb, 1).unwrap();
    c.removed(&usb, true).unwrap();
    drop(c);
    let path = p.root().join(JOURNAL);
    let current = fs::read(&path).unwrap();
    let parsed: Value = serde_json::from_slice(&current).unwrap();
    for (key, value) in [
        ("route", json!("complete")),
        ("policy", json!("unknown")),
        ("missing_sectors", json!(0)),
        ("image_sha256", json!("0".repeat(64))),
        ("total_sectors", json!(3)),
        ("attempt", json!(2)),
        ("saved_generation", json!(0)),
        ("saved_generation", json!(999)),
    ] {
        let mut invalid = parsed.clone();
        invalid["disks"]["1"]["usb_triage"][key] = value;
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let edited = fs::read(&path).unwrap();
        assert!(status(&p).is_err(), "accepted {key}");
        assert!(Coordinator::open(p.clone(), Some(2), false).is_err());
        assert_eq!(fs::read(&path).unwrap(), edited);
    }
    let mut legacy = parsed;
    legacy["disks"]["1"]
        .as_object_mut()
        .unwrap()
        .remove("usb_triage");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let state = status(&p).unwrap();
    assert_eq!(state["recommended_gw_disk"], 1);
    assert_eq!(
        state["recovery_priorities"][0]["waiting_generations"],
        Value::Null
    );
    let c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    assert_eq!(
        c.status()["recovery_priorities"],
        state["recovery_priorities"]
    );
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn tampered_source_receipt_prevents_offline_priority_reporting() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    let usb = start(&mut c, Station::Usb, 1);
    evidence(&p, 1, 1, Station::Usb, &[1], 17);
    c.complete(&usb, 1).unwrap();
    c.removed(&usb, true).unwrap();
    drop(c);
    let path = p.images_dir().join("001_attempt_001.img");
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(status(&p).is_err());
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn durable_pause_blocks_new_authorization_but_allows_active_receipts_and_removal() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let usb = c.claim(Station::Usb, 1).unwrap();
    c.set_paused(true).unwrap();
    let paused_bytes = fs::read(p.root().join(JOURNAL)).unwrap();
    assert!(c.confirm(&usb, "1", true).unwrap_err().contains("PAUSED"));
    assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), paused_bytes);
    c.set_paused(false).unwrap();
    c.confirm(&usb, "1", true).unwrap();
    let offer = c.claim(Station::Greaseweazle, 2).unwrap();
    c.set_paused(true).unwrap();
    evidence(&p, 1, 1, Station::Usb, &[1], 17);
    c.complete(&usb, 1).unwrap();
    c.removed(&usb, true).unwrap();
    let saved_bytes = fs::read(p.root().join(JOURNAL)).unwrap();
    assert!(c.claim(Station::Usb, 3).unwrap_err().contains("PAUSED"));
    assert!(
        c.select_queued_instead(&offer, 1)
            .unwrap_err()
            .contains("PAUSED")
    );
    assert!(c.confirm(&offer, "2", true).unwrap_err().contains("PAUSED"));
    assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), saved_bytes);
    drop(c);
    let mut c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    assert!(c.paused());
    assert_eq!(c.status()["usb_recovery_queue"], json!([1]));
    assert_eq!(c.status()["disks"]["2"]["phase"], "interrupted");
    assert!(c.claim(Station::Usb, 3).is_err());
    c.set_paused(false).unwrap();
    assert_eq!(c.status()["disks"]["2"]["phase"], "interrupted");
    assert_eq!(
        imaging::load_attempts_for_disk(&p.images_dir(), 2)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(c.claim(Station::Usb, 3).unwrap().disk, 3);
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn legacy_production_journal_without_pause_field_reopens_as_feeding_enabled() {
    let p = project();
    let c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    drop(c);
    let path = p.root().join(JOURNAL);
    let mut legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("paused");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    assert!(!c.paused());
    assert_eq!(c.next_fresh_disk(), Some(1));
    assert!(c.status()["disks"].as_object().unwrap().is_empty());
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn remaining_fresh_count_deduplicates_occupied_receipts_after_restart() {
    let p = project();
    evidence(&p, 2, 1, Station::Greaseweazle, &[], 14);
    let mut c = Coordinator::open(p.clone(), Some(5), false).unwrap();
    assert_eq!(c.status()["remaining_unclaimed_fresh_labels"], 4);
    let usb = start(&mut c, Station::Usb, 1);
    assert_eq!(c.status()["remaining_unclaimed_fresh_labels"], 3);
    assert_eq!(c.status()["pending_initial_reads"], 1);
    evidence(&p, 1, 1, Station::Usb, &[], 14);
    c.complete(&usb, 1).unwrap();
    drop(c);
    let c = Coordinator::open(p.clone(), Some(5), false).unwrap();
    assert_eq!(c.status()["remaining_unclaimed_fresh_labels"], 3);
    assert_eq!(c.status()["pending_initial_reads"], 0);
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn both_stations_take_fresh_disks_and_usb_partial_returns_by_its_old_label() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let usb = start(&mut c, Station::Usb, 1);
    let gw = start(&mut c, Station::Greaseweazle, 2);
    assert_eq!(c.next_fresh_disk(), Some(3));
    assert!(c.claim(Station::Greaseweazle, 1).is_err());
    evidence(&p, 1, 1, Station::Usb, &[1], 17);
    evidence(&p, 2, 1, Station::Greaseweazle, &[2], 18);
    c.complete(&usb, 1).unwrap();
    c.complete(&gw, 1).unwrap();
    assert!(
        c.status()["usb_recovery_queue"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(c.removed(&usb, false).is_err());
    c.removed(&usb, true).unwrap();
    c.removed(&gw, true).unwrap();
    assert_eq!(c.status()["usb_recovery_queue"], json!([1]));
    assert_eq!(c.status()["disks"]["2"]["phase"], "partial");
    let next = start(&mut c, Station::Usb, 3);
    let recover = start(&mut c, Station::Greaseweazle, 1);
    assert_eq!(recover.work, Work::UsbRecovery);
    assert_eq!(next.work, Work::Fresh);
    evidence(&p, 1, 2, Station::Greaseweazle, &[], 17);
    evidence(&p, 3, 1, Station::Usb, &[], 19);
    c.complete(&recover, 2).unwrap();
    c.complete(&next, 1).unwrap();
    c.removed(&recover, true).unwrap();
    c.removed(&next, true).unwrap();
    assert_eq!(c.next_fresh_disk(), None);
    assert_eq!(c.status()["disks"]["1"]["phase"], "complete");
    assert_eq!(
        fs::read(p.images_dir().join("001_attempt_001.img")).unwrap()[512],
        0
    );
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn blank_wrong_label_unprotected_duplicate_station_and_stale_ticket_never_authorize_reads() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), None, false).unwrap();
    assert!(c.claim(Station::Usb, 2).is_err());
    let t = c.claim(Station::Usb, 1).unwrap();
    for answer in ["", "2", "READ 1", "QUIT"] {
        assert!(c.confirm(&t, answer, true).is_err());
    }
    assert!(c.confirm(&t, "1", false).is_err());
    assert!(c.claim(Station::Usb, 2).is_err());
    assert!(c.claim(Station::Greaseweazle, 1).is_err());
    assert_eq!(c.status()["disks"]["1"]["phase"], "reserved");
    c.confirm(&t, "001", true).unwrap();
    c.failed(&t, "synthetic read failure").unwrap();
    let retry = c.claim(Station::Usb, 1).unwrap();
    assert_ne!(retry.generation, t.generation);
    assert!(c.confirm(&t, "1", true).is_err());
    assert!(c.complete(&t, 1).is_err());
    c.confirm(&retry, "1", true).unwrap();
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn restart_preserves_custody_saved_outputs_and_queued_transfers_without_auto_read() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(4), false).unwrap();
    let usb = start(&mut c, Station::Usb, 1);
    let gw = start(&mut c, Station::Greaseweazle, 2);
    evidence(&p, 1, 1, Station::Usb, &[1], 20);
    c.complete(&usb, 1).unwrap();
    drop(c);
    let mut c = Coordinator::open(p.clone(), Some(4), false).unwrap();
    assert_eq!(c.status()["disks"]["1"]["phase"], "saved");
    assert_eq!(c.status()["disks"]["2"]["phase"], "interrupted");
    assert!(c.claim(Station::Greaseweazle, 1).is_err());
    assert!(c.confirm(&gw, "2", true).is_err());
    c.removed(&usb, true).unwrap();
    let retry = start(&mut c, Station::Greaseweazle, 2);
    assert_ne!(gw.generation, retry.generation);
    evidence(&p, 2, 1, Station::Greaseweazle, &[], 21);
    c.complete(&retry, 1).unwrap();
    c.removed(&retry, true).unwrap();
    drop(c);
    let mut c = Coordinator::open(p.clone(), Some(4), false).unwrap();
    assert_eq!(c.status()["usb_recovery_queue"], json!([1]));
    assert_eq!(
        c.claim(Station::Greaseweazle, 1).unwrap().work,
        Work::UsbRecovery
    );
    assert_eq!(c.next_fresh_disk(), Some(3));
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn cross_station_identity_conflict_and_unknown_shared_bytes_preserve_usb_queue_and_images() {
    for bad in [vec![1], vec![0, 1, 2, 3]] {
        let p = project();
        let mut c = Coordinator::open(p.clone(), None, false).unwrap();
        let usb = start(&mut c, Station::Usb, 1);
        evidence(&p, 1, 1, Station::Usb, &bad, 23);
        c.complete(&usb, 1).unwrap();
        c.removed(&usb, true).unwrap();
        let gw = start(&mut c, Station::Greaseweazle, 1);
        let original = fs::read(p.images_dir().join("001_attempt_001.img")).unwrap();
        evidence(&p, 1, 2, Station::Greaseweazle, &[], 24);
        let before = c.status();
        let journal = fs::read(p.root().join(JOURNAL)).unwrap();
        assert!(c.complete(&gw, 2).is_err());
        assert_eq!(c.status(), before);
        assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), journal);
        assert_eq!(
            fs::read(p.images_dir().join("001_attempt_001.img")).unwrap(),
            original
        );
        assert!(p.images_dir().join("001_attempt_002.img").is_file());
        drop(c);
        fs::remove_dir_all(p.root()).unwrap();
    }
}

#[test]
fn completed_backend_identity_and_hash_map_bindings_are_required_before_routing() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), None, false).unwrap();
    let t = start(&mut c, Station::Usb, 1);
    assert!(c.complete(&t, 1).is_err());
    evidence(&p, 1, 1, Station::Greaseweazle, &[], 12);
    assert!(c.complete(&t, 1).is_err());
    evidence(&p, 1, 2, Station::Usb, &[1], 12);
    let image = p.images_dir().join("001_attempt_002.img");
    let bytes = fs::read(&image).unwrap();
    fs::write(&image, b"changed source").unwrap();
    assert!(c.complete(&t, 2).is_err());
    fs::write(&image, bytes).unwrap();
    c.complete(&t, 2).unwrap();
    c.removed(&t, true).unwrap();
    drop(c);
    let journal = fs::read(p.root().join(JOURNAL)).unwrap();
    let path = p.logs_dir().join("001_attempt_002.log");
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, b"changed log").unwrap();
    assert!(Coordinator::open(p.clone(), None, false).is_err());
    assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), journal);
    fs::write(path, bytes).unwrap();
    drop(Coordinator::open(p.clone(), None, false).unwrap());
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn owner_endpoint_no_verify_and_malformed_control_gates_preserve_prior_state() {
    let p = project();
    assert!(Coordinator::open(p.clone(), None, true).is_err());
    assert!(!p.root().join(JOURNAL).exists());
    let c = Coordinator::open(p.clone(), Some(5), false).unwrap();
    assert!(Coordinator::open(p.clone(), Some(5), false).is_err());
    assert_eq!(status(&p).unwrap()["coordinator_owner_active"], true);
    drop(c);
    let before = fs::read(p.root().join(JOURNAL)).unwrap();
    assert!(Coordinator::open(p.clone(), Some(6), false).is_err());
    assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), before);
    let mut value: Value = serde_json::from_slice(&before).unwrap();
    value["schema"] = json!(2);
    let corrupt = serde_json::to_vec(&value).unwrap();
    fs::write(p.root().join(JOURNAL), &corrupt).unwrap();
    assert!(Coordinator::open(p.clone(), Some(5), false).is_err());
    assert!(status(&p).is_err());
    assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), corrupt);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn simulated_usb_continues_to_next_disk_while_gw_reader_still_runs() {
    let p = project();
    let c = Arc::new(Mutex::new(
        Coordinator::open(p.clone(), Some(3), false).unwrap(),
    ));
    let usb = start(&mut c.lock().unwrap(), Station::Usb, 1);
    let gw = start(&mut c.lock().unwrap(), Station::Greaseweazle, 2);
    let barrier = Arc::new(Barrier::new(3));
    let (usb_done_tx, usb_done_rx) = mpsc::channel();
    let (gw_release_tx, gw_release_rx) = mpsc::channel();
    let usb_thread = {
        let c = c.clone();
        let p = p.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            evidence(&p, 1, 1, Station::Usb, &[1], 30);
            {
                let mut c = c.lock().unwrap();
                c.complete(&usb, 1).unwrap();
                c.removed(&usb, true).unwrap();
            }
            let third = start(&mut c.lock().unwrap(), Station::Usb, 3);
            evidence(&p, 3, 1, Station::Usb, &[], 32);
            {
                let mut c = c.lock().unwrap();
                c.complete(&third, 1).unwrap();
                c.removed(&third, true).unwrap();
            }
            usb_done_tx.send(()).unwrap();
        })
    };
    let gw_thread = {
        let c = c.clone();
        let p = p.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            gw_release_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            evidence(&p, 2, 1, Station::Greaseweazle, &[], 31);
            let mut c = c.lock().unwrap();
            c.complete(&gw, 1).unwrap();
            c.removed(&gw, true).unwrap();
        })
    };
    barrier.wait();
    usb_done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert_eq!(c.lock().unwrap().status()["disks"]["2"]["phase"], "reading");
    assert_eq!(
        c.lock().unwrap().status()["disks"]["3"]["phase"],
        "complete"
    );
    gw_release_tx.send(()).unwrap();
    usb_thread.join().unwrap();
    gw_thread.join().unwrap();
    assert_eq!(c.lock().unwrap().status()["usb_recovery_queue"], json!([1]));
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn modelled_136_disk_dual_cohort_reopens_without_duplicate_labels_or_lost_queue() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(136), false).unwrap();
    for number in (1..=136).step_by(2) {
        let usb = start(&mut c, Station::Usb, number);
        let gw = start(&mut c, Station::Greaseweazle, number + 1);
        evidence(
            &p,
            number,
            1,
            Station::Usb,
            if number % 5 == 1 { &[1] } else { &[] },
            30,
        );
        evidence(&p, number + 1, 1, Station::Greaseweazle, &[], 31);
        c.complete(&gw, 1).unwrap();
        c.removed(&gw, true).unwrap();
        c.complete(&usb, 1).unwrap();
        c.removed(&usb, true).unwrap();
        if number == 61 {
            drop(c);
            c = Coordinator::open(p.clone(), Some(136), false).unwrap();
        }
    }
    assert!(c.next_fresh_disk().is_none());
    let pending = c.status()["recovery_priorities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["disk"].as_u64().unwrap() as u32)
        .collect::<Vec<_>>();
    assert_eq!(pending.len(), 14);
    for number in pending {
        let gw = start(&mut c, Station::Greaseweazle, number);
        evidence(&p, number, 2, Station::Greaseweazle, &[], 30);
        c.complete(&gw, 2).unwrap();
        c.removed(&gw, true).unwrap();
    }
    drop(c);
    let c = Coordinator::open(p.clone(), Some(136), false).unwrap();
    assert_eq!(c.status()["disks"].as_object().unwrap().len(), 136);
    assert!(
        c.status()["usb_recovery_queue"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        c.status()["disks"]
            .as_object()
            .unwrap()
            .values()
            .all(|d| d["phase"] == "complete")
    );
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn offline_preview_and_usb_shortcut_have_explicit_scope_and_no_media_side_effects() {
    let p = project();
    let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let preview = crate::cli::run(
        &args(&["scan", "--double", "--plan", "--last-disk", "20", "--json"]),
        p.root(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&preview.output).unwrap();
    assert_eq!(value["preview_only"], true);
    assert_eq!(value["physical_media_access"], false);
    assert_eq!(value["next_fresh_disk"], 1);
    assert!(!p.root().join(JOURNAL).exists());
    for items in [
        vec!["scan", "--double"],
        vec!["scan", "--double", "--plan", "--no-verify"],
        vec!["scan", "--double", "--usb"],
        vec!["status", "--double"],
        vec!["scan", "--plan"],
        vec!["scan", "--double", "--plan", "--retries", "0"],
    ] {
        assert!(crate::cli::run(&args(&items), p.root()).is_err());
    }
    let status = crate::cli::run(&args(&["production", "status", "--json"]), p.root()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&status.output).unwrap()["initialized"],
        false
    );
    // The alias retains its pre-media protection gate; it never falls into GW.
    assert!(
        crate::cli::run(&args(&["scan", "--usb"]), p.root())
            .unwrap_err()
            .contains("independently verified")
    );
    assert!(!p.root().join(JOURNAL).exists());
    let mut coordinator = Coordinator::open(p.clone(), Some(20), false).unwrap();
    coordinator.claim(Station::Usb, 1).unwrap();
    coordinator.claim(Station::Greaseweazle, 2).unwrap();
    let control = fs::read(p.root().join(JOURNAL)).unwrap();
    assert_eq!(super::preview(&p, None).unwrap()["next_fresh_disk"], 3);
    assert!(super::preview(&p, Some(21)).is_err());
    assert_eq!(fs::read(p.root().join(JOURNAL)).unwrap(), control);
    drop(coordinator);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn external_journal_edit_is_not_overwritten_and_new_offline_acquisitions_are_not_reassigned() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), None, false).unwrap();
    let before = c.status();
    let path = p.root().join(JOURNAL);
    let journal = fs::read(&path).unwrap();
    fs::write(&path, b"operator edit").unwrap();
    assert!(
        c.claim(Station::Usb, 1)
            .unwrap_err()
            .contains("externally edited")
    );
    assert_eq!(c.status(), before);
    assert_eq!(fs::read(&path).unwrap(), b"operator edit");
    fs::write(&path, journal).unwrap();
    drop(c);
    evidence(&p, 1, 1, Station::Usb, &[], 12);
    let mut c = Coordinator::open(p.clone(), None, false).unwrap();
    assert_eq!(c.next_fresh_disk(), Some(2));
    assert!(c.claim(Station::Usb, 1).is_err());
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn forged_custody_routes_and_changed_transfer_evidence_are_refused_before_confirmation() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), None, false).unwrap();
    let usb = start(&mut c, Station::Usb, 1);
    evidence(&p, 1, 1, Station::Usb, &[1], 12);
    c.complete(&usb, 1).unwrap();
    c.removed(&usb, true).unwrap();
    let transfer = c.claim(Station::Greaseweazle, 1).unwrap();
    let source = p.images_dir().join("001_attempt_001.img");
    let image = fs::read(&source).unwrap();
    fs::write(&source, b"changed evidence").unwrap();
    assert!(c.confirm(&transfer, "1", true).is_err());
    assert_eq!(c.status()["disks"]["1"]["phase"], "reserved");
    fs::write(source, image).unwrap();
    let mut forged = c.journal.clone();
    forged
        .disks
        .get_mut(&1)
        .unwrap()
        .ticket
        .as_mut()
        .unwrap()
        .work = Work::Fresh;
    assert!(validate(&forged).is_err());
    let mut forged = c.journal.clone();
    forged.disks.get_mut(&1).unwrap().phase = Phase::Complete;
    assert!(validate(&forged).is_err());
    c.confirm(&transfer, "1", true).unwrap();
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn typing_queued_label_can_replace_only_an_unread_fresh_gw_offer() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(4), false).unwrap();
    let usb = start(&mut c, Station::Usb, 1);
    evidence(&p, 1, 1, Station::Usb, &[1], 14);
    c.complete(&usb, 1).unwrap();
    c.removed(&usb, true).unwrap();
    let offer = c.claim(Station::Greaseweazle, 2).unwrap();
    let other = start(&mut c, Station::Usb, 3);
    let before = c.status();
    assert!(c.select_queued_instead(&offer, 4).is_err());
    assert_eq!(c.status(), before);
    let selected = c.select_queued_instead(&offer, 1).unwrap();
    assert_eq!(selected.work, Work::UsbRecovery);
    assert_eq!(c.next_fresh_disk(), Some(2));
    assert_eq!(c.status()["disks"]["3"]["phase"], "reading");
    assert!(c.confirm(&offer, "2", true).is_err());
    c.confirm(&selected, "1", true).unwrap();
    evidence(&p, 1, 2, Station::Greaseweazle, &[], 14);
    c.complete(&selected, 2).unwrap();
    c.removed(&selected, true).unwrap();
    c.failed(&other, "synthetic interruption").unwrap();
    let gw = start(&mut c, Station::Greaseweazle, 2);
    assert!(c.select_queued_instead(&gw, 1).is_err());
    c.failed(&gw, "synthetic interruption").unwrap();
    let retry = c.claim(Station::Greaseweazle, 2).unwrap();
    assert!(retry.retry);
    assert!(c.select_queued_instead(&retry, 1).is_err());
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}
