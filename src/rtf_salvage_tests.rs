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
    assert!(d.stop.starts_with("Readable root"));
    assert_eq!(d.gaps.len(), 1);
    assert_eq!(d.gaps[0].encoded_byte, b'A');
    let d = text::decode(
        br"{\rtf1\deff0{\fonttbl{\f0\fcharset2\cpg1250 Explicit;}}\'f5}",
        "boundary",
    );
    assert_eq!(d.text, "ő");
}

#[test]
fn unsupported_symbol_glyphs_are_explicit_and_known_font_state_continues() {
    let input = br"{\rtf1\ansicpg1250\deff0{\fonttbl{\f0\fcharset238 Known;}{\f10\fcharset2 Wingdings;}}A{\f10\'a1\{\\}\'f5Z}";
    let d = text::decode(input, "boundary");
    assert_eq!(d.text, "AőZ");
    assert_eq!(d.gaps.len(), 3);
    for (g, byte) in d.gaps.iter().zip([0xa1, b'{', b'\\']) {
        assert_eq!(g.font_id, 10);
        assert_eq!(g.output_utf8_offset, 1);
        assert_eq!(g.encoded_byte, byte);
        assert!(g.input_end > g.input_start && g.input_end <= d.through);
    }
    assert_eq!(&input[d.gaps[0].input_start..d.gaps[0].input_end], br"\'a1");
    assert!(d.stop.starts_with("Readable root"));
    // Unknown encodings, missing font definitions and explicit unsupported
    // code pages still STOP: they may be multibyte or hide structural meaning.
    for definition in [
        br"\fcharset128".as_slice(),
        br"\fcharset2\cpg932",
        br"\fcharset99",
    ] {
        let input = [
            br"{\rtf1{\fonttbl{\f0 Known;}{\f1".as_slice(),
            definition,
            br" Unknown;}}A\f1\'a1\f0 BAD}",
        ]
        .concat();
        let d = text::decode(&input, "boundary");
        assert_eq!(d.text, "A");
        assert!(d.gaps.is_empty() && d.stop.contains("code page"));
    }
    let d = text::decode(br"{\rtf1 A\f99 B}", "boundary");
    assert_eq!(d.text, "A");
    assert!(d.gaps.is_empty());
    let hidden = text::decode(
        br"{\rtf1{\fonttbl{\f1\fcharset2 Symbol;}}A{\f1\v BAD}{\f1\deleted BAD}Z}",
        "boundary",
    );
    assert_eq!(hidden.text, "AZ");
    assert!(hidden.gaps.is_empty());
}

#[test]
fn symbol_gaps_are_bounded_and_never_cross_an_unreadable_boundary() {
    let prefix = br"{\rtf1{\fonttbl{\f0 Known;}{\f1\fcharset2 Symbol;}}A{\f1\'a1}";
    let d = text::decode(prefix, "First acquisition-unreadable byte");
    assert_eq!(d.text, "A");
    assert_eq!(d.gaps.len(), 1);
    assert_eq!(d.through, prefix.len());
    assert!(d.stop.contains("acquisition-unreadable"));
    let prefix = [
        br"{\rtf1{\fonttbl{\f1\fcharset2 Symbol;}}\f1 ".as_slice(),
        &vec![b'A'; 4097],
    ]
    .concat();
    let d = text::decode(&prefix, "boundary");
    assert!(d.text.is_empty());
    assert_eq!(d.gaps.len(), 4096);
    assert!(d.stop.contains("gap ceiling"));
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
    for props in [br"\fcharset0\fcharset2".as_slice(), br"\cpg1250\cpg932"] {
        let input = [
            br"{\rtf1{\fonttbl{\f0".as_slice(),
            props,
            br" Unknown;}}\f0 BAD}",
        ]
        .concat();
        let d = text::decode(&input, "boundary");
        assert!(d.text.is_empty() && d.gaps.is_empty() && d.stop.contains("Duplicate RTF font"));
    }
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
    fixture_with_prefix(prefix)
}

fn fixture_with_prefix(
    prefix: Vec<u8>,
) -> (
    crate::project::ProjectState,
    crate::imaging::AttemptSummary,
    Vec<u8>,
) {
    fixture_with_name(prefix, b"BROKEN  RTF")
}

fn fixture_with_name(
    prefix: Vec<u8>,
    short_name: &[u8; 11],
) -> (
    crate::project::ProjectState,
    crate::imaging::AttemptSummary,
    Vec<u8>,
) {
    let mut image = fat12::tests::image();
    fat12::tests::entry(
        &mut image,
        19 * 512,
        short_name,
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
    assert_eq!(result.editable_editions.len(), 1);
    assert_eq!(result.editable_editions[0].paths.len(), 1);
    let editable = result.editable_editions[0].paths[0].clone();
    let editable_before = fs::read(&editable).unwrap();
    assert!(run().unwrap().editable_editions[0].reused);
    assert_eq!(fs::read(&editable).unwrap(), editable_before);
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
    assert!(
        human
            .output
            .contains(&format!("RTF prefix text engine v{ENGINE_VERSION}"))
    );
    assert!(human.output.contains("Open readable edition:"));
    assert!(human.output.contains("Open editable recovery edition:"));
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
        html.contains("MEGÁLLT / A FENNMARADÓ RÉSZ NEM ÉRTELMEZETT")
            && html.contains("default-src 'none'")
    );
    assert!(!html.contains("<script>"));
    fs::write(html_path, b"operator changed edition").unwrap();
    assert!(run().unwrap_err().contains("preserved"));
    assert_eq!(fs::read(html_path).unwrap(), b"operator changed edition");
    assert_eq!(fs::read(p.images_dir().join("001.img")).unwrap(), image);
}

#[test]
fn changed_editable_docx_is_preserved_and_refuses_reuse() {
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
    let path = &result.editable_editions[0].paths[0];
    fs::write(path, b"operator edited the DOCX").unwrap();
    assert!(run().unwrap_err().contains("preserved"));
    assert_eq!(fs::read(path).unwrap(), b"operator edited the DOCX");
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
        ENGINE_VERSION,
    );
    assert!(!html.contains("<img ") && !html.contains("<script>"));
    assert!(html.contains("&lt;img") && html.contains("\\u{0000}"));
}

#[test]
fn glyph_gaps_publish_separate_hash_bound_text_segments_and_tamper_refuses_reuse() {
    let prefix = [br"{\rtf1\ansicpg1250\deff0{\fonttbl{\f0\fcharset238 Known;}{\f1\fcharset2 Wingdings;}}FIRST{\f1\'a1}SECOND ".as_slice(), &vec![b'x';1200]].concat();
    let (p, attempt, image) = fixture_with_prefix(prefix);
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
    let salvage = result.rtf_salvage.unwrap();
    assert_eq!(salvage.unsupported_font_glyphs, 1);
    assert_eq!(salvage.text_segments, 2);
    assert_eq!(salvage.documents_with_text, 1);
    let report: Report = serde_json::from_slice(&fs::read(&salvage.report_path).unwrap()).unwrap();
    let d = &report.documents[0];
    assert!(d.text_path.is_none());
    assert_eq!(d.unsupported_font_gaps[0].encoded_byte, 0xa1);
    let mut joined_for_test = Vec::new();
    for s in &d.text_segments {
        let bytes = fs::read(salvage.output_directory.join(&s.path)).unwrap();
        assert_eq!(hash(&bytes), s.sha256);
        assert_eq!(bytes.len(), s.bytes);
        assert_eq!(s.decoded_utf8_end - s.decoded_utf8_start, s.bytes);
        joined_for_test.extend(bytes);
    }
    assert!(joined_for_test.starts_with(b"FIRSTSECOND "));
    assert_eq!(hash(&joined_for_test), d.text_sha256);
    assert_eq!(
        fs::read_to_string(salvage.output_directory.join(&d.text_segments[0].path)).unwrap(),
        "FIRST"
    );
    let html = fs::read_to_string(&salvage.readable_edition_paths[0]).unwrap();
    assert!(html.contains("NEM ÉRTELMEZETT SZIMBÓLUMKARAKTER"));
    assert!(!html.contains("FIRSTSECOND"));
    assert!(run().unwrap().rtf_salvage.unwrap().reused);
    let destination = p.root().with_extension("rtf-segments-package");
    fs::create_dir(&destination).unwrap();
    let packed = crate::package::build_package(
        &crate::package::PackageRequest {
            project_root: p.root().to_path_buf(),
            destination,
            project_name: "rtf-segments-fixture".into(),
        },
        &|_| {},
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(packed.zip_path).unwrap()).unwrap();
    for name in d
        .text_segments
        .iter()
        .map(|s| s.path.as_str())
        .chain([d.edition_path.as_ref().unwrap().as_str(), "rtf-text.json"])
    {
        assert!(
            archive
                .by_name(&format!(
                    "Recovery/001/{}/{name}",
                    salvage
                        .output_directory
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                ))
                .is_ok()
        );
    }
    drop(archive);
    let legacy = salvage
        .output_directory
        .with_file_name("attempt_001_rtf_text_v1");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("rtf-text.json"), b"old v1 report").unwrap();
    assert!(run().unwrap().rtf_salvage.unwrap().reused);
    assert_eq!(
        fs::read(legacy.join("rtf-text.json")).unwrap(),
        b"old v1 report"
    );
    let segment = salvage.output_directory.join(&d.text_segments[1].path);
    fs::write(&segment, b"operator changed segment").unwrap();
    assert!(run().unwrap_err().contains("preserved"));
    assert_eq!(fs::read(segment).unwrap(), b"operator changed segment");
    assert_eq!(fs::read(p.images_dir().join("001.img")).unwrap(), image);
}

#[test]
fn readable_rtf_signature_selects_decoder_despite_doc_extension() {
    let prefix = [br"{\rtf1 Misnamed source ".as_slice(), &vec![b'x'; 1200]].concat();
    let (p, attempt, image) = fixture_with_name(prefix, b"BROKEN  DOC");
    let result = crate::fat12_recovery::recover_attempt(
        &p.images_dir(),
        &p.extracted_dir(),
        &p.recovery_dir(),
        1,
        &attempt,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.files, 0);
    assert_eq!(result.document_salvage.unwrap().documents_with_text, 0);
    let salvage = result.rtf_salvage.unwrap();
    assert_eq!(salvage.documents_with_text, 1);
    let report: Report = serde_json::from_slice(&fs::read(&salvage.report_path).unwrap()).unwrap();
    assert_eq!(report.documents[0].parent.record.path, "BROKEN.DOC");
    assert!(report.documents[0].content_detection.is_some());
    assert!(
        fs::read_to_string(&salvage.readable_edition_paths[0])
            .unwrap()
            .contains("megtévesztő fájlkiterjesztés")
    );
    assert_eq!(fs::read(p.images_dir().join("001.img")).unwrap(), image);
}

#[test]
fn cli_language_switch_keeps_rtf_source_and_both_immutable_editions() {
    let prefix = [br"{\rtf1{\fonttbl{\f0 Known;}{\f1\fcharset2 Symbol;}}STOPPED / REMAINDER NOT INTERPRETED __FV_LOCALE_SLOT_0__ <script>{\f1\'a1}TAIL ".as_slice(), &vec![b'x'; 1200]].concat();
    let (p, _, image) = fixture_with_prefix(prefix);
    let metadata = fs::read(p.root().join("project.json")).unwrap();
    let missing = crate::cli::run(
        &[
            "recovery".into(),
            "documents".into(),
            "999".into(),
            "--language".into(),
            "en".into(),
        ],
        p.root(),
    )
    .unwrap_err();
    assert!(missing.contains("saved"));
    assert_eq!(fs::read(p.root().join("project.json")).unwrap(), metadata);
    let run = |language: &str| {
        let response = crate::cli::run(
            &[
                "recovery".into(),
                "documents".into(),
                "1".into(),
                "--language".into(),
                language.into(),
                "--json".into(),
            ],
            p.root(),
        )
        .unwrap();
        assert_eq!(response.exit_code, 3);
        let result: serde_json::Value = serde_json::from_str(&response.output).unwrap();
        assert_eq!(result["physical_media_access"], false);
        assert_eq!(result["native_recovery"]["files"], 0);
        let directory = PathBuf::from(result["rtf_salvage"]["output_directory"].as_str().unwrap());
        let report: Report =
            serde_json::from_slice(&fs::read(directory.join("rtf-text.json")).unwrap()).unwrap();
        assert_eq!(report.customer_language, language);
        (directory, report, result)
    };
    let (hu_dir, hu, _) = run("hu");
    let hu_doc = &hu.documents[0];
    let hu_html = fs::read(hu_dir.join(hu_doc.edition_path.as_ref().unwrap())).unwrap();
    let (en_dir, en, _) = run("en");
    assert_ne!(hu_dir, en_dir);
    let en_html =
        fs::read_to_string(en_dir.join(en.documents[0].edition_path.as_ref().unwrap())).unwrap();
    assert!(
        en_html.contains("<html lang=\"en\">")
            && en_html.contains("UNINTERPRETED SYMBOL-FONT GLYPH")
    );
    let hu_text = String::from_utf8(hu_html.clone()).unwrap();
    assert!(
        hu_text.contains("<html lang=\"hu\">")
            && hu_text.contains("NEM ÉRTELMEZETT SZIMBÓLUMKARAKTER")
    );
    for html in [&hu_text, &en_html] {
        assert!(
            html.contains(
                "STOPPED / REMAINDER NOT INTERPRETED __FV_LOCALE_SLOT_0__ &lt;script&gt;"
            )
        );
        assert!(!html.contains("<script>"));
    }
    let mut hu_proof = serde_json::to_value(&hu.documents).unwrap();
    let mut en_proof = serde_json::to_value(&en.documents).unwrap();
    for docs in [&mut hu_proof, &mut en_proof] {
        for doc in docs.as_array_mut().unwrap() {
            doc.as_object_mut().unwrap().remove("edition_sha256");
            doc.as_object_mut().unwrap().remove("edition_path");
        }
    }
    assert_eq!(hu_proof, en_proof);
    for segment in &hu_doc.text_segments {
        assert_eq!(
            fs::read(hu_dir.join(&segment.path)).unwrap(),
            fs::read(en_dir.join(&segment.path)).unwrap()
        );
    }
    assert!(run("hu").2["rtf_salvage"]["reused"].as_bool().unwrap());
    assert_eq!(
        fs::read(hu_dir.join(hu_doc.edition_path.as_ref().unwrap())).unwrap(),
        hu_html
    );
    assert_eq!(fs::read(p.images_dir().join("001.img")).unwrap(), image);
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
            if label == 22 && attempt == 2 {
                assert!(decoded.text.encode_utf16().count() >= 5318);
                assert!(!decoded.gaps.is_empty());
            }
            for (engine, language) in [
                (1, None),
                (2, None),
                (3, Some(crate::final_report::Language::Hungarian)),
                (3, Some(crate::final_report::Language::English)),
            ] {
                let suffix = language.map_or(String::new(), |l| format!("_{}", l.code()));
                let published = project.recovery_dir().join(format!(
                    "{label:03}/attempt_{attempt:03}_rtf_text_v{engine}{suffix}/rtf-text.json"
                ));
                if published.exists() {
                    let decoded = if engine == 1 {
                        text::decode_v1(&bytes, &boundary)
                    } else {
                        text::decode(&bytes, &boundary)
                    };
                    let report: Report =
                        serde_json::from_slice(&fs::read(&published).unwrap()).unwrap();
                    if let Some(language) = language {
                        assert_eq!(report.customer_language, language.code());
                    }
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
                    assert_eq!(d.unsupported_font_gaps, decoded.gaps);
                    for g in &d.unsupported_font_gaps {
                        assert!(g.input_end <= decoded.through && g.input_start < g.input_end);
                        assert!(decoded.text.is_char_boundary(g.output_utf8_offset));
                    }
                    for s in &d.text_segments {
                        let text =
                            &decoded.text.as_bytes()[s.decoded_utf8_start..s.decoded_utf8_end];
                        assert_eq!(
                            fs::read(published.parent().unwrap().join(&s.path)).unwrap(),
                            text
                        );
                        assert_eq!(hash(text), s.sha256);
                        assert!(
                            !decoded
                                .gaps
                                .iter()
                                .any(|g| g.output_utf8_offset > s.decoded_utf8_start
                                    && g.output_utf8_offset < s.decoded_utf8_end)
                        );
                    }
                    if let Some(path) = &d.text_path {
                        assert_eq!(
                            fs::read(published.parent().unwrap().join(path)).unwrap(),
                            decoded.text.as_bytes()
                        );
                    }
                    if let Some(path) = &d.edition_path {
                        let html = fs::read(published.parent().unwrap().join(path)).unwrap();
                        assert_eq!(hash(&html), d.edition_sha256);
                        let replay = match language {
                            Some(language) => edition_in_language(
                                d,
                                &decoded.text,
                                &report.source_image,
                                &report.source_sha256,
                                language,
                            )
                            .unwrap(),
                            None => edition(
                                d,
                                &decoded.text,
                                &report.source_image,
                                &report.source_sha256,
                                engine,
                            ),
                        };
                        assert_eq!(html, replay.as_bytes());
                    }
                }
            }
        }
    }
    assert!(with_text > 0, "No saved RTF body text recovered");
}
