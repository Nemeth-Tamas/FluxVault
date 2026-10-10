use super::*;
use serde_json::json;

fn fixture() -> ProjectState {
    let p = ProjectState::create_without_session(std::env::temp_dir().join(format!(
        "fv-preferred-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )))
    .unwrap();
    for n in 1..=2 {
        let mut bytes = crate::fat12::tests::image();
        crate::fat12::tests::file(
            &mut bytes,
            19 * 512,
            b"HELLO   TXT",
            2,
            format!("exact payload version {n}").as_bytes(),
        );
        let sha = hash(&bytes);
        let stem = format!("001_attempt_{n:03}");
        let log = p.logs_dir().join(format!("{stem}.log"));
        fs::write(p.images_dir().join(format!("{stem}.img")), bytes).unwrap();
        fs::write(&log, format!("BEGIN | disk=1 | attempt={n}\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\nEND | status=OK | bytes=1474560 | sha256={sha}\n")).unwrap();
        let meta = json!({"fluxvault_version":"synthetic","disk_number":1,"attempt_number":n,"status":"OK","source_backend":"windows-raw-sector","source_device":"synthetic","image_file":format!("{stem}.img"),"log_file":log,"timestamp_unix_ms":n,
            "geometry":{"cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":1474560,"format_guess":"synthetic"},"sector_retries":0,"total_sectors":2880,"bytes_written":1474560,"retry_recovered_sectors":0,"bad_sector_count":0,"bad_sectors":[],"sha256":sha});
        fs::write(
            p.images_dir().join(format!("{stem}.json")),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
    }
    p
}
fn best(p: &ProjectState) -> u32 {
    imaging::best_attempt(&imaging::load_attempts_for_disk(&p.images_dir(), 1).unwrap())
        .unwrap()
        .attempt_number
}

#[test]
fn preferred_attempt_drives_native_payload_inventory_and_audit() {
    let p = fixture();
    for (choice, n) in [(Some(1), 1), (None, 2)] {
        set(&p, 1, choice).unwrap();
        let attempts = imaging::load_attempts_for_disk(&p.images_dir(), 1).unwrap();
        let selected = imaging::best_attempt(&attempts).unwrap();
        let inspection = crate::sector_inspection::inspect(&p, 1, 33, 1, None).unwrap();
        assert_eq!(inspection["attempt"], n);
        let result = crate::fat12_recovery::recover_attempt(
            &p.images_dir(),
            &p.extracted_dir(),
            &p.recovery_dir(),
            1,
            selected,
            &|_| {},
        )
        .unwrap();
        assert_eq!(result.files, 1);
        assert_eq!(
            fs::read(result.output_directory.join("HELLO.TXT")).unwrap(),
            format!("exact payload version {n}").as_bytes()
        );
        let manifest = crate::manifest::build_manifest(
            &crate::manifest::ManifestRequest {
                extracted_root: p.extracted_dir(),
                images_directory: p.images_dir(),
                reports_directory: p.reports_dir(),
            },
            &|_| {},
        )
        .unwrap();
        assert_eq!(manifest.file_count, 1);
        let audit = crate::audit::run_audit(&p, &|_| {}).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(audit.json_path).unwrap()).unwrap();
        assert_eq!(value["disks"][0]["attempt"], n);
        assert_eq!(value["disks"][0]["image_sha256"], selected.sha256);
    }
}

#[test]
fn cli_preference_refreshes_native_inventory_and_reports_under_one_owner() {
    let p = fixture();
    let response = crate::cli::run(
        &[
            "disk".into(),
            "prefer".into(),
            "1".into(),
            "1".into(),
            "--json".into(),
        ],
        p.root(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&response.output).unwrap();
    assert!(matches!(response.exit_code, 0 | 3));
    assert_eq!(value["automatic_refresh"]["native_extraction"]["files"], 1);
    assert!(Path::new(value["automatic_refresh"]["manifest"].as_str().unwrap()).is_file());
    assert!(Path::new(value["automatic_refresh"]["workbook"].as_str().unwrap()).is_file());
    assert_eq!(value["automatic_refresh"]["physical_media_access"], false);
    assert_eq!(best(&p), 1);
    assert!(crate::project_work::reserve(p.root()).is_ok());
}

#[test]
fn selecting_older_and_restoring_auto_preserves_evidence_and_history() {
    let p = fixture();
    let before = fs::read(p.images_dir().join("001_attempt_001.img")).unwrap();
    assert_eq!(best(&p), 2);
    set(&p, 1, Some(1)).unwrap();
    assert_eq!(best(&p), 1);
    let lifecycle = crate::disk_record::inspect(
        &p,
        1,
        &imaging::load_attempts_for_disk(&p.images_dir(), 1).unwrap(),
    )
    .unwrap();
    assert_eq!(lifecycle["preferred_image"]["attempt"], 1);
    assert_eq!(
        lifecycle["preferred_image"]["selection"],
        "operator hash-bound preference"
    );
    set(&p, 1, None).unwrap();
    assert_eq!(best(&p), 2);
    assert_eq!(p.current_disk_number(), 1);
    assert_eq!(
        fs::read(p.images_dir().join("001_attempt_001.img")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_dir(directory(&p.images_dir()).unwrap())
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".history.json"))
            .count(),
        1
    );
}

#[test]
fn mutated_image_metadata_and_log_refuse_silent_fallback() {
    for path in [
        "Images/001_attempt_001.img",
        "Images/001_attempt_001.json",
        "Logs/001_attempt_001.log",
    ] {
        let p = fixture();
        set(&p, 1, Some(1)).unwrap();
        let path = p.root().join(path);
        let original = fs::read(&path).unwrap();
        let mut changed = original.clone();
        changed.push(b' ');
        fs::write(&path, changed).unwrap();
        assert!(
            imaging::load_attempts_for_disk(&p.images_dir(), 1).is_err(),
            "{}",
            path.display()
        );
        fs::write(&path, original).unwrap();
        assert_eq!(best(&p), 1);
    }
}

#[test]
fn missing_attempt_is_explicitly_clearable_and_duplicate_is_refused() {
    let p = fixture();
    set(&p, 1, Some(1)).unwrap();
    let meta = p.images_dir().join("001_attempt_001.json");
    let bytes = fs::read(&meta).unwrap();
    fs::remove_file(&meta).unwrap();
    assert!(imaging::load_attempts_for_disk(&p.images_dir(), 1).is_err());
    set(&p, 1, None).unwrap();
    assert_eq!(best(&p), 2);
    fs::write(&meta, bytes).unwrap();
    set(&p, 1, Some(1)).unwrap();
    let mut attempts = imaging::load_unselected_attempts(&p.images_dir(), 1).unwrap();
    attempts.push(attempts[0].clone());
    assert!(apply(&p.images_dir(), 1, &mut attempts).is_err());
}

#[test]
fn absent_attempt_busy_owner_and_oversized_control_are_refused() {
    let p = fixture();
    assert!(set(&p, 1, Some(99)).is_err());
    assert!(!directory(&p.images_dir()).unwrap().exists());
    let owner = crate::project_work::reserve(p.root()).unwrap();
    assert!(set(&p, 1, Some(1)).is_err());
    drop(owner);
    set(&p, 1, Some(1)).unwrap();
    fs::write(
        directory(&p.images_dir()).unwrap().join("001.json"),
        vec![b' '; 16385],
    )
    .unwrap();
    assert!(imaging::load_attempts_for_disk(&p.images_dir(), 1).is_err());
}
