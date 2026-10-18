use super::*;
use crate::{fat12, project::ProjectState};
use serde_json::json;
use std::io::{self, Cursor};

struct FaultReader {
    bytes: Cursor<Vec<u8>>,
    lba: Option<usize>,
    failures: usize,
    calls: Vec<(usize, usize)>,
}
// Deliberately has NO Write implementation or device-opening path.
impl Read for FaultReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let at = self.bytes.position() as usize;
        self.calls.push((at, buf.len()));
        if let Some(lba) = self.lba
            && self.failures > 0
            && at <= lba * 512
            && at + buf.len() > lba * 512
        {
            if buf.len() == 512 {
                self.failures -= 1;
            }
            return Err(io::Error::other("injected read-only sector failure"));
        }
        self.bytes.read(buf)
    }
}
impl Seek for FaultReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(pos)
    }
}
fn geometry() -> DiskGeometry {
    DiskGeometry {
        cylinders: 80,
        heads: 2,
        sectors_per_track: 18,
        bytes_per_sector: 512,
        media_type: 0,
    }
}
fn source(lba: Option<usize>, failures: usize) -> FaultReader {
    let mut image = fat12::tests::image();
    fat12::tests::file(
        &mut image,
        19 * 512,
        b"FIRST   TXT",
        2,
        b"exact readable prefix file",
    );
    fat12::tests::file(
        &mut image,
        19 * 512 + 32,
        b"SECOND  TXT",
        3,
        b"this failed sector must not become a whole file",
    );
    FaultReader {
        bytes: Cursor::new(image),
        lba,
        failures,
        calls: vec![],
    }
}
pub(crate) fn original_bytes() -> Vec<u8> {
    source(None, 0).bytes.into_inner()
}
fn capture(
    lba: Option<usize>,
    failures: usize,
    retries: usize,
) -> (FaultReader, Vec<u8>, Outcome, Vec<String>) {
    let mut source = source(lba, failures);
    let mut output = vec![];
    let mut log = vec![];
    let (tx, _) = mpsc::channel();
    let result = read(&mut source, &mut output, geometry(), retries, &tx, &mut log).unwrap();
    (source, output, result, log)
}

#[test]
fn clean_handoff_uses_track_reads_and_retains_every_original_byte() {
    let (s, b, r, log) = capture(None, 0, 2);
    assert_eq!(b, *s.bytes.get_ref());
    assert_eq!(s.calls.len(), 160);
    assert!(s.calls.iter().all(|(_, n)| *n == 18 * 512));
    assert!(r.failed.is_empty());
    assert_eq!(r.summary.attempted_sectors, 2880);
    assert!(r.summary.unattempted().is_empty());
    assert!(!log.iter().any(|l| l.contains("UNREAD_RANGE")));
}

#[test]
fn persistent_error_stops_all_device_access_after_bounded_retries_and_preserves_prefix() {
    let (s, b, r, log) = capture(Some(34), usize::MAX, 2);
    assert_eq!(&b[..34 * 512], &s.bytes.get_ref()[..34 * 512]);
    assert!(b[34 * 512..].iter().all(|n| *n == 0));
    assert_eq!(b.len(), 2880 * 512);
    assert_eq!(r.failed.keys().copied().collect::<Vec<_>>(), vec![34]);
    assert_eq!(r.failed[&34].len(), 3);
    assert_eq!(r.summary.attempted_sectors, 35);
    assert_eq!(r.summary.unattempted().count(), 2845);
    assert_eq!(
        s.calls
            .iter()
            .filter(|(at, n)| *at == 34 * 512 && *n == 512)
            .count(),
        3
    );
    assert_eq!(s.calls.last(), Some(&(34 * 512, 512)));
    assert!(!s.calls.iter().any(|(at, _)| *at > 34 * 512));
    assert!(log.iter().any(|l| l.contains("not_attempted=2845")));
    assert_eq!(r.recovered, 0);
}

#[test]
fn retry_resolved_error_continues_to_end_without_unknown_placeholders() {
    let (s, b, r, log) = capture(Some(34), 2, 2);
    assert_eq!(b, *s.bytes.get_ref());
    assert_eq!(r.recovered, 1);
    assert!(r.failed.is_empty());
    assert_eq!(r.summary.first_failed_lba, None);
    assert!(r.summary.unattempted().is_empty());
    assert!(
        log.iter()
            .any(|l| l.contains("retry_pass=2 | direction=forward"))
    );
}

#[test]
fn zero_one_two_retry_budgets_and_last_sector_have_exact_coverage() {
    for retries in 0..=2 {
        for lba in [0, 2879] {
            let (s, _, r, _) = capture(Some(lba), usize::MAX, retries);
            assert_eq!(r.failed[&lba].len(), retries + 1);
            assert_eq!(r.summary.unattempted().count(), 2879 - lba);
            assert_eq!(s.calls.last(), Some(&(lba * 512, 512)));
        }
    }
}

#[test]
fn cancellation_or_output_failure_never_returns_a_completed_capture() {
    let token = crate::cancellation::Token::default();
    token.request();
    {
        let _scope = crate::cancellation::enter(token);
        let mut s = source(Some(1), usize::MAX);
        let (tx, _) = mpsc::channel();
        assert!(read(&mut s, &mut Vec::new(), geometry(), 2, &tx, &mut vec![]).is_err());
        assert!(s.calls.is_empty());
    }
    struct NoSpace;
    impl Write for NoSpace {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("full"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (tx, _) = mpsc::channel();
    let mut s = source(None, 0);
    assert!(read(&mut s, &mut NoSpace, geometry(), 2, &tx, &mut vec![]).is_err());
    assert_eq!(s.calls.len(), 1);
}

/// Feed the SAME generic capture engine into ordinary project evidence loading.
pub(crate) fn publish_fixture(
    project: &ProjectState,
    disk: u32,
    lba: Option<usize>,
    retries: usize,
) -> Vec<u8> {
    let (_, bytes, outcome, events) = capture(lba, usize::MAX, retries);
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let mut text = format!(
        "BEGIN | disk={disk} | attempt=1 | source=synthetic-read-only\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\nRETRY_POLICY | retries={retries}\n{}\n",
        events.join("\n")
    );
    for l in outcome.failed.keys() {
        text.push_str(&format!("BAD_SECTOR | LBA={l}\n"));
    }
    let bad = outcome
        .failed
        .keys()
        .map(|l| *l as u64)
        .chain(outcome.summary.unattempted())
        .collect::<Vec<_>>();
    let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
    text.push_str(&format!(
        "END | status={status} | bytes=1474560 | retry_recovered=0 | sha256={sha}\n"
    ));
    let stem = format!("{disk:03}_attempt_001");
    let log = project.logs_dir().join(format!("{stem}.log"));
    fs::write(&log, text).unwrap();
    fs::write(project.images_dir().join(format!("{stem}.img")), &bytes).unwrap();
    let meta = json!({"fluxvault_version":"synthetic", "status":status,"disk_number":disk,"attempt_number":1,
        "source_backend":"windows-raw-sector","source_device":"synthetic readonly","image_file":format!("{stem}.img"),"log_file":log,
        "timestamp_unix_ms":1,"geometry":{"cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":1474560,"format_guess":"IBM"},
        "sector_retries":retries,"total_sectors":2880,"bytes_written":1474560,"retry_recovered_sectors":0,"bad_sector_count":bad.len(),
        "bad_sectors":bad.iter().map(|l|json!({"lba":l,"cylinder":l/36,"head":l/18%2,"sector":l%18+1})).collect::<Vec<_>>(),"sha256":sha,"usb_fast_handoff":outcome.summary});
    fs::write(
        project.images_dir().join(format!("{stem}.json")),
        serde_json::to_vec(&meta).unwrap(),
    )
    .unwrap();
    bytes
}

fn project() -> ProjectState {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-handoff-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
    .unwrap()
}

#[test]
fn saved_handoff_keeps_unattempted_origins_unknown_and_extracts_only_readable_file() {
    let p = project();
    let before = publish_fixture(&p, 1, Some(34), 2);
    let attempts = load_attempts_for_disk(&p.images_dir(), 1).unwrap();
    let a = &attempts[0];
    assert_eq!(a.bad_sectors.len(), 2846);
    assert_eq!(a.parsed_log.as_ref().unwrap().unread_sectors.len(), 2845);
    crate::fat12_recovery::validate_sector_evidence(a, 2880, &a.sha256).unwrap();
    let log = fs::read_to_string(&a.log_file).unwrap();
    let rows = crate::usb_passes::index(&log, 2880, &a.bad_sectors.iter().copied().collect(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(rows[33]["outcome"], "readable_saved_sector");
    assert_eq!(rows[34]["outcome"], "unreadable_placeholder");
    assert_eq!(rows[35]["outcome"], "not_attempted_placeholder");
    assert!(rows[35]["last_attempted_pass"].is_null());
    let r = crate::fat12_recovery::recover_attempt(
        &p.images_dir(),
        &p.extracted_dir(),
        &p.recovery_dir(),
        1,
        a,
        &|_| {},
    )
    .unwrap();
    assert_eq!(r.files, 1);
    assert_eq!(
        fs::read(r.output_directory.join("FIRST.TXT")).unwrap(),
        b"exact readable prefix file"
    );
    assert_eq!(
        fs::read(p.images_dir().join("001_attempt_001.img")).unwrap(),
        before
    );
    fs::remove_dir_all(p.root()).unwrap();
}

#[test]
fn malformed_handoff_policy_extent_or_log_cannot_make_unknown_bytes_readable() {
    let p = project();
    publish_fixture(&p, 1, Some(34), 2);
    let path = p.images_dir().join("001_attempt_001.json");
    let original = fs::read(&path).unwrap();
    for (key, value) in [
        ("attempted_sectors", json!(2880)),
        ("first_failed_lba", json!(u64::MAX)),
        ("retries", json!(1)),
        ("schema", json!(2)),
    ] {
        let mut meta: serde_json::Value = serde_json::from_slice(&original).unwrap();
        meta["usb_fast_handoff"][key] = value;
        fs::write(&path, serde_json::to_vec(&meta).unwrap()).unwrap();
        assert!(load_attempts_for_disk(&p.images_dir(), 1).is_err());
    }
    let mut meta: serde_json::Value = serde_json::from_slice(&original).unwrap();
    meta.as_object_mut().unwrap().remove("usb_fast_handoff");
    fs::write(&path, serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(load_attempts_for_disk(&p.images_dir(), 1).is_err());
    fs::write(&path, original).unwrap();
    let log = p.logs_dir().join("001_attempt_001.log");
    let original = fs::read_to_string(&log).unwrap();
    for text in [
        original.replace("first=35", "first=36"),
        original.replace("retry_pass=2/2", "retry_pass=1/2"),
        original.replace("END | status=PARTIAL", "END | status=OK"),
    ] {
        fs::write(&log, text).unwrap();
        assert!(load_attempts_for_disk(&p.images_dir(), 1).is_err());
    }
    fs::remove_dir_all(p.root()).unwrap();
}
