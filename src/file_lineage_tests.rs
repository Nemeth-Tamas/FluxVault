use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture(ProjectState);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-lineage-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        // Exclusive reservation: never reopen another fixture, even if the
        // workstation timestamp has identical resolution on parallel threads.
        fs::create_dir(&root).unwrap();
        Self(ProjectState::create_without_session(root).unwrap())
    }
    fn image(&self, bytes: &[u8]) {
        let p = &self.0;
        let hash = sha(bytes);
        let sectors = bytes.len() / 512;
        fs::write(p.images_dir().join("001_attempt_001.img"), bytes).unwrap();
        fs::write(p.logs_dir().join("001_attempt_001.log"),format!("BEGIN | disk=001 | attempt=001 | source=fixture\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors={sectors} | total_bytes={}\nRETRY_POLICY | retries=0\nEND | status=OK | bad_sectors=0 | retry_recovered=0 | bytes={} | sha256={hash}",bytes.len(),bytes.len())).unwrap();
        fs::write(p.images_dir().join("001_attempt_001.json"),serde_json::to_vec(&json!({"fluxvault_version":"0.1.0","sector_retries":0,"disk_number":1,"attempt_number":1,"status":"OK","image_file":"001_attempt_001.img","log_file":"001_attempt_001.log","source_backend":"windows-raw-sector","source_device":"test-fixture-never-opened","timestamp_unix_ms":1,"geometry":{"format_guess":"fixture HD","cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":bytes.len()},"total_sectors":sectors,"bytes_written":bytes.len(),"bad_sector_count":0,"bad_sectors":[],"retry_recovered_sectors":0,"sha256":hash})).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(self.0.root()) {
            if std::thread::panicking() {
                eprintln!("Fixture cleanup failed; original panic retained: {error}");
            } else {
                panic!("Fixture cleanup failed: {error}");
            }
        }
    }
}
#[test]
fn paths_refuse_devices_streams_parents_wildcards_and_cross_label() {
    let f = Fixture::new();
    for p in ["", "../x", "C:/x", "/x", "Extracted/001/x:stream"] {
        assert!(relative(p).is_err());
    }
    assert!(
        inspect(&f.0, 2, "Extracted/001/x.txt")
            .unwrap_err()
            .contains("requested disk")
    );
    assert!(inspect(&f.0, 0, "Extracted/001/x.txt").is_err());
    let _owner = crate::project_work::reserve(f.0.root()).unwrap();
    assert!(inspect(&f.0, 1, "Extracted/001/x.txt").is_err());
}
#[test]
fn native_file_replays_payload_sectors_and_bound_usb_pass_without_writes() {
    let f = Fixture::new();
    let mut image = crate::fat12::tests::image();
    crate::fat12::tests::file(
        &mut image,
        19 * 512,
        b"TEST    TXT",
        2,
        b"readable exact text",
    );
    f.image(&image);
    let attempt = imaging::load_attempts_for_disk(&f.0.images_dir(), 1)
        .unwrap()
        .remove(0);
    let recovered = crate::fat12_recovery::recover_attempt(
        &f.0.images_dir(),
        &f.0.extracted_dir(),
        &f.0.recovery_dir(),
        1,
        &attempt,
        &|_| {},
    )
    .unwrap();
    let output = recovered.output_directory.join("TEST.TXT");
    let relative = source_relative(&f.0, &output).unwrap();
    let rel = relative.to_string_lossy();
    let original = fs::read(&output).unwrap();
    let v = inspect(&f.0, 1, &rel).unwrap();
    assert_eq!(v["file"]["kind"], "native_chain_recovery");
    assert_eq!(v["file"]["source_image_binding"]["attempt_number"], 1);
    assert_eq!(v["file"]["source_image_binding"]["sha256"], sha(&image));
    assert_eq!(v["file"]["source_byte_extents"][0]["lba"], 33);
    assert!(
        v["file"]["metadata_origins"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert_eq!(
        v["file"]["source_byte_extents"][0]["origin"]["usb_pass"]["successful_pass"],
        0
    );
    assert_eq!(fs::read(&output).unwrap(), original);
    assert_eq!(
        fs::read(f.0.images_dir().join("001_attempt_001.img")).unwrap(),
        image
    );
    let cli = crate::cli::run(
        &[
            "recovery".into(),
            "trace".into(),
            "1".into(),
            rel.to_string(),
            "--json".into(),
        ],
        f.0.root(),
    )
    .unwrap();
    assert_eq!(cli.exit_code, 3);
    assert!(cli.output.contains("native_chain_recovery"));
    let snapshot = sector_inspection::load_snapshot(&f.0, 1, Some(1)).unwrap();
    assert!(metadata_origins(&snapshot, &[2880]).is_err());
    assert!(metadata_origins(&snapshot, &[u64::MAX]).is_err());
    fs::write(&output, b"changed").unwrap();
    assert!(inspect(&f.0, 1, &rel).is_err());
}
#[test]
fn manual_import_binding_reports_external_allocation_unknown_and_refuses_changed_manifest() {
    let f = Fixture::new();
    let dir = f.0.extracted_dir().join("001/manual_test");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("test.rtf"), b"{\\rtf1 retained}").unwrap();
    let marker = dir.join("__fluxvault-manual-recovery.json");
    let v = json!({"schema_version":1,"disk_number":1,"source_directory":"old external source","source_dmde_log":"old.log","dmde_log_sha256":"unproven-existing-external-log","files":[{"relative_path":"test.rtf","bytes":16,"sha256":sha(b"{\\rtf1 retained}")}]});
    // Exact imported byte count, not a guessed Word/RTF semantic certificate.
    let mut v = v;
    v["files"][0]["bytes"] = json!(fs::metadata(dir.join("test.rtf")).unwrap().len());
    fs::write(&marker, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = inspect(&f.0, 1, "Extracted/001/manual_test/test.rtf").unwrap();
    assert_eq!(r["file"]["kind"], "manual_import");
    assert!(r["file"]["external_source"]["sector_origins"].is_null());
    v["disk_number"] = json!(2);
    fs::write(marker, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(inspect(&f.0, 1, "Extracted/001/manual_test/test.rtf").is_err());
}
#[test]
fn recovery_fragment_replays_source_and_refuses_tampered_or_out_of_bounds_extents() {
    let f = Fixture::new();
    let image = crate::fat12::tests::image();
    f.image(&image);
    let dir = f.0.recovery_dir().join("001/attempt_001_fragments_v2");
    fs::create_dir_all(&dir).unwrap();
    let bytes = &image[33 * 512..34 * 512];
    fs::write(dir.join("fragment.bin"), bytes).unwrap();
    let report = dir.join("fragments.json");
    let mut v = json!({"schema_version":1,"disk":1,"attempt":1,"source_sha256":sha(&image),"fragments":[{"path":"fragment.bin","sha256":sha(bytes),"source_extents":[{"file_offset":0,"source_byte_offset":33*512,"bytes":512}]}]});
    fs::write(&report, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = inspect(
        &f.0,
        1,
        "Recovery/001/attempt_001_fragments_v2/fragment.bin",
    )
    .unwrap();
    assert_eq!(r["file"]["source_byte_extents"][0]["lba"], 33);
    assert_eq!(
        r["file"]["confidence"],
        "byte_replayed_incomplete_fragment_not_whole_file"
    );
    v["fragments"][0]["source_extents"][0]["source_byte_offset"] = json!(image.len());
    fs::write(report, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(
        inspect(
            &f.0,
            1,
            "Recovery/001/attempt_001_fragments_v2/fragment.bin"
        )
        .is_err()
    );
}
#[test]
fn untracked_saved_file_is_explicit_unknown_not_invented_provenance() {
    let f = Fixture::new();
    let dir = f.0.extracted_dir().join("001/old");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("legacy.txt"), b"old").unwrap();
    let r = inspect(&f.0, 1, "Extracted/001/old/legacy.txt").unwrap();
    assert_eq!(r["file"]["kind"], "unbound_saved_file");
    assert!(r["file"]["source_byte_extents"].is_null());
    assert_eq!(r["files_written"], 0);
}

#[test]
fn deleted_and_signature_outputs_replay_their_distinct_native_origins() {
    let f = Fixture::new();
    let mut image = crate::fat12::tests::image();
    crate::fat12::tests::file(
        &mut image,
        19 * 512,
        b"DELETED TXT",
        2,
        b"deleted chain candidate",
    );
    image[19 * 512] = 0xe5;
    let signature = br"{\rtf1 retained unallocated text}";
    image[100 * 512..100 * 512 + signature.len()].copy_from_slice(signature);
    for copy in 0..2 {
        crate::fat12::tests::set_fat(&mut image, copy, 69, 0xfff);
    }
    f.image(&image);
    let attempt = imaging::load_attempts_for_disk(&f.0.images_dir(), 1)
        .unwrap()
        .remove(0);
    let deleted = crate::fat12_recovery::recover_deleted_attempt(
        &f.0.images_dir(),
        &f.0.extracted_dir(),
        &f.0.recovery_dir(),
        1,
        &attempt,
        &|_| {},
    )
    .unwrap();
    let proof = extraction::verified_native_report(&deleted.output_directory)
        .unwrap()
        .unwrap();
    let chain = &proof.deleted.as_ref().unwrap().surviving_chain_files[0];
    let rel = source_relative(&f.0, &deleted.output_directory.join(&chain.path)).unwrap();
    let traced = inspect(&f.0, 1, &rel.to_string_lossy()).unwrap();
    assert_eq!(traced["file"]["kind"], "deleted_surviving_chain_candidate");
    assert_eq!(traced["file"]["source_byte_extents"][0]["lba"], 33);
    let native = crate::fat12_recovery::recover_attempt(
        &f.0.images_dir(),
        &f.0.extracted_dir(),
        &f.0.recovery_dir(),
        1,
        &attempt,
        &|_| {},
    )
    .unwrap();
    let proof = extraction::verified_native_report(&native.output_directory)
        .unwrap()
        .unwrap();
    let candidate = proof
        .carving
        .as_ref()
        .unwrap()
        .files
        .iter()
        .find(|c| {
            c.source_extents
                .iter()
                .any(|e| e.source_byte_offset == 100 * 512)
        })
        .unwrap();
    let rel = source_relative(&f.0, &native.output_directory.join(&candidate.path)).unwrap();
    let traced = inspect(&f.0, 1, &rel.to_string_lossy()).unwrap();
    assert_eq!(traced["file"]["kind"], candidate.recovery_method);
    assert_eq!(traced["file"]["source_byte_extents"][0]["lba"], 100);
    assert_eq!(traced["file"]["original_name_known"], false);
    assert_eq!(
        fs::read(f.0.images_dir().join("001_attempt_001.img")).unwrap(),
        image
    );
}

#[test]
fn converted_file_is_hash_bound_to_import_not_invented_sector_or_semantic_mapping() {
    let f = Fixture::new();
    let dir = f.0.extracted_dir().join("001/manual");
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.rtf");
    let input = br"{\rtf1 original}";
    fs::write(&source, input).unwrap();
    fs::write(dir.join("__fluxvault-manual-recovery.json"), serde_json::to_vec(&json!({
        "schema_version":1,"disk_number":1,"files":[{"relative_path":"source.rtf","bytes":input.len(),"sha256":sha(input)}]
    })).unwrap()).unwrap();
    let out_dir = f.0.converted_dir().join("001");
    fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("source.docx");
    // A recorded converter receipt binds bytes, not valid DOCX semantics.
    let output = b"fixture output, deliberately not a semantic certificate";
    fs::write(&out, output).unwrap();
    let success = json!({"state":"Ok","detail":"fixture receipt","retryable":false,"retry_count":0,"output_sha256":sha(output)});
    let job = json!({"floppy":"001","source_path":source,"original_forensic_path":"source.rtf","delivery_original_path":"001/source.rtf","recovery_method":"manual","source_type":"RTF","source_sha256":sha(input),"modern_format":"docx","modern_filter":"fixture","pdf_filter":"fixture","modern_path":out,"pdf_path":out_dir.join("source.pdf")});
    let mut state = json!({"schema_version":1,"project_root":f.0.root().canonicalize().unwrap(),"result":{
        "planning":{"disk_count":1,"mirrored_files":1,"reused_files":0,"conversion_candidates":1,"path_map":"fixture.csv","conversion_plan":"fixture.csv","jobs":[job]},
        "ok":1,"partial":0,"failed":0,"timed_out":0,"reused_outputs":0,"retried_outputs":0,"issues":[],"summary_path":"fixture.csv","failures_path":"fixture.csv",
        "rows":[{"job":job,"modern":success,"pdf":{"state":"Failed","detail":"not requested","retryable":false,"retry_count":0},"duration_seconds":1.0}]
    }});
    let control_path = f.0.reports_dir().join("ConversionState.json");
    fs::write(&control_path, serde_json::to_vec(&state).unwrap()).unwrap();
    let traced = inspect(&f.0, 1, "Converted/001/source.docx").unwrap();
    assert_eq!(traced["file"]["kind"], "recorded_conversion");
    assert_eq!(
        traced["file"]["conversion"]["source"]["kind"],
        "manual_import"
    );
    assert!(traced["file"]["conversion"]["source"]["source_byte_extents"].is_null());
    state["result"]["rows"][0]["job"]["source_sha256"] = json!("changed");
    fs::write(control_path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(inspect(&f.0, 1, "Converted/001/source.docx").is_err());
}

#[test]
fn text_html_and_editable_trace_declared_sources_without_claiming_character_equivalence() {
    for name in ["word-text.json", "rtf-text.json", "editable-documents.json"] {
        let f = Fixture::new();
        let mut image = crate::fat12::tests::image();
        image[33 * 512..33 * 512 + 4].copy_from_slice(b"text");
        f.image(&image);
        let dir = f.0.recovery_dir().join("001/forensic_fixture");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("text.txt"), b"text").unwrap();
        fs::write(dir.join("edition.html"), b"<p>text</p>").unwrap();
        fs::write(dir.join("edition.docx"), b"hash-bound presentation fixture").unwrap();
        let mut report = json!({"schema_version":1,"disk":1,"attempt":1,"source_sha256":sha(&image),"documents":[{
            "text_path":"text.txt","text_sha256":sha(b"text"),"edition_path":"edition.html","edition_sha256":sha(b"<p>text</p>"),
            "editable_edition":{"path":"edition.docx","sha256":sha(b"hash-bound presentation fixture")},
            "source_extents":[{"file_offset":0,"source_byte_offset":33*512,"bytes":4}]
        }]});
        fs::write(dir.join(name), serde_json::to_vec(&report).unwrap()).unwrap();
        for file in ["text.txt", "edition.html", "edition.docx"] {
            let traced =
                inspect(&f.0, 1, &format!("Recovery/001/forensic_fixture/{file}")).unwrap();
            assert_eq!(
                traced["file"]["encoded_source_relations"][0]["extent_origins"][0]["lba"],
                33
            );
            assert!(traced["file"]["decoded_character_to_input_byte_mapping"].is_null());
            assert!(
                traced["file"]["confidence"]
                    .as_str()
                    .unwrap()
                    .contains("not_independently_redecoded")
            );
            if file == "edition.docx" {
                assert_eq!(traced["file"]["kind"], "editable_text_edition");
            }
        }
        report["documents"][0]["source_extents"][0]["source_byte_offset"] = json!(image.len());
        fs::write(dir.join(name), serde_json::to_vec(&report).unwrap()).unwrap();
        assert!(inspect(&f.0, 1, "Recovery/001/forensic_fixture/edition.html").is_err());
    }
}
