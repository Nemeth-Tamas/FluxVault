//! Offline readable editions: generated presentation, never a repaired original.
use super::Document;
use std::fmt::Write;

fn escaped(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\r' | '\n' | '\t' => out.push(c),
            c if c.is_control() => {
                write!(out, "\\u{{{:04X}}}", c as u32).unwrap();
            }
            _ => out.push(c),
        }
    }
    out
}
pub(super) fn render(doc: &Document, source: &str, sha: &str) -> Result<String, String> {
    if doc.segments.is_empty() {
        return Err("No readable text for edition".into());
    }
    let mut html = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"><title>FluxVault — recovered text edition</title><style>
:root{color-scheme:light}*{box-sizing:border-box}body{margin:0;background:#f1f4f8;color:#203047;font:16px/1.65 system-ui,Segoe UI,sans-serif}main{max-width:940px;margin:36px auto;padding:0 24px 48px}header,.segment,aside{background:#fff;border:1px solid #d6dfeb;border-radius:12px;padding:24px;margin-bottom:18px}.eyebrow{font-size:12px;font-weight:700;letter-spacing:.12em;color:#426391}h1{font-size:27px;line-height:1.2;overflow-wrap:anywhere}h2{font-size:16px;margin:0 0 12px}.warning,.gap{border-left:5px solid #b23336;background:#fff4f3}.gap{padding:16px 20px;margin:16px 0;border-radius:6px;color:#792627}.meta{font-size:13px;color:#556880;overflow-wrap:anywhere}.segment pre{font:16px/1.7 system-ui,Segoe UI,sans-serif;white-space:pre-wrap;overflow-wrap:anywhere;margin:0}.range{font-size:12px;letter-spacing:.04em;color:#426391}code{font-size:12px}footer{font-size:12px;color:#556880}@media(max-width:600px){main{margin:18px auto;padding:0 12px 28px}header,.segment,aside{padding:18px}}@media print{body{background:white}main{margin:0;max-width:none;padding:0}header,.segment,aside{border-radius:0}h2,.range{break-after:avoid}.gap{break-inside:avoid}}
</style></head><body><main><header><div class="eyebrow">FLUXVAULT / FORENSIC RECOVERY EDITION</div>"#,
    );
    write!(html,"<h1>{}</h1><p>Readable main-text segments from an incomplete Word document.</p><div class=\"meta\">Source image: {}<br>SHA-256: <code>{}</code><br>Declared main-text positions: {} · Readable: {} · Missing/invalid: {}</div></header>",escaped(&doc.parent.path),escaped(source),escaped(sha),doc.main_characters.unwrap_or(0),doc.segments.iter().map(|s|s.cp_count).sum::<usize>(),doc.missing_text.iter().map(|g|g.cp_count).sum::<usize>()).unwrap();
    html.push_str("<aside class=\"warning\"><h2>Generated edition — not the original document</h2><p>Only source-mapped readable text is shown. Missing positions are marked, never guessed. Segment boundaries remain visible. Formatting, tables, pictures, fields, revisions and original meaning are not reconstructed or certified. This HTML is a new presentation of forensic text, not a repaired DOC.</p><p class=\"meta\">CP ranges are zero-based Word character positions (UTF-16 code units). Non-whitespace control characters are displayed as literal Unicode escape labels. Consult word-text.json and the separate text files for original decoded characters and exact source extents.</p></aside>");
    if let Some(evidence) = &doc.cfb_recovery {
        write!(html,"<aside class=\"warning\"><h2>Partial compound-file metadata</h2><p>{}</p><p class=\"meta\">{} unavailable directory entries; {} unrelated allocation exceptions. Entire original container remains unverified.</p></aside>",escaped(&evidence.warning),evidence.missing_directory_stream_ids.len(),evidence.allocation_issues.len()).unwrap();
    }
    let mut events = doc
        .segments
        .iter()
        .enumerate()
        .map(|(i, s)| (s.cp_start, true, i))
        .chain(
            doc.missing_text
                .iter()
                .enumerate()
                .map(|(i, g)| (g.cp_start, false, i)),
        )
        .collect::<Vec<_>>();
    events.sort_unstable();
    for (_, segment, index) in events {
        if segment {
            let s = &doc.segments[index];
            write!(html,"<section class=\"segment\"><div class=\"range\">READABLE SEGMENT {} / CP {}–{} (end exclusive)</div><h2>{} positions · {}</h2><pre>{}</pre></section>",index+1,s.cp_start,s.cp_start+s.cp_count,s.cp_count,escaped(&s.encoding),escaped(&s.text)).unwrap();
        } else {
            let g = &doc.missing_text[index];
            write!(html,"<section class=\"gap\"><strong>MISSING / INVALID TEXT: CP {}–{} ({} positions)</strong><br>{}<br>No replacement text has been invented.</section>",g.cp_start,g.cp_start+g.cp_count,g.cp_count,escaped(&g.reason)).unwrap();
        }
        if html.len() > crate::fat12::MAX_IMAGE_BYTES {
            return Err("Readable edition size ceiling; text segments remain available".into());
        }
    }
    html.push_str("<footer>Generated by FluxVault. No active scripts, macros, remote assets or original-document execution. Text fragments/editions do not count as recovered whole files or certified customer documents.</footer></main></body></html>");
    if html.len() > crate::fat12::MAX_IMAGE_BYTES {
        return Err("Readable edition size ceiling; text segments remain available".into());
    }
    Ok(html)
}
