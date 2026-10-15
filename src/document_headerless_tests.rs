use super::*;

fn put(image: &mut [u8], parent: &fat12::FileRecord, offset: usize, bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        let at = offset + i;
        image[parent.data_lbas[at / 512] as usize * 512 + at % 512] = *b;
    }
}
fn header(image: &[u8], parent: &fat12::FileRecord) -> Vec<u8> {
    image[parent.data_lbas[0] as usize * 512..parent.data_lbas[0] as usize * 512 + 512].to_vec()
}
fn entry(image: &[u8], parent: &fat12::FileRecord, name: &str) -> usize {
    let bytes = fat12::file_bytes(image, parent);
    (512..bytes.len())
        .step_by(128)
        .find(|at| {
            let n = u16at(&bytes, *at + 64) as usize;
            (2..=64).contains(&n)
                && n.is_multiple_of(2)
                && String::from_utf16(
                    &bytes[*at..*at + n - 2]
                        .chunks_exact(2)
                        .map(|b| u16at(b, 0))
                        .collect::<Vec<_>>(),
                )
                .is_ok_and(|s| s == name)
        })
        .unwrap()
}

#[test]
fn headerless_regular_streams_recover_exact_text_with_inference_and_real_source_extents() {
    for compressed in [false, true] {
        let (image, parent, lbas) = fixture(compressed, false);
        let good = extract(&image, &parent, &BTreeSet::new()).unwrap();
        let got = extract(&image, &parent, &BTreeSet::from([lbas[0] as u64])).unwrap();
        assert_eq!(
            got.segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            good.segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>()
        );
        assert!(
            got.missing_text.is_empty()
                && !got.repaired_original
                && !got.customer_delivery_certified
        );
        let e = got.cfb_recovery.as_ref().unwrap();
        let inferred = e.inferred_header.as_ref().unwrap();
        assert_eq!(inferred.unavailable_header_bytes, 512);
        assert_eq!(inferred.assumed_sector_bytes, 512);
        assert!(!inferred.header_reconstructed && !inferred.mini_fat_mapping_known);
        assert!(!got.metadata_lbas.contains(&(lbas[0] as u64)));
        assert!(e.selected_streams.iter().all(|s| !s.mini_stream));
        for s in &got.segments {
            assert_eq!(
                s.source_extents.iter().map(|e| e.bytes).sum::<usize>(),
                s.encoded_bytes
            );
        }
        let html = readable_html::render(&got, "test.img", "test").unwrap();
        assert!(html.contains("HEADERLESS STRUCTURAL TEXT CANDIDATE"));
    }
}

#[test]
fn headerless_regular_word6_needs_no_table_stream_to_decode_supported_charset() {
    let (mut image, parent, lbas) = fixture(false, false);
    let at = entry(&image, &parent, "WordDocument");
    let bytes = fat12::file_bytes(&image, &parent);
    let stream = (u32at(&bytes, at + 116) + 1) * 512;
    for (offset, value) in [
        (0, 0xa5dcu16),
        (2, 0x65),
        (6, 0x040e),
        (10, 0),
        (12, 0x65),
        (18, 0),
        (20, 238),
    ] {
        put(&mut image, &parent, stream + offset, &value.to_le_bytes());
    }
    for (offset, value) in [(24, 768u32), (28, 770), (32, 8192), (52, 2)] {
        put(&mut image, &parent, stream + offset, &value.to_le_bytes());
    }
    put(&mut image, &parent, stream + 768, b"\xf5\r");
    let doc = extract(&image, &parent, &BTreeSet::from([lbas[0] as u64])).unwrap();
    assert_eq!(doc.segments[0].text, "ő\r");
    assert!(doc.missing_text.is_empty());
    assert_eq!(doc.legacy_text.unwrap().code_page, 1250);
    let e = doc.cfb_recovery.unwrap();
    assert!(e.inferred_header.is_some() && e.selected_streams.len() == 1);
}

#[test]
fn headerless_missing_text_is_a_gap_not_placeholder_letters() {
    let (image, parent, lbas) = fixture(false, false);
    let mapped = word_mapping(&image, &parent);
    let text_lba = mapped.sources[1536].unwrap() / 512;
    let d = extract(
        &image,
        &parent,
        &BTreeSet::from([lbas[0] as u64, text_lba as u64]),
    )
    .unwrap();
    assert_eq!(d.segments.iter().map(|s| s.cp_count).sum::<usize>(), 644);
    assert_eq!(
        d.missing_text.iter().map(|s| s.cp_count).sum::<usize>(),
        256
    );
}

#[test]
fn headerless_does_not_replace_readable_corruption_or_infer_partial_headers_and_mini_streams() {
    let (mut image, parent, lbas) = fixture(false, false);
    put(&mut image, &parent, 0, b"bad head");
    assert!(extract(&image, &parent, &BTreeSet::new()).is_err());
    let mut map =
        sparse_parent_mapping(&image, &parent, &BTreeSet::from([lbas[0] as u64])).unwrap();
    map.sources[0] = Some(lbas[0] * 512);
    assert!(cfb_recovery::headerless::streams(&map.bytes, &map.sources).is_err());
    let (image, parent, lbas) = fixture(false, true);
    assert!(
        extract(&image, &parent, &BTreeSet::from([lbas[0] as u64]))
            .unwrap_err()
            .contains("mini-stream")
    );
}

#[test]
fn headerless_requires_unique_fat_and_root_and_all_directory_metadata() {
    let (image, parent, lbas) = fixture(false, false);
    let h = header(&image, &parent);
    let fat = u32at(&h, 76);
    let dir = u32at(&h, 48);
    for sid in [fat, dir] {
        assert!(
            extract(
                &image,
                &parent,
                &BTreeSet::from([lbas[0] as u64, lbas[sid + 1] as u64])
            )
            .is_err()
        );
    }
    let bytes = fat12::file_bytes(&image, &parent);
    let mut duplicate = bytes[(fat + 1) * 512..(fat + 2) * 512].to_vec();
    let second = (0..bytes.len() / 512 - 1)
        .find(|sid| *sid != fat && *sid != dir)
        .unwrap();
    duplicate[fat * 4..fat * 4 + 4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
    duplicate[second * 4..second * 4 + 4].copy_from_slice(&0xffff_fffdu32.to_le_bytes());
    let mut bad = image.clone();
    put(&mut bad, &parent, (second + 1) * 512, &duplicate);
    assert!(
        extract(&bad, &parent, &BTreeSet::from([lbas[0] as u64]))
            .unwrap_err()
            .contains("not unique")
    );
    let mut bad = image;
    put(
        &mut bad,
        &parent,
        (second + 1) * 512,
        &bytes[(dir + 1) * 512..(dir + 1) * 512 + 128],
    );
    assert!(
        extract(&bad, &parent, &BTreeSet::from([lbas[0] as u64]))
            .unwrap_err()
            .contains("not unique")
    );
}

#[test]
fn headerless_refuses_cycles_crosslinks_detached_names_and_encryption() {
    let (image, parent, lbas) = fixture(false, false);
    let word_entry = entry(&image, &parent, "WordDocument");
    let table_entry = entry(&image, &parent, "1Table");
    let bytes = fat12::file_bytes(&image, &parent);
    let word_start = u32at(&bytes, word_entry + 116);
    for mode in 0..4 {
        let mut bad = image.clone();
        match mode {
            0 => put(
                &mut bad,
                &parent,
                table_entry + 116,
                &(word_start as u32).to_le_bytes(),
            ),
            1 => {
                let fat = u32at(&header(&image, &parent), 76);
                put(
                    &mut bad,
                    &parent,
                    (fat + 1) * 512 + word_start * 4,
                    &(word_start as u32).to_le_bytes(),
                );
            }
            2 => {
                let dir = u32at(&header(&image, &parent), 48);
                put(
                    &mut bad,
                    &parent,
                    (dir + 1) * 512 + 76,
                    &0xffff_ffffu32.to_le_bytes(),
                );
            }
            _ => put(
                &mut bad,
                &parent,
                (word_start + 1) * 512 + 10,
                &0x0300u16.to_le_bytes(),
            ),
        }
        assert!(
            extract(&bad, &parent, &BTreeSet::from([lbas[0] as u64])).is_err(),
            "mode {mode}"
        );
    }
    assert!(
        extract(&image, &parent, &BTreeSet::from([0, lbas[0] as u64]))
            .unwrap_err()
            .contains("parent")
    );
}
