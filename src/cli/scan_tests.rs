use super::*;
use sha2::{Digest, Sha256};
use std::io::Cursor;
fn project() -> ProjectState {
    ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-usb-feed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )))
    .unwrap()
}
fn options() -> Options {
    Options {
        json: true,
        drive: "A:".into(),
        retries: 0,
        count: None,
        last: Some(2),
        protected: true,
        workers: 4,
        acquisition_only: true,
        sound: false,
        color: false,
    }
}

#[test]
fn pause_resume_keeps_custody_and_saved_pipeline_finishes_without_rereading() {
    let mut p = project();
    let mut c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    let fake = p.root().join("unused-tool.exe");
    fs::write(&fake, []).unwrap();
    let request = crate::pipeline::PipelineRequest {
        project: p.clone(),
        seven_zip_executable: fake.clone(),
        libreoffice_executable: fake,
        command_audit_path: p.logs_dir().join("audit.jsonl"),
        conversion_workers: 4,
    };
    let queue = crate::processing::Queue::start_shared(request, c.owner()).unwrap();
    let mut o = options();
    o.last = Some(1);
    let mut reads = 0;
    let mut output = Vec::new();
    feed(
        &mut p,
        &mut c,
        &o,
        Cursor::new(b"PAUSE\n1\nSTATUS\nRESUME\n1\nOUT\n"),
        &mut output,
        |p, t| {
            reads += 1;
            Ok(evidence(p, t.disk, &[1]))
        },
        |p, d, a| enqueue(p, &queue, d, a),
    )
    .unwrap();
    assert_eq!(reads, 1);
    assert!(!c.paused());
    assert_eq!(p.current_disk_number(), 2);
    let before = fs::read(p.images_dir().join("001_attempt_001.img")).unwrap();
    let outcome = queue.finish();
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert_eq!(outcome.processed_jobs, 1);
    assert!(
        p.reports_dir()
            .join("OfflineRecoveryDecisions.json")
            .exists()
    );
    assert!(p.reports_dir().join("EvidenceAudit.json").exists());
    assert_eq!(
        fs::read(p.images_dir().join("001_attempt_001.img")).unwrap(),
        before
    );
    assert!(String::from_utf8(output).unwrap().contains("Type RESUME"));
}
fn evidence(p: &ProjectState, disk: u32, bad: &[u64]) -> u32 {
    let attempt = 1;
    let mut bytes = vec![0x34; 2048];
    for l in bad {
        bytes[*l as usize * 512..(*l as usize + 1) * 512].fill(0);
    }
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let stem = format!("{disk:03}_attempt_{attempt:03}");
    fs::write(p.images_dir().join(format!("{stem}.img")), bytes).unwrap();
    let log = p.logs_dir().join(format!("{stem}.log"));
    let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
    let mut text = format!(
        "BEGIN | disk={disk} | attempt={attempt}\nGEOMETRY | cylinders=2 | heads=1 | sectors_per_track=2 | bytes_per_sector=512 | total_sectors=4 | total_bytes=2048\n"
    );
    for l in bad {
        text.push_str(&format!("BAD_SECTOR | LBA={l}\n"));
    }
    text.push_str(&format!(
        "END | status={status} | bytes=2048 | sha256={sha}\n"
    ));
    fs::write(&log, text).unwrap();
    let meta = json!({"fluxvault_version":"synthetic","disk_number":disk,"attempt_number":attempt,"status":status,"source_backend":"windows-raw-sector","source_device":"synthetic","image_file":format!("{stem}.img"),"log_file":log,"timestamp_unix_ms":1,
        "geometry":{"cylinders":2,"heads":1,"sectors_per_track":2,"bytes_per_sector":512,"total_bytes":2048,"format_guess":"synthetic"},"sector_retries":0,"total_sectors":4,"bytes_written":2048,"retry_recovered_sectors":0,"bad_sector_count":bad.len(),"bad_sectors":bad.iter().map(|l|json!({"lba":l,"cylinder":l/2,"head":0,"sector":l%2+1})).collect::<Vec<_>>(),"sha256":sha});
    fs::write(
        p.images_dir().join(format!("{stem}.json")),
        serde_json::to_vec(&meta).unwrap(),
    )
    .unwrap();
    attempt
}

#[test]
fn labels_partial_transfer_and_endpoint_are_durable() {
    let mut p = project();
    let root = p.root().to_owned();
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let mut reads = Vec::new();
    let mut output = Vec::new();
    let mut handoffs = Vec::new();
    let response = feed(
        &mut p,
        &mut c,
        &options(),
        Cursor::new(b"\n2\n1\n1\n2\nOUT\n"),
        &mut output,
        |p, t| {
            reads.push(t.disk);
            Ok(evidence(p, t.disk, if t.disk == 1 { &[1] } else { &[] }))
        },
        |p, d, a| {
            assert!(p.current_disk_number() <= d + 1);
            handoffs.push((d, a));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(reads, vec![1, 2]);
    assert_eq!(response.exit_code, 3);
    assert_eq!(p.current_disk_number(), 3);
    assert_eq!(c.status()["usb_recovery_queue"], json!([1]));
    assert!(c.held(PhysicalStation::Usb).is_none());
    assert!(handoffs.contains(&(1, 1)));
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("SET ASIDE FOR GW")
    );
    drop(c);
    let c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    assert_eq!(c.status()["usb_recovery_queue"], json!([1]));
    drop(c);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn handoff_failure_keeps_cursor_and_resumes_saved_receipt_without_read() {
    let mut p = project();
    let root = p.root().to_owned();
    let mut c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    let mut o = options();
    o.last = Some(1);
    let error = feed(
        &mut p,
        &mut c,
        &o,
        Cursor::new(b"1\n"),
        &mut Vec::new(),
        |p, t| Ok(evidence(p, t.disk, &[])),
        |_, _, _| Err("handoff injection".into()),
    )
    .unwrap_err();
    assert_eq!(error, "handoff injection");
    assert_eq!(p.current_disk_number(), 1);
    assert_eq!(c.held(PhysicalStation::Usb).unwrap().1, "saved");
    drop(c);
    let mut c = Coordinator::open(p.clone(), Some(1), false).unwrap();
    feed(
        &mut p,
        &mut c,
        &o,
        Cursor::new(b"OUT\n"),
        &mut Vec::new(),
        |_, _| panic!("Saved receipt must not reread"),
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(p.current_disk_number(), 2);
    assert!(c.held(PhysicalStation::Usb).is_none());
    drop(c);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_or_unverified_completion_never_advances() {
    for corrupt in [false, true] {
        let mut p = project();
        let root = p.root().to_owned();
        let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
        let result = feed(
            &mut p,
            &mut c,
            &options(),
            Cursor::new(b"1\n"),
            &mut Vec::new(),
            |p, t| {
                if !corrupt {
                    return Err("synthetic hardware failure".into());
                }
                let a = evidence(p, t.disk, &[]);
                fs::write(p.images_dir().join("001_attempt_001.img"), b"tamper").unwrap();
                Ok(a)
            },
            |_, _, _| panic!("Unverified completion must not enqueue"),
        );
        assert!(result.is_err());
        assert_eq!(p.current_disk_number(), 1);
        drop(c);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn gates_eof_and_stop_never_start_reads() {
    let mut p = project();
    let root = p.root().to_owned();
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    let mut o = options();
    o.protected = false;
    assert!(
        feed(
            &mut p,
            &mut c,
            &o,
            Cursor::new(b"1\n"),
            &mut Vec::new(),
            |_, _| panic!(),
            |_, _, _| Ok(())
        )
        .is_err()
    );
    o.protected = true;
    feed(
        &mut p,
        &mut c,
        &o,
        Cursor::new(b""),
        &mut Vec::new(),
        |_, _| panic!(),
        |_, _, _| Ok(()),
    )
    .unwrap();
    let token = crate::cancellation::Token::default();
    let scope = crate::cancellation::enter(token);
    let e = feed(
        &mut p,
        &mut c,
        &o,
        Cursor::new(b"STOP\n"),
        &mut Vec::new(),
        |_, _| panic!(),
        |_, _, _| Ok(()),
    )
    .unwrap_err();
    assert!(crate::cancellation::stopped(&e));
    assert_eq!(p.current_disk_number(), 1);
    drop(scope);
    drop(c);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restarted_held_partial_is_not_implicitly_removed_on_quit() {
    let mut p = project();
    let root = p.root().to_owned();
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    feed(
        &mut p,
        &mut c,
        &options(),
        Cursor::new(b"1\nQUIT\n"),
        &mut Vec::new(),
        |p, t| Ok(evidence(p, t.disk, &[1])),
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(c.status()["usb_recovery_queue"], json!([]));
    assert_eq!(c.status()["usb_transfer_pending"], json!([1]));
    drop(c);
    let mut c = Coordinator::open(p.clone(), Some(2), false).unwrap();
    feed(
        &mut p,
        &mut c,
        &options(),
        Cursor::new(b"QUIT\n"),
        &mut Vec::new(),
        |_, _| panic!(),
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(c.held(PhysicalStation::Usb).unwrap().1, "saved");
    drop(c);
    fs::remove_dir_all(root).unwrap();
}
