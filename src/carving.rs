//! Bounded signature recovery from saved, acquisition-reported readable bytes.
//! Names are reconstructed; structural validity never certifies original custody/content.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
};

#[cfg(test)]
#[path = "carving_tests.rs"]
mod tests;

const MAX_PROBES: usize = 1024;
const MAX_FILES: usize = 256;
const MAX_EXPANDED: u64 = 32 * 1024 * 1024;
const MAX_VALIDATION_WORK: u64 = 128 * 1024 * 1024;

fn charge(work: &mut u64, bytes: u64) -> Result<(), String> {
    *work = work.saturating_add(bytes);
    if *work > MAX_VALIDATION_WORK {
        Err("Aggregate validation work ceiling exceeded".into())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Region {
    pub lbas: Vec<u64>,
    pub method: String,
    pub allocation_scope: String,
    pub fat_links: Vec<crate::fat12::FatLink>,
    pub metadata_lbas: Vec<u64>,
    pub byte_length: Option<usize>,
    pub parent_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extent {
    pub file_offset: usize,
    pub source_byte_offset: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarvedFile {
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
    pub format: String,
    pub validation: String,
    pub recovery_method: String,
    pub allocation_scope: String,
    pub source_extents: Vec<Extent>,
    #[serde(default)]
    pub fat_links: Vec<crate::fat12::FatLink>,
    #[serde(default)]
    pub metadata_lbas: Vec<u64>,
    #[serde(default)]
    pub parent_file: Option<String>,
    pub original_name_known: bool,
    pub customer_delivery_certified: bool,
    #[serde(default)]
    pub missing_fat_link_cluster: Option<u16>,
    #[serde(default)]
    pub candidate_tail_start_cluster: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub source_byte_offset: usize,
    pub format: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub files: Vec<CarvedFile>,
    pub rejected: Vec<RejectedCandidate>,
    pub bad_lbas: Vec<u64>,
    pub probes: usize,
    pub scanned_bytes: usize,
    pub limits_reached: bool,
    pub allocation_warning: Option<String>,
    #[serde(default)]
    pub orphan_chains_scanned: usize,
    #[serde(default)]
    pub allocation_issues: Vec<String>,
    #[serde(default)]
    pub fragmented_suffix_trials: usize,
    #[serde(default)]
    pub fragmented_parents_with_candidates: usize,
    #[serde(default)]
    pub ambiguous_fragmented_parents: usize,
}

pub(crate) fn raw_readable_regions(image: &[u8], bad: &[u64]) -> Vec<Region> {
    let bad = bad.iter().copied().collect::<BTreeSet<_>>();
    let mut regions: Vec<Region> = vec![];
    for lba in 0..image.len() / 512 {
        if bad.contains(&(lba as u64)) {
            continue;
        }
        if let Some(last) = regions.last_mut()
            && last
                .lbas
                .last()
                .is_some_and(|previous| *previous + 1 == lba as u64)
        {
            last.lbas.push(lba as u64);
        } else {
            regions.push(Region {
                lbas: vec![lba as u64],
                method: "contiguous_readable_signature".into(),
                allocation_scope: "unknown_filesystem_live_or_deleted_unknown".into(),
                fat_links: vec![],
                metadata_lbas: vec![],
                byte_length: None,
                parent_file: None,
            });
        }
    }
    regions
}

pub(crate) fn analyze(
    image: &[u8],
    bad: &[u64],
    regions: &[Region],
    known_hashes: &[String],
) -> Result<Analysis, String> {
    analyze_with_work(image, bad, regions, known_hashes, &mut 0)
}

pub(crate) fn analyze_with_work(
    image: &[u8],
    bad: &[u64],
    regions: &[Region],
    known_hashes: &[String],
    validation_work: &mut u64,
) -> Result<Analysis, String> {
    if image.len() > crate::fat12::MAX_IMAGE_BYTES || !image.len().is_multiple_of(512) {
        return Err("Carving requires a bounded complete sector image".into());
    }
    let bad_set = bad.iter().copied().collect::<BTreeSet<_>>();
    if bad_set.len() != bad.len() || bad_set.iter().any(|lba| *lba >= (image.len() / 512) as u64) {
        return Err("Invalid carving bad-sector map".into());
    }
    let mut result = Analysis {
        bad_lbas: bad.to_vec(),
        ..Analysis::default()
    };
    let mut hashes = known_hashes.iter().cloned().collect::<BTreeSet<_>>();
    let mut total_region_bytes = 0;
    let mut emitted_bytes = 0;
    for region in regions {
        let mut unique = BTreeSet::new();
        if region.lbas.iter().any(|lba| {
            *lba >= (image.len() / 512) as u64 || bad_set.contains(lba) || !unique.insert(*lba)
        }) {
            return Err("Carving region includes unknown, repeated or out-of-range bytes".into());
        }
        total_region_bytes += region.lbas.len() * 512;
        if total_region_bytes > crate::fat12::MAX_IMAGE_BYTES * 2 {
            return Err("Carving region work ceiling exceeded".into());
        }
        let mut bytes = region
            .lbas
            .iter()
            .flat_map(|lba| {
                image[*lba as usize * 512..(*lba as usize + 1) * 512]
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();
        if let Some(n) = region.byte_length {
            if n > bytes.len() {
                return Err("Carving region length exceeds mapped bytes".into());
            }
            bytes.truncate(n);
        }
        let mut at = 0;
        while at < bytes.len() {
            if result.probes >= MAX_PROBES
                || result.files.len() >= MAX_FILES
                || emitted_bytes >= crate::fat12::MAX_IMAGE_BYTES
            {
                result.limits_reached = true;
                break;
            }
            result.scanned_bytes += 1;
            let Some(format) = signature(&bytes[at..]) else {
                at += 1;
                continue;
            };
            result.probes += 1;
            let source_offset = region.lbas[at / 512] as usize * 512 + at % 512;
            match validate(&bytes[at..], format, validation_work) {
                Ok((length, extension, validation)) => {
                    if emitted_bytes + length > crate::fat12::MAX_IMAGE_BYTES {
                        result.limits_reached = true;
                        break;
                    }
                    let sha256 = hash(&bytes[at..at + length]);
                    if hashes.insert(sha256.clone()) {
                        emitted_bytes += length;
                        result.files.push(CarvedFile {
                            path: format!(
                                "SignatureRecovery/carved_{source_offset:08x}_{}.{}",
                                &sha256[..12],
                                extension
                            ),
                            bytes: length,
                            sha256,
                            format: format.into(),
                            validation,
                            recovery_method: region.method.clone(),
                            allocation_scope: region.allocation_scope.clone(),
                            source_extents: extents(&region.lbas, at, length),
                            original_name_known: false,
                            fat_links: region.fat_links.clone(),
                            metadata_lbas: region.metadata_lbas.clone(),
                            parent_file: region.parent_file.clone(),
                            customer_delivery_certified: false,
                            missing_fat_link_cluster: None,
                            candidate_tail_start_cluster: None,
                        });
                    } else {
                        result.rejected.push(RejectedCandidate { source_byte_offset: source_offset, format: format.into(), reason: "Duplicate of an already recovered payload; source offset retained, no redundant file exported".into() });
                    }
                    result.scanned_bytes += length.saturating_sub(1);
                    at += length;
                }
                Err(reason) => {
                    result.rejected.push(RejectedCandidate {
                        source_byte_offset: source_offset,
                        format: format.into(),
                        reason,
                    });
                    at += 1;
                    if *validation_work > MAX_VALIDATION_WORK {
                        result.limits_reached = true;
                        break;
                    }
                }
            }
        }
        if result.limits_reached {
            break;
        }
    }
    result.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn signature(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("jpeg")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"PK\x03\x04") {
        Some("zip")
    } else if bytes.starts_with(b"{\\rtf1") {
        Some("rtf")
    } else if bytes.starts_with(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1") {
        Some("compound_document")
    } else {
        None
    }
}

fn extents(lbas: &[u64], start: usize, length: usize) -> Vec<Extent> {
    let mut extents: Vec<Extent> = vec![];
    let mut offset = start;
    let end = start + length;
    while offset < end {
        let source = lbas[offset / 512] as usize * 512 + offset % 512;
        let n = (512 - offset % 512).min(end - offset);
        if let Some(last) = extents.last_mut()
            && last.source_byte_offset + last.bytes == source
        {
            last.bytes += n;
        } else {
            extents.push(Extent {
                file_offset: offset - start,
                source_byte_offset: source,
                bytes: n,
            });
        }
        offset += n;
    }
    extents
}

pub(crate) fn file_bytes(image: &[u8], file: &CarvedFile) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for e in &file.source_extents {
        if e.file_offset != out.len()
            || e.bytes == 0
            || e.source_byte_offset
                .checked_add(e.bytes)
                .is_none_or(|end| end > image.len())
        {
            return Err("Invalid carved-file source extents".into());
        }
        out.extend_from_slice(&image[e.source_byte_offset..e.source_byte_offset + e.bytes]);
    }
    if out.len() != file.bytes || hash(&out) != file.sha256 {
        return Err("Carved-file source/hash binding changed".into());
    }
    Ok(out)
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn u16le(bytes: &[u8], at: usize) -> Result<usize, String> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or("Truncated header")?
            .try_into()
            .unwrap(),
    ) as usize)
}
fn u32le(bytes: &[u8], at: usize) -> Result<usize, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or("Truncated header")?
            .try_into()
            .unwrap(),
    ) as usize)
}

fn validate(
    bytes: &[u8],
    format: &str,
    work: &mut u64,
) -> Result<(usize, &'static str, String), String> {
    charge(work, bytes.len() as u64)?;
    match format {
        "png" => validate_image(bytes, png_length(bytes)?, image::ImageFormat::Png, "png", work),
        "jpeg" => validate_image(bytes, jpeg_length(bytes)?, image::ImageFormat::Jpeg, "jpg", work),
        "bmp" => {
            let n = u32le(bytes, 2)?;
            if n < 26 || u32le(bytes,10)? >= n { return Err("Invalid BMP envelope".into()); }
            validate_image(bytes, n, image::ImageFormat::Bmp, "bmp", work)
        }
        "gif" => validate_image(bytes, gif_length(bytes)?, image::ImageFormat::Gif, "gif", work),
        "rtf" => Ok((rtf_length(bytes)?, "rtf", "Balanced RTF group/control/binary envelope; rendering and original text semantics unverified".into())),
        "zip" => validate_zip(bytes, work),
        "compound_document" => validate_compound(bytes, work),
        _ => Err("Unsupported signature".into()),
    }
}

fn validate_image(
    bytes: &[u8],
    n: usize,
    format: image::ImageFormat,
    extension: &'static str,
    work: &mut u64,
) -> Result<(usize, &'static str, String), String> {
    let bytes = bytes
        .get(..n)
        .ok_or("Image extends beyond its fully readable region")?;
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits.clone());
    let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("Invalid/oversized pixel dimensions".into());
    }
    let pixels = width as u64 * height as u64 * 4;
    if pixels > 64 * 1024 * 1024 {
        return Err("Pixel allocation ceiling exceeded".into());
    }
    charge(work, pixels)?;
    if format == image::ImageFormat::Gif {
        use image::{AnimationDecoder, ImageDecoder};
        let mut decoder =
            image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        decoder.set_limits(limits).map_err(|e| e.to_string())?;
        let mut count = 0;
        for frame in decoder.into_frames() {
            count += 1;
            if count > 64 {
                return Err("GIF frame ceiling exceeded".into());
            }
            charge(work, pixels)?;
            frame.map_err(|e| format!("GIF frame decode failed: {e}"))?;
        }
        if count == 0 {
            return Err("GIF has no decoded frames".into());
        }
        return Ok((
            n,
            extension,
            format!(
                "All {count} GIF frames decoded with bounded pixel allocation; original name/custody unverified"
            ),
        ));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| format!("Image decoder refused candidate: {e}"))?;
    if decoded.width() == 0 || decoded.height() == 0 {
        return Err("Empty decoded image".into());
    }
    Ok((
        n,
        extension,
        format!(
            "{}x{} pixels decoded with bounded allocation; original name/custody unverified",
            decoded.width(),
            decoded.height()
        ),
    ))
}

fn png_length(bytes: &[u8]) -> Result<usize, String> {
    let mut at = 8;
    for index in 0..4096 {
        let head = bytes
            .get(at..at + 8)
            .ok_or("PNG chunk/header truncated by readable-region boundary")?;
        let n = u32::from_be_bytes(head[..4].try_into().unwrap()) as usize;
        if index == 0 && (&head[4..] != b"IHDR" || n != 13) {
            return Err("PNG must begin with IHDR".into());
        }
        let chunk_start = at;
        at = at
            .checked_add(n)
            .and_then(|v| v.checked_add(12))
            .ok_or("PNG chunk overflow")?;
        if at > bytes.len() {
            return Err("PNG crosses a missing/unmapped byte boundary".into());
        }
        let crc = u32::from_be_bytes(bytes[at - 4..at].try_into().unwrap());
        if crc32fast::hash(&bytes[chunk_start + 4..at - 4]) != crc {
            return Err("PNG chunk CRC differs".into());
        }
        if &head[4..] == b"IEND" {
            if n != 0 {
                return Err("Invalid PNG IEND".into());
            }
            return Ok(at);
        }
    }
    Err("PNG chunk ceiling exceeded".into())
}

fn jpeg_length(bytes: &[u8]) -> Result<usize, String> {
    let mut at = 2;
    let mut scan = false;
    let mut saw_scan = false;
    while at < bytes.len() {
        if scan && bytes[at] != 0xff {
            at += 1;
            continue;
        }
        if bytes[at] != 0xff {
            return Err("Invalid JPEG marker boundary".into());
        }
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at).ok_or("Truncated JPEG marker")?;
        at += 1;
        if scan && (marker == 0 || (0xd0..=0xd7).contains(&marker)) {
            continue;
        }
        if marker == 0xd9 {
            return if saw_scan {
                Ok(at)
            } else {
                Err("JPEG has no image scan".into())
            };
        }
        if marker == 0xd8 || marker == 0 || (0xd0..=0xd7).contains(&marker) {
            return Err("Unexpected JPEG marker".into());
        }
        if marker == 1 {
            continue;
        }
        let n = u16::from_be_bytes(
            bytes
                .get(at..at + 2)
                .ok_or("JPEG segment truncated")?
                .try_into()
                .unwrap(),
        ) as usize;
        if n < 2 {
            return Err("Invalid JPEG segment length".into());
        }
        at = at.checked_add(n).ok_or("JPEG overflow")?;
        if at > bytes.len() {
            return Err("JPEG crosses a missing/unmapped byte boundary".into());
        }
        scan = marker == 0xda;
        saw_scan |= scan;
    }
    Err("JPEG missing complete EOI within readable region".into())
}

fn gif_length(bytes: &[u8]) -> Result<usize, String> {
    let flags = *bytes.get(10).ok_or("Truncated GIF header")?;
    let mut at = 13
        + if flags & 0x80 != 0 {
            3 * (1usize << ((flags & 7) + 1))
        } else {
            0
        };
    let mut frames = 0;
    for _ in 0..4096 {
        match *bytes
            .get(at)
            .ok_or("GIF crosses readable-region boundary")?
        {
            0x3b if frames > 0 => return Ok(at + 1),
            0x2c => {
                let local = *bytes.get(at + 9).ok_or("Truncated GIF image descriptor")?;
                at += 10
                    + if local & 0x80 != 0 {
                        3 * (1usize << ((local & 7) + 1))
                    } else {
                        0
                    };
                at += 1; // LZW code-size byte
                frames += 1;
            }
            0x21 => at += 2, // Extension introducer + label
            _ => return Err("Invalid GIF block".into()),
        }
        loop {
            let n = *bytes.get(at).ok_or("Truncated GIF sub-block")? as usize;
            at += 1;
            if n == 0 {
                break;
            }
            at += n;
            if at > bytes.len() {
                return Err("Truncated GIF data".into());
            }
        }
    }
    Err("GIF block ceiling exceeded".into())
}

fn rtf_length(bytes: &[u8]) -> Result<usize, String> {
    if !bytes.starts_with(b"{\\rtf1")
        || bytes
            .get(6)
            .is_none_or(|b| b.is_ascii_digit() || b.is_ascii_alphabetic())
    {
        return Err("Unsupported RTF version/control delimiter".into());
    }
    let mut depth = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'{' => {
                depth += 1;
                if depth > 256 {
                    return Err("RTF nesting ceiling exceeded".into());
                }
                at += 1;
            }
            b'}' => {
                depth = depth.checked_sub(1).ok_or("Unbalanced RTF")?;
                at += 1;
                if depth == 0 {
                    return Ok(at);
                }
            }
            b'\\' => {
                at += 1;
                let begin = at;
                if bytes.get(at) == Some(&b'\'') {
                    if bytes
                        .get(at + 1..at + 3)
                        .is_none_or(|x| !x.iter().all(u8::is_ascii_hexdigit))
                    {
                        return Err("Malformed RTF hex escape".into());
                    }
                    at += 3;
                    continue;
                }
                while bytes.get(at).is_some_and(u8::is_ascii_alphabetic) {
                    at += 1;
                }
                if begin == at {
                    at += 1;
                    continue;
                }
                let word = &bytes[begin..at];
                let number = at;
                if bytes.get(at) == Some(&b'-') {
                    at += 1;
                }
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                let argument = &bytes[number..at];
                if bytes.get(at) == Some(&b' ') {
                    at += 1;
                }
                if word == b"bin" {
                    let n = std::str::from_utf8(argument)
                        .ok()
                        .and_then(|s| s.parse::<usize>().ok())
                        .ok_or("Invalid RTF binary count")?;
                    at = at
                        .checked_add(n)
                        .filter(|end| *end <= bytes.len())
                        .ok_or("RTF binary bytes unavailable")?;
                }
            }
            _ => at += 1,
        }
    }
    Err("RTF closing group unavailable".into())
}

fn safe_zip_name(name: &str) -> bool {
    let trimmed = name.trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.len() > 1024
        || trimmed
            .chars()
            .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
    {
        return false;
    }
    let parts = trimmed.split('/').collect::<Vec<_>>();
    parts.len() <= 32
        && parts.iter().all(|p| {
            let stem = p.split('.').next().unwrap_or_default().to_ascii_uppercase();
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && !p.ends_with(['.', ' '])
                && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !["COM", "LPT"].iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|n| {
                        matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                    })
                })
        })
}

fn validate_zip(bytes: &[u8], work: &mut u64) -> Result<(usize, &'static str, String), String> {
    let mut attempts = 0;
    for at in 4..bytes.len().saturating_sub(21) {
        if !bytes[at..].starts_with(b"PK\x05\x06") {
            continue;
        }
        attempts += 1;
        if attempts > 32 {
            return Err("ZIP envelope ceiling exceeded".into());
        }
        if u16le(bytes, at + 4)? != 0 || u16le(bytes, at + 6)? != 0 {
            continue;
        }
        let length = at + 22 + u16le(bytes, at + 20)?;
        let Some(candidate) = bytes.get(..length) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(candidate)) else {
            continue;
        };
        if zip.is_empty() || zip.len() > 4096 {
            continue;
        }
        let mut total = 0u64;
        let mut valid = true;
        let mut names = BTreeSet::new();
        for index in 0..zip.len() {
            let mut member = match zip.by_index(index) {
                Ok(m) => m,
                Err(_) => {
                    valid = false;
                    break;
                }
            };
            if member.enclosed_name().is_none()
                || !safe_zip_name(member.name())
                || member
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                || !names.insert(member.name().trim_end_matches('/').to_uppercase())
            {
                valid = false;
                break;
            }
            total = total.saturating_add(member.size());
            if total > MAX_EXPANDED {
                valid = false;
                break;
            }
            let expected = member.size();
            charge(work, expected)?;
            match std::io::copy(
                &mut member.by_ref().take(expected + 1),
                &mut std::io::sink(),
            ) {
                Ok(size) if size == expected => {}
                _ => {
                    valid = false;
                    break;
                }
            }
        }
        if valid {
            return Ok((length, "zip", "Every bounded ZIP member decompressed with CRC verification; no archive member extracted or executed".into()));
        }
    }
    Err("No complete, safe, CRC-readable ZIP within readable region".into())
}

fn validate_compound(
    bytes: &[u8],
    work: &mut u64,
) -> Result<(usize, &'static str, String), String> {
    let version = u16le(bytes, 26)?;
    let shift = u16le(bytes, 30)?;
    if !matches!((version, shift), (3, 9) | (4, 12))
        || u16le(bytes, 28)? != 0xfffe
        || u16le(bytes, 32)? != 6
    {
        return Err("Unsupported compound-file header".into());
    }
    let sector = 1usize << shift;
    let fats = u32le(bytes, 44)?;
    if fats == 0 || fats > 109 || u32le(bytes, 72)? != 0 {
        return Err("Compound-file FAT/DIFAT exceeds floppy recovery bounds".into());
    }
    let mut highest = 0;
    let mut fat_ids = BTreeSet::new();
    for i in 0..fats {
        let id = u32le(bytes, 76 + i * 4)?;
        if !fat_ids.insert(id) {
            return Err("Duplicate compound FAT sector".into());
        }
        let at = id
            .checked_add(1)
            .and_then(|v| v.checked_mul(sector))
            .ok_or("Compound sector overflow")?;
        let fat = bytes
            .get(at..at + sector)
            .ok_or("Compound FAT crosses unreadable-region boundary")?;
        highest = highest.max(id);
        for (entry, value) in fat.chunks_exact(4).enumerate() {
            if u32::from_le_bytes(value.try_into().unwrap()) != 0xffff_ffff {
                highest = highest.max(i * (sector / 4) + entry);
            }
        }
    }
    let n = highest
        .checked_add(2)
        .and_then(|v| v.checked_mul(sector))
        .ok_or("Compound size overflow")?;
    let candidate = bytes
        .get(..n)
        .ok_or("Compound streams cross a missing/unmapped boundary")?;
    let mut compound = cfb::OpenOptions::new()
        .strict()
        .max_buffer_size(8192)
        .open_with(Cursor::new(candidate))
        .map_err(|e| format!("Compound metadata validation: {e}"))?;
    let entries = compound
        .walk()
        .filter(|e| e.is_stream())
        .map(|e| (e.path().to_path_buf(), e.len()))
        .collect::<Vec<_>>();
    if entries.is_empty() || entries.len() > 4096 {
        return Err("Compound stream ceiling/empty document".into());
    }
    let mut total = 0u64;
    let mut word = false;
    let mut excel = false;
    for (path, length) in entries {
        total = total.saturating_add(length);
        if total > MAX_EXPANDED {
            return Err("Compound stream work ceiling exceeded".into());
        }
        charge(work, length)?;
        let mut stream = compound.open_stream(&path).map_err(|e| e.to_string())?;
        let mut prefix = vec![0; length.min(8) as usize];
        stream
            .read_exact(&mut prefix)
            .map_err(|e| format!("Compound stream truncated: {e}"))?;
        if path.parent() == Some(std::path::Path::new("/"))
            && path.file_name().is_some_and(|n| n == "WordDocument")
            && prefix.starts_with(b"\xec\xa5")
        {
            word = true;
        }
        if path.parent() == Some(std::path::Path::new("/"))
            && path
                .file_name()
                .is_some_and(|n| n == "Workbook" || n == "Book")
            && prefix.starts_with(b"\x09\x08")
        {
            excel = true;
        }
        let read = std::io::copy(
            &mut stream.take(length - prefix.len() as u64 + 1),
            &mut std::io::sink(),
        )
        .map_err(|e| e.to_string())?;
        if read + prefix.len() as u64 != length {
            return Err("Compound stream length differs".into());
        }
    }
    let extension = match (word, excel) {
        (true, false) => "doc",
        (false, true) => "xls",
        _ => "ole",
    };
    Ok((n, extension, "Strict compound directory/FAT/mini-FAT parse and all streams read; minimal allocated container extent reconstructed (trailing free padding/original length unknown); Office text/rendering semantics remain unverified".into()))
}
