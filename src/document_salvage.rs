//! Saved-image Word text salvage, NOT guessed bytes or a repaired DOC container.
//! MS-DOC 2.4.1: readable FIB/CLX maps character positions to WordDocument bytes.
use crate::{carving::Extent, fat12, fat12_recovery::RecoveryReport};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_CHARACTERS: usize = 200_000;
const MAX_SEGMENTS: usize = 4096;
const MAX_WORK: usize = 64 * 1024 * 1024;
pub const ENGINE_VERSION: u32 = 2;
#[path = "document_cfb.rs"]
mod cfb_recovery;
pub use cfb_recovery::Evidence as CfbEvidence;
#[path = "document_recovery_html.rs"]
mod readable_html;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadableEdition {
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
    pub repaired_original: bool,
    #[serde(skip)]
    html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextSegment {
    pub path: String,
    pub cp_start: usize,
    pub cp_count: usize,
    pub word_stream_offset: usize,
    pub encoded_bytes: usize,
    pub encoding: String,
    pub bytes: usize,
    pub sha256: String,
    pub source_extents: Vec<Extent>,
    #[serde(skip)]
    text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextGap {
    pub cp_start: usize,
    pub cp_count: usize,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub parent: fat12::FileRecord,
    pub status: String,
    pub refusal: Option<String>,
    pub main_characters: Option<usize>,
    pub table_stream: Option<String>,
    pub metadata_lbas: Vec<u64>,
    pub segments: Vec<TextSegment>,
    pub missing_text: Vec<TextGap>,
    pub repaired_original: bool,
    pub customer_delivery_certified: bool,
    #[serde(default)]
    pub cfb_recovery: Option<CfbEvidence>,
    #[serde(default)]
    pub strict_parser_refusal: Option<String>,
    #[serde(default)]
    pub readable_edition: Option<ReadableEdition>,
    #[serde(default)]
    pub edition_refusal: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Report {
    schema_version: u32,
    engine_version: u32,
    disk: u32,
    attempt: u32,
    source_image: String,
    source_sha256: String,
    bad_lbas: Vec<u64>,
    documents: Vec<Document>,
    limits_reached: bool,
    warning: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct SalvageResult {
    pub engine_version: u32,
    pub selectively_mapped_documents: usize,
    pub readable_editions: usize,
    pub readable_edition_paths: Vec<PathBuf>,
    pub output_directory: PathBuf,
    pub report_path: PathBuf,
    pub documents_examined: usize,
    pub documents_with_text: usize,
    pub text_segments: usize,
    pub recovered_character_positions: usize,
    pub missing_character_positions: usize,
    pub refused_documents: usize,
    pub limits_reached: bool,
    pub repaired_originals: usize,
    pub reused: bool,
}

// The CFB parser first sees only known bytes, so damaged allocation/directory
// metadata is never interpreted from placeholder zeros. Its subsequent bulk
// stream reads are traced, and placeholder bytes carry a None source mapping.
// No unknown byte is decoded/exported; trace length/content must match the
// returned stream exactly, otherwise that document is refused.
struct Trace {
    allow_holes: bool,
    reads: Vec<(usize, usize)>,
    work: usize,
}
struct Reader {
    bytes: Vec<u8>,
    sources: Vec<Option<usize>>,
    position: usize,
    trace: Arc<Mutex<Trace>>,
}
impl Read for Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out
            .len()
            .min(self.bytes.len().saturating_sub(self.position));
        let mut t = self.trace.lock().unwrap();
        t.work = t.work.saturating_add(n);
        if t.work > MAX_WORK || t.reads.len() > 32_768 {
            return Err(io::Error::other("Document read-work ceiling"));
        }
        let end = self.position + n;
        if !t.allow_holes && self.sources[self.position..end].iter().any(Option::is_none) {
            return Err(io::Error::other(
                "CFB metadata touches missing/unmapped acquisition bytes",
            ));
        }
        out[..n].copy_from_slice(&self.bytes[self.position..end]);
        if n > 0 {
            t.reads.push((self.position, n));
        }
        self.position = end;
        Ok(n)
    }
}
impl Seek for Reader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let n = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        if n < 0 || n > self.bytes.len() as i128 {
            return Err(io::Error::other("Document seek outside declared parent"));
        }
        self.position = n as usize;
        Ok(n as u64)
    }
}
struct SparseStream {
    bytes: Vec<u8>,
    sources: Vec<Option<usize>>,
}
fn stream(
    compound: &mut cfb::CompoundFile<Reader>,
    name: &str,
    trace: &Arc<Mutex<Trace>>,
    parent_bytes: &[u8],
    parent_sources: &[Option<usize>],
) -> Result<SparseStream, String> {
    let length = compound.entry(name).map_err(|e| e.to_string())?.len();
    if length > fat12::MAX_IMAGE_BYTES as u64 {
        return Err("Compound stream exceeds recovery bound".into());
    }
    trace.lock().unwrap().reads.clear();
    let mut bytes = Vec::new();
    compound
        .open_stream(name)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut replay = Vec::new();
    let mut sources = Vec::new();
    for &(at, n) in &trace.lock().unwrap().reads {
        replay.extend_from_slice(&parent_bytes[at..at + n]);
        sources.extend_from_slice(&parent_sources[at..at + n]);
    }
    if bytes.len() != length as usize || replay != bytes || sources.len() != bytes.len() {
        return Err(
            "CFB stream byte trace does not match returned data; no inferred mapping".into(),
        );
    }
    Ok(SparseStream { bytes, sources })
}
fn known(stream: &SparseStream, at: usize, n: usize) -> Result<&[u8], String> {
    let end = at.checked_add(n).ok_or("Document range overflow")?;
    let bytes = stream
        .bytes
        .get(at..end)
        .ok_or("Document range outside stream")?;
    if stream.sources[at..end].iter().any(Option::is_none) {
        return Err("Required Word metadata intersects missing/unmapped bytes".into());
    }
    Ok(bytes)
}
fn u16at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn u32at(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}
fn source_lbas(s: &SparseStream, at: usize, n: usize) -> Vec<u64> {
    s.sources[at..at + n]
        .iter()
        .flatten()
        .map(|b| (*b / 512) as u64)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn compressed_character(b: u8) -> char {
    let mapped = match b {
        0x82 => 0x201a,
        0x83 => 0x0192,
        0x84 => 0x201e,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02c6,
        0x89 => 0x2030,
        0x8a => 0x0160,
        0x8b => 0x2039,
        0x8c => 0x0152,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201c,
        0x94 => 0x201d,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02dc,
        0x99 => 0x2122,
        0x9a => 0x0161,
        0x9b => 0x203a,
        0x9c => 0x0153,
        0x9f => 0x0178,
        _ => u32::from(b),
    };
    char::from_u32(mapped).unwrap()
}
fn extents(sources: &[Option<usize>]) -> Vec<Extent> {
    let mut result: Vec<Extent> = vec![];
    for (offset, source) in sources.iter().enumerate() {
        let source = source.unwrap();
        if let Some(last) = result.last_mut()
            && last.source_byte_offset + last.bytes == source
        {
            last.bytes += 1;
        } else {
            result.push(Extent {
                file_offset: offset,
                source_byte_offset: source,
                bytes: 1,
            });
        }
    }
    result
}
fn gap(gaps: &mut Vec<TextGap>, cp: usize, count: usize, reason: &str) {
    if let Some(last) = gaps.last_mut()
        && last.cp_start + last.cp_count == cp
        && last.reason == reason
    {
        last.cp_count += count;
    } else {
        gaps.push(TextGap {
            cp_start: cp,
            cp_count: count,
            reason: reason.into(),
        });
    }
}

fn parent_mapping(
    image: &[u8],
    parent: &fat12::FileRecord,
    bad: &BTreeSet<u64>,
) -> Result<SparseStream, String> {
    if parent.bytes > fat12::MAX_IMAGE_BYTES || parent.bytes < 512 {
        return Err("Unsupported document size".into());
    }
    let mut bytes = vec![0; parent.bytes];
    let mut sources = vec![None; parent.bytes];
    for (index, lba) in parent.data_lbas.iter().enumerate() {
        if index * 512 >= parent.bytes {
            break;
        }
        if *lba >= (image.len() / 512) as u64 {
            return Err("Document source sector out of range".into());
        }
        if bad.contains(lba) {
            continue;
        }
        let n = (parent.bytes - index * 512).min(512);
        let at = *lba as usize * 512;
        bytes[index * 512..index * 512 + n].copy_from_slice(&image[at..at + n]);
        for j in 0..n {
            sources[index * 512 + j] = Some(at + j);
        }
    }
    if bytes[..8] != [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]
        || sources[..512].iter().any(Option::is_none)
    {
        return Err("No readable supported compound-document header; legacy/non-OLE Word remains unresolved".into());
    }
    Ok(SparseStream { bytes, sources })
}
fn extract(
    image: &[u8],
    parent: &fat12::FileRecord,
    bad: &BTreeSet<u64>,
) -> Result<Document, String> {
    let mapping = parent_mapping(image, parent, bad)?;
    match extract_strict(&mapping.bytes, &mapping.sources, parent) {
        Ok(d) => Ok(d),
        Err(strict) => {
            let (word, table, mut metadata, evidence) =
                cfb_recovery::streams(&mapping.bytes, &mapping.sources)
                    .map_err(|e| format!("{strict}; selective CFB refusal: {e}"))?;
            metadata.extend(parent.metadata_lbas.iter().copied());
            let mut doc = decode_streams(parent, &word, &table, metadata)
                .map_err(|e| format!("{strict}; selective Word refusal: {e}"))?;
            doc.cfb_recovery = Some(evidence);
            doc.strict_parser_refusal = Some(strict);
            Ok(doc)
        }
    }
}
fn extract_strict(
    bytes: &[u8],
    sources: &[Option<usize>],
    parent: &fat12::FileRecord,
) -> Result<Document, String> {
    let trace = Arc::new(Mutex::new(Trace {
        allow_holes: false,
        reads: vec![],
        work: 0,
    }));
    let reader = Reader {
        bytes: bytes.to_vec(),
        sources: sources.to_vec(),
        position: 0,
        trace: trace.clone(),
    };
    let mut compound = cfb::OpenOptions::new()
        .strict()
        .max_buffer_size(8192)
        .open_with(reader)
        .map_err(|e| format!("Compound metadata refusal: {e}"))?;
    let mut metadata = parent
        .metadata_lbas
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for &(at, n) in &trace.lock().unwrap().reads {
        metadata.extend(
            sources[at..at + n]
                .iter()
                .flatten()
                .map(|b| (*b / 512) as u64),
        );
    }
    trace.lock().unwrap().allow_holes = true;
    let word = stream(&mut compound, "/WordDocument", &trace, &bytes, &sources)?;
    let fib = known(&word, 0, 426)?;
    let name = if u16at(fib, 10) & 0x200 != 0 {
        "/1Table"
    } else {
        "/0Table"
    };
    let table = stream(&mut compound, name, &trace, bytes, sources)?;
    decode_streams(parent, &word, &table, metadata)
}
fn decode_streams(
    parent: &fat12::FileRecord,
    word: &SparseStream,
    table: &SparseStream,
    mut metadata: BTreeSet<u64>,
) -> Result<Document, String> {
    let fib = known(&word, 0, 426)?;
    let version = u16at(fib, 2);
    let flags = u16at(fib, 10);
    let base_expected = match version {
        0xc1 => 0x5d,
        0xd9 => 0x6c,
        0x101 => 0x88,
        0x10c => 0xa4,
        0x112 => 0xb7,
        _ => return Err("Unsupported Word FIB version; not guessed".into()),
    };
    if u16at(fib, 0) != 0xa5ec
        || flags & 0x0100 != 0
        || u16at(fib, 32) != 14
        || u16at(fib, 62) != 22
    {
        return Err("Unsupported, encrypted or inconsistent Word FIB".into());
    }
    let pairs = u16at(fib, 152) as usize;
    if ![0x5d, 0x6c, 0x88, 0xa4, 0xb7].contains(&pairs) {
        return Err("Unsupported Word FIB field-count version".into());
    }
    let new_at = 154 + pairs * 8;
    let new_count = u16at(known(&word, new_at, 2)?, 0) as usize;
    if new_count > 5 {
        return Err("Unsupported Word extended FIB count".into());
    }
    let new_words = known(&word, new_at + 2, new_count * 2)?;
    let valid_new = match new_count {
        0 => pairs == base_expected,
        2 => matches!(
            (u16at(new_words, 0), pairs),
            (0xd9, 0x6c) | (0x101, 0x88) | (0x10c, 0xa4)
        ),
        5 => u16at(new_words, 0) == 0x112 && pairs == 0xb7,
        _ => false,
    };
    if !valid_new {
        return Err("Inconsistent Word nFibNew/field-count binding".into());
    }
    let fib_end = new_at + 2 + new_words.len();
    let cb_mac = u32at(fib, 64);
    let main = u32at(fib, 76);
    if main == 0 || main > MAX_CHARACTERS || cb_mac > word.bytes.len() || cb_mac < 426 {
        return Err("Word main-text/meaningful-byte ceiling or invalid size".into());
    }
    let name = if flags & 0x200 != 0 {
        "/1Table"
    } else {
        "/0Table"
    };
    let clx_at = u32at(fib, 418);
    let clx_len = u32at(fib, 422);
    if clx_len > 64 * 1024 || clx_len < 5 {
        return Err("Word CLX ceiling/invalid length".into());
    }
    let clx = known(&table, clx_at, clx_len)?;
    metadata.extend(source_lbas(&word, 0, 426));
    metadata.extend(source_lbas(&word, new_at, 2 + new_words.len()));
    metadata.extend(source_lbas(&table, clx_at, clx_len));
    let mut at = 0;
    while clx.get(at) == Some(&1) {
        let header = clx.get(at..at + 3).ok_or("Truncated Word Prc")?;
        at = at
            .checked_add(3 + u16at(header, 1) as usize)
            .ok_or("Word Prc overflow")?;
        if at > clx.len() {
            return Err("Word Prc crosses CLX".into());
        }
    }
    let header = clx.get(at..at + 5).ok_or("Missing Word piece table")?;
    let n = u32at(header, 1);
    if header[0] != 2 || n < 4 || (n - 4) % 12 != 0 || at + 5 + n != clx.len() {
        return Err("Invalid Word piece-table envelope".into());
    }
    let plc = &clx[at + 5..];
    let pieces = (n - 4) / 12;
    if pieces == 0 || pieces > 4096 || u32at(plc, 0) != 0 {
        return Err("Word piece-table count/start refused".into());
    }
    let cps = (0..=pieces).map(|i| u32at(plc, i * 4)).collect::<Vec<_>>();
    if cps.windows(2).any(|w| w[0] >= w[1])
        || cps[pieces] < main
        || cps[pieces] > MAX_CHARACTERS * 4
    {
        return Err("Word character-position sequence/range refused".into());
    }
    let mut doc = Document {
        parent: parent.clone(),
        status: "text_salvaged_original_still_incomplete".into(),
        refusal: None,
        main_characters: Some(main),
        table_stream: Some(name.into()),
        metadata_lbas: metadata.into_iter().collect(),
        segments: vec![],
        missing_text: vec![],
        repaired_original: false,
        customer_delivery_certified: false,
        cfb_recovery: None,
        strict_parser_refusal: None,
        readable_edition: None,
        edition_refusal: None,
    };
    for i in 0..pieces {
        if cps[i] >= main {
            break;
        }
        let pcd = (pieces + 1) * 4 + i * 8;
        if u16at(plc, pcd) & 4 != 0 {
            return Err("Dirty Word piece descriptor refused".into());
        }
        let fc = u32at(plc, pcd + 2);
        let compressed = fc & 0x4000_0000 != 0;
        // MS-DOC FcCompressed: the reserved high bit must be ignored by readers.
        let fc = fc & 0x3fff_ffff;
        let width = if compressed { 1 } else { 2 };
        if compressed && fc % 2 != 0 {
            return Err("Invalid compressed Word offset".into());
        }
        let begin = if compressed { fc / 2 } else { fc };
        let end_cp = cps[i + 1].min(main);
        let length = (end_cp - cps[i]) * width;
        if begin < fib_end || begin.checked_add(length).is_none_or(|end| end > cb_mac) {
            return Err("Word piece crosses meaningful stream bounds".into());
        }
        let mut cursor = cps[i];
        while cursor < end_cp {
            let offset = begin + (cursor - cps[i]) * width;
            if word.sources[offset..offset + width]
                .iter()
                .any(Option::is_none)
            {
                gap(
                    &mut doc.missing_text,
                    cursor,
                    1,
                    "acquisition_unreadable_or_allocation_unmapped",
                );
                cursor += 1;
                continue;
            }
            let first = u16::from(word.bytes[offset]);
            let (character, count) = if compressed {
                (Some(compressed_character(first as u8)), 1)
            } else {
                let unit = u16at(&word.bytes, offset);
                if (0xd800..=0xdbff).contains(&unit) {
                    if cursor + 1 < end_cp
                        && word.sources[offset + 2..offset + 4]
                            .iter()
                            .all(Option::is_some)
                    {
                        let low = u16at(&word.bytes, offset + 2);
                        if (0xdc00..=0xdfff).contains(&low) {
                            (
                                char::from_u32(
                                    0x10000
                                        + ((u32::from(unit) - 0xd800) << 10)
                                        + (u32::from(low) - 0xdc00),
                                ),
                                2,
                            )
                        } else {
                            (None, 1)
                        }
                    } else {
                        (None, 1)
                    }
                } else {
                    (char::from_u32(u32::from(unit)), 1)
                }
            };
            let Some(character) = character else {
                gap(
                    &mut doc.missing_text,
                    cursor,
                    1,
                    "invalid_or_split_UTF16_surrogate_not_replaced",
                );
                cursor += 1;
                continue;
            };
            // Group only contiguous CP/stream bytes with the same encoding;
            // UTF-16 pairs never bridge a missing byte or a piece boundary.
            let encoding = if compressed {
                "MS-DOC compressed Unicode"
            } else {
                "UTF-16LE"
            };
            let append = doc.segments.last().is_some_and(|s| {
                s.cp_start + s.cp_count == cursor
                    && s.word_stream_offset + s.encoded_bytes == offset
                    && s.encoding == encoding
            });
            if !append {
                if doc.segments.len() >= MAX_SEGMENTS {
                    return Err("Word text-segment ceiling reached".into());
                }
                doc.segments.push(TextSegment {
                    path: String::new(),
                    cp_start: cursor,
                    cp_count: 0,
                    word_stream_offset: offset,
                    encoded_bytes: 0,
                    encoding: encoding.into(),
                    bytes: 0,
                    sha256: String::new(),
                    source_extents: vec![],
                    text: String::new(),
                });
            }
            let s = doc.segments.last_mut().unwrap();
            s.text.push(character);
            s.cp_count += count;
            s.encoded_bytes += count * width;
            cursor += count;
        }
    }
    for s in &mut doc.segments {
        s.source_extents =
            extents(&word.sources[s.word_stream_offset..s.word_stream_offset + s.encoded_bytes]);
        s.bytes = s.text.len();
        s.sha256 = hash(s.text.as_bytes());
        s.path = format!(
            "word_{:08x}_cp_{:08x}_{}.txt",
            parent.directory_entry_offset,
            s.cp_start,
            &s.sha256[..12]
        );
    }
    if doc.segments.is_empty() {
        doc.status = "no_readable_main_text".into();
    }
    Ok(doc)
}

pub(crate) fn preserve(
    image: &[u8],
    source: &Path,
    recovery_disk: &Path,
    native: &RecoveryReport,
) -> Result<Option<SalvageResult>, String> {
    let Some(a) = &native.analysis else {
        return Ok(None);
    };
    if native.deleted.is_some() {
        return Ok(None);
    }
    let bad = a.bad_lbas.iter().copied().collect::<BTreeSet<_>>();
    let mut documents = vec![];
    let mut limits = false;
    let mut positions = 0;
    let mut edition_bytes = 0;
    for parent in &a.unrecovered_files {
        if !fat12::partial_file_safe(parent, a) {
            continue;
        }
        let name = parent.record.path.to_ascii_lowercase();
        if !name.ends_with(".doc") && !name.ends_with(".dot") {
            continue;
        }
        if documents.len() >= 64 || positions >= 500_000 {
            limits = true;
            break;
        }
        let mut d = match extract(image, &parent.record, &bad) {
            Ok(doc) => doc,
            Err(e) => Document {
                parent: parent.record.clone(),
                status: "refused_preserved_as_raw_fragments".into(),
                refusal: Some(e),
                main_characters: None,
                table_stream: None,
                metadata_lbas: parent.record.metadata_lbas.clone(),
                segments: vec![],
                missing_text: vec![],
                repaired_original: false,
                customer_delivery_certified: false,
                cfb_recovery: None,
                strict_parser_refusal: None,
                readable_edition: None,
                edition_refusal: None,
            },
        };
        if positions + d.main_characters.unwrap_or(0) > 500_000 {
            limits = true;
            break;
        }
        positions += d.main_characters.unwrap_or(0);
        if !d.segments.is_empty() {
            match readable_html::render(&d, &native.source_image, &native.source_sha256) {
                Ok(html) if edition_bytes + html.len() <= fat12::MAX_IMAGE_BYTES * 2 => {
                    edition_bytes += html.len();
                    let sha = hash(html.as_bytes());
                    d.readable_edition = Some(ReadableEdition {
                        path: format!(
                            "word_{:08x}_readable_{}.html",
                            d.parent.directory_entry_offset,
                            &sha[..12]
                        ),
                        bytes: html.len(),
                        sha256: sha,
                        repaired_original: false,
                        html,
                    });
                }
                Ok(_) => {
                    limits = true;
                    d.edition_refusal =
                        Some("Combined readable edition size ceiling; raw text retained".into());
                }
                Err(e) => {
                    limits = true;
                    d.edition_refusal = Some(e);
                }
            }
        }
        documents.push(d);
    }
    if documents.is_empty() {
        return Ok(None);
    }
    let report=Report{schema_version:1,engine_version:ENGINE_VERSION,disk:native.disk,attempt:native.attempt,source_image:native.source_image.clone(),source_sha256:native.source_sha256.clone(),bad_lbas:a.bad_lbas.clone(),documents,limits_reached:limits,
        warning:"FORENSIC TEXT SALVAGE, NOT A REPAIRED DOC OR COMPLETE DOCUMENT. Only readable main-document characters with validated CFB/FIB/CLX mappings are decoded to separate UTF-8 segments. Character-position gaps remain explicit in text reports and generated offline HTML editions; segments are not silently concatenated, unknown bytes never become text, and invalid UTF-16 is not replaced. Formatting, tables, revisions, headers/footnotes, objects, field interpretation and original semantics are not recovered/certified. Encrypted/unsupported documents or missing required metadata are refused. Selective mapping may bypass unrelated missing directory entries, but never certifies the entire compound container. No dictionary/brute-force guessing; original images/native results remain unchanged. Text/editions are excluded from normal whole-file counts, conversion and delivery.".into()};
    let serialized = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    if serialized.len() > 16 * 1024 * 1024 {
        return Err("Document provenance ceiling; no publication".into());
    }
    let output = recovery_disk.join(format!(
        "attempt_{:03}_word_text_v{ENGINE_VERSION}",
        native.attempt
    ));
    let report_path = output.join("word-text.json");
    let segments = report
        .documents
        .iter()
        .flat_map(|d| &d.segments)
        .collect::<Vec<_>>();
    let reused = output.exists();
    let editions = report
        .documents
        .iter()
        .filter_map(|d| d.readable_edition.as_ref())
        .collect::<Vec<_>>();
    if reused {
        if !fs::symlink_metadata(&output)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_dir()
            || output.canonicalize().map_err(|e| e.to_string())?.parent() != Some(recovery_disk)
        {
            return Err("Document salvage directory escapes Recovery; preserved".into());
        }
        if regular_read(&report_path)? != serialized
            || fs::read_dir(&output).map_err(|e| e.to_string())?.count()
                != segments.len() + editions.len() + 1
        {
            return Err("Document salvage report/inventory changed; preserved".into());
        }
        for s in &segments {
            if regular_read(&output.join(&s.path))? != s.text.as_bytes() {
                return Err("Document salvage text changed; preserved".into());
            }
        }
        for e in &editions {
            if regular_read(&output.join(&e.path))? != e.html.as_bytes() {
                return Err("Document readable edition changed; preserved".into());
            }
        }
    } else {
        let staging = recovery_disk.join(format!(
            ".tmp-word-text-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms()
        ));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        for s in &segments {
            write_new(&staging.join(&s.path), s.text.as_bytes())?;
        }
        for e in &editions {
            write_new(&staging.join(&e.path), e.html.as_bytes())?;
        }
        write_new(&staging.join("word-text.json"), &serialized)?;
        if hash(&regular_read(source)?) != native.source_sha256 {
            return Err("Source changed; document salvage staging retained".into());
        }
        if output.exists() {
            return Err("Document salvage destination appeared; staging retained".into());
        }
        fs::rename(&staging, &output)
            .map_err(|e| format!("Document salvage staging retained: {e}"))?;
    }
    Ok(Some(SalvageResult {
        engine_version: ENGINE_VERSION,
        selectively_mapped_documents: report
            .documents
            .iter()
            .filter(|d| d.cfb_recovery.is_some())
            .count(),
        readable_editions: editions.len(),
        readable_edition_paths: editions.iter().map(|e| output.join(&e.path)).collect(),
        output_directory: output,
        report_path,
        documents_examined: report.documents.len(),
        documents_with_text: report
            .documents
            .iter()
            .filter(|d| !d.segments.is_empty())
            .count(),
        text_segments: segments.len(),
        recovered_character_positions: segments.iter().map(|s| s.cp_count).sum(),
        missing_character_positions: report
            .documents
            .iter()
            .flat_map(|d| &d.missing_text)
            .map(|g| g.cp_count)
            .sum(),
        refused_documents: report
            .documents
            .iter()
            .filter(|d| d.refusal.is_some())
            .count(),
        limits_reached: limits,
        repaired_originals: 0,
        reused,
    }))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn regular_read(path: &Path) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Document evidence is not a regular file".into());
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Document evidence size ceiling".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "document_salvage_tests.rs"]
mod tests;
