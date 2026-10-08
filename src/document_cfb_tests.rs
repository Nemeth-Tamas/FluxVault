use super::*;

fn scatter(payload: Vec<u8>, mut parent: fat12::FileRecord) -> (Vec<u8>, fat12::FileRecord) {
    parent.bytes = payload.len();
    parent.data_lbas = (0..payload.len().div_ceil(512))
        .map(|i| (2 + i * 3) as u64)
        .collect();
    let mut image = vec![0; (parent.data_lbas.last().unwrap() + 1) as usize * 512];
    for (i, b) in payload.iter().enumerate() {
        image[parent.data_lbas[i / 512] as usize * 512 + i % 512] = *b;
    }
    (image, parent)
}
fn fixture_sparse(mini: bool, v4: bool) -> (Vec<u8>, fat12::FileRecord, usize) {
    let (image, parent, _) = fixture(false, mini);
    let word = word_mapping(&image, &parent).bytes;
    let mut old = cfb::CompoundFile::open(Cursor::new(fat12::file_bytes(&image, &parent))).unwrap();
    let mut table = vec![];
    old.open_stream("/1Table")
        .unwrap()
        .read_to_end(&mut table)
        .unwrap();
    let mut c = cfb::CompoundFile::create_with_version(
        if v4 {
            cfb::Version::V4
        } else {
            cfb::Version::V3
        },
        Cursor::new(Vec::new()),
    )
    .unwrap();
    c.create_stream("/WordDocument")
        .unwrap()
        .write_all(&word[..4096])
        .unwrap();
    c.create_stream("/1Table")
        .unwrap()
        .write_all(&table)
        .unwrap();
    {
        let mut stream = c.open_stream("/WordDocument").unwrap();
        stream.seek(SeekFrom::End(0)).unwrap();
        stream.write_all(&word[4096..]).unwrap();
    }
    c.create_storage("/ObjectPool").unwrap();
    for i in 0..8 {
        c.create_stream(format!("/ObjectPool/A{i:03}"))
            .unwrap()
            .write_all(&vec![b'a'; 4096])
            .unwrap();
    }
    c.create_stream("/Auxiliary")
        .unwrap()
        .write_all(b"metadata")
        .unwrap();
    let payload = c.into_inner().into_inner();
    let offset = directory_offsets(&payload)[4];
    let (image, parent) = scatter(payload, parent);
    let bad = (parent.data_lbas[offset / 512]) as usize;
    (image, parent, bad)
}
fn sector_size(payload: &[u8]) -> usize {
    1 << u16at(payload, 30)
}
fn fat_offset(payload: &[u8], sid: u32) -> usize {
    let size = sector_size(payload);
    let fat = u32at(payload, 76 + sid as usize / (size / 4) * 4);
    (fat + 1) * size + (sid as usize % (size / 4)) * 4
}
fn directory_offsets(payload: &[u8]) -> Vec<usize> {
    let size = sector_size(payload);
    let mut sid = u32at(payload, 48) as u32;
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    while sid != 0xffff_fffe {
        assert!(seen.insert(sid));
        let at = (sid as usize + 1) * size;
        result.extend((0..size).step_by(128).map(|n| at + n));
        sid = u32at(payload, fat_offset(payload, sid)) as u32;
    }
    result
}
fn root_child_offsets(payload: &[u8]) -> Vec<usize> {
    let offsets = directory_offsets(payload);
    let mut ids = vec![u32at(payload, offsets[0] + 76) as u32];
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    while let Some(id) = ids.pop() {
        if id == u32::MAX {
            continue;
        }
        assert!(seen.insert(id));
        let at = offsets[id as usize];
        result.push(at);
        ids.extend([
            u32at(payload, at + 68) as u32,
            u32at(payload, at + 72) as u32,
        ]);
    }
    result
}
fn entry_named(payload: &[u8], name: &str) -> usize {
    directory_offsets(payload)
        .into_iter()
        .find(|at| {
            let n = u16at(payload, *at + 64) as usize;
            n >= 2
                && n <= 64
                && String::from_utf16(
                    &payload[*at..*at + n - 2]
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                        .collect::<Vec<_>>(),
                )
                .is_ok_and(|s| s == name)
        })
        .unwrap()
}
fn source_replay(image: &[u8], doc: &Document, bad: &BTreeSet<u64>) {
    assert!(doc.metadata_lbas.iter().all(|l| !bad.contains(l)));
    for segment in &doc.segments {
        let bytes = segment
            .source_extents
            .iter()
            .flat_map(|e| {
                image[e.source_byte_offset..e.source_byte_offset + e.bytes]
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();
        assert!(
            segment
                .source_extents
                .iter()
                .all(|e| !bad.contains(&((e.source_byte_offset / 512) as u64)))
        );
        let text = if segment.encoding == "UTF-16LE" {
            String::from_utf16(
                &bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        } else {
            bytes.into_iter().map(compressed_character).collect()
        };
        assert_eq!(text, segment.text);
    }
    for link in &doc.cfb_recovery.as_ref().unwrap().allocation_links {
        assert_eq!(
            u32at(image, link.source_byte_offset) as u32,
            link.next_sector_id
        );
        assert!(!bad.contains(&((link.source_byte_offset / 512) as u64)));
    }
}

#[test]
fn selective_mapping_recovers_root_text_past_unrelated_missing_directory_entries_in_v3_and_v4() {
    for (mini, v4) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut image, parent, bad) = fixture_sparse(mini, v4);
        image[bad * 512..(bad + 1) * 512].fill(0xff);
        let mapped = parent_mapping(&image, &parent, &BTreeSet::from([bad as u64])).unwrap();
        assert!(extract_strict(&mapped.bytes, &mapped.sources, &parent).is_err());
        let doc = extract(&image, &parent, &BTreeSet::from([bad as u64])).unwrap();
        assert_eq!(doc.segments.iter().map(|s| s.cp_count).sum::<usize>(), 900);
        assert!(doc.missing_text.is_empty());
        let evidence = doc.cfb_recovery.as_ref().unwrap();
        assert_eq!(evidence.missing_directory_stream_ids, vec![4, 5, 6, 7]);
        assert!(!evidence.whole_container_verified);
        assert!(
            evidence
                .selected_streams
                .iter()
                .find(|s| s.name == "1Table")
                .unwrap()
                .mini_stream
                == mini
        );
        assert!(doc.strict_parser_refusal.is_some());
        source_replay(&image, &doc, &BTreeSet::from([bad as u64]));
    }
}
#[test]
fn sparse_container_and_missing_word_text_preserve_exact_gaps_and_never_decode_placeholder_bytes() {
    let (mut image, parent, bad) = fixture_sparse(true, false);
    let word = word_mapping(&image, &parent);
    let text_bad = word.sources[1536].unwrap() / 512;
    for lba in [bad, text_bad] {
        image[lba * 512..(lba + 1) * 512].fill(0xff);
    }
    let bad = BTreeSet::from([bad as u64, text_bad as u64]);
    let doc = extract(&image, &parent, &bad).unwrap();
    assert_eq!(doc.segments.iter().map(|s| s.cp_count).sum::<usize>(), 644);
    assert_eq!(doc.missing_text[0].cp_start, 256);
    assert_eq!(doc.missing_text[0].cp_count, 256);
    source_replay(&image, &doc, &bad);
}
#[test]
fn unavailable_root_sibling_nodes_cannot_be_replaced_with_detached_named_entries() {
    let (image, parent, bad) = fixture_sparse(false, false);
    let mut payload = fat12::file_bytes(&image, &parent);
    let offsets = directory_offsets(&payload);
    payload[offsets[0] + 76..offsets[0] + 80].copy_from_slice(&4u32.to_le_bytes());
    let (image, parent) = scatter(payload, parent);
    let error = extract(&image, &parent, &BTreeSet::from([bad as u64])).unwrap_err();
    assert!(error.contains("root sibling path"));
}
#[test]
fn selected_stream_crosslinks_known_regular_mini_or_directory_allocation_are_refused() {
    for mode in 0..5 {
        let (image, parent, bad) = fixture_sparse(mode == 1, false);
        let mut payload = fat12::file_bytes(&image, &parent);
        let target = entry_named(&payload, if mode == 1 { "1Table" } else { "WordDocument" });
        if mode >= 2 {
            let target = if mode >= 3 {
                entry_named(&payload, "A004")
            } else {
                target
            };
            let dir = if mode == 3 {
                u32at(&payload, directory_offsets(&payload)[0] + 116) as u32
            } else if mode == 4 {
                u32at(&payload, 60) as u32
            } else {
                u32at(&payload, 48) as u32
            };
            payload[target + 116..target + 120].copy_from_slice(&dir.to_le_bytes());
        } else {
            let other = entry_named(&payload, "A004"); // readable nested slot beyond the missing page
            let start = payload[target + 116..target + 120].to_vec();
            let size = payload[target + 120..target + 128].to_vec();
            payload[other + 116..other + 120].copy_from_slice(&start);
            payload[other + 120..other + 128].copy_from_slice(&size);
        }
        let (image, parent) = scatter(payload, parent);
        assert!(
            extract(&image, &parent, &BTreeSet::from([bad as u64])).is_err(),
            "mode {mode}"
        );
    }
}
#[test]
fn duplicate_root_names_cycles_bad_fat_links_and_missing_selected_entries_are_refused() {
    for mode in 0..5 {
        let (image, parent, bad) = fixture_sparse(false, false);
        let mut payload = fat12::file_bytes(&image, &parent);
        let root_nodes = root_child_offsets(&payload);
        let word = entry_named(&payload, "WordDocument");
        let mut holes = BTreeSet::from([bad as u64]);
        match mode {
            0 => {
                let aux = entry_named(&payload, "Auxiliary");
                let name = payload[word..word + 66].to_vec();
                payload[aux..aux + 66].copy_from_slice(&name);
            }
            1 => {
                let root = directory_offsets(&payload)[0];
                let child = payload[root + 76..root + 80].to_vec();
                payload[root_nodes[0] + 68..root_nodes[0] + 72].copy_from_slice(&child);
            }
            2 => {
                let dir = u32at(&payload, 48) as u32;
                let at = fat_offset(&payload, dir);
                payload[at..at + 4].copy_from_slice(&dir.to_le_bytes());
            }
            3 => {
                let sid = u32at(&payload, word + 116) as u32;
                holes.insert(parent.data_lbas[fat_offset(&payload, sid) / 512]);
            }
            _ => {
                holes.insert(parent.data_lbas[word / 512]);
            }
        }
        let (image, parent) = scatter(payload, parent);
        assert!(extract(&image, &parent, &holes).is_err(), "mode {mode}");
    }
}
#[test]
fn zero_table_selection_follows_fib_and_never_substitutes_the_other_root_table() {
    let (image, parent, bad) = fixture_sparse(true, false);
    let mut payload = fat12::file_bytes(&image, &parent);
    let table = entry_named(&payload, "1Table");
    payload[table..table + 2].copy_from_slice(&(b'0' as u16).to_le_bytes());
    let word = entry_named(&payload, "WordDocument");
    let word_at = (u32at(&payload, word + 116) + 1) * sector_size(&payload);
    let flags = u16at(&payload, word_at + 10) & !0x200;
    payload[word_at + 10..word_at + 12].copy_from_slice(&flags.to_le_bytes());
    let (image, parent) = scatter(payload, parent);
    let holes = BTreeSet::from([bad as u64]);
    let doc = extract(&image, &parent, &holes).unwrap();
    assert_eq!(doc.table_stream.as_deref(), Some("/0Table"));
    source_replay(&image, &doc, &holes);
    let mut payload = fat12::file_bytes(&image, &parent);
    payload[word_at + 10..word_at + 12].copy_from_slice(&(flags | 0x200).to_le_bytes());
    let (image, parent) = scatter(payload, parent);
    assert!(
        extract(&image, &parent, &holes)
            .unwrap_err()
            .contains("no 1Table stream")
    );
}

#[test]
fn malformed_selective_metadata_is_bounded_and_does_not_panic() {
    let (image, parent, bad) = fixture_sparse(true, false);
    let mut original = parent_mapping(&image, &parent, &BTreeSet::from([bad as u64])).unwrap();
    let mut seed = 1234567u32;
    let offsets = directory_offsets(&original.bytes);
    let targets = [
        26,
        28,
        30,
        32,
        40,
        44,
        48,
        56,
        60,
        64,
        68,
        72,
        76,
        offsets[0] + 76,
        offsets[1] + 116,
        offsets[2] + 120,
    ];
    for i in 0..128 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let mut bytes = original.bytes.clone();
        let at = targets[i % targets.len()];
        bytes[at..at + 4].copy_from_slice(&seed.to_le_bytes());
        let _ = cfb_recovery::streams(&bytes, &original.sources);
    }
    original.sources[0] = None;
    assert!(cfb_recovery::streams(&original.bytes, &original.sources).is_err());
}
#[test]
fn readable_html_escapes_untrusted_markup_and_shows_gap_and_partial_metadata_warnings() {
    let (image, parent, bad) = fixture_sparse(true, false);
    let word = word_mapping(&image, &parent);
    let text_bad = word.sources[1536].unwrap() / 512;
    let mut doc = extract(
        &image,
        &parent,
        &BTreeSet::from([bad as u64, text_bad as u64]),
    )
    .unwrap();
    doc.parent.path = "<script>alert(1)</script>.doc".into();
    doc.segments[0].text = "<img src=x onerror=alert(1)> &\u{13}\t".into();
    let html = readable_html::render(&doc, "<unsafe>", &"f".repeat(64)).unwrap();
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img "));
    assert!(html.contains("&lt;img src=x"));
    assert!(html.contains("\\u{0013}"));
    assert!(html.contains("MISSING / INVALID TEXT: CP 256–512"));
    assert!(html.contains("Partial compound-file metadata"));
    assert!(html.contains("default-src 'none'"));
    assert!(html.contains("not a repaired DOC"));
}

#[test]
#[ignore = "requires FLUXVAULT_WORD_TEST_PROJECT; read-only replay of saved v2 Word text/HTML reports"]
fn saved_word_reports_replay_source_extents_and_readable_editions() {
    let project = crate::project::ProjectState::open_without_session(std::path::PathBuf::from(
        std::env::var("FLUXVAULT_WORD_TEST_PROJECT").unwrap(),
    ))
    .unwrap();
    let mut documents = 0;
    let mut selective = 0;
    for disk in fs::read_dir(project.recovery_dir()).unwrap().flatten() {
        if !disk.file_type().unwrap().is_dir() {
            continue;
        }
        for folder in fs::read_dir(disk.path()).unwrap().flatten() {
            if !folder.file_type().unwrap().is_dir()
                || !folder
                    .file_name()
                    .to_string_lossy()
                    .ends_with("_word_text_v2")
            {
                continue;
            }
            let mut report: Report =
                serde_json::from_slice(&fs::read(folder.path().join("word-text.json")).unwrap())
                    .unwrap();
            assert_eq!(report.engine_version, ENGINE_VERSION);
            let image = fs::read(project.images_dir().join(&report.source_image)).unwrap();
            assert_eq!(hash(&image), report.source_sha256);
            let bad = report.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
            for doc in &mut report.documents {
                if doc.segments.is_empty() {
                    continue;
                }
                documents += 1;
                assert!(!doc.repaired_original && !doc.customer_delivery_certified);
                assert!(doc.metadata_lbas.iter().all(|l| !bad.contains(l)));
                for s in &mut doc.segments {
                    s.text =
                        String::from_utf8(fs::read(folder.path().join(&s.path)).unwrap()).unwrap();
                    assert_eq!(hash(s.text.as_bytes()), s.sha256);
                    assert_eq!(s.text.len(), s.bytes);
                    let encoded = s
                        .source_extents
                        .iter()
                        .flat_map(|e| {
                            assert!(
                                (e.source_byte_offset..e.source_byte_offset + e.bytes)
                                    .all(|i| !bad.contains(&((i / 512) as u64)))
                            );
                            image[e.source_byte_offset..e.source_byte_offset + e.bytes]
                                .iter()
                                .copied()
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(encoded.len(), s.encoded_bytes);
                    let replay: String = if s.encoding == "UTF-16LE" {
                        assert_eq!(encoded.len() % 2, 0);
                        String::from_utf16(
                            &encoded
                                .chunks_exact(2)
                                .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                                .collect::<Vec<_>>(),
                        )
                        .unwrap()
                    } else {
                        encoded.into_iter().map(compressed_character).collect()
                    };
                    assert_eq!(replay, s.text);
                    assert_eq!(s.text.encode_utf16().count(), s.cp_count);
                }
                let mut ranges = doc
                    .segments
                    .iter()
                    .map(|s| (s.cp_start, s.cp_count))
                    .chain(doc.missing_text.iter().map(|g| (g.cp_start, g.cp_count)))
                    .collect::<Vec<_>>();
                ranges.sort_unstable();
                let mut end = 0;
                for (start, n) in ranges {
                    assert_eq!(start, end);
                    end += n;
                }
                assert_eq!(Some(end), doc.main_characters);
                if let Some(e) = &doc.cfb_recovery {
                    selective += 1;
                    assert!(!e.whole_container_verified);
                    for link in &e.allocation_links {
                        assert!(!bad.contains(&((link.source_byte_offset / 512) as u64)));
                        assert_eq!(
                            u32at(&image, link.source_byte_offset) as u32,
                            link.next_sector_id
                        );
                    }
                    for entry in &e.root_sibling_entries {
                        let actual = (entry.directory_parent_byte_offset
                            ..entry.directory_parent_byte_offset + 128)
                            .map(|at| {
                                let lba = doc.parent.data_lbas[at / 512];
                                assert!(!bad.contains(&lba));
                                format!("{:02x}", image[lba as usize * 512 + at % 512])
                            })
                            .collect::<String>();
                        assert_eq!(actual, entry.raw_entry_hex);
                    }
                }
                let edition = doc.readable_edition.as_ref().unwrap();
                let html = fs::read(folder.path().join(&edition.path)).unwrap();
                assert_eq!(html.len(), edition.bytes);
                assert_eq!(hash(&html), edition.sha256);
                assert_eq!(
                    html,
                    readable_html::render(doc, &report.source_image, &report.source_sha256)
                        .unwrap()
                        .as_bytes()
                );
                assert!(!edition.repaired_original);
            }
        }
    }
    assert!(documents > 0, "No saved text results to verify");
    eprintln!(
        "Replayed {documents} document(s), {selective} selective CFB mapping(s), all text/HTML hashes and CP coverage verified"
    );
}
