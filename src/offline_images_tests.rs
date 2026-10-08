use super::*;
use crate::{
    composite::{self, CompositeRequest, CompositeSource},
    fat12, fat12_recovery, imaging,
    pipeline::{self, PipelineRequest},
};
use std::sync::atomic::{AtomicU64, Ordering};

fn fixture(maps: &[Vec<u64>]) -> (ProjectState, Vec<u8>) {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "fluxvault-offline-images-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let project = ProjectState::create_without_session(root).unwrap();
    let mut original = fat12::tests::image();
    fat12::tests::file(
        &mut original,
        19 * 512,
        b"FIRST   TXT",
        2,
        b"first exact customer payload",
    );
    fat12::tests::file(
        &mut original,
        19 * 512 + 32,
        b"SECOND  TXT",
        3,
        b"second exact customer payload",
    );
    for (index, bad) in maps.iter().enumerate() {
        let attempt = index + 1;
        let stem = format!("001_attempt_{attempt:03}");
        let mut bytes = original.clone();
        for l in bad {
            bytes[*l as usize * 512..(*l as usize + 1) * 512].fill(0);
        }
        let sha = hash(&bytes);
        let log = project.logs_dir().join(format!("{stem}.log"));
        let mut text = format!(
            "BEGIN | disk=1 | attempt={attempt} | source=synthetic-test\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\n"
        );
        for l in bad {
            text.push_str(&format!("BAD_SECTOR | LBA={l}\n"));
        }
        let status = if bad.is_empty() { "OK" } else { "PARTIAL" };
        text.push_str(&format!(
            "END | status={status} | bytes=1474560 | sha256={sha}\n"
        ));
        fs::write(&log, text).unwrap();
        fs::write(project.images_dir().join(format!("{stem}.img")), &bytes).unwrap();
        let meta = json!({"fluxvault_version":"test", "status":status, "disk_number":1, "attempt_number":attempt,
            "source_backend":"synthetic-test", "source_device":"none", "image_file":format!("{stem}.img"), "log_file":log,
            "timestamp_unix_ms":attempt, "geometry":{"cylinders":80, "heads":2, "sectors_per_track":18,
                "bytes_per_sector":512, "total_bytes":1474560, "format_guess":"fixture"}, "sector_retries":0,
            "total_sectors":2880, "bytes_written":1474560, "retry_recovered_sectors":0, "bad_sector_count":bad.len(),
            "bad_sectors":bad.iter().map(|l| json!({"lba":l,"cylinder":l/36,"head":l/18%2,"sector":l%18+1})).collect::<Vec<_>>(), "sha256":sha});
        fs::write(
            project.images_dir().join(format!("{stem}.json")),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
    }
    (project, original)
}
fn request(project: &ProjectState) -> PipelineRequest {
    let unused = project.root().join("unused-tool.exe");
    fs::write(&unused, []).unwrap();
    PipelineRequest {
        project: project.clone(),
        seven_zip_executable: unused.clone(),
        libreoffice_executable: unused,
        command_audit_path: project.logs_dir().join("external-tools.jsonl"),
        conversion_workers: 2,
    }
}
fn managed(project: &ProjectState, attempt: &AttemptSummary) -> fat12_recovery::RecoveryResult {
    fat12_recovery::recover_attempt(
        &project.images_dir(),
        &project.extracted_dir(),
        &project.recovery_dir(),
        1,
        attempt,
        &|_| {},
    )
    .unwrap()
}
fn composite(project: &ProjectState) -> (Vec<AttemptSummary>, CompositeResult) {
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), 1).unwrap();
    let result = composite::run_composite(
        &CompositeRequest {
            disk_number: 1,
            images_directory: project.images_dir(),
            recovery_root: project.recovery_dir(),
            sources: attempts
                .iter()
                .map(|a| CompositeSource {
                    attempt_number: a.attempt_number,
                    image_path: project.images_dir().join(&a.image_file),
                    expected_sha256: Some(a.sha256.clone()),
                    total_sectors: a.total_sectors,
                    bad_sectors: a.bad_sectors.clone(),
                })
                .collect(),
        },
        &|_| {},
    )
    .unwrap();
    (attempts, result)
}
fn files(project: &ProjectState) -> Vec<String> {
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), 1).unwrap();
    let a = imaging::best_attempt(&attempts).unwrap();
    let r = managed(project, a);
    let report: fat12_recovery::RecoveryReport =
        serde_json::from_slice(&fs::read(r.report_path).unwrap()).unwrap();
    report
        .analysis
        .unwrap()
        .recovered_files
        .into_iter()
        .map(|f| f.path)
        .collect()
}

#[test]
fn complementary_boot_directory_and_payload_gaps_flow_through_the_whole_pipeline() {
    let (project, original) = fixture(&[vec![0, 33], vec![19, 34]]);
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), 1).unwrap();
    let prior = managed(&project, &attempts[0]);
    assert_eq!(prior.files, 1);
    let before = attempts
        .iter()
        .map(|a| fs::read(project.images_dir().join(&a.image_file)).unwrap())
        .collect::<Vec<_>>();
    let result = pipeline::run_pipeline(&request(&project), &|_| {}).unwrap();
    assert_eq!(result.published_recovery_images, 1);
    assert_eq!(result.declined_recovery_publications, 0);
    assert_eq!(files(&project), vec!["FIRST.TXT", "SECOND.TXT"]);
    let all = imaging::load_attempts_for_disk(&project.images_dir(), 1).unwrap();
    assert_eq!(all.len(), 3);
    let best = imaging::best_attempt(&all).unwrap();
    assert_eq!(best.status, "DERIVED");
    assert!(best.attention_required);
    assert!(best.bad_sectors.is_empty());
    assert_eq!(
        fs::read(project.images_dir().join(&best.image_file)).unwrap(),
        original
    );
    assert_eq!(
        fs::read(project.converted_dir().join("001/FIRST.TXT")).unwrap(),
        b"first exact customer payload"
    );
    assert_eq!(
        fs::read(project.converted_dir().join("001/SECOND.TXT")).unwrap(),
        b"second exact customer payload"
    );
    assert!(prior.output_directory.is_dir());
    for (a, b) in attempts.iter().zip(before) {
        assert_eq!(
            fs::read(project.images_dir().join(&a.image_file)).unwrap(),
            b
        );
    }
    let audit: Value = serde_json::from_slice(&fs::read(result.audit.json_path).unwrap()).unwrap();
    assert_eq!(audit["disks"][0]["evidence_status"], "DERIVED_RECOVERY");
    assert_eq!(audit["disks"][0]["extracted_files"], 2);
    assert_eq!(audit["customer_delivery_certified"], false);
    let again = pipeline::run_pipeline(&request(&project), &|_| {}).unwrap();
    assert_eq!(again.reused_recovery_images, 1);
    assert_eq!(
        imaging::load_attempts_for_disk(&project.images_dir(), 1)
            .unwrap()
            .len(),
        3
    );
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
fn composite_then_mirrored_fat_replays_without_hiding_remaining_payload_holes() {
    let (project, _) = fixture(&[vec![1, 33, 34], vec![1, 19, 34]]);
    let result = pipeline::run_pipeline(&request(&project), &|_| {}).unwrap();
    assert_eq!(result.composited_disks, 1);
    assert_eq!(result.reconstructed_disks, 1);
    assert_eq!(result.published_recovery_images, 1);
    assert_eq!(result.declined_recovery_publications, 0);
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), 1).unwrap();
    let best = imaging::best_attempt(&attempts).unwrap();
    assert_eq!(best.bad_sectors, vec![34]);
    assert_eq!(files(&project), vec!["FIRST.TXT"]);
    assert!(!project.converted_dir().join("001/SECOND.TXT").exists());
    let meta: Value = serde_json::from_slice(&fs::read(&best.metadata_path).unwrap()).unwrap();
    let p: Publication = serde_json::from_slice(
        &fs::read(
            project
                .reports_dir()
                .join(meta["offline_provenance"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(p.recipe.steps.len(), 2);
    assert_eq!(p.unresolved, vec![34]);
    assert!(
        pipeline::run_pipeline(&request(&project), &|_| {})
            .unwrap()
            .reused_recovery_images
            > 0
    );
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
fn standalone_mirrored_fat_handoff_preserves_derived_attention_even_with_no_gaps() {
    let (project, original) = fixture(&[vec![1]]);
    let result = pipeline::run_pipeline(&request(&project), &|_| {}).unwrap();
    assert_eq!(result.reconstructed_disks, 1);
    assert_eq!(result.published_recovery_images, 1);
    assert_eq!(result.audit.attention_disks, 1);
    assert_eq!(result.audit.verified_disks, 0);
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), 1).unwrap();
    let best = imaging::best_attempt(&attempts).unwrap();
    assert_eq!(best.status, "DERIVED");
    assert_eq!(
        fs::read(project.images_dir().join(&best.image_file)).unwrap(),
        original
    );
    assert_eq!(files(&project).len(), 2);
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
fn source_image_metadata_log_and_stage_evidence_tampering_refuse_all_derived_consumers() {
    let (project, _) = fixture(&[vec![0, 33], vec![19, 34]]);
    let (attempts, c) = composite(&project);
    let p = publish(&project, 1, &attempts, Some(&c), None).unwrap();
    let derived = imaging::load_attempts_for_disk(&project.images_dir(), 1)
        .unwrap()
        .into_iter()
        .find(|a| a.status == "DERIVED")
        .unwrap();
    let paths = [
        project.images_dir().join(&attempts[0].image_file),
        attempts[0].metadata_path.clone(),
        PathBuf::from(&attempts[0].log_file),
        c.provenance_path.clone().unwrap(),
        c.derived_image.clone().unwrap(),
        p.report.clone(),
        p.image.clone(),
        project
            .logs_dir()
            .join(format!("001_attempt_{:03}.log", p.attempt)),
    ];
    for path in paths {
        let before = fs::read(&path).unwrap();
        fs::write(&path, b"changed evidence").unwrap();
        assert!(
            imaging::load_attempts_for_disk(&project.images_dir(), 1).is_err(),
            "Accepted {}",
            path.display()
        );
        assert!(
            fat12_recovery::recover_attempt(
                &project.images_dir(),
                &project.extracted_dir(),
                &project.recovery_dir(),
                1,
                &derived,
                &|_| {}
            )
            .is_err(),
            "Direct extraction accepted changed {}",
            path.display()
        );
        fs::write(&path, before).unwrap();
        assert!(imaging::load_attempts_for_disk(&project.images_dir(), 1).is_ok());
    }
    let metadata = project
        .images_dir()
        .join(format!("001_attempt_{:03}.json", p.attempt));
    let before = fs::read(&metadata).unwrap();
    let mut value: Value = serde_json::from_slice(&before).unwrap();
    value["status"] = json!("OK");
    fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(imaging::load_attempts_for_disk(&project.images_dir(), 1).is_err());
    value["source_backend"] = json!("ordinary-acquisition");
    value.as_object_mut().unwrap().remove("offline_provenance");
    value
        .as_object_mut()
        .unwrap()
        .remove("offline_provenance_sha256");
    fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(imaging::load_attempts_for_disk(&project.images_dir(), 1).is_err());
    fs::write(&metadata, before).unwrap();
    let mut relabelled = derived.clone();
    relabelled.status = "OK".into();
    assert!(
        fat12_recovery::recover_attempt(
            &project.images_dir(),
            &project.extracted_dir(),
            &project.recovery_dir(),
            1,
            &relabelled,
            &|_| {}
        )
        .is_err()
    );
    assert!(
        publish(&project, 1, &attempts, Some(&c), None)
            .unwrap()
            .reused
    );
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
fn replay_refuses_forged_sector_choices_bounds_and_redirected_binding_names() {
    let (project, _) = fixture(&[vec![0, 33], vec![19, 34]]);
    let (attempts, c) = composite(&project);
    let published = publish(&project, 1, &attempts, Some(&c), None).unwrap();
    let p: Publication = serde_json::from_slice(&fs::read(&published.report).unwrap()).unwrap();
    let root = project.root().canonicalize().unwrap();
    let mut recipe = p.recipe.clone();
    if let Step::Composite { copies, .. } = &mut recipe.steps[0] {
        copies[0].target_lba = 100;
    }
    assert!(replay(&root, &recipe).is_err());
    let mut recipe = p.recipe.clone();
    recipe.sources.push(recipe.sources[0].clone());
    assert!(replay(&root, &recipe).is_err());
    let mut recipe = p.recipe.clone();
    recipe.sources[0].image.path = "../outside.img".into();
    assert!(replay(&root, &recipe).is_err());
    let mut recipe = p.recipe.clone();
    recipe.sources[0].bad.push(2880);
    assert!(replay(&root, &recipe).is_err());
    let mut recipe = p.recipe.clone();
    recipe.sources[0].total_sectors = usize::MAX;
    assert!(geometry(&root, &recipe).is_err());
    assert!(replay(&root, &recipe).is_err());
    let mut recipe = p.recipe.clone();
    recipe.sources[0].status = "DERIVED".into();
    assert!(replay(&root, &recipe).is_err());
    assert!(publish(&project, 1, &[], None, None).is_err());
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
fn durable_publication_boundaries_resume_one_attempt_without_overwriting_operator_files() {
    for boundary in 0..5 {
        let (project, _) = fixture(&[vec![0, 33], vec![19, 34]]);
        let (attempts, c) = composite(&project);
        let p = publish(&project, 1, &attempts, Some(&c), None).unwrap();
        let meta = project
            .images_dir()
            .join(format!("001_attempt_{:03}.json", p.attempt));
        let log = project
            .logs_dir()
            .join(format!("001_attempt_{:03}.log", p.attempt));
        // Remove only outputs from this disposable fixture to model each durable
        // checkpoint: reservation, report, image, log, completed metadata.
        if boundary < 4 {
            fs::remove_file(&meta).unwrap();
        }
        if boundary < 3 {
            fs::remove_file(&log).unwrap();
        }
        if boundary < 2 {
            fs::remove_file(&p.image).unwrap();
        }
        if boundary < 1 {
            fs::remove_file(&p.report).unwrap();
        }
        let resumed = publish(&project, 1, &attempts, Some(&c), None).unwrap();
        assert_eq!(resumed.attempt, p.attempt);
        assert_eq!(
            imaging::load_attempts_for_disk(&project.images_dir(), 1)
                .unwrap()
                .len(),
            3
        );
        fs::write(&p.image, b"operator replacement").unwrap();
        assert!(publish(&project, 1, &attempts, Some(&c), None).is_err());
        assert_eq!(fs::read(&p.image).unwrap(), b"operator replacement");
        fs::remove_dir_all(project.root()).unwrap();
    }
}

#[test]
fn ordinary_clean_acquisition_stays_preferred_and_manual_recovery_is_not_replaced() {
    let (project, _) = fixture(&[vec![0, 33], vec![19, 34]]);
    let manual = project.extracted_dir().join("001");
    fs::create_dir(&manual).unwrap();
    fs::write(manual.join("customer.txt"), b"operator recovery").unwrap();
    let result = pipeline::run_pipeline(&request(&project), &|_| {}).unwrap();
    assert_eq!(result.extraction.manual_disks, 1);
    assert_eq!(
        fs::read(manual.join("customer.txt")).unwrap(),
        b"operator recovery"
    );
    assert!(!manual.join("attempt_003_native_v4").exists());
    assert!(!manual.join("attempt_003_native_v5").exists());
    fs::remove_dir_all(project.root()).unwrap();
    let (project, _) = fixture(&[vec![0, 33], vec![]]);
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), 1).unwrap();
    assert_eq!(imaging::best_attempt(&attempts).unwrap().status, "OK");
    assert_eq!(
        crate::recovery_plan::plan_project(&project.images_dir()).unwrap()[0].action,
        crate::recovery_plan::RecoveryAction::Complete
    );
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
fn packages_include_replay_reports_exclude_reservations_and_refuse_changed_lineage() {
    let (project, _) = fixture(&[vec![0, 33], vec![19, 34]]);
    let (attempts, c) = composite(&project);
    let p = publish(&project, 1, &attempts, Some(&c), None).unwrap();
    let destination = project.root().with_file_name(format!(
        "{}-packages",
        project.root().file_name().unwrap().to_string_lossy()
    ));
    fs::create_dir(&destination).unwrap();
    let request = crate::package::PackageRequest {
        project_root: project.root().to_owned(),
        destination: destination.clone(),
        project_name: "offline-recovery".into(),
    };
    let package = crate::package::build_package(&request, &|_| {}).unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(package.zip_path).unwrap()).unwrap();
    assert!(
        archive
            .by_name(&format!(
                "Reports/{}",
                p.report.file_name().unwrap().to_string_lossy()
            ))
            .is_ok()
    );
    assert!(
        archive
            .by_name(&format!("Images/001_attempt_{:03}.img", p.attempt))
            .is_ok()
    );
    assert!(
        archive
            .by_name(&format!("Images/001_attempt_{:03}.partial.json", p.attempt))
            .is_err()
    );
    let mut readme = String::new();
    archive
        .by_name("README.txt")
        .unwrap()
        .read_to_string(&mut readme)
        .unwrap();
    assert!(readme.contains("Offline DERIVED images"));
    drop(archive);
    fs::write(&attempts[0].log_file, b"changed lineage").unwrap();
    assert!(crate::package::build_package(&request, &|_| {}).is_err());
    fs::remove_dir_all(&destination).unwrap();
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
fn oversized_source_cohort_declines_publication_without_stopping_native_processing() {
    let maps = (0..17)
        .map(|n| vec![if n % 2 == 0 { 33 } else { 34 }])
        .collect::<Vec<_>>();
    let (project, _) = fixture(&maps);
    let result = pipeline::run_pipeline(&request(&project), &|_| {}).unwrap();
    assert_eq!(result.declined_recovery_publications, 1);
    assert_eq!(result.published_recovery_images, 0);
    assert_eq!(result.extraction.extracted_disks, 1);
    assert_eq!(
        imaging::load_attempts_for_disk(&project.images_dir(), 1)
            .unwrap()
            .len(),
        17
    );
    let decisions: Value =
        serde_json::from_slice(&fs::read(&result.recovery_decisions_path).unwrap()).unwrap();
    assert!(
        decisions["decisions"][0]["publication_error"]
            .as_str()
            .unwrap()
            .contains("source count")
    );
    assert_eq!(crate::processing::summary(&result)["exit_code"], 3);
    fs::remove_dir_all(project.root()).unwrap();
}

#[test]
#[ignore = "requires FV_OFFLINE_HANDOFF_SOURCE and FV_OFFLINE_HANDOFF_OUTPUT; saved 007/009 only, complementary damage simulated on new copies"]
fn saved_hd_dd_images_replay_to_original_payloads_on_new_isolated_copies() {
    let source = ProjectState::open_without_session(PathBuf::from(
        std::env::var_os("FV_OFFLINE_HANDOFF_SOURCE").unwrap(),
    ))
    .unwrap();
    let output = PathBuf::from(std::env::var_os("FV_OFFLINE_HANDOFF_OUTPUT").unwrap());
    assert!(!output.exists(), "Refuse to reuse saved validation output");
    let project = ProjectState::create_without_session(output).unwrap();
    let mut checks = Vec::new();
    for disk in [7, 9] {
        let originals = imaging::load_attempts_for_disk(&source.images_dir(), disk).unwrap();
        let original_attempt = imaging::best_attempt(&originals).unwrap();
        assert_eq!(original_attempt.status, "OK");
        assert!(original_attempt.bad_sectors.is_empty());
        let original_path = crate::recovery_plan::resolve_image_path(
            &source.images_dir(),
            &original_attempt.image_file,
        )
        .unwrap();
        let original = read(&original_path, crate::fat12::MAX_IMAGE_BYTES as u64).unwrap();
        let source_hash = hash(&original);
        assert_eq!(source_hash, original_attempt.sha256);
        let baseline = fat12::analyze(&original, &[]).unwrap();
        let data = baseline.layout.data_start as u64;
        let root = baseline.layout.root_start as u64;
        for (number, missing) in [(1, vec![0, data]), (2, vec![root, data + 1])] {
            let stem = format!("{disk:03}_attempt_{number:03}");
            let mut bytes = original.clone();
            for l in &missing {
                bytes[*l as usize * 512..(*l as usize + 1) * 512].fill(0);
            }
            let sha = hash(&bytes);
            fs::write(project.images_dir().join(format!("{stem}.img")), &bytes).unwrap();
            let sectors = bytes.len() / 512;
            let spt = sectors / 160;
            let log = project.logs_dir().join(format!("{stem}.log"));
            let mut text = format!(
                "BEGIN | disk={disk} | attempt={number} | source=simulated-saved-copy\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track={spt} | bytes_per_sector=512 | total_sectors={sectors} | total_bytes={}\n",
                bytes.len()
            );
            for l in &missing {
                text.push_str(&format!("BAD_SECTOR | LBA={l}\n"));
            }
            text.push_str(&format!(
                "END | status=PARTIAL | bytes={} | sha256={sha}\n",
                bytes.len()
            ));
            fs::write(&log, text).unwrap();
            let meta = json!({"fluxvault_version":"validation", "status":"PARTIAL", "disk_number":disk,"attempt_number":number,
                "source_backend":"simulated-saved-copy", "source_device":"no hardware", "image_file":format!("{stem}.img"), "log_file":log,
                "timestamp_unix_ms":number, "geometry":{"cylinders":80,"heads":2,"sectors_per_track":spt,"bytes_per_sector":512,
                    "total_bytes":bytes.len(),"format_guess":"saved source geometry"},"sector_retries":0,"total_sectors":sectors,"bytes_written":bytes.len(),
                "retry_recovered_sectors":0,"bad_sector_count":missing.len(),"bad_sectors":missing.iter().map(|l| json!({"lba":l,"cylinder":l/(spt as u64*2),"head":l/spt as u64%2,"sector":l%spt as u64+1})).collect::<Vec<_>>(),"sha256":sha});
            fs::write(
                project.images_dir().join(format!("{stem}.json")),
                serde_json::to_vec(&meta).unwrap(),
            )
            .unwrap();
        }
        let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk).unwrap();
        let c = composite::run_composite(
            &CompositeRequest {
                disk_number: disk,
                images_directory: project.images_dir(),
                recovery_root: project.recovery_dir(),
                sources: attempts
                    .iter()
                    .map(|a| CompositeSource {
                        attempt_number: a.attempt_number,
                        image_path: project.images_dir().join(&a.image_file),
                        expected_sha256: Some(a.sha256.clone()),
                        total_sectors: a.total_sectors,
                        bad_sectors: a.bad_sectors.clone(),
                    })
                    .collect(),
            },
            &|_| {},
        )
        .unwrap();
        let p = publish(&project, disk, &attempts, Some(&c), None).unwrap();
        assert_eq!(fs::read(&p.image).unwrap(), original);
        let all = imaging::load_attempts_for_disk(&project.images_dir(), disk).unwrap();
        let best = imaging::best_attempt(&all).unwrap();
        assert_eq!(best.status, "DERIVED");
        let recovered = fat12_recovery::recover_attempt(
            &project.images_dir(),
            &project.extracted_dir(),
            &project.recovery_dir(),
            disk,
            best,
            &|_| {},
        )
        .unwrap();
        let report: fat12_recovery::RecoveryReport =
            serde_json::from_slice(&fs::read(&recovered.report_path).unwrap()).unwrap();
        let payloads = |a: &fat12::Analysis| {
            a.recovered_files
                .iter()
                .map(|f| (f.path.clone(), f.sha256.clone(), f.bytes))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            payloads(report.analysis.as_ref().unwrap()),
            payloads(&baseline)
        );
        assert!(
            fat12_recovery::recover_attempt(
                &project.images_dir(),
                &project.extracted_dir(),
                &project.recovery_dir(),
                disk,
                best,
                &|_| {}
            )
            .unwrap()
            .reused
        );
        assert!(
            publish(&project, disk, &attempts, Some(&c), None)
                .unwrap()
                .reused
        );
        assert_eq!(
            hash(&read(&original_path, crate::fat12::MAX_IMAGE_BYTES as u64).unwrap()),
            source_hash
        );
        checks.push(json!({"disk":disk,"source_sha256":source_hash,"bytes":original.len(),"native_chain_files":baseline.recovered_files.len(),
            "derived_attempt":p.attempt,"simulated_missing_lbas":[[0,data],[root,data+1]],"original_image_and_payloads_identical":true,"source_unchanged":true}));
    }
    let planning = crate::conversion::build_conversion_plan(
        &crate::conversion::ConversionPlanningRequest {
            extracted_root: project.extracted_dir(),
            converted_root: project.converted_dir(),
            reports_directory: project.reports_dir(),
        },
        &|_| {},
    )
    .unwrap();
    fs::write(project.reports_dir().join("OfflineDerivedValidation.json"), serde_json::to_vec_pretty(&json!({"checks":checks,
        "delivery_originals":planning.mirrored_files,"simulation_not_new_customer_recovery":true,"physical_media_access":false})).unwrap()).unwrap();
    eprintln!(
        "Saved HD/DD isolated validation retained: {}",
        project.root().display()
    );
}
