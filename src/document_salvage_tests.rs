use super::*;
use std::io::Cursor;
#[path = "document_cfb_tests.rs"]
mod cfb_tests;

fn fixture(compressed: bool, mini: bool) -> (Vec<u8>, fat12::FileRecord, Vec<usize>) {
    let mut word = vec![0; 8192];
    word[..2].copy_from_slice(&0xa5ecu16.to_le_bytes());
    word[2..4].copy_from_slice(&0xc1u16.to_le_bytes());
    word[10..12].copy_from_slice(&0x0200u16.to_le_bytes());
    word[32..34].copy_from_slice(&14u16.to_le_bytes());
    word[62..64].copy_from_slice(&22u16.to_le_bytes());
    word[64..68].copy_from_slice(&8192u32.to_le_bytes());
    word[76..80].copy_from_slice(&900u32.to_le_bytes());
    word[152..154].copy_from_slice(&0x5du16.to_le_bytes());
    word[418..422].copy_from_slice(&0u32.to_le_bytes());
    word[422..426].copy_from_slice(&21u32.to_le_bytes());
    if compressed {
        word[1024..1924].fill(b'x');
        word[1923] = 13;
    } else {
        for i in 0..900 {
            word[1024 + i * 2..1026 + i * 2]
                .copy_from_slice(&(if i == 899 { 13u16 } else { 0x0151u16 }).to_le_bytes());
        }
    }
    let mut table = vec![0; if mini { 512 } else { 4096 }];
    table[0] = 2;
    table[1..5].copy_from_slice(&16u32.to_le_bytes());
    table[5..9].copy_from_slice(&0u32.to_le_bytes());
    table[9..13].copy_from_slice(&900u32.to_le_bytes());
    let fc = if compressed {
        0x4000_0000u32 | 2048
    } else {
        1024
    };
    table[15..19].copy_from_slice(&fc.to_le_bytes());
    let mut compound =
        cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/WordDocument")
        .unwrap()
        .write_all(&word)
        .unwrap();
    compound
        .create_stream("/1Table")
        .unwrap()
        .write_all(&table)
        .unwrap();
    let bytes = compound.into_inner().into_inner();
    let sectors = bytes.len().div_ceil(512);
    // Deliberately put every logical CFB sector in non-adjacent floppy sectors.
    let lbas = (0..sectors).map(|i| 2 + i * 2).collect::<Vec<_>>();
    let mut image = vec![0; (lbas[sectors - 1] + 1) * 512];
    for (i, lba) in lbas.iter().enumerate() {
        let n = (bytes.len() - i * 512).min(512);
        image[*lba * 512..*lba * 512 + n].copy_from_slice(&bytes[i * 512..i * 512 + n]);
    }
    let parent = fat12::FileRecord {
        path: "Broken.doc".into(),
        short_name_hex: "42".into(),
        short_name_case_flags: 0,
        name_method: "fixture".into(),
        long_name: None,
        bytes: bytes.len(),
        sha256: String::new(),
        directory_entry_offset: 64,
        metadata_lbas: vec![0],
        data_lbas: lbas.iter().map(|n| *n as u64).collect(),
        clusters: vec![],
        fat_links: vec![],
        modified_dos_date: 0,
        modified_dos_time: 0,
    };
    (image, parent, lbas)
}
fn word_mapping(image: &[u8], parent: &fat12::FileRecord) -> SparseStream {
    let bytes = fat12::file_bytes(image, parent);
    let sources = parent
        .data_lbas
        .iter()
        .flat_map(|lba| (*lba as usize * 512..(*lba as usize + 1) * 512).map(Some))
        .take(bytes.len())
        .collect::<Vec<_>>();
    let trace = Arc::new(Mutex::new(Trace {
        allow_holes: false,
        reads: vec![],
        work: 0,
    }));
    let r = Reader {
        bytes: bytes.clone(),
        sources: sources.clone(),
        position: 0,
        trace: trace.clone(),
    };
    let mut c = cfb::OpenOptions::new().strict().open_with(r).unwrap();
    trace.lock().unwrap().allow_holes = true;
    stream(&mut c, "/WordDocument", &trace, &bytes, &sources).unwrap()
}
#[test]
fn unicode_text_on_both_sides_of_a_missing_sector_has_precise_positions_and_extents() {
    let (mut image, parent, _) = fixture(false, true);
    let word = word_mapping(&image, &parent);
    let missing = word.sources[1536].unwrap() / 512;
    image[missing * 512..(missing + 1) * 512].fill(0xff); // unreadable backing bytes must never leak
    let doc = extract(&image, &parent, &BTreeSet::from([missing as u64])).unwrap();
    assert_eq!(doc.main_characters, Some(900));
    assert_eq!(doc.segments.len(), 2);
    assert_eq!(
        (doc.segments[0].cp_start, doc.segments[0].cp_count),
        (0, 256)
    );
    assert_eq!(
        (doc.segments[1].cp_start, doc.segments[1].cp_count),
        (512, 388)
    );
    assert_eq!(
        (doc.missing_text[0].cp_start, doc.missing_text[0].cp_count),
        (256, 256)
    );
    assert_eq!(doc.segments[0].text, "ő".repeat(256));
    assert_eq!(doc.segments[1].text, format!("{}\r", "ő".repeat(387)));
    for s in &doc.segments {
        assert!(
            s.source_extents
                .iter()
                .all(|e| e.source_byte_offset / 512 != missing)
        );
        let actual = s
            .source_extents
            .iter()
            .flat_map(|e| {
                image[e.source_byte_offset..e.source_byte_offset + e.bytes]
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), s.encoded_bytes);
        let units = actual
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(String::from_utf16(&units).unwrap(), s.text);
    }
    assert!(!doc.repaired_original && !doc.customer_delivery_certified);
}
#[test]
fn compressed_text_regular_table_and_unused_missing_sector_are_supported() {
    let (image, parent, _) = fixture(true, false);
    let word = word_mapping(&image, &parent);
    let missing = word.sources[4096].unwrap() / 512;
    let d = extract(&image, &parent, &BTreeSet::from([missing as u64])).unwrap();
    assert_eq!(d.segments.len(), 1);
    assert_eq!(d.segments[0].text, format!("{}\r", "x".repeat(899)));
    assert!(d.missing_text.is_empty());
    assert_eq!(d.status, "text_salvaged_original_still_incomplete");
}
#[test]
fn missing_header_fib_or_piece_metadata_is_refused_instead_of_using_placeholders() {
    let (image, parent, lbas) = fixture(false, true);
    let word = word_mapping(&image, &parent);
    assert!(extract(&image, &parent, &BTreeSet::from([lbas[0] as u64])).is_err());
    assert!(
        extract(
            &image,
            &parent,
            &BTreeSet::from([(word.sources[0].unwrap() / 512) as u64])
        )
        .is_err()
    );
    // Find the table's trace using the same strict reader, including mini streams.
    let bytes = fat12::file_bytes(&image, &parent);
    let sources = parent
        .data_lbas
        .iter()
        .flat_map(|lba| (*lba as usize * 512..(*lba as usize + 1) * 512).map(Some))
        .take(bytes.len())
        .collect::<Vec<_>>();
    let trace = Arc::new(Mutex::new(Trace {
        allow_holes: false,
        reads: vec![],
        work: 0,
    }));
    let mut c = cfb::OpenOptions::new()
        .strict()
        .open_with(Reader {
            bytes: bytes.clone(),
            sources: sources.clone(),
            position: 0,
            trace: trace.clone(),
        })
        .unwrap();
    trace.lock().unwrap().allow_holes = true;
    let table = stream(&mut c, "/1Table", &trace, &bytes, &sources).unwrap();
    assert!(
        extract(
            &image,
            &parent,
            &BTreeSet::from([(table.sources[0].unwrap() / 512) as u64])
        )
        .is_err()
    );
}
#[test]
fn encrypted_bad_counts_offsets_and_utf16_are_not_repaired_by_guessing() {
    let (original, parent, _) = fixture(false, true);
    let word = word_mapping(&original, &parent);
    for (offset, replacement) in [
        (10, 0x0300u32.to_le_bytes().to_vec()[..2].to_vec()),
        (76, u32::MAX.to_le_bytes().to_vec()),
        (418, u32::MAX.to_le_bytes().to_vec()),
    ] {
        let mut image = original.clone();
        for (i, b) in replacement.iter().enumerate() {
            image[word.sources[offset + i].unwrap()] = *b;
        }
        assert!(extract(&image, &parent, &BTreeSet::new()).is_err());
    }
    let mut image = original.clone();
    let bytes = 0xd800u16.to_le_bytes();
    for (i, b) in bytes.iter().enumerate() {
        image[word.sources[1024 + i].unwrap()] = *b;
    }
    let doc = extract(&image, &parent, &BTreeSet::new()).unwrap();
    assert_eq!(doc.missing_text[0].cp_start, 0);
    assert!(doc.missing_text[0].reason.contains("surrogate"));
    assert_eq!(doc.segments[0].cp_start, 1);
}
#[test]
fn actual_surrogate_pair_and_compressed_spec_mapping_preserve_text_without_substitution() {
    let (mut image, parent, _) = fixture(false, true);
    let word = word_mapping(&image, &parent);
    for (i, b) in [0x3d, 0xd8, 0x00, 0xde].iter().enumerate() {
        image[word.sources[1024 + i].unwrap()] = *b;
    }
    let d = extract(&image, &parent, &BTreeSet::new()).unwrap();
    assert!(d.segments[0].text.starts_with('😀'));
    assert_eq!(d.segments[0].cp_count, 900);
    assert_eq!(compressed_character(0x93), '“');
    assert_eq!(compressed_character(0xe1), 'á');
    assert_eq!(compressed_character(0x80), '\u{80}'); // not a guessed Windows-1252 Euro
}
#[test]
fn bounds_and_unknown_allocation_tail_refuse_required_metadata() {
    let (image, mut parent, _) = fixture(false, true);
    parent.bytes = fat12::MAX_IMAGE_BYTES + 1;
    assert!(extract(&image, &parent, &BTreeSet::new()).is_err());
    parent.bytes = 4096;
    parent.data_lbas.truncate(1);
    assert!(extract(&image, &parent, &BTreeSet::new()).is_err());
}

#[test]
fn extended_fib_version_is_bound_to_field_count_and_unencrypted_obfuscation_flag_is_ignored() {
    let (mut image, parent, _) = fixture(false, true);
    let mapping = word_mapping(&image, &parent);
    let mut compound =
        cfb::CompoundFile::open(Cursor::new(fat12::file_bytes(&image, &parent))).unwrap();
    {
        let mut word = compound.open_stream("/WordDocument").unwrap();
        word.seek(SeekFrom::Start(4096)).unwrap();
        word.write_all(&mapping.bytes[1024..2824]).unwrap();
        word.seek(SeekFrom::Start(10)).unwrap();
        word.write_all(&0x8200u16.to_le_bytes()).unwrap();
        word.seek(SeekFrom::Start(152)).unwrap();
        word.write_all(&0x88u16.to_le_bytes()).unwrap();
        word.seek(SeekFrom::Start(154 + 0x88 * 8)).unwrap();
        word.write_all(&2u16.to_le_bytes()).unwrap();
        word.write_all(&0x101u16.to_le_bytes()).unwrap();
        word.write_all(&0u16.to_le_bytes()).unwrap();
    }
    {
        let mut table = compound.open_stream("/1Table").unwrap();
        table.seek(SeekFrom::Start(15)).unwrap();
        table.write_all(&4096u32.to_le_bytes()).unwrap();
    }
    let payload = compound.into_inner().into_inner();
    for (i, b) in payload.iter().enumerate() {
        image[parent.data_lbas[i / 512] as usize * 512 + i % 512] = *b;
    }
    let d = extract(&image, &parent, &BTreeSet::new()).unwrap();
    assert_eq!(d.main_characters, Some(900));
    assert_eq!(d.segments[0].text, format!("{}\r", "ő".repeat(899)));
    let word = word_mapping(&image, &parent);
    image[word.sources[154 + 0x88 * 8 + 2].unwrap()] = 0xd9;
    assert!(
        extract(&image, &parent, &BTreeSet::new())
            .unwrap_err()
            .contains("nFibNew")
    );
}

#[test]
fn native_service_cli_archive_and_tamper_checks_keep_text_out_of_whole_file_counts() {
    let (source, parent, _) = fixture(false, true);
    let word = word_mapping(&source, &parent);
    let physical = word.sources[1536].unwrap();
    let logical = parent
        .data_lbas
        .iter()
        .position(|l| *l == (physical / 512) as u64)
        .unwrap();
    let payload = fat12::file_bytes(&source, &parent);
    let mut image = fat12::tests::image();
    let clusters = (0..payload.len().div_ceil(512))
        .map(|i| 100 + (i as u16) * 2)
        .collect::<Vec<_>>();
    fat12::tests::entry(
        &mut image,
        19 * 512,
        b"BROKEN  DOC",
        clusters[0],
        payload.len() as u32,
        false,
    );
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
        let n = (payload.len() - i * 512).min(512);
        image[at..at + n].copy_from_slice(&payload[i * 512..i * 512 + n]);
    }
    let bad_lba = 33 + clusters[logical] as usize - 2;
    image[bad_lba * 512..(bad_lba + 1) * 512].fill(0);
    let root = std::env::temp_dir().join(format!(
        "fv-word-integration-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let p = crate::project::ProjectState::create_without_session(root).unwrap();
    let source_path = p.images_dir().join("001.img");
    let sha = hash(&image);
    fs::write(&source_path, &image).unwrap();
    fs::write(p.logs_dir().join("001.log"),format!("BEGIN | disk=1 | attempt=1\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\nBAD_SECTOR | lba={bad_lba}\nEND | status=PARTIAL | bytes=1474560 | sha256={sha}\n")).unwrap();
    let attempt = crate::imaging::load_attempts_for_disk(&p.images_dir(), 1)
        .unwrap()
        .remove(0);
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
    let first = run().unwrap();
    assert_eq!(first.files, 0);
    let text = first.document_salvage.unwrap();
    assert_eq!(text.text_segments, 2);
    assert_eq!(text.recovered_character_positions, 644);
    assert_eq!(text.missing_character_positions, 256);
    assert_eq!(text.repaired_originals, 0);
    assert_eq!(text.engine_version, 2);
    assert_eq!(text.readable_editions, 1);
    let edition = &text.readable_edition_paths[0];
    let edition_bytes = fs::read(edition).unwrap();
    assert!(String::from_utf8_lossy(&edition_bytes).contains("MISSING / INVALID TEXT"));
    assert!(!text.reused);
    // Old generations are operator/evidence-owned and must never be migrated in place.
    let legacy = text
        .output_directory
        .with_file_name("attempt_001_word_text_v1");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("word-text.json"), b"old forensic report").unwrap();
    let repeated = run().unwrap();
    assert!(repeated.reused);
    assert!(repeated.document_salvage.unwrap().reused);
    assert_eq!(
        fs::read(legacy.join("word-text.json")).unwrap(),
        b"old forensic report"
    );
    let cli = crate::cli::run(
        &[
            "recovery".into(),
            "documents".into(),
            "1".into(),
            "--json".into(),
        ],
        p.root(),
    )
    .unwrap();
    assert_eq!(cli.exit_code, 3);
    let value: serde_json::Value = serde_json::from_str(&cli.output).unwrap();
    assert_eq!(value["physical_media_access"], false);
    assert_eq!(value["document_salvage"]["text_segments"], 2);
    assert_eq!(value["document_salvage"]["readable_editions"], 1);
    let human = crate::cli::run(
        &["recovery".into(), "documents".into(), "1".into()],
        p.root(),
    )
    .unwrap();
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
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&text.report_path).unwrap()).unwrap();
    let name = report["documents"][0]["segments"][0]["path"]
        .as_str()
        .unwrap();
    let target = text.output_directory.join(name);
    let expected = fs::read(&target).unwrap();
    let destination = p.root().with_extension("package");
    fs::create_dir(&destination).unwrap();
    let packed = crate::package::build_package(
        &crate::package::PackageRequest {
            project_root: p.root().to_path_buf(),
            destination: destination.clone(),
            project_name: "word-text-fixture".into(),
        },
        &|_| {},
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&packed.zip_path).unwrap()).unwrap();
    assert!(
        archive
            .by_name(&format!(
                "Recovery/001/attempt_{:03}_word_text_v2/{name}",
                attempt.attempt_number
            ))
            .is_ok()
    );
    assert!(
        archive
            .by_name(&format!(
                "Recovery/001/attempt_{:03}_word_text_v2/{}",
                attempt.attempt_number,
                edition.file_name().unwrap().to_string_lossy()
            ))
            .is_ok()
    );
    drop(archive);
    fs::write(edition, b"edited HTML").unwrap();
    assert!(run().unwrap_err().contains("edition changed"));
    assert_eq!(fs::read(edition).unwrap(), b"edited HTML");
    fs::write(edition, &edition_bytes).unwrap();
    let extra = text.output_directory.join("operator-note.txt");
    fs::write(&extra, b"do not overwrite").unwrap();
    assert!(run().unwrap_err().contains("report/inventory changed"));
    assert_eq!(fs::read(&extra).unwrap(), b"do not overwrite");
    fs::remove_file(extra).unwrap();
    fs::write(&target, b"operator edit").unwrap();
    assert!(run().unwrap_err().contains("text changed"));
    assert_eq!(fs::read(&target).unwrap(), b"operator edit");
    fs::write(&target, &expected).unwrap();
    fs::write(&text.report_path, b"{}").unwrap();
    assert!(run().unwrap_err().contains("report/inventory changed"));
    assert_eq!(hash(&fs::read(source_path).unwrap()), sha);
    fs::remove_dir_all(p.root()).unwrap();
    fs::remove_dir_all(destination).unwrap();
}
