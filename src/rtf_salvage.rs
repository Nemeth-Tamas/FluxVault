//! Saved-image RTF readable-prefix text, never a repaired RTF or a whole file.
use crate::{carving::Extent, fat12, fat12_recovery::RecoveryReport};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
#[path = "rtf_text.rs"]
mod text;
const ENGINE_VERSION: u32 = 1;
const MAX_PREFIX: usize = 256 * 1024;
const WARNING: &str = "FORENSIC RTF PREFIX TEXT, NOT A REPAIRED ORIGINAL. Only a source-mapped readable prefix is decoded; parsing stops at the first unreadable/unmapped byte, unsupported encoding/control or malformed structure. No state is guessed across holes and no later fragments are silently joined. Remaining text length, original formatting, tables, objects and semantics are unknown. Hidden/deleted text, fields and non-body destinations are excluded. UTF-8/HTML are separate generated evidence, not whole-file counts, normal Office conversions or customer-certified documents.";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Document {
    parent: fat12::UnrecoveredFile,
    readable_prefix_bytes: usize,
    readable_prefix_sha256: String,
    prefix_boundary_reason: String,
    source_extents: Vec<Extent>,
    decoded_through_file_byte: usize,
    stop_reason: String,
    remaining_file_bytes_uninterpreted: usize,
    remaining_text_length_known: bool,
    text_path: Option<String>,
    text_sha256: String,
    text_bytes: usize,
    readable_utf16_positions: usize,
    edition_path: Option<String>,
    edition_sha256: String,
    repaired_original: bool,
    customer_delivery_certified: bool,
}
#[derive(Serialize, Deserialize)]
struct Report {
    schema_version: u32,
    engine_version: u32,
    source_image: String,
    source_sha256: String,
    disk: u32,
    attempt: u32,
    bad_lbas: Vec<u64>,
    documents: Vec<Document>,
    limits_reached: bool,
    warning: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct SalvageResult {
    pub engine_version: u32,
    pub documents_examined: usize,
    pub documents_with_text: usize,
    pub readable_utf16_positions: usize,
    pub readable_edition_paths: Vec<PathBuf>,
    pub output_directory: PathBuf,
    pub report_path: PathBuf,
    pub limits_reached: bool,
    pub repaired_originals: usize,
    pub reused: bool,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn prefix(
    image: &[u8],
    parent: &fat12::FileRecord,
    bad: &BTreeSet<u64>,
) -> Result<(Vec<u8>, Vec<Extent>, String), String> {
    if parent.bytes > fat12::MAX_IMAGE_BYTES {
        return Err("RTF parent size ceiling".into());
    }
    let mut bytes = Vec::new();
    let mut extents: Vec<Extent> = Vec::new();
    let mut seen = BTreeSet::new();
    for lba in &parent.data_lbas {
        if bytes.len() >= parent.bytes {
            break;
        }
        if bad.contains(lba) {
            return Ok((
                bytes,
                extents,
                "First acquisition-unreadable sector; no parser resynchronization".into(),
            ));
        }
        if bytes.len() >= MAX_PREFIX {
            return Ok((
                bytes,
                extents,
                "Readable-prefix input ceiling; search not exhaustive".into(),
            ));
        }
        let n = (parent.bytes - bytes.len())
            .min(512)
            .min(MAX_PREFIX - bytes.len());
        let at = usize::try_from(*lba)
            .ok()
            .and_then(|l| l.checked_mul(512))
            .ok_or("RTF source offset overflow")?;
        let source = image
            .get(at..at.checked_add(n).ok_or("RTF source range overflow")?)
            .ok_or("RTF source outside image")?;
        if !seen.insert(*lba) {
            return Err("Repeated RTF source sector".into());
        }
        let file_offset = bytes.len();
        bytes.extend_from_slice(source);
        if let Some(last) = extents.last_mut()
            && last.source_byte_offset + last.bytes == at
        {
            last.bytes += n;
        } else {
            extents.push(Extent {
                file_offset,
                source_byte_offset: at,
                bytes: n,
            });
        }
    }
    let reason = if bytes.len() == parent.bytes {
        "Declared file EOF; original still unverified"
    } else {
        "First allocation-unmapped byte; no parser resynchronization"
    };
    Ok((bytes, extents, reason.into()))
}
fn escaped(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() && !matches!(c, '\n' | '\r' | '\t') => {
                out.push_str(&format!("\\u{{{:04X}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}
fn edition(doc: &Document, body: &str, source: &str, sha: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>FluxVault — recovered RTF prefix</title><style>body{{background:#f1f4f8;color:#203047;font:16px/1.7 system-ui,Segoe UI,sans-serif;margin:0}}main{{max-width:940px;margin:32px auto;padding:0 20px 40px}}header,aside,section{{padding:22px;background:white;border:1px solid #d6dfeb;border-radius:10px;margin-bottom:18px}}aside{{border-left:5px solid #b23336;background:#fff4f3}}h1{{font-size:26px;overflow-wrap:anywhere}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit}}.meta{{font-size:13px;overflow-wrap:anywhere}}@media(max-width:600px){{main{{padding:0 12px}}header,aside,section{{padding:16px}}}}</style></head><body><main><header><p>FLUXVAULT / FORENSIC RTF PREFIX EDITION</p><h1>{}</h1><p class=\"meta\">Source: {}<br>SHA-256: {}<br>{} readable UTF-16 positions · {} input prefix bytes · decoded through file byte {}</p></header><aside><h2>Generated partial text — not a repaired RTF</h2><p>{}</p></aside><section><h2>Readable prefix text</h2><pre>{}</pre></section><aside><h2>STOPPED / REMAINDER NOT INTERPRETED</h2><p>{}</p><p>{} file bytes remain uninterpreted. Their text length is unknown; no replacement words or formatting state have been guessed.</p></aside></main></body></html>",
        escaped(&doc.parent.record.path),
        escaped(source),
        escaped(sha),
        doc.readable_utf16_positions,
        doc.readable_prefix_bytes,
        doc.decoded_through_file_byte,
        escaped(WARNING),
        escaped(body),
        escaped(&doc.stop_reason),
        doc.remaining_file_bytes_uninterpreted
    )
}

/// Caller owns the native per-disk lock and verified acquisition/report roots.
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
    let mut docs = Vec::new();
    let mut payloads = Vec::new();
    let mut limits = false;
    let mut input_bytes = 0;
    let mut text_bytes = 0;
    for parent in &a.unrecovered_files {
        if !parent.record.path.to_ascii_lowercase().ends_with(".rtf")
            || !fat12::partial_file_safe(parent, a)
        {
            continue;
        }
        if docs.len() >= 64 || input_bytes >= fat12::MAX_IMAGE_BYTES || text_bytes >= 500_000 {
            limits = true;
            break;
        }
        // Unsupported/malformed parent mappings are per-document refusals,
        // not failures that discard the disk's independently recovered files.
        let (bytes, extents, boundary, refusal) = match prefix(image, &parent.record, &bad) {
            Ok((bytes, extents, boundary)) => (bytes, extents, boundary, None),
            Err(reason) => (Vec::new(), Vec::new(), reason.clone(), Some(reason)),
        };
        input_bytes += bytes.len();
        if input_bytes > fat12::MAX_IMAGE_BYTES {
            limits = true;
            break;
        }
        let decoded = refusal.map_or_else(
            || text::decode(&bytes, &boundary),
            |reason| text::Decoded {
                text: String::new(),
                through: 0,
                stop: reason,
            },
        );
        if text_bytes + decoded.text.len() > 500_000 {
            limits = true;
            break;
        }
        text_bytes += decoded.text.len();
        limits |= boundary.contains("ceiling") || decoded.stop.contains("ceiling");
        let sha = hash(decoded.text.as_bytes());
        let mut doc = Document {
            parent: parent.clone(),
            readable_prefix_bytes: bytes.len(),
            readable_prefix_sha256: hash(&bytes),
            prefix_boundary_reason: boundary,
            source_extents: extents,
            decoded_through_file_byte: decoded.through,
            stop_reason: decoded.stop,
            remaining_file_bytes_uninterpreted: parent.record.bytes.saturating_sub(decoded.through),
            remaining_text_length_known: false,
            text_path: None,
            text_sha256: sha.clone(),
            text_bytes: decoded.text.len(),
            readable_utf16_positions: decoded.text.encode_utf16().count(),
            edition_path: None,
            edition_sha256: String::new(),
            repaired_original: false,
            customer_delivery_certified: false,
        };
        if !decoded.text.is_empty() {
            let name = format!(
                "rtf_{:08x}_{}.txt",
                parent.record.directory_entry_offset,
                &sha[..12]
            );
            doc.text_path = Some(name.clone());
            let html = edition(
                &doc,
                &decoded.text,
                &native.source_image,
                &native.source_sha256,
            );
            doc.edition_sha256 = hash(html.as_bytes());
            let edition_name = format!(
                "rtf_{:08x}_readable_{}.html",
                parent.record.directory_entry_offset,
                &doc.edition_sha256[..12]
            );
            doc.edition_path = Some(edition_name.clone());
            payloads.push((name, decoded.text.into_bytes()));
            payloads.push((edition_name, html.into_bytes()));
        }
        docs.push(doc);
    }
    if docs.is_empty() {
        return Ok(None);
    }
    let report = Report {
        schema_version: 1,
        engine_version: ENGINE_VERSION,
        source_image: native.source_image.clone(),
        source_sha256: native.source_sha256.clone(),
        disk: native.disk,
        attempt: native.attempt,
        bad_lbas: a.bad_lbas.clone(),
        documents: docs,
        limits_reached: limits,
        warning: WARNING.into(),
    };
    let serialized = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    if serialized.len() > 16 * 1024 * 1024 {
        return Err("RTF provenance size ceiling".into());
    }
    let output = recovery_disk.join(format!(
        "attempt_{:03}_rtf_text_v{ENGINE_VERSION}",
        native.attempt
    ));
    let report_path = output.join("rtf-text.json");
    let reused = output.exists();
    if reused {
        if !fs::symlink_metadata(&output)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_dir()
            || output.canonicalize().map_err(|e| e.to_string())?.parent() != Some(recovery_disk)
        {
            return Err("RTF salvage output escapes Recovery; preserved".into());
        }
        if crate::document_salvage::regular_read(&report_path)? != serialized
            || fs::read_dir(&output).map_err(|e| e.to_string())?.count() != payloads.len() + 1
        {
            return Err("RTF salvage report/inventory changed; preserved".into());
        }
        for (name, bytes) in &payloads {
            if crate::document_salvage::regular_read(&output.join(name))? != *bytes {
                return Err("RTF text/edition changed; preserved".into());
            }
        }
    } else {
        let staging = recovery_disk.join(format!(
            ".tmp-rtf-text-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms()
        ));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        for (name, bytes) in &payloads {
            crate::document_salvage::write_new(&staging.join(name), bytes)?;
        }
        crate::document_salvage::write_new(&staging.join("rtf-text.json"), &serialized)?;
        if hash(&crate::document_salvage::regular_read(source)?) != native.source_sha256 {
            return Err("RTF source changed; staging retained".into());
        }
        if output.exists() {
            return Err("RTF destination appeared; staging retained".into());
        }
        fs::rename(&staging, &output).map_err(|e| format!("RTF staging retained: {e}"))?;
    }
    Ok(Some(SalvageResult {
        engine_version: ENGINE_VERSION,
        documents_examined: report.documents.len(),
        documents_with_text: report
            .documents
            .iter()
            .filter(|d| d.text_path.is_some())
            .count(),
        readable_utf16_positions: report
            .documents
            .iter()
            .map(|d| d.readable_utf16_positions)
            .sum(),
        readable_edition_paths: report
            .documents
            .iter()
            .filter_map(|d| d.edition_path.as_ref())
            .map(|p| output.join(p))
            .collect(),
        output_directory: output,
        report_path,
        limits_reached: limits,
        repaired_originals: 0,
        reused,
    }))
}

#[cfg(test)]
#[path = "rtf_salvage_tests.rs"]
mod tests;
