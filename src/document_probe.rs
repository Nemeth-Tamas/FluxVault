//! Read-only document triage. Never publishes a repaired document or selects a
//! disputed byte winner. Legacy Word mapping is byte attribution, not decoding.
use super::*;
use serde_json::{Value, json};

pub(crate) fn inspect(
    image: &[u8],
    parent: &fat12::FileRecord,
    bad: &BTreeSet<u64>,
    disputed: &BTreeSet<u64>,
) -> Value {
    let excluded = bad.union(disputed).copied().collect::<BTreeSet<_>>();
    let text = if parent.metadata_lbas.iter().any(|l| excluded.contains(l)) {
        json!({"state":"refused","reason":"Parent allocation/name metadata depends on missing/disputed sectors"})
    } else {
        match extract(image, parent, &excluded) {
            Ok(doc) => json!({"state":"mapped_forensic_text","main_positions":doc.main_characters,
                "readable_positions":doc.segments.iter().map(|s|s.cp_count).sum::<usize>(),
                "missing_positions":doc.missing_text.iter().map(|g|g.cp_count).sum::<usize>(),
                "segments":doc.segments,"gaps":doc.missing_text,"metadata_lbas":doc.metadata_lbas,
                "incomplete_fib":doc.incomplete_fib,"strict_parser_refusal":doc.strict_parser_refusal,
                "cfb_recovery":doc.cfb_recovery,"legacy_text":doc.legacy_text,"repaired_original":false}),
            Err(reason) => json!({"state":"refused","reason":reason}),
        }
    };
    let compound = compound(image, parent, bad, disputed)
        .unwrap_or_else(|e| json!({"state":"refused","reason":e}));
    json!({"word_text":text,"compound":compound,"semantics_certified":false,"files_written":0})
}

fn compound(
    image: &[u8],
    parent: &fat12::FileRecord,
    bad: &BTreeSet<u64>,
    disputed: &BTreeSet<u64>,
) -> Result<Value, String> {
    let mapping = parent_mapping(image, parent, bad)?;
    let trace = Arc::new(Mutex::new(Trace {
        allow_holes: false,
        reads: vec![],
        work: 0,
    }));
    let reader = Reader {
        bytes: mapping.bytes.clone(),
        sources: mapping.sources.clone(),
        position: 0,
        trace: trace.clone(),
    };
    let mut cfb = cfb::OpenOptions::new()
        .strict()
        .max_buffer_size(8192)
        .open_with(reader)
        .map_err(|e| format!("Compound metadata refusal: {e}"))?;
    let metadata = trace
        .lock()
        .unwrap()
        .reads
        .iter()
        .flat_map(|(at, n)| {
            mapping.sources[*at..at + n]
                .iter()
                .flatten()
                .map(|s| (*s / 512) as u64)
        })
        .collect::<BTreeSet<_>>();
    if metadata
        .iter()
        .chain(&parent.metadata_lbas)
        .any(|l| disputed.contains(l))
    {
        return Err("Disputed compound/parent metadata; stream-role attribution refused".into());
    }
    let entries = cfb
        .walk()
        .filter(|e| e.is_stream())
        .take(257)
        .map(|e| (e.path().to_string_lossy().into_owned(), e.len()))
        .collect::<Vec<_>>();
    if entries.len() > 256
        || entries
            .iter()
            .any(|(_, n)| *n > fat12::MAX_IMAGE_BYTES as u64)
        || entries.iter().map(|(_, n)| *n).sum::<u64>() > 16 * 1024 * 1024
    {
        return Err("Compound inspection stream/work ceiling".into());
    }
    trace.lock().unwrap().allow_holes = true;
    let mut streams = Vec::new();
    let mut legacy = None;
    for (name, _) in entries {
        let s = stream(&mut cfb, &name, &trace, &mapping.bytes, &mapping.sources)?;
        let mut touches = Vec::new();
        for lba in disputed {
            let offsets = s
                .sources
                .iter()
                .enumerate()
                .filter_map(|(i, source)| source.filter(|s| (*s / 512) as u64 == *lba).map(|_| i))
                .collect::<Vec<_>>();
            if let Some(first) = offsets.first() {
                touches.push(json!({"lba":lba,"stream_first_offset":first,"stream_last_offset_exclusive":offsets.last().unwrap()+1,"bytes":offsets.len()}));
            }
        }
        if name == "/WordDocument"
            && s.bytes.len() >= 4
            && s.sources[..4].iter().all(Option::is_some)
        {
            let ident = u16at(&s.bytes, 0);
            let version = u16at(&s.bytes, 2);
            if ident == 0xa5dc && version == 0x65 {
                let mut masked = SparseStream {
                    bytes: s.bytes.clone(),
                    sources: s.sources.clone(),
                };
                for source in &mut masked.sources {
                    if source.is_some_and(|at| disputed.contains(&((at / 512) as u64))) {
                        *source = None;
                    }
                }
                legacy=Some(legacy_word6_map(&masked).unwrap_or_else(|e|json!({"state":"refused","reason":e,"format":"Word6_0x65_byte_mapping_only"})));
            }
            streams.push(json!({"name":name,"bytes":s.bytes.len(),"unavailable_bytes":s.sources.iter().filter(|x|x.is_none()).count(),"disputed_sectors":touches,"word_ident":format!("0x{ident:04x}"),"word_version":format!("0x{version:04x}")}));
        } else {
            streams.push(json!({"name":name,"bytes":s.bytes.len(),"unavailable_bytes":s.sources.iter().filter(|x|x.is_none()).count(),"disputed_sectors":touches}));
        }
    }
    Ok(
        json!({"state":"observed_strict_cfb_mapping","metadata_lbas":metadata,"streams":streams,"legacy_word6_main_text_map":legacy,"whole_container_certified":false}),
    )
}

// Word6's FIB/PLCF layout follows LibreOffice's ww8scan.cxx reader. Narrow
// admission: Windows Word6 0x65 only, no encryption/Mac/DBCS interpretation.
// No character encoding is guessed: this returns CP-to-observed-byte extents.
pub(super) fn legacy_word6_map(word: &SparseStream) -> Result<Value, String> {
    let base = known(word, 0, 88)?;
    let flags = u16at(base, 10);
    if u16at(base, 0) != 0xa5dc
        || u16at(base, 2) != 0x65
        || u16at(base, 12) != 0x65
        || base[18] != 0
        || base[19] != 0
        || flags & 0xc100 != 0
    {
        return Err("Unsupported/encrypted/platform-specific legacy Word FIB".into());
    }
    let main = u32at(base, 52);
    let cb = u32at(base, 32);
    let minimum = u32at(base, 24);
    if main == 0
        || main > MAX_CHARACTERS
        || cb > word.bytes.len()
        || minimum < 768
        || minimum > cb
        || u32at(base, 28) < minimum
        || u32at(base, 28) > cb
    {
        return Err("Legacy Word count/meaningful stream bounds refused".into());
    }
    let width = if flags & 0x1000 != 0 { 2 } else { 1 };
    let mut metadata = source_lbas(word, 0, 88)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut pieces = Vec::new();
    if flags & 4 == 0 {
        if minimum.checked_add(main * width).is_none_or(|end| end > cb) {
            return Err("Legacy contiguous text outside meaningful stream".into());
        }
        pieces.push((0, main, minimum));
    } else {
        let pair = known(word, 352, 8)?;
        let at = u32at(pair, 0);
        let len = u32at(pair, 4);
        if !(5..=65536).contains(&len)
            || at < minimum
            || at.checked_add(len).is_none_or(|end| end > cb)
        {
            return Err("Legacy CLX extent refused".into());
        }
        let clx = known(word, at, len)?;
        metadata.extend(source_lbas(word, 352, 8));
        metadata.extend(source_lbas(word, at, len));
        let mut cursor = 0;
        while clx.get(cursor) == Some(&1) {
            let header = clx.get(cursor..cursor + 3).ok_or("Truncated legacy PRC")?;
            cursor = cursor
                .checked_add(3 + u16at(header, 1) as usize)
                .ok_or("Legacy PRC overflow")?;
            if cursor > clx.len() {
                return Err("Legacy PRC crosses CLX".into());
            }
        }
        let header = clx.get(cursor..cursor + 5).ok_or("Missing legacy PLCF")?;
        let len = u32at(header, 1);
        if header[0] != 2
            || len < 4
            || !(len - 4).is_multiple_of(12)
            || cursor + 5 + len != clx.len()
        {
            return Err("Legacy PLCF envelope refused".into());
        }
        let plc = &clx[cursor + 5..];
        let count = (len - 4) / 12;
        if count == 0 || count > 4096 {
            return Err("Legacy piece count refused".into());
        }
        let cps = (0..=count).map(|i| u32at(plc, i * 4)).collect::<Vec<_>>();
        if cps[0] != 0
            || cps.windows(2).any(|w| w[0] >= w[1])
            || cps[count] < main
            || cps[count] > MAX_CHARACTERS * 4
        {
            return Err("Legacy CP sequence refused".into());
        }
        for i in 0..count {
            let descriptor = (count + 1) * 4 + i * 8;
            let begin = u32at(plc, descriptor + 2);
            let length = (cps[i + 1] - cps[i]) * width;
            if u16at(plc, descriptor) & 4 != 0
                || begin < minimum
                || begin.checked_add(length).is_none_or(|end| end > cb)
                || (begin < at + clx.len() && begin + length > at)
            {
                return Err("Legacy piece bounds/dirty descriptor refused".into());
            }
            if cps[i] < main {
                pieces.push((cps[i], cps[i + 1].min(main) - cps[i], begin));
            }
        }
    }
    let mut mapped = Vec::new();
    let mut unavailable = 0;
    for (cp, count, begin) in pieces {
        let sources = &word.sources[begin..begin + count * width];
        unavailable += sources.iter().filter(|s| s.is_none()).count();
        // Compact exact, contiguous observed runs; None always splits a run.
        let mut extents: Vec<(usize, usize, usize)> = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            let Some(source) = *source else {
                continue;
            };
            let offset = begin + index;
            if let Some(last) = extents.last_mut()
                && last.0 + last.2 == offset
                && last.1 + last.2 == source
            {
                last.2 += 1;
            } else {
                extents.push((offset, source, 1));
            }
        }
        mapped.push(json!({"cp_start":cp,"cp_count":count,"word_stream_offset":begin,"encoded_bytes":count*width,"source_extents":extents.into_iter().map(|(at,source,n)|json!({"word_stream_offset":at,"source_byte_offset":source,"bytes":n})).collect::<Vec<_>>(),
            "observed_sha256":if sources.iter().all(Option::is_some){Some(hash(&word.bytes[begin..begin+count*width]))}else{None}}));
    }
    Ok(
        json!({"state":"mapped_bytes_not_decoded_text","format":"Word6_0x65_byte_mapping_only","main_positions":main,"character_width":width,"metadata_lbas":metadata,"unavailable_or_disputed_main_text_bytes":unavailable,"pieces":mapped,"text_encoding_verified":false,"editable_document_repaired":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn legacy(complex: bool, wide: bool) -> SparseStream {
        let mut bytes = vec![0; 2048];
        bytes[0..2].copy_from_slice(&0xa5dcu16.to_le_bytes());
        bytes[2..4].copy_from_slice(&0x65u16.to_le_bytes());
        bytes[12..14].copy_from_slice(&0x65u16.to_le_bytes());
        let flags = if complex { 4u16 } else { 0 } | if wide { 0x1000 } else { 0 };
        bytes[10..12].copy_from_slice(&flags.to_le_bytes());
        for (at, value) in [(24, 768u32), (28, 808), (32, 2048), (52, 20)] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[768..808].fill(b'x');
        if complex {
            bytes[352..356].copy_from_slice(&1500u32.to_le_bytes());
            bytes[356..360].copy_from_slice(&33u32.to_le_bytes());
            bytes[1500] = 2;
            bytes[1501..1505].copy_from_slice(&28u32.to_le_bytes());
            for (i, cp) in [0u32, 10, 20].iter().enumerate() {
                bytes[1505 + i * 4..1509 + i * 4].copy_from_slice(&cp.to_le_bytes());
            }
            for (i, fc) in [768u32, 1024].iter().enumerate() {
                let at = 1505 + 12 + i * 8 + 2;
                bytes[at..at + 4].copy_from_slice(&fc.to_le_bytes());
            }
        }
        SparseStream {
            sources: (0..bytes.len()).map(|at| Some(10000 + at)).collect(),
            bytes,
        }
    }
    #[test]
    fn legacy_simple_complex_and_wide_maps_keep_exact_offsets_without_decoding() {
        for complex in [false, true] {
            for wide in [false, true] {
                let word = legacy(complex, wide);
                let value = legacy_word6_map(&word).unwrap();
                assert_eq!(value["main_positions"], 20);
                assert_eq!(value["character_width"], if wide { 2 } else { 1 });
                assert_eq!(value["unavailable_or_disputed_main_text_bytes"], 0);
                assert_eq!(value["text_encoding_verified"], false);
                let pieces = value["pieces"].as_array().unwrap();
                assert_eq!(pieces.len(), if complex { 2 } else { 1 });
                for piece in pieces {
                    let at = piece["word_stream_offset"].as_u64().unwrap() as usize;
                    let n = piece["encoded_bytes"].as_u64().unwrap() as usize;
                    assert_eq!(piece["observed_sha256"], hash(&word.bytes[at..at + n]));
                    assert_eq!(piece["source_extents"][0]["source_byte_offset"], 10000 + at);
                }
            }
        }
    }
    #[test]
    fn missing_text_splits_exact_runs_and_missing_metadata_refuses() {
        let mut word = legacy(true, false);
        word.sources[1025] = None;
        let value = legacy_word6_map(&word).unwrap();
        assert_eq!(value["unavailable_or_disputed_main_text_bytes"], 1);
        assert!(value["pieces"][1]["observed_sha256"].is_null());
        assert_eq!(
            value["pieces"][1]["source_extents"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        word.sources[1506] = None;
        assert!(legacy_word6_map(&word).is_err());
    }
    #[test]
    fn legacy_format_encryption_counts_piece_bounds_and_metadata_aliases_refuse() {
        for (at, value) in [
            (0, 0),
            (2, 0xc1),
            (12, 0x69),
            (10, 0x104),
            (10, 0x4004),
            (18, 1),
            (19, 1),
        ] {
            let mut word = legacy(true, false);
            word.bytes[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
            assert!(legacy_word6_map(&word).is_err());
        }
        for (at, value) in [
            (52, 200001u32),
            (352, u32::MAX),
            (356, 65537),
            (1501, 32),
            (1509, 0),
            (1517 + 2, 1520),
            (1517 + 10, 2040),
        ] {
            let mut word = legacy(true, false);
            word.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(
                legacy_word6_map(&word).is_err(),
                "accepted mutation at {at}"
            );
        }
    }
    #[test]
    fn bounded_malformed_legacy_headers_and_truncated_inputs_do_not_panic() {
        for n in 0..360 {
            let mut word = legacy(true, false);
            word.bytes.truncate(n);
            word.sources.truncate(n);
            assert!(legacy_word6_map(&word).is_err());
        }
        let original = legacy(true, false);
        let mut seed = 7u32;
        for i in 0..256 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let mut word = SparseStream {
                bytes: original.bytes.clone(),
                sources: original.sources.clone(),
            };
            let at = [24, 28, 32, 52, 352, 356, 1501, 1505, 1509, 1513, 1519, 1527][i % 12];
            word.bytes[at..at + 4].copy_from_slice(&seed.to_le_bytes());
            let _ = legacy_word6_map(&word);
        }
    }
}
