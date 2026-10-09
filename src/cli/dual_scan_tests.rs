use super::*;

#[test]
fn stop_cancels_both_active_mock_stations_without_receipts_and_reopens_same_labels() {
    let p = project();
    let c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let token = crate::cancellation::Token::default();
    let worker_token = token.clone();
    let reader: Reader = Arc::new(move |_, _, _| {
        let start = Instant::now();
        while !crate::cancellation::requested() {
            assert!(start.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(5));
        }
        Err(crate::cancellation::MESSAGE.into())
    });
    let (tx, rx) = mpsc::sync_channel(64);
    let producer = tx.clone();
    let worker = thread::spawn(move || {
        let _scope = crate::cancellation::enter(worker_token);
        feed(
            session(c),
            rx,
            producer,
            reader,
            None,
            None,
            &mut vec![],
            false,
            None,
        )
        .unwrap()
    });
    for command in ["u1", "g2"] {
        tx.send(Event::Input(Some(command.into()))).unwrap();
    }
    wait_phase(&p, 1, "reading");
    wait_phase(&p, 2, "reading");
    tx.send(Event::Input(Some("STOP".into()))).unwrap();
    let result = worker.join().unwrap();
    assert_eq!(result["stopped"], true);
    assert!(token.requested());
    assert_eq!(result["completed_this_session"], 0);
    for disk in ["1", "2"] {
        assert_eq!(result["state"]["disks"][disk]["phase"], "interrupted");
    }
    assert!(
        imaging::load_project_statistics(&p.images_dir())
            .unwrap()
            .disks
            .is_empty()
    );
    let mut reopened = Coordinator::open(p.clone(), Some(2), false).unwrap();
    assert!(begin(&mut reopened, Station::Usb, 2).is_err());
    assert!(begin(&mut reopened, Station::Greaseweazle, 1).is_err());
    for (station, disk) in [(Station::Usb, 1), (Station::Greaseweazle, 2)] {
        let ticket = begin(&mut reopened, station, disk).unwrap();
        evidence(&p, &ticket, 1, &[]);
        reopened.complete(&ticket, 1).unwrap();
        reopened.removed(&ticket, true).unwrap();
    }
    drop(reopened);
    fs::remove_dir_all(p.root()).unwrap();
}
use sha2::{Digest, Sha256};

#[test]
fn completed_out_of_scope_and_skipped_labels_never_start_or_release_a_disk() {
    let p = project();
    let mut selected = p.clone();
    selected
        .set_current_disk_number_without_session(53)
        .unwrap();
    let mut c = Coordinator::open(selected, Some(75), false).unwrap();
    let first = begin(&mut c, Station::Greaseweazle, 53).unwrap();
    evidence(&p, &first, 1, &[]);
    c.complete(&first, 1).unwrap();
    for station in [Station::Usb, Station::Greaseweazle] {
        for number in [1, 52, 53, 55, 76, 999] {
            let before = fs::read(p.root().join(".fluxvault-production.json")).unwrap();
            assert!(begin(&mut c, station, number).is_err());
            assert_eq!(
                fs::read(p.root().join(".fluxvault-production.json")).unwrap(),
                before
            );
            assert_eq!(c.held(Station::Greaseweazle).unwrap().0.disk, 53);
        }
    }
    c.removed(&first, true).unwrap();
    for number in [53, 76] {
        assert!(begin(&mut c, Station::Greaseweazle, number).is_err());
    }
    assert_eq!(c.next_fresh_disk(), Some(54));
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn live_display_keeps_actions_and_percentages_without_repeated_help_or_host_dump() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    begin(&mut c, Station::Greaseweazle, 1).unwrap();
    let mut live = session(c);
    live.progress
        .insert(Station::Greaseweazle as u8, "50% Read pass (tracks)".into());
    let mut output = vec![];
    display(&live, &mut output, true, false).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("50% Read pass"));
    assert!(output.contains("\x1b[1;36m  ACTION:"));
    assert!(!output.contains("Shared numbering") && !output.contains("PAUSE / RESUME / QUIT"));
    assert!(!output.contains("USB -> GW pending: []"));
    assert!(!output.contains("not a completion promise") && !output.contains("need 3"));
    assert!(
        concise_stage("Detected ibm.720: reason. Evidence: C:\\long\\path").unwrap()
            == "Format: ibm.720"
    );
    assert_eq!(
        concise_stage("Fast pass: reading the whole floppy [capture budget: 600s]"),
        Some("Fast read pass".into())
    );
    drop(live);
    fs::remove_dir_all(p.root()).unwrap();
}

fn project() -> ProjectState {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-dual-live-model-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        )))
    .unwrap()
}

fn evidence(p: &ProjectState, t: &Ticket, attempt: u32, bad: &[u64]) -> Vec<u8> {
    let mut bytes = vec![(t.disk % 250) as u8; 2048];
    for lba in bad {
        bytes[*lba as usize * 512..(*lba as usize + 1) * 512].fill(0);
    }
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let stem = format!("{:03}_attempt_{attempt:03}", t.disk);
    let backend = match t.station {
        Station::Usb => "windows-raw-sector",
        Station::Greaseweazle => "greaseweazle-derived",
    };
    let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
    let log = p.logs_dir().join(format!("{stem}.log"));
    let mut text = format!(
        "BEGIN | disk={} | attempt={attempt} | source={backend}\nGEOMETRY | cylinders=2 | heads=1 | sectors_per_track=2 | bytes_per_sector=512 | total_sectors=4 | total_bytes=2048\n",
        t.disk
    );
    for l in bad {
        text.push_str(&format!("BAD_SECTOR | lba={l}\n"));
    }
    text.push_str(&format!(
        "END | status={status} | bytes=2048 | sha256={sha}\n"
    ));
    let _snapshot = crate::project_work::snapshot(p.root()).unwrap();
    fs::write(p.images_dir().join(format!("{stem}.img")), &bytes).unwrap();
    fs::write(&log, text).unwrap();
    let value = json!({"fluxvault_version":"synthetic","status":status,"disk_number":t.disk,"attempt_number":attempt,
        "source_backend":backend,"source_device":"synthetic only","image_file":format!("{stem}.img"),"log_file":log,"timestamp_unix_ms":attempt,
        "geometry":{"cylinders":2,"heads":1,"sectors_per_track":2,"bytes_per_sector":512,"total_bytes":2048,"format_guess":"synthetic"},
        "sector_retries":0,"total_sectors":4,"bytes_written":2048,"retry_recovered_sectors":0,"bad_sector_count":bad.len(),
        "bad_sectors":bad.iter().map(|l| json!({"lba":l,"cylinder":l/2,"head":0,"sector":l%2+1})).collect::<Vec<_>>(),"sha256":sha});
    fs::write(
        p.images_dir().join(format!("{stem}.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    bytes
}

fn wait_phase(p: &ProjectState, disk: u32, phase: &str) {
    let started = Instant::now();
    loop {
        // This helper bypasses the production snapshot gate. Windows' durable
        // replacement briefly rotates the old file; retry that observation,
        // without weakening the coordinator's actual parsing/ownership checks.
        let value = fs::read(p.root().join(".fluxvault-production.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if value
            .as_ref()
            .is_some_and(|v| v["disks"][disk.to_string()]["phase"] == phase)
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out waiting for {disk} {phase}: {value:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn session(c: Coordinator) -> Session {
    Session {
        coordinator: c,
        workers: BTreeMap::new(),
        started: BTreeMap::new(),
        progress: BTreeMap::new(),
        pace: crate::dual_benchmark::Live::new(),
        closing: Arc::new(AtomicBool::new(false)),
    }
}

#[test]
fn station_commands_require_positive_exact_labels_and_never_accept_blank_enter() {
    for text in ["", "1", "u", "g0", "READ 1", "u4294967295", "g-1"] {
        assert!(command(text).is_err());
    }
    assert!(matches!(
        command("u001"),
        Ok(Command::Read(Station::Usb, 1))
    ));
    assert!(matches!(
        command("G 2"),
        Ok(Command::Read(Station::Greaseweazle, 2))
    ));
    assert!(matches!(command("u out"), Ok(Command::Out(Station::Usb))));
    assert!(matches!(command("quit"), Ok(Command::Quit)));
    assert!(matches!(command("p"), Ok(Command::Pause)));
    assert!(matches!(command("resume"), Ok(Command::Resume)));
}

#[test]
fn status_keeps_held_usb_partials_visible_and_never_releases_or_starts_a_read() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let usb = begin(&mut c, Station::Usb, 1).unwrap();
    evidence(&p, &usb, 1, &[1]);
    c.complete(&usb, 1).unwrap();
    let gw = begin(&mut c, Station::Greaseweazle, 2).unwrap();
    let journal = fs::read(p.root().join(".fluxvault-production.json")).unwrap();
    let state = c.status();
    assert_eq!(state["usb_transfer_pending"], json!([1]));
    assert_eq!(state["usb_recovery_queue"], json!([]));
    let mut live = session(c);
    live.started.insert(
        Station::Greaseweazle as u8,
        Instant::now() - Duration::from_secs(87),
    );
    live.progress.insert(
        Station::Greaseweazle as u8,
        "Offline format identification".into(),
    );
    let mut output = Vec::new();
    display(&live, &mut output, false, false).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("USB -> GW pending: [1]"));
    assert!(text.contains("REMOVE 001 / SET ASIDE FOR GW"));
    assert!(text.contains("WAIT / DO NOT REMOVE 002"));
    assert!(text.contains("87s elapsed / Offline format identification"));
    assert!(!text.contains('\x1b'));
    let offline = crate::production::status(&p).unwrap();
    let text = saved_status(&offline);
    assert!(text.contains("saved state; not a live reader probe"));
    assert!(text.contains("check the original console"));
    assert!(text.contains("Removal-confirmed queue: []"));
    assert_eq!(
        fs::read(p.root().join(".fluxvault-production.json")).unwrap(),
        journal
    );
    assert_eq!(
        imaging::load_attempts_for_disk(&p.images_dir(), 2)
            .unwrap()
            .len(),
        0
    );
    live.coordinator.failed(&gw, "fixture interrupted").unwrap();
    let mut draining = Vec::new();
    display(&live, &mut draining, true, true).unwrap();
    let text = String::from_utf8(draining).unwrap();
    assert!(text.contains("DRAINING / no new reads"));
    assert!(!text.contains("INSERT fresh"));
    live.coordinator.removed(&usb, true).unwrap();
    assert_eq!(
        live.coordinator.status()["usb_transfer_pending"],
        json!([1])
    );
    assert_eq!(live.coordinator.status()["usb_recovery_queue"], json!([1]));
    drop(live);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn station_actions_distinguish_waiting_retry_transfer_and_shared_fresh_numbers() {
    let state = json!({"next_fresh_disk":53, "usb_transfer_pending":[22]});
    assert_eq!(
        next_action(&state, Station::Usb, Some((7, "reading"))),
        "WAIT / DO NOT REMOVE 007"
    );
    assert!(next_action(&state, Station::Usb, Some((7, "interrupted"))).contains("SAME 007"));
    let gw = next_action(&state, Station::Greaseweazle, None);
    assert!(gw.contains("fresh 053") && gw.contains("partial 022") && gw.contains("g22"));
    let usb = next_action(&state, Station::Usb, None);
    assert!(usb.contains("u53") && !usb.contains("g22"));
    let no_fresh = json!({"next_fresh_disk":null,"usb_transfer_pending":[22]});
    assert!(next_action(&no_fresh, Station::Greaseweazle, None).contains("g22"));
    assert_eq!(
        next_action(&no_fresh, Station::Usb, None),
        "NO FRESH DISKS / station ready"
    );
    assert!(saved_status(&json!({"initialized":false})).contains("No dual scan started"));
    let paused = json!({"paused":true,"next_fresh_disk":53,"usb_transfer_pending":[22]});
    assert!(next_action(&paused, Station::Greaseweazle, None).starts_with("PAUSED"));
    assert_eq!(
        next_action(&paused, Station::Usb, Some((7, "reading"))),
        "WAIT / DO NOT REMOVE 007"
    );
    assert!(next_action(&paused, Station::Usb, Some((22, "saved"))).contains("RESUME required"));
}

#[test]
fn ranked_actions_and_queue_inspection_are_advisory_and_keep_exact_labels() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(4), false).unwrap();
    for (disk, bad) in [(1, &[1, 2, 3][..]), (2, &[1][..])] {
        let t = begin(&mut c, Station::Usb, disk).unwrap();
        evidence(&p, &t, 1, bad);
        c.complete(&t, 1).unwrap();
        c.removed(&t, true).unwrap();
    }
    let state = c.status();
    let action = next_action(&state, Station::Greaseweazle, None);
    assert!(action.starts_with("MOVE recommended USB partial 002"));
    assert!(action.contains("g2") && action.contains("fresh 003"));
    assert!(
        next_action(&state, Station::Greaseweazle, Some((4, "saved")))
            .contains("NEXT recommended USB partial 002")
    );
    let before = fs::read(p.root().join(".fluxvault-production.json")).unwrap();
    for json_output in [false, true] {
        let mut args = vec!["production".into(), "queue".into()];
        if json_output {
            args.push("--json".into());
        }
        let result = super::super::run(&args, p.root()).unwrap();
        assert_eq!(result.exit_code, 0);
        if json_output {
            let result: Value = serde_json::from_str(&result.output).unwrap();
            assert_eq!(result["recommended_gw_disk"], 2);
            assert_eq!(result["physical_media_access"], false);
        } else {
            assert!(result.output.contains("1. Disk 002") && result.output.contains("2. Disk 001"));
            assert!(result.output.contains("heuristic, not a yield guarantee"));
        }
    }
    assert_eq!(
        fs::read(p.root().join(".fluxvault-production.json")).unwrap(),
        before
    );
    assert!(c.held(Station::Greaseweazle).is_none());
    let t = begin(&mut c, Station::Greaseweazle, 1).unwrap(); // Explicit availability wins.
    assert_eq!(t.disk, 1);
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn pause_preserves_saved_custody_and_resume_itself_launches_no_reader() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let usb = begin(&mut c, Station::Usb, 1).unwrap();
    evidence(&p, &usb, 1, &[1]);
    c.complete(&usb, 1).unwrap();
    c.set_paused(true).unwrap();
    let before = fs::read(p.root().join(".fluxvault-production.json")).unwrap();
    for (station, disk) in [(Station::Usb, 2), (Station::Greaseweazle, 1)] {
        assert!(begin(&mut c, station, disk).unwrap_err().contains("PAUSED"));
    }
    assert_eq!(
        fs::read(p.root().join(".fluxvault-production.json")).unwrap(),
        before
    );
    let reader: Reader =
        Arc::new(|_, _, _| panic!("pause/resume/status/out must never launch a reader"));
    let (tx, rx) = mpsc::sync_channel(16);
    for text in ["u2", "g1", "RESUME", "STATUS", "PAUSE", "u out", "QUIT"] {
        tx.send(Event::Input(Some(text.into()))).unwrap();
    }
    let mut output = Vec::new();
    let value = feed(
        session(c),
        rx,
        tx,
        reader,
        None,
        None,
        &mut output,
        false,
        None,
    )
    .unwrap();
    assert_eq!(value["completed_this_session"], 0);
    assert_eq!(value["state"]["paused"], true);
    assert_eq!(value["state"]["usb_recovery_queue"], json!([1]));
    assert!(value["state"]["disks"].get("2").is_none());
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("FEEDING RESUMED / NO AUTOMATIC READ"));
    assert!(text.contains("No custody changed and no read started"));
    let c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    assert!(c.paused());
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn live_event_pump_overlaps_both_readers_and_transfers_old_usb_label_without_stalling_usb() {
    let p = project();
    let c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let gate = Arc::new(AtomicBool::new(false));
    let release = gate.clone();
    let fixture = p.clone();
    let reader: Reader = Arc::new(move |ticket, guard, _| {
        if ticket.station == Station::Greaseweazle && ticket.disk == 2 {
            let start = Instant::now();
            while !release.load(Ordering::Acquire) {
                if start.elapsed() > Duration::from_secs(10) {
                    return Err("model GW gate timed out".into());
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        let attempt = if ticket.disk == 1 && ticket.station == Station::Greaseweazle {
            2
        } else {
            1
        };
        let bad = if ticket.station == Station::Usb && ticket.disk == 1 {
            vec![1]
        } else {
            vec![]
        };
        let mut candidate = vec![(ticket.disk % 250) as u8; 2048];
        for l in &bad {
            candidate[*l as usize * 512..(*l as usize + 1) * 512].fill(0);
        }
        if let Some(g) = guard {
            g.check(&candidate, &bad)?;
        }
        evidence(&fixture, &ticket, attempt, &bad);
        Ok(ReadDone {
            attempt,
            flux: None,
        })
    });
    let (tx, rx) = mpsc::sync_channel(64);
    let producer = tx.clone();
    let telemetry_project = p.clone();
    let worker = thread::spawn(move || {
        let mut output = Vec::new();
        let mut telemetry = crate::benchmark::Session::start_dual(
            &telemetry_project,
            json!({"mode":"dual","project_root":telemetry_project.root(),"paused":false}),
        )
        .unwrap();
        let value = feed(
            session(c),
            rx,
            producer,
            reader,
            None,
            None,
            &mut output,
            true,
            Some(&mut telemetry),
        )
        .unwrap();
        crate::dual_benchmark::record(&mut telemetry, "dual_session_finished", json!({})).unwrap();
        (value, String::from_utf8(output).unwrap())
    });
    for text in ["u1", "g2", "g99", ""] {
        tx.send(Event::Input(Some(text.into()))).unwrap();
    }
    wait_phase(&p, 1, "saved");
    wait_phase(&p, 2, "reading");
    let usb_source = fs::read(p.images_dir().join("001_attempt_001.img")).unwrap();
    tx.send(Event::Input(Some("u3".into()))).unwrap();
    wait_phase(&p, 3, "saved");
    wait_phase(&p, 1, "await_gw");
    wait_phase(&p, 2, "reading");
    gate.store(true, Ordering::Release);
    wait_phase(&p, 2, "saved");
    tx.send(Event::Input(Some("g1".into()))).unwrap();
    wait_phase(&p, 1, "saved");
    for text in ["g out", "u out", "QUIT"] {
        tx.send(Event::Input(Some(text.into()))).unwrap();
    }
    let (value, output) = worker.join().unwrap();
    assert_eq!(value["completed_this_session"], 4);
    assert!(
        value["state"]["disks"]
            .as_object()
            .unwrap()
            .values()
            .all(|d| d["phase"] == "complete")
    );
    assert!(value["errors"].as_array().unwrap().is_empty());
    assert!(output.contains("\x1b[1;31m") && output.contains("USB / PARTIAL SAVED 001"));
    assert!(output.contains("\x1b[1;32m") && output.contains("GW / OK SAVED 001"));
    assert!(output.contains("NO NEW READ"));
    assert!(output.contains("PACE (this invocation"));
    let benchmark = crate::dual_benchmark::report(&p).unwrap();
    assert_eq!(benchmark["timed_saved_unique_labels"], 3);
    assert_eq!(benchmark["unique_timed_receipts"], 4);
    assert_eq!(benchmark["numbered_read_confirmations"], 4);
    assert_eq!(benchmark["finished_sessions"], 1);
    assert_eq!(benchmark["reader_failures"], 0);
    assert_eq!(benchmark["timings"].as_array().unwrap().len(), 4);
    assert!(benchmark["warnings"].as_array().unwrap().is_empty());
    assert_eq!(
        fs::read(p.images_dir().join("001_attempt_001.img")).unwrap(),
        usb_source
    );
    assert_eq!(
        ProjectState::open_without_session(p.root().into())
            .unwrap()
            .current_disk_number(),
        4
    );
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn quit_drains_inflight_work_ignores_later_commands_and_retains_removal_on_reopen() {
    let p = project();
    let fixture = p.clone();
    let c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let reader: Reader = Arc::new(move |t, _, _| {
        thread::sleep(Duration::from_millis(40));
        evidence(&fixture, &t, 1, &[]);
        Ok(ReadDone {
            attempt: 1,
            flux: None,
        })
    });
    let (tx, rx) = mpsc::sync_channel(64);
    for text in ["g1", "QUIT", "u2"] {
        tx.send(Event::Input(Some(text.into()))).unwrap();
    }
    let value = feed(
        session(c),
        rx,
        tx,
        reader,
        None,
        None,
        &mut Vec::new(),
        false,
        None,
    )
    .unwrap();
    assert_eq!(value["completed_this_session"], 1);
    assert!(!p.images_dir().join("002_attempt_001.img").exists());
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    assert_eq!(c.held(Station::Greaseweazle).unwrap().1, "saved");
    let old = c.held(Station::Greaseweazle).unwrap().0;
    c.removed(&old, true).unwrap();
    assert_eq!(c.next_fresh_disk(), Some(2));
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn failure_keeps_usb_identity_but_gw_still_finishes_and_saved_usb_retry_adopts_without_drive_access()
 {
    let p = project();
    let fixture = p.clone();
    let c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let reader: Reader = Arc::new(move |t, _, _| {
        if t.station == Station::Usb {
            return Err("synthetic USB interruption".into());
        }
        evidence(&fixture, &t, 1, &[]);
        Ok(ReadDone {
            attempt: 1,
            flux: None,
        })
    });
    let (tx, rx) = mpsc::sync_channel(64);
    for text in ["u1", "g2", "QUIT"] {
        tx.send(Event::Input(Some(text.into()))).unwrap();
    }
    let value = feed(
        session(c),
        rx,
        tx,
        reader,
        None,
        None,
        &mut Vec::new(),
        false,
        None,
    )
    .unwrap();
    assert_eq!(value["state"]["disks"]["1"]["phase"], "interrupted");
    assert_eq!(value["state"]["disks"]["2"]["phase"], "saved");
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let t = begin(&mut c, Station::Usb, 1).unwrap();
    let bytes = evidence(&p, &t, 1, &[1]);
    c.failed(&t, "model crash after publication before receipt")
        .unwrap();
    drop(c);
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let retry = begin(&mut c, Station::Usb, 1).unwrap();
    let (tx, _) = mpsc::sync_channel(1);
    let result = usb_read(&p, &retry, "NO DEVICE SHOULD BE OPENED", &tx).unwrap();
    c.complete(&retry, result.attempt).unwrap();
    assert_eq!(
        imaging::load_attempts_for_disk(&p.images_dir(), 1)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fs::read(p.images_dir().join("001_attempt_001.img")).unwrap(),
        bytes
    );
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn direct_saved_usb_transfer_needs_no_extra_out_command_and_wrong_target_never_releases_custody() {
    let p = project();
    let mut c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let u = begin(&mut c, Station::Usb, 1).unwrap();
    evidence(&p, &u, 1, &[1]);
    c.complete(&u, 1).unwrap();
    assert!(begin(&mut c, Station::Usb, 3).is_err());
    assert_eq!(c.held(Station::Usb).unwrap().1, "saved");
    let g = begin(&mut c, Station::Greaseweazle, 1).unwrap();
    assert_eq!(g.work, crate::production::Work::UsbRecovery);
    assert!(c.held(Station::Usb).is_none());
    let guard = c.transfer_guard(&g).unwrap().unwrap();
    assert!(guard.check(&vec![9; 2048], &[]).is_err());
    assert!(!p.images_dir().join("001_attempt_002.img").exists());
    drop(c);
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn shared_background_owner_survives_an_exited_worker_and_draining_queue() {
    let p = project();
    let c = Coordinator::open(p.clone(), Some(3), false).unwrap();
    let queue = crate::processing::Queue::start_shared(
        crate::pipeline::PipelineRequest {
            project: p.clone(),
            seven_zip_executable: p.root().join("not-used.exe"),
            libreoffice_executable: p.root().join("not-used-office.exe"),
            command_audit_path: p.logs_dir().join("audit.jsonl"),
            conversion_workers: 4,
        },
        c.owner(),
    )
    .unwrap();
    assert!(crate::project_work::reserve(p.root()).is_err());
    queue.finish();
    assert!(crate::project_work::reserve(p.root()).is_err());
    drop(c);
    drop(crate::project_work::reserve(p.root()).unwrap());
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn dual_preflight_settings_are_strict_resume_selectors_and_no_verify_never_opens_or_creates_state()
{
    let p = project();
    let options = Options {
        usb: None,
        gw: None,
        last: Some(6),
        workers: 4,
        verified: true,
        acquisition_only: true,
        json: true,
        color: false,
    };
    let selected = settings(&p, &options).unwrap();
    save_settings(&p, &selected).unwrap();
    assert_eq!(
        settings(
            &p,
            &Options {
                last: None,
                ..options
            }
        )
        .unwrap(),
        selected
    );
    let bad = Options {
        usb: Some("B:".into()),
        gw: None,
        last: None,
        workers: 4,
        verified: true,
        acquisition_only: true,
        json: true,
        color: false,
    };
    assert!(settings(&p, &bad).is_err());
    let unverified = Options {
        verified: false,
        ..bad
    };
    assert!(settings(&p, &unverified).is_err());
    assert!(
        super::super::run(
            &["scan".into(), "--double".into(), "--no-verify".into()],
            p.root()
        )
        .unwrap_err()
        .contains("exact label")
    );
    assert!(!p.root().join(".fluxvault-production.json").exists());
    fs::remove_dir_all(p.root()).unwrap();
}
