//! Narrow Windows Word6 text decoding using the existing validated byte map.
//! Charset/locale fallback follows LibreOffice WW8Fib::GetFIBCharset:
//! https://docs.libreoffice.org/sw/html/ww8scan_8cxx_source.html
use super::*;

pub(super) fn decode(
    parent: &fat12::FileRecord,
    word: &SparseStream,
    mut metadata: BTreeSet<u64>,
) -> Result<Document, String> {
    let mapping = probe::legacy_word6_map(word)?;
    if mapping["character_width"] != 1 {
        return Err("Word6 wide/DBCS text remains unsupported; no encoding guessed".into());
    }
    let fib = known(word, 0, 88)?;
    let charset = u16at(fib, 20);
    let language = u16at(fib, 6);
    let (page, basis) = match (charset, language) {
        (238, _) => (1250, "explicit_FIB_EastEuropean_charset"),
        (0, 0x040e) => (1250, "FIB_Hungarian_locale_fallback"),
        (0, 0x0409 | 0x0809) => (1252, "FIB_English_locale_fallback"),
        _ => return Err("Unsupported Word6 charset/locale; no default code page guessed".into()),
    };
    let integer = |v: &serde_json::Value| -> Result<usize, String> {
        v.as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| "Invalid validated Word6 map integer".into())
    };
    for lba in mapping["metadata_lbas"]
        .as_array()
        .ok_or("Missing Word6 metadata map")?
    {
        metadata.insert(lba.as_u64().ok_or("Invalid Word6 metadata LBA")?);
    }
    let mut doc = Document {
        parent: parent.clone(),
        status: "text_salvaged_original_still_incomplete".into(),
        refusal: None,
        main_characters: Some(integer(&mapping["main_positions"])?),
        table_stream: Some("/WordDocument (Word6 embedded piece table)".into()),
        metadata_lbas: metadata.into_iter().collect(),
        segments: vec![], missing_text: vec![],
        repaired_original: false, customer_delivery_certified: false,
        cfb_recovery: None, strict_parser_refusal: None,
        readable_edition: None, edition_refusal: None, incomplete_fib: None,
        content_detection: None,
        legacy_text: Some(LegacyTextEvidence {
            fib_version: 0x65, fib_charset: charset, fib_language: language,
            code_page: page, encoding_basis: basis.into(), font_overrides_interpreted: false,
            warning: "Legacy Windows Word6 text-only interpretation. The code page comes from a supported FIB charset or recorded Hungarian/English locale fallback, not a recovered font-run map. Font-specific glyphs, formatting, fields, revisions and original meaning remain unverified. Missing or undefined bytes are explicit gaps; no original DOC is repaired.".into(),
        }),
    };
    for piece in mapping["pieces"].as_array().ok_or("Missing Word6 pieces")? {
        let cp = integer(&piece["cp_start"])?;
        let count = integer(&piece["cp_count"])?;
        let begin = integer(&piece["word_stream_offset"])?;
        let encoding = format!("Windows-{page}; {basis}; font runs unverified");
        for i in 0..count {
            let cursor = cp + i;
            let offset = begin + i;
            if word.sources[offset].is_none() {
                gap(
                    &mut doc.missing_text,
                    cursor,
                    1,
                    "acquisition_unreadable_or_allocation_unmapped",
                );
                continue;
            }
            let byte = word.bytes[offset];
            // Word paragraph/tab/line separators have stable text meanings. Other
            // control bytes may be fields, objects or glyphs; never execute them.
            let character = match byte {
                b'\r' | b'\n' | b'\t' => Some(byte as char),
                0..=31 | 127 => None,
                _ => crate::rtf_salvage::text::byte_character(byte, Some(i32::from(page))).ok(),
            };
            let Some(character) = character else {
                gap(
                    &mut doc.missing_text,
                    cursor,
                    1,
                    "legacy_undefined_codepage_byte_or_uninterpreted_control",
                );
                continue;
            };
            let append = doc.segments.last().is_some_and(|s| {
                s.cp_start + s.cp_count == cursor
                    && s.word_stream_offset + s.encoded_bytes == offset
            });
            if !append {
                if doc.segments.len() >= MAX_SEGMENTS {
                    return Err("Word6 text segment ceiling".into());
                }
                doc.segments.push(TextSegment {
                    path: String::new(),
                    cp_start: cursor,
                    cp_count: 0,
                    word_stream_offset: offset,
                    encoded_bytes: 0,
                    encoding: encoding.clone(),
                    bytes: 0,
                    sha256: String::new(),
                    source_extents: vec![],
                    text: String::new(),
                });
            }
            let segment = doc.segments.last_mut().unwrap();
            segment.text.push(character);
            segment.cp_count += 1;
            segment.encoded_bytes += 1;
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
