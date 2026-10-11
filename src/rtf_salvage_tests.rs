use super::*;

#[test]
fn escaped_text_group_state_and_unicode_fallback_are_source_deterministic() {
    let d = text::decode(
        br"{\rtf1\ansi\ansicpg1250 A\'f5{\uc0\u337 }\u337?\{\}\\\~\_\-\par\tab Z}",
        "boundary",
    );
    assert_eq!(d.text, "Aőőő{}\\\u{00a0}\u{2011}\u{00ad}\n\tZ");
    assert!(d.stop.starts_with("Readable root group ended"));
    let d = text::decode(br"{\rtf1\uc2 \u337\'3f?X}", "boundary");
    assert_eq!(d.text, "őX");
}

#[test]
fn font_code_pages_are_scoped_and_symbol_fonts_are_never_guessed() {
    let d=text::decode(br"{\rtf1\ansi\ansicpg1250\deff0{\fonttbl{\f0\fcharset238 Hungarian;}{\f1\fcharset0 Western;}{\f2\fcharset2 Symbol;}}\f0\'f5{\f1\'f5}\'f5\f2 A}","boundary");
    assert_eq!(d.text, "őõő");
    assert!(d.stop.contains("selected font code page"));
    let d = text::decode(
        br"{\rtf1\deff0{\fonttbl{\f0\fcharset2\cpg1250 Explicit;}}\'f5}",
        "boundary",
    );
    assert_eq!(d.text, "ő");
}

#[test]
fn hidden_deleted_destinations_and_binary_payloads_never_leak_into_body() {
    let d=text::decode(br"{\rtf1{\fonttbl{\f0 HIDDEN;}}A{\info LEAK}{\*\unknown SKIP}{\pict\bin4 {\}}}\v HIDE\v0 B{\deleted BAD\v0 STILLBAD}C{\field SECRET}D}","boundary");
    assert_eq!(d.text, "ABCD");
    assert!(d.stop.starts_with("Readable root"), "{}", d.stop);
}

#[test]
fn unknown_ignorable_metadata_and_duplicate_definitions_cannot_rebind_fonts() {
    let d = text::decode(
        br"{\rtf1\deff0{\fonttbl{\f0\fcharset0 Known;{\*\unknown\f0\cpg932}}}\'f5}",
        "boundary",
    );
    assert_eq!(d.text, "õ");
    let d=text::decode(br"{\rtf1\deff0{\fonttbl{\f0\fcharset0 Known;}}{\*\unknown{\fonttbl{\f0\cpg932 INVALID;}}}\'f5}","boundary");
    assert_eq!(d.text, "õ");
    let d = text::decode(
        br"{\rtf1\deff0{\fonttbl{\f0\fcharset0 First;}{\f0\fcharset238 Second;}}\'f5}",
        "boundary",
    );
    assert!(d.text.is_empty() && d.stop.contains("Duplicate RTF font"));
}

#[test]
fn holes_truncated_tokens_unknown_controls_and_invalid_units_stop_without_resync() {
    for (input, expected) in [
        (br"{\rtf1 good\'f".as_slice(), "good"),
        (br"{\rtf1 good\unknown BAD}", "good"),
        (br"{\rtf1 good\u-10179?BAD}", "good"),
        (br"{\rtf1 good\'81 BAD}", "good"),
        (br"{\rtf1 good\bin2 XX}", "good"),
        (br"{\rtf1 good\ansicpg932 BAD}", "good"),
        (br"{\rtf1 good\uc99 BAD}", "good"),
        (br"{\rtf1 good\pict BAD}", "good"),
        (br"{\rtf1 good\u999999999999 BAD}", "good"),
        (br"{\rtf1 good\fs20", "good"),
    ] {
        let d = text::decode(input, "First unreadable byte");
        assert_eq!(d.text, expected, "{}", d.stop);
        assert!(d.through < input.len());
    }
    let d = text::decode(b"{\\rtf1 known prefix", "First unreadable byte");
    assert_eq!(d.text, "known prefix");
    assert_eq!(d.stop, "First unreadable byte");
    assert_eq!(d.through, 19);
}

#[test]
fn arbitrary_bounded_bytes_do_not_panic_and_depth_work_output_stay_bounded() {
    for i in 0..256 {
        let bytes = [b"{\\rtf1 ".as_slice(), &vec![i as u8; 1024]].concat();
        let d = text::decode(&bytes, "boundary");
        assert!(d.through <= bytes.len());
    }
    let depth = [b"{\\rtf1 ".as_slice(), &[b'{'; 129]].concat();
    assert!(
        text::decode(&depth, "boundary")
            .stop
            .contains("nesting ceiling")
    );
    let many = [b"{\\rtf1 ".as_slice(), &vec![b'x'; 210_000]].concat();
    let d = text::decode(&many, "boundary");
    assert_eq!(d.text.len(), 200_000);
    assert!(d.stop.contains("output ceiling"));
}

fn fixture() -> (
    crate::project::ProjectState,
    crate::imaging::AttemptSummary,
    Vec<u8>,
) {
    let prefix = [
        br"{\rtf1\ansi\ansicpg1250 FIRST READABLE ".as_slice(),
        &vec![b'x'; 1200],
        br" LOST THEN LATER SECRET}",
    ]
    .concat();
    let mut image = fat12::tests::image();
    fat12::tests::entry(
        &mut image,
        19 * 512,
        b"BROKEN  RTF",
        100,
        prefix.len() as u32,
        false,
    );
    let clusters = [100u16, 103, 106];
    for (i, c) in clusters.iter().enumerate() {
        for copy in 0..2 {
            fat12::tests::set_fat(
                &mut image,
                copy,
                *c,
                clusters.get(i + 1).copied().unwrap_or(0xfff),
            );
        }
        let at = (33 + *c as usize - 2) * 512;
        let n = (prefix.len() - i * 512).min(512);
        image[at..at + n].copy_from_slice(&prefix[i * 512..i * 512 + n]);
    }
    let bad = 33 + 103 - 2;
    image[bad * 512..(bad + 1) * 512].fill(0xff);
    let root = std::env::temp_dir().join(format!(
        "fv-rtf-integration-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let p = crate::project::ProjectState::create_without_session(root).unwrap();
    fs::write(p.images_dir().join("001.img"), &image).unwrap();
    fs::write(p.logs_dir().join("001.log"),format!("BEGIN | disk=1 | attempt=1\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\nBAD_SECTOR | lba={bad}\nEND | status=PARTIAL | bytes=1474560 | sha256={}\n",hash(&image))).unwrap();
    let attempt = crate::imaging::load_attempts_for_disk(&p.images_dir(), 1)
        .unwrap()
        .remove(0);
    (p, attempt, image)
}

#[test]
fn native_pipeline_cli_reuse_tamper_and_archival_evidence_keep_zero_whole_files() {
    let (p, attempt, image) = fixture();
    let run = || {
        crate::fat12_recovery::recover_attempt(
            &p.images_dir(),
            &p.extracted_dir(),
            &p.recovery_dir(),
            1,
            &attempt,
            &|_| {},
        )
    };
    let result = run().unwrap();
    assert_eq!(result.files, 0);
    let text = result.rtf_salvage.unwrap();
    assert_eq!(text.documents_with_text, 1);
    let report: Report = serde_json::from_slice(&fs::read(&text.report_path).unwrap()).unwrap();
    let doc = &report.documents[0];
    assert_eq!(doc.readable_prefix_bytes, 512);
    assert!(
        !doc.repaired_original
            && !doc.customer_delivery_certified
            && !doc.remaining_text_length_known
    );
    let path = text.output_directory.join(doc.text_path.as_ref().unwrap());
    let body = fs::read_to_string(&path).unwrap();
    assert!(body.starts_with("FIRST READABLE "));
    assert!(!body.contains("LATER SECRET") && !body.contains('\u{fffd}'));
    let reassembled = doc
        .source_extents
        .iter()
        .flat_map(|e| {
            image[e.source_byte_offset..e.source_byte_offset + e.bytes]
                .iter()
                .copied()
        })
        .collect::<Vec<_>>();
    assert_eq!(hash(&reassembled), doc.readable_prefix_sha256);
    assert_eq!(
        text::decode(&reassembled, &doc.prefix_boundary_reason).text,
        body
    );
    assert!(run().unwrap().rtf_salvage.unwrap().reused);
    let response = crate::cli::run(
        &[
            "recovery".into(),
            "documents".into(),
            "1".into(),
            "--json".into(),
        ],
        p.root(),
    )
    .unwrap();
    assert_eq!(response.exit_code, 3);
    let value: serde_json::Value = serde_json::from_str(&response.output).unwrap();
    assert_eq!(value["native_recovery"]["files"], 0);
    assert_eq!(value["rtf_salvage"]["documents_with_text"], 1);
    let human = crate::cli::run(
        &["recovery".into(), "documents".into(), "1".into()],
        p.root(),
    )
    .unwrap();
    assert!(human.output.contains("RTF prefix text engine v1"));
    assert!(human.output.contains("Open readable edition:"));
    let manifest = crate::manifest::build_manifest(
        &crate::manifest::ManifestRequest {
            extracted_root: p.extracted_dir(),
            images_directory: p.images_dir(),
            reports_directory: p.reports_dir(),
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(manifest.file_count, 0);
    let plan = crate::conversion::build_conversion_plan(
        &crate::conversion::ConversionPlanningRequest {
            extracted_root: p.extracted_dir(),
            converted_root: p.converted_dir(),
            reports_directory: p.reports_dir(),
        },
        &|_| {},
    )
    .unwrap();
    assert!(plan.jobs.is_empty());
    let destination = p.root().with_extension("rtf-package");
    fs::create_dir(&destination).unwrap();
    let pending = p.recovery_dir().join("001/.tmp-rtf-text-test");
    fs::create_dir(&pending).unwrap();
    fs::write(pending.join("unpublished.txt"), b"not committed evidence").unwrap();
    let packed = crate::package::build_package(
        &crate::package::PackageRequest {
            project_root: p.root().to_path_buf(),
            destination,
            project_name: "rtf-text-fixture".into(),
        },
        &|_| {},
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(packed.zip_path).unwrap()).unwrap();
    for name in [
        doc.text_path.as_ref().unwrap(),
        doc.edition_path.as_ref().unwrap(),
        &"rtf-text.json".to_string(),
    ] {
        assert!(
            archive
                .by_name(&format!(
                    "Recovery/001/{}/{name}",
                    text.output_directory.file_name().unwrap().to_string_lossy()
                ))
                .is_ok()
        );
    }
    assert!(
        archive
            .by_name("Recovery/001/.tmp-rtf-text-test/unpublished.txt")
            .is_err()
    );
    drop(archive);
    let html_path = &text.readable_edition_paths[0];
    let html = fs::read_to_string(html_path).unwrap();
    assert!(
        html.contains("STOPPED / REMAINDER NOT INTERPRETED") && html.contains("default-src 'none'")
    );
    assert!(!html.contains("<script>"));
    fs::write(html_path, b"operator changed edition").unwrap();
    assert!(run().unwrap_err().contains("preserved"));
    assert_eq!(fs::read(html_path).unwrap(), b"operator changed edition");
    assert_eq!(fs::read(p.images_dir().join("001.img")).unwrap(), image);
}

#[test]
fn html_escapes_all_source_names_text_and_stop_reasons() {
    let (p, attempt, _) = fixture();
    let result = crate::fat12_recovery::recover_attempt(
        &p.images_dir(),
        &p.extracted_dir(),
        &p.recovery_dir(),
        1,
        &attempt,
        &|_| {},
    )
    .unwrap();
    let report: Report =
        serde_json::from_slice(&fs::read(result.rtf_salvage.unwrap().report_path).unwrap())
            .unwrap();
    let html = edition(
        &report.documents[0],
        "<img src=x onerror=evil()> &\0",
        "<script>",
        &"a".repeat(64),
    );
    assert!(!html.contains("<img ") && !html.contains("<script>"));
    assert!(html.contains("&lt;img") && html.contains("\\u{0000}"));
}

#[test]
fn prefix_mapping_clips_eof_rejects_bad_ranges_and_never_bridges_holes() {
    let (p, _, image) = fixture();
    let analysis = fat12::analyze(&image, &[134]).unwrap();
    let mut parent = analysis.unrecovered_files[0].record.clone();
    let first = parent.data_lbas[0];
    let empty = BTreeSet::new();
    parent.bytes = 519;
    let (bytes, extents, _) = prefix(&image, &parent, &empty).unwrap();
    assert_eq!(bytes.len(), 519);
    assert_eq!(extents.iter().map(|e| e.bytes).sum::<usize>(), 519);
    parent.bytes = 1024;
    parent.data_lbas = vec![first, first];
    assert!(
        prefix(&image, &parent, &empty)
            .unwrap_err()
            .contains("Repeated")
    );
    parent.data_lbas = vec![u64::MAX];
    assert!(prefix(&image, &parent, &empty).is_err());
    parent.data_lbas = vec![first];
    let (bytes, _, reason) = prefix(&image, &parent, &empty).unwrap();
    assert_eq!(bytes.len(), 512);
    assert!(reason.contains("unmapped"));
    let (bytes, _, reason) = prefix(&image, &parent, &BTreeSet::from([first])).unwrap();
    assert!(bytes.is_empty() && reason.contains("unreadable"));
    parent.bytes = fat12::MAX_IMAGE_BYTES + 1;
    assert!(prefix(&image, &parent, &empty).is_err());
    let mut oversized = analysis.clone();
    oversized.unrecovered_files[0].record = parent.clone();
    let native = RecoveryReport {
        schema_version: 1,
        native_engine_version: 5,
        method: "test".into(),
        source_image: "001.img".into(),
        source_sha256: hash(&image),
        disk: 1,
        attempt: 1,
        analysis: Some(oversized),
        carving: None,
        filesystem_error: None,
        deleted: None,
        warning: "synthetic oversized incomplete parent".into(),
    };
    let refused_dir = p.recovery_dir().join("refusal");
    fs::create_dir(&refused_dir).unwrap();
    let result = preserve(
        &image,
        &p.images_dir().join("001.img"),
        &refused_dir.canonicalize().unwrap(),
        &native,
    )
    .unwrap()
    .unwrap();
    assert!(result.limits_reached && result.documents_with_text == 0);
    let report: Report = serde_json::from_slice(&fs::read(result.report_path).unwrap()).unwrap();
    assert!(report.documents[0].stop_reason.contains("size ceiling"));
    parent.bytes = MAX_PREFIX + 512;
    parent.data_lbas = (0..513).collect();
    let (bytes, _, reason) = prefix(&image, &parent, &empty).unwrap();
    assert_eq!(bytes.len(), MAX_PREFIX);
    assert!(reason.contains("ceiling"));
}

#[test]
#[ignore = "requires FLUXVAULT_RTF_TEST_PROJECT; saved-image-only prefix replay, no physical media"]
fn saved_damaged_rtf_prefixes_replay_acquisition_bytes_and_published_editions() {
    let project = crate::project::ProjectState::open_without_session(PathBuf::from(
        std::env::var("FLUXVAULT_RTF_TEST_PROJECT").unwrap(),
    ))
    .unwrap();
    let mut with_text = 0;
    for (label, attempt) in [(22, 1), (22, 2), (128, 2)] {
        let folder = project
            .extracted_dir()
            .join(format!("{label:03}/attempt_{attempt:03}_native_v5"));
        let native = crate::extraction::verified_native_report(&folder)
            .unwrap()
            .unwrap();
        let image = fs::read(project.images_dir().join(&native.source_image)).unwrap();
        assert_eq!(hash(&image), native.source_sha256);
        let a = native.analysis.as_ref().unwrap();
        let bad = a.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
        for parent in &a.unrecovered_files {
            if !parent.record.path.to_lowercase().ends_with(".rtf")
                || !fat12::partial_file_safe(parent, a)
            {
                continue;
            }
            let (bytes, extents, boundary) = prefix(&image, &parent.record, &bad).unwrap();
            let decoded = text::decode(&bytes, &boundary);
            for e in &extents {
                assert!(
                    (e.source_byte_offset..e.source_byte_offset + e.bytes)
                        .all(|at| !bad.contains(&((at / 512) as u64)))
                );
            }
            if !decoded.text.is_empty() {
                with_text += 1;
            }
            eprintln!(
                "Saved {label:03}: {} positions, {} readable prefix bytes, decoded through {}, stop: {}",
                decoded.text.encode_utf16().count(),
                bytes.len(),
                decoded.through,
                decoded.stop
            );
            let published = project.recovery_dir().join(format!(
                "{label:03}/attempt_{attempt:03}_rtf_text_v1/rtf-text.json"
            ));
            if published.exists() {
                let report: Report =
                    serde_json::from_slice(&fs::read(&published).unwrap()).unwrap();
                let d = report
                    .documents
                    .iter()
                    .find(|d| d.parent.record.path == parent.record.path)
                    .unwrap();
                assert_eq!(d.readable_prefix_sha256, hash(&bytes));
                assert_eq!(
                    serde_json::to_value(&d.source_extents).unwrap(),
                    serde_json::to_value(&extents).unwrap()
                );
                assert_eq!(d.decoded_through_file_byte, decoded.through);
                assert_eq!(d.stop_reason, decoded.stop);
                assert_eq!(d.text_sha256, hash(decoded.text.as_bytes()));
                if let Some(path) = &d.text_path {
                    assert_eq!(
                        fs::read(published.parent().unwrap().join(path)).unwrap(),
                        decoded.text.as_bytes()
                    );
                }
                if let Some(path) = &d.edition_path {
                    let html = fs::read(published.parent().unwrap().join(path)).unwrap();
                    assert_eq!(hash(&html), d.edition_sha256);
                    assert_eq!(
                        html,
                        edition(
                            d,
                            &decoded.text,
                            &report.source_image,
                            &report.source_sha256
                        )
                        .as_bytes()
                    );
                }
            }
        }
    }
    assert!(with_text > 0, "No saved RTF body text recovered");
}
