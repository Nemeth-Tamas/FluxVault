use super::*;
use std::io::Write;

fn image_payload(format: image::ImageFormat) -> Vec<u8> {
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
        image::Rgb([x as u8 * 30, y as u8 * 30, 100])
    }));
    let mut cursor = Cursor::new(vec![]);
    image.write_to(&mut cursor, format).unwrap();
    cursor.into_inner()
}
fn sector_image(payload: &[u8], offset: usize) -> Vec<u8> {
    let mut image = vec![0; (payload.len() + offset + 512).div_ceil(512) * 512];
    image[offset..offset + payload.len()].copy_from_slice(payload);
    image
}
fn scan(image: &[u8], bad: &[u64]) -> Analysis {
    analyze(image, bad, &raw_readable_regions(image, bad), &[]).unwrap()
}
fn zip_payload(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));
    writer
        .start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(payload).unwrap();
    writer.finish().unwrap().into_inner()
}
fn compound_payload() -> Vec<u8> {
    let mut file = cfb::CompoundFile::create(Cursor::new(vec![])).unwrap();
    file.create_stream("/WordDocument")
        .unwrap()
        .write_all(b"\xec\xa5container only; Word semantics unverified")
        .unwrap();
    file.create_stream("/Data")
        .unwrap()
        .write_all(&vec![b'x'; 8192])
        .unwrap();
    file.flush().unwrap();
    file.into_inner().into_inner()
}

#[test]
fn complete_images_use_decoders_and_preserve_exact_bytes_and_raw_offsets() {
    for format in [
        image::ImageFormat::Png,
        image::ImageFormat::Jpeg,
        image::ImageFormat::Bmp,
        image::ImageFormat::Gif,
    ] {
        let payload = image_payload(format);
        let image = sector_image(&payload, 497);
        let result = scan(&image, &[]);
        assert_eq!(result.files.len(), 1, "{format:?}: {:?}", result.rejected);
        let file = &result.files[0];
        assert_eq!(file.source_extents[0].source_byte_offset, 497);
        assert_eq!(file_bytes(&image, file).unwrap(), payload);
        assert!(!file.original_name_known && !file.customer_delivery_certified);
        assert!(file.path.starts_with("SignatureRecovery/"));
    }
}
#[test]
fn zip_crc_is_checked_and_unsafe_members_are_not_exported() {
    let payload = zip_payload("folder/safe.txt", b"known contents");
    assert_eq!(scan(&sector_image(&payload, 31), &[]).files.len(), 1);
    let mut corrupted = payload.clone();
    corrupted[30 + "folder/safe.txt".len()] ^= 1;
    assert!(scan(&sector_image(&corrupted, 0), &[]).files.is_empty());
    for name in [
        "../escape.txt",
        "C:/escape.txt",
        "folder\\escape.txt",
        "trailing. /bad.txt",
        "folder/./alias.txt",
        "folder/CON.txt",
        "folder/LPT9.txt",
        "bad?.txt",
    ] {
        assert!(
            scan(&sector_image(&zip_payload(name, b"x"), 0), &[])
                .files
                .is_empty(),
            "{name}"
        );
    }
}

#[test]
fn embedded_workbook_does_not_override_root_word_type_and_ambiguous_roots_stay_generic() {
    for ambiguous in [false, true] {
        let mut file = cfb::CompoundFile::create(Cursor::new(vec![])).unwrap();
        file.create_stream("/WordDocument")
            .unwrap()
            .write_all(b"\xec\xa5word")
            .unwrap();
        file.create_storage("/Embedded").unwrap();
        file.create_stream(if ambiguous {
            "/Workbook"
        } else {
            "/Embedded/Workbook"
        })
        .unwrap()
        .write_all(b"\x09\x08workbook")
        .unwrap();
        file.flush().unwrap();
        let payload = file.into_inner().into_inner();
        let result = scan(&sector_image(&payload, 0), &[]);
        assert_eq!(result.files.len(), 1);
        assert!(
            result.files[0]
                .path
                .ends_with(if ambiguous { ".ole" } else { ".doc" })
        );
    }
}
#[test]
fn png_crc_and_oversized_pixel_headers_are_rejected() {
    let mut payload = image_payload(image::ImageFormat::Png);
    payload[20] ^= 1;
    assert!(scan(&sector_image(&payload, 0), &[]).files.is_empty());
    payload = image_payload(image::ImageFormat::Png);
    payload[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    let crc = crc32fast::hash(&payload[12..29]);
    payload[29..33].copy_from_slice(&crc.to_be_bytes());
    assert!(scan(&sector_image(&payload, 0), &[]).files.is_empty());
}
#[test]
fn compound_streams_and_ministreams_are_read_and_missing_tail_is_refused() {
    let payload = compound_payload();
    let image = sector_image(&payload, 512);
    let result = scan(&image, &[]);
    assert_eq!(result.files.len(), 1, "{:?}", result.rejected);
    assert!(result.files[0].path.ends_with(".doc"));
    assert!(
        result.files[0]
            .validation
            .contains("semantics remain unverified")
    );
    assert_eq!(file_bytes(&image, &result.files[0]).unwrap(), payload);
    assert!(scan(&image, &[2]).files.is_empty());
    let mut corrupt = payload.clone();
    corrupt[30..32].copy_from_slice(&15u16.to_le_bytes());
    assert!(scan(&sector_image(&corrupt, 0), &[]).files.is_empty());
}
#[test]
fn rtf_binary_bytes_and_escaped_braces_do_not_change_group_boundaries() {
    let payload = b"{\\rtf1 escaped \\{ \\} \\'7b {nested} \\bin3 }{} end}";
    let image = sector_image(payload, 77);
    let result = scan(&image, &[]);
    assert_eq!(result.files.len(), 1);
    assert_eq!(file_bytes(&image, &result.files[0]).unwrap(), payload);
    for malformed in [
        b"{\\rtf1 no end".as_slice(),
        b"{\\rtf10 nope}",
        b"{\\rtf1 \\'zz}",
        b"{\\rtf1 \\bin-1 x}",
    ] {
        assert!(scan(&sector_image(malformed, 0), &[]).files.is_empty());
    }
}
#[test]
fn bad_sector_holes_never_become_valid_candidate_bytes() {
    let mut payload = b"{\\rtf1 ".to_vec();
    payload.extend(vec![b'x'; 1500]);
    payload.push(b'}');
    let image = sector_image(&payload, 0);
    assert_eq!(scan(&image, &[]).files.len(), 1);
    assert!(scan(&image, &[1]).files.is_empty());
    let mut severe = vec![0; 512 * 900];
    let good = b"{\\rtf1 survivor}";
    severe[800 * 512..800 * 512 + good.len()].copy_from_slice(good);
    let bad = (0..400).collect::<Vec<u64>>();
    let result = scan(&severe, &bad);
    assert_eq!(result.files.len(), 1);
    assert_eq!(file_bytes(&severe, &result.files[0]).unwrap(), good);
}
#[test]
fn known_hashes_and_duplicate_candidates_do_not_emit_duplicate_payloads() {
    let payload = b"{\\rtf1 one}";
    let mut image = sector_image(payload, 3);
    image[512..512 + payload.len()].copy_from_slice(payload);
    let result = scan(&image, &[]);
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.rejected.len(), 1);
    let result = analyze(
        &image,
        &[],
        &raw_readable_regions(&image, &[]),
        &[hash(payload)],
    )
    .unwrap();
    assert!(result.files.is_empty());
    assert_eq!(result.rejected.len(), 2);
}
#[test]
fn invalid_regions_maps_and_tampered_extents_are_refused() {
    let image = sector_image(b"{\\rtf1 good}", 0);
    assert!(analyze(&image, &[0, 0], &[], &[]).is_err());
    assert!(analyze(&image, &[999], &[], &[]).is_err());
    let mut regions = raw_readable_regions(&image, &[]);
    regions[0].lbas.push(0);
    assert!(analyze(&image, &[], &regions, &[]).is_err());
    let mut file = scan(&image, &[]).files.remove(0);
    file.source_extents[0].source_byte_offset += 1;
    assert!(file_bytes(&image, &file).is_err());
}
#[test]
fn false_signature_flood_and_validation_work_have_explicit_ceilings() {
    let image = b"BM".repeat(2048);
    let result = scan(&image, &[]);
    assert!(result.limits_reached);
    assert_eq!(result.probes, MAX_PROBES);
    let mut work = 0;
    assert!(charge(&mut work, MAX_VALIDATION_WORK + 1).is_err());
}
#[test]
fn truncated_and_mutated_headers_do_not_panic_or_export_incomplete_containers() {
    let payloads = [
        image_payload(image::ImageFormat::Png),
        image_payload(image::ImageFormat::Jpeg),
        image_payload(image::ImageFormat::Gif),
        image_payload(image::ImageFormat::Bmp),
        zip_payload("a.txt", b"hello"),
        compound_payload(),
    ];
    for payload in &payloads {
        for len in [1, 8, 16, 32, 64] {
            let n = len.min(payload.len() - 1);
            if let Some(format) = signature(&payload[..n]) {
                assert!(
                    validate(&payload[..n], format, &mut 0).is_err(),
                    "{format} truncated at {n}"
                );
            }
        }
        for index in (8..payload.len().min(512)).step_by(17) {
            let mut mutation = payload.clone();
            mutation[index] ^= 0xff;
            let _ = scan(&sector_image(&mutation, 0), &[]);
        }
    }
}
