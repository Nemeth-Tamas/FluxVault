use super::*;
use crate::production::{Coordinator, Station};
use sha2::{Digest, Sha256};

struct Fixture(PathBuf, ProjectState);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-production-endpoint-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        Self(root, project)
    }
    fn record(&self, dual: bool) -> Record {
        Record {
            schema: 1,
            project: self.1.root().canonicalize().unwrap(),
            first: 1,
            last: 1,
            dual,
            destination: self.0.join("delivery"),
            workers: 2,
            scan_args: if dual {
                vec!["--double".into()]
            } else {
                vec![]
            },
            phase: Phase::Feeding,
            error: None,
            package: None,
            acquisition_attention: false,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn evidence(project: &ProjectState, attempt: u32, station: Station, bad: &[u64]) {
    // Four-sector fake image: exercise only saved receipt checks, not devices.
    let mut bytes = vec![17; 4 * 512];
    for l in bad {
        bytes[*l as usize * 512..(*l as usize + 1) * 512].fill(0);
    }
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let stem = format!("001_attempt_{attempt:03}");
    let backend = if station == Station::Usb {
        "windows-raw-sector"
    } else {
        "greaseweazle-derived"
    };
    let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
    fs::write(project.images_dir().join(format!("{stem}.img")), bytes).unwrap();
    let log = project.logs_dir().join(format!("{stem}.log"));
    let mut text = format!(
        "BEGIN | disk=1 | attempt={attempt} | source={backend}\nGEOMETRY | cylinders=2 | heads=1 | sectors_per_track=2 | bytes_per_sector=512 | total_sectors=4 | total_bytes=2048\n"
    );
    for l in bad {
        text.push_str(&format!("BAD_SECTOR | LBA={l}\n"));
    }
    text.push_str(&format!(
        "END | status={status} | bytes=2048 | sha256={sha}\n"
    ));
    fs::write(&log, text).unwrap();
    fs::write(project.images_dir().join(format!("{stem}.json")),json!({"fluxvault_version":"synthetic", "status":status,
        "disk_number":1,"attempt_number":attempt,"source_backend":backend,"source_device":"synthetic only",
        "image_file":format!("{stem}.img"),"log_file":log,"timestamp_unix_ms":attempt,
        "geometry":{"cylinders":2,"heads":1,"sectors_per_track":2,"bytes_per_sector":512,"total_bytes":2048,"format_guess":"synthetic"},
        "sector_retries":0,"total_sectors":4,"bytes_written":2048,"retry_recovered_sectors":0,"bad_sector_count":bad.len(),
        "bad_sectors":bad.iter().map(|l|json!({"lba":l,"cylinder":l/2,"head":0,"sector":l%2+1})).collect::<Vec<_>>(),"sha256":sha}).to_string()).unwrap();
}

#[test]
fn dual_endpoint_waits_for_usb_transfer_and_rechecks_current_evidence() {
    let f = Fixture::new();
    let record = f.record(true);
    assert!(!ready(&f.1, &record).unwrap());
    let mut coordinator = Coordinator::open(f.1.clone(), Some(1), false).unwrap();
    let usb = coordinator.claim(Station::Usb, 1).unwrap();
    coordinator.confirm(&usb, "1", true).unwrap();
    evidence(&f.1, 1, Station::Usb, &[1]);
    coordinator.complete(&usb, 1).unwrap();
    assert!(!ready(&f.1, &record).unwrap());
    coordinator.removed(&usb, true).unwrap();
    assert!(
        !ready(&f.1, &record).unwrap(),
        "removed USB partial still needs GW"
    );
    let gw = coordinator.claim(Station::Greaseweazle, 1).unwrap();
    coordinator.confirm(&gw, "1", true).unwrap();
    assert!(!ready(&f.1, &record).unwrap());
    evidence(&f.1, 2, Station::Greaseweazle, &[]);
    coordinator.complete(&gw, 2).unwrap();
    assert!(
        ready(&f.1, &record).unwrap(),
        "saved endpoint may finish offline while prompting removal"
    );
    coordinator.removed(&gw, true).unwrap();
    drop(coordinator);
    assert!(ready(&f.1, &record).unwrap());
    let mut foreign_range = record.clone();
    foreign_range.last = 2;
    assert!(ready(&f.1, &foreign_range).is_err());
    fs::write(f.1.images_dir().join("001_attempt_002.img"), vec![99; 2048]).unwrap();
    assert!(
        ready(&f.1, &record).is_err(),
        "a journal phase is not current integrity proof"
    );
}

#[test]
fn cursor_alone_never_authorizes_an_endpoint_archive() {
    let f = Fixture::new();
    let r = f.record(false);
    assert!(!ready(&f.1, &r).unwrap());
    assert!(super::super::flux_scan::raw_only_endpoint(&f.1, 1, 1).is_err());
    fs::write(
        f.1.root().join(".fluxvault-gw-scan.json"),
        json!({"schema_version":1,"last_disk":1,"completed":[],"pending":null}).to_string(),
    )
    .unwrap();
    // Malformed/missing scan settings must refuse, not trust the cursor.
    assert!(!matches!(ready(&f.1, &r), Ok(true)));
}

#[test]
fn saved_options_exclude_shortcuts_and_parse_paths_without_evaluation() {
    let args: Vec<String> = [
        "production",
        "start",
        "--double",
        "--last-disk",
        "20",
        "--conversion-workers",
        "12",
        "--policy",
        "policy.json",
        "--no-verify",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert!(parse(&args, Path::new("C:\\work")).is_err());
    let mut args = args;
    args.retain(|a| a != "--double");
    args.extend(["--sound".into(), "on".into()]);
    let p = parse(&args, Path::new("C:\\work")).unwrap();
    assert_eq!(p.last, Some(20));
    assert_eq!(p.workers, Some(12));
    assert_eq!(p.persistent, vec!["--policy", "C:\\work\\policy.json"]);
    assert_eq!(p.ephemeral, vec!["--no-verify", "--sound", "on"]);
}

#[test]
fn receipts_reject_foreign_oversized_and_unsafe_saved_settings() {
    let f = Fixture::new();
    let r = f.record(false);
    save(&f.1, &r).unwrap();
    assert!(load(&f.1).unwrap().is_some());
    for changed in [
        {
            let mut c = r.clone();
            c.phase = Phase::CompleteClean;
            c
        },
        {
            let mut c = r.clone();
            c.project = f.0.clone();
            c
        },
        {
            let mut c = r.clone();
            c.workers = 0;
            c
        },
        {
            let mut c = r.clone();
            c.last = 4097;
            c
        },
        {
            let mut c = r.clone();
            c.destination = PathBuf::from("A:\\delivery");
            c
        },
        {
            let mut c = r.clone();
            c.scan_args = vec!["--no-verify".into()];
            c
        },
    ] {
        fs::write(
            f.1.root().join(RECORD),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(load(&f.1).is_err());
    }
    fs::write(f.1.root().join(RECORD), vec![b' '; LIMIT as usize + 1]).unwrap();
    assert!(load(&f.1).is_err());
}

#[test]
fn archive_destination_cannot_be_the_project_or_an_ancestor() {
    let f = Fixture::new();
    let root = f.1.root().canonicalize().unwrap();
    for bad in [
        root.clone(),
        f.0.canonicalize().unwrap(),
        root.join("Reports"),
        PathBuf::from("A:\\delivery"),
    ] {
        assert!(destination(&root, Some(bad)).is_err());
    }
    let dest = destination(&root, None).unwrap();
    assert_eq!(dest, f.0.join("project-Delivery").canonicalize().unwrap());
}
