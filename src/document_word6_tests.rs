use super::*;

fn word6(complex: bool, body: &[u8]) -> SparseStream {
    let mut bytes = vec![0; 8192];
    for (at, value) in [
        (0, 0xa5dcu16),
        (2, 0x65),
        (6, 0x040e),
        (10, if complex { 4 } else { 0 }),
        (12, 0x65),
    ] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (24, 768u32),
        (28, 768 + body.len() as u32),
        (32, 8192),
        (52, body.len() as u32),
    ] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[768..768 + body.len()].copy_from_slice(body);
    if complex {
        let split = body.len() / 2;
        bytes[2048..2048 + body.len() - split].copy_from_slice(&body[split..]);
        bytes[352..356].copy_from_slice(&4096u32.to_le_bytes());
        bytes[356..360].copy_from_slice(&33u32.to_le_bytes());
        bytes[4096] = 2;
        bytes[4097..4101].copy_from_slice(&28u32.to_le_bytes());
        for (i, cp) in [0, split as u32, body.len() as u32].iter().enumerate() {
            bytes[4101 + i * 4..4105 + i * 4].copy_from_slice(&cp.to_le_bytes());
        }
        bytes[4115..4119].copy_from_slice(&768u32.to_le_bytes());
        bytes[4123..4127].copy_from_slice(&2048u32.to_le_bytes());
    }
    SparseStream {
        sources: (0..bytes.len()).map(|i| Some(10000 + i)).collect(),
        bytes,
    }
}
fn decode(word: &SparseStream) -> Result<Document, String> {
    let (_, parent, _) = fixture(false, true);
    legacy::decode(&parent, word, BTreeSet::from([0]))
}

#[test]
fn word6_simple_and_fragmented_piece_tables_decode_hungarian_without_a_table_stream() {
    for complex in [false, true] {
        let word = word6(complex, b"\xc1rv\xedzt\xfbr\xf5 \r");
        let doc = decode(&word).unwrap();
        assert_eq!(
            doc.segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            "Árvíztűrő \r"
        );
        assert!(doc.missing_text.is_empty());
        let evidence = doc.legacy_text.as_ref().unwrap();
        assert_eq!(evidence.code_page, 1250);
        assert_eq!(evidence.encoding_basis, "FIB_Hungarian_locale_fallback");
        assert!(!evidence.font_overrides_interpreted);
        assert_eq!(doc.segments.iter().map(|s| s.cp_count).sum::<usize>(), 11);
        assert!(doc.metadata_lbas.contains(&0));
        assert!(!doc.repaired_original && !doc.customer_delivery_certified);
        for segment in &doc.segments {
            assert_eq!(segment.sha256, hash(segment.text.as_bytes()));
            assert_eq!(
                segment
                    .source_extents
                    .iter()
                    .map(|e| e.bytes)
                    .sum::<usize>(),
                segment.encoded_bytes
            );
        }
    }
}

#[test]
fn word6_charset_overrides_locale_but_unknown_symbol_dbcs_and_encrypted_inputs_refuse() {
    let mut explicit = word6(false, b"\xf5");
    explicit.bytes[20..22].copy_from_slice(&238u16.to_le_bytes());
    explicit.bytes[6..8].copy_from_slice(&0x0411u16.to_le_bytes());
    assert_eq!(decode(&explicit).unwrap().segments[0].text, "ő");
    let mut english = word6(false, b"\xe9");
    english.bytes[6..8].copy_from_slice(&0x0409u16.to_le_bytes());
    assert_eq!(decode(&english).unwrap().segments[0].text, "é");
    for (at, value) in [
        (20, 2u16),
        (20, 128),
        (20, 256),
        (6, 0x0411),
        (6, 0),
        (10, 0x100),
        (10, 0x1000),
        (18, 1),
    ] {
        let mut word = word6(false, b"visible source");
        word.bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        assert!(decode(&word).is_err(), "offset {at}, value {value}");
    }
}

#[test]
fn word6_missing_payload_and_undefined_control_bytes_are_explicit_position_gaps() {
    let mut word = word6(true, b"AB\x81CD\x13EFGH\r");
    word.sources[2049] = None;
    let doc = decode(&word).unwrap();
    assert_eq!(doc.main_characters, Some(11));
    assert_eq!(
        doc.missing_text.iter().map(|g| g.cp_count).sum::<usize>(),
        3
    );
    assert_eq!(doc.segments.iter().map(|s| s.cp_count).sum::<usize>(), 8);
    assert!(
        doc.missing_text
            .iter()
            .any(|g| g.reason.contains("acquisition"))
    );
    for s in &doc.segments {
        assert!(
            word.sources[s.word_stream_offset..s.word_stream_offset + s.encoded_bytes]
                .iter()
                .all(Option::is_some)
        );
    }
    word.sources[4105] = None;
    assert!(
        decode(&word).is_err(),
        "missing CLX metadata must not become text"
    );
}

#[test]
fn word6_strict_compound_dispatch_needs_no_word97_0table_or_1table() {
    for complex in [false, true] {
        let word = word6(complex, b"Word6 readable body\r");
        let mut c =
            cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new()))
                .unwrap();
        c.create_stream("/WordDocument")
            .unwrap()
            .write_all(&word.bytes)
            .unwrap();
        let bytes = c.into_inner().into_inner();
        let (_, mut parent, _) = fixture(false, true);
        parent.bytes = bytes.len();
        parent.data_lbas = (0..bytes.len().div_ceil(512))
            .map(|i| (2 + i * 2) as u64)
            .collect();
        let mut image = vec![0; (parent.data_lbas.last().unwrap() + 1) as usize * 512];
        for (i, b) in bytes.iter().enumerate() {
            image[parent.data_lbas[i / 512] as usize * 512 + i % 512] = *b;
        }
        let doc = extract(&image, &parent, &BTreeSet::new()).unwrap();
        assert!(doc.legacy_text.is_some());
        assert_eq!(
            doc.segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            "Word6 readable body\r"
        );
        assert!(doc.cfb_recovery.is_none());
        assert!(
            extract(&image, &parent, &BTreeSet::from([0])).is_err(),
            "unavailable parent metadata cannot authorize a text map"
        );
    }
}

#[test]
fn word6_hungarian_and_english_html_disclose_locale_interpretation_without_translating_text() {
    let doc = decode(&word6(false, b"\xc1rv\xedzt\xfbr\xf5 \r")).unwrap();
    let hu = readable_html::render_in_language(
        &doc,
        "133.img",
        "hash",
        crate::final_report::Language::Hungarian,
    )
    .unwrap();
    let en = readable_html::render_in_language(
        &doc,
        "133.img",
        "hash",
        crate::final_report::Language::English,
    )
    .unwrap();
    for html in [&hu, &en] {
        assert!(html.contains("Árvíztűrő"));
        assert!(html.contains("1250"));
    }
    assert!(hu.contains("Régi Word6 szöveges értelmezés"));
    assert!(hu.contains("betűtípusonkénti"));
    assert!(!hu.contains("Legacy Windows Word6 text-only"));
    assert!(en.contains("font-run map"));
}

#[test]
fn word6_malformed_piece_tables_and_header_holes_never_produce_a_decode() {
    for (at, value) in [
        (0, 0u32),
        (32, 8193),
        (52, 200001),
        (352, u32::MAX),
        (356, 65537),
        (4097, 32),
        (4105, 0),
        (4115, 4096),
    ] {
        let mut word = word6(true, b"a readable test string\r");
        word.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert!(decode(&word).is_err(), "offset {at}");
    }
    let mut word = word6(false, b"readable test string\r");
    word.sources[6] = None;
    assert!(decode(&word).is_err());
}

#[test]
fn word6_native_recovery_publishes_readable_and_editable_copies_with_exact_gaps_and_reuse() {
    native_word6_case(true);
}

#[test]
fn word6_complete_legacy_files_also_get_separate_text_editions_without_replacing_originals() {
    native_word6_case(false);
}

fn native_word6_case(damaged: bool) {
    let word = word6(false, &vec![b'x'; 900]);
    let mut c =
        cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new())).unwrap();
    c.create_stream("/WordDocument")
        .unwrap()
        .write_all(&word.bytes)
        .unwrap();
    let payload = c.into_inner().into_inner();
    let (_, mut parent, _) = fixture(false, true);
    parent.bytes = payload.len();
    parent.data_lbas = (0..payload.len().div_ceil(512)).map(|i| i as u64).collect();
    let logical = word_mapping(&payload, &parent).sources[1024].unwrap() / 512;
    let mut image = fat12::tests::image();
    let clusters = (0..payload.len().div_ceil(512))
        .map(|i| 100 + i as u16 * 2)
        .collect::<Vec<_>>();
    fat12::tests::entry(
        &mut image,
        19 * 512,
        b"OLDWORD DOC",
        clusters[0],
        payload.len() as u32,
        false,
    );
    for (i, cluster) in clusters.iter().enumerate() {
        for copy in 0..2 {
            fat12::tests::set_fat(
                &mut image,
                copy,
                *cluster,
                clusters.get(i + 1).copied().unwrap_or(0xfff),
            );
        }
        let at = (33 + *cluster as usize - 2) * 512;
        let n = (payload.len() - i * 512).min(512);
        image[at..at + n].copy_from_slice(&payload[i * 512..i * 512 + n]);
    }
    let bad = 33 + clusters[logical] as usize - 2;
    if damaged {
        image[bad * 512..(bad + 1) * 512].fill(0);
    }
    let root = std::env::temp_dir().join(format!(
        "fv-word6-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let project =
        crate::project::ProjectState::create_without_session(root.join("project")).unwrap();
    let source = project.images_dir().join("001.img");
    fs::write(&source, &image).unwrap();
    let bad_line = if damaged {
        format!("BAD_SECTOR | lba={bad}\n")
    } else {
        String::new()
    };
    fs::write(project.logs_dir().join("001.log"),format!("BEGIN | disk=1 | attempt=1\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\n{bad_line}END | status={} | bytes=1474560 | sha256={}\n",if damaged {"PARTIAL"} else {"OK"},hash(&image))).unwrap();
    let attempt = crate::imaging::load_attempts_for_disk(&project.images_dir(), 1)
        .unwrap()
        .remove(0);
    let run = || {
        crate::fat12_recovery::recover_attempt(
            &project.images_dir(),
            &project.extracted_dir(),
            &project.recovery_dir(),
            1,
            &attempt,
            &|_| {},
        )
        .unwrap()
    };
    let result = run();
    assert_eq!(result.files, usize::from(!damaged));
    let text = result.document_salvage.unwrap();
    assert_eq!(
        text.recovered_character_positions,
        if damaged { 388 } else { 900 }
    );
    assert_eq!(
        text.missing_character_positions,
        if damaged { 512 } else { 0 }
    );
    assert_eq!(text.readable_editions, 1);
    let report: Report = serde_json::from_slice(&fs::read(&text.report_path).unwrap()).unwrap();
    assert!(report.documents[0].legacy_text.is_some());
    assert_eq!(result.editable_editions.len(), 1);
    let editable = &result.editable_editions[0].paths[0];
    let edition_before = fs::read(editable).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(&edition_before)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.contains("Word6 szöveges értelmezés"));
    if damaged {
        assert!(xml.contains("HIÁNY"));
    }
    assert!(run().document_salvage.unwrap().reused);
    assert_eq!(fs::read(editable).unwrap(), edition_before);
    assert_eq!(fs::read(source).unwrap(), image);
    fs::remove_dir_all(root).unwrap();
}
