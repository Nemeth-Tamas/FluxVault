//! Editable editions of verified recovered text, never repaired originals.
//! Minimal WordprocessingML package: Microsoft Open XML documentation.
//! https://learn.microsoft.com/en-us/office/open-xml/general/how-to-create-a-package
use crate::{fat12_recovery::RecoveryReport, final_report::Language, sector_inspection};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
const MAX: usize = 8 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[derive(Debug, Clone, Serialize)]
pub struct Editions {
    pub report_path: PathBuf,
    pub paths: Vec<PathBuf>,
    pub refused_documents: usize,
    pub reused: bool,
    pub repaired_originals: usize,
    pub customer_delivery_certified: bool,
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    sector_inspection::regular(path, false)?;
    let mut b = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX as u64 + 1)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() > MAX {
        return Err("Editable text input exceeds 8 MiB".into());
    }
    Ok(b)
}
fn safe_leaf(s: &str) -> Result<(), String> {
    let base = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if s.is_empty()
        || s.len() > 255
        || s.chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        || matches!(s, "." | "..")
        || s.ends_with([' ', '.'])
        || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
    {
        return Err("Editable edition requires a report-bound leaf filename".into());
    }
    Ok(())
}
fn escape(s: &str) -> Result<String, String> {
    if s.chars().any(|c| {
        !(matches!(c, '\t' | '\r' | '\n') || c >= '\u{20}' && c != '\u{fffe}' && c != '\u{ffff}')
    }) {
        return Err(
            "Source contains XML-incompatible controls; text retained, no substituted character"
                .into(),
        );
    }
    Ok(s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}
fn paragraph(body: &mut String, text: &str, style: &str) -> Result<(), String> {
    let escaped = escape(text)?
        .replace("\r\n", "\n")
        .replace('\t', "</w:t><w:tab/><w:t xml:space=\"preserve\">")
        .replace(['\n', '\r'], "</w:t><w:br/><w:t xml:space=\"preserve\">");
    body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{escaped}</w:t></w:r></w:p>"));
    Ok(())
}
fn docx(name: &str, blocks: &[(bool, String)], language: Language) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    paragraph(
        &mut body,
        language.text("Kimentett szerkeszthető szöveg", "Recovered editable text"),
        "Title",
    )?;
    paragraph(&mut body, language.text("Ez a kiadás csak az ellenőrzött olvasható szövegrészeket tartalmazza. A hiányokat külön jelöljük. Nem az eredeti dokumentum javított példánya; az eredeti formázás, objektumok és a tartalom teljessége nem igazolt.", "This edition contains verified readable text segments with explicit gaps. It is not a repaired original. Original formatting, objects and semantic completeness are not established."), "Notice")?;
    paragraph(
        &mut body,
        &format!(
            "{} {name}",
            language.text("Eredeti fájlnév:", "Original filename:")
        ),
        "Normal",
    )?;
    for (editorial, text) in blocks {
        paragraph(
            &mut body,
            text,
            if *editorial { "Notice" } else { "Normal" },
        )?;
    }
    if body.len() > MAX {
        return Err("Editable edition exceeds XML size bound".into());
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr></w:body></w:document>"
    );
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\"/><w:sz w:val=\"22\"/><w:color w:val=\"000000\"/><w:lang w:val=\"{}\"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"160\" w:line=\"276\" w:lineRule=\"auto\"/><w:widowControl/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:keepNext/><w:spacing w:after=\"240\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"36\"/><w:color w:val=\"000000\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Notice\"><w:name w:val=\"Recovery notice\"/><w:basedOn w:val=\"Normal\"/><w:rPr><w:b/><w:color w:val=\"000000\"/></w:rPr></w:style></w:styles>",
        if language == Language::Hungarian {
            "hu-HU"
        } else {
            "en-US"
        }
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, data) in [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/></Types>",
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
        ),
        (
            "word/_rels/document.xml.rels",
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>",
        ),
        ("word/styles.xml", &styles),
        ("word/document.xml", &xml),
    ] {
        zip.start_file(path, options).map_err(|e| e.to_string())?;
        zip.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}
fn text(directory: &Path, row: &Value) -> Result<String, String> {
    let name = row["path"].as_str().ok_or("Text segment has no filename")?;
    safe_leaf(name)?;
    let b = read(&directory.join(name))?;
    if row["sha256"] != hash(&b) || row["bytes"].as_u64().is_some_and(|n| n != b.len() as u64) {
        return Err("Editable source text hash/size changed".into());
    }
    String::from_utf8(b).map_err(|e| e.to_string())
}
fn blocks(
    directory: &Path,
    doc: &Value,
    word: bool,
    language: Language,
) -> Result<Vec<(bool, String)>, String> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut previous = 0u64;
    let mut characters = 0usize;
    let segments = doc[if word { "segments" } else { "text_segments" }].as_array();
    if let Some(segments) = segments.filter(|s| !s.is_empty()) {
        if segments.len() > 4096 {
            return Err("Editable segment count exceeds bound".into());
        }
        for row in segments {
            let start = row[if word {
                "cp_start"
            } else {
                "decoded_utf8_start"
            }]
            .as_u64()
            .ok_or("Missing segment start")?;
            if start < previous {
                return Err("Editable source segments overlap or reorder".into());
            }
            if start > previous || !word && !out.is_empty() {
                out.push((true, language.text("HIÁNY vagy nem értelmezett szimbólum a következő szövegrész előtt. A szövegrészeket nem kapcsoltuk össze találgatással.", "GAP or uninterpreted symbol before the next segment. No missing text has been guessed or silently joined.").into()));
            }
            let s = text(directory, row)?;
            let n = if word {
                s.encode_utf16().count()
            } else {
                s.len()
            };
            let end = if word {
                start
                    .checked_add(row["cp_count"].as_u64().ok_or("Missing CP count")?)
                    .ok_or("CP overflow")?
            } else {
                row["decoded_utf8_end"].as_u64().ok_or("Missing UTF8 end")?
            };
            if end.checked_sub(start) != Some(n as u64) {
                return Err("Editable source span does not match decoded text length".into());
            }
            if word
                && doc["main_characters"]
                    .as_u64()
                    .is_none_or(|total| total > 500_000 || end > total)
            {
                return Err("Editable Word segment exceeds declared main-text scope".into());
            }
            characters += s.len();
            if characters > MAX {
                return Err("Editable decoded text exceeds bound".into());
            }
            if word && start == previous && out.last().is_some_and(|(editorial, _)| !editorial) {
                out.last_mut().unwrap().1.push_str(&s);
            } else {
                out.push((false, s));
            }
            previous = end;
        }
    } else if !word && let Some(path) = doc["text_path"].as_str() {
        out.push((
            false,
            text(
                directory,
                &json!({"path":path,"sha256":doc["text_sha256"],"bytes":doc["text_bytes"]}),
            )?,
        ));
    }
    if !out.iter().any(|(editorial, s)| !editorial && !s.is_empty()) {
        return Err("No verified decoded text to reconstruct".into());
    }
    if word && !doc["cfb_recovery"]["inferred_header"].is_null() {
        out.insert(0, (true, language.text(
            "FEJLÉC NÉLKÜLI SZERKEZETI SZÖVEGJELÖLT: a tároló fejléce olvashatatlan. Az 512 bájtos, egy-FAT-os elrendezés következtetés; a mini-FAT és a teljes eredeti tárolóazonosság nem igazolt. Csak a külön ellenőrzött adatfolyamok megfigyelt szövege szerepel, nem javított eredeti dokumentum.",
            "HEADERLESS STRUCTURAL TEXT CANDIDATE: the compound header is unavailable. The 512-byte single-FAT geometry is inferred; mini-FAT and whole original container identity remain unverified. Only observed text from independently checked streams is included, not a repaired original document.",
        ).into()));
    }
    if word && !doc["legacy_text"].is_null() {
        out.insert(0, (true, language.text(
            "Word6 szöveges értelmezés a rögzített karakterkészlet vagy területi beállítás alapján. A betűtípusonkénti szimbólumok és az eredeti formázás/jelentés nem igazolt; részletek a word-text.json fájlban.",
            "Word6 text interpretation using the recorded charset or locale fallback. Font-specific glyphs and original formatting/meaning remain unverified; see word-text.json.",
        ).into()));
    }
    if word
        && doc["main_characters"]
            .as_u64()
            .is_some_and(|n| n > previous)
        || !word
    {
        out.push((true, language.text("A fennmaradó tartalom nem igazolt. A hiányzó szöveg és az eredeti formázás nem került helyreállításra.", "Remaining content is not established. Missing text and original formatting have not been reconstructed.").into()));
    }
    Ok(out)
}

pub(crate) fn preserve(
    report_path: &Path,
    source: &Path,
    recovery_disk: &Path,
    native: &RecoveryReport,
) -> Result<Editions, String> {
    sector_inspection::regular(recovery_disk, true)?;
    let directory = report_path
        .parent()
        .ok_or("Missing text-report directory")?;
    sector_inspection::regular(directory, true)?;
    if directory
        .canonicalize()
        .map_err(|e| e.to_string())?
        .parent()
        != Some(recovery_disk)
    {
        return Err("Editable source report escapes recovery disk".into());
    }
    let input = read(report_path)?;
    let report: Value = serde_json::from_slice(&input).map_err(|e| e.to_string())?;
    if report["schema_version"] != 1
        || report["disk"] != native.disk
        || report["attempt"] != native.attempt
        || report["source_sha256"] != native.source_sha256
    {
        return Err("Editable source-report identity mismatch".into());
    }
    let language = match report["customer_language"].as_str().unwrap_or("en") {
        "hu" => Language::Hungarian,
        "en" => Language::English,
        _ => return Err("Unsupported edition language".into()),
    };
    let word = report_path
        .file_name()
        .is_some_and(|n| n == "word-text.json");
    let docs = report["documents"]
        .as_array()
        .ok_or("Missing text document records")?;
    if docs.len() > 64 {
        return Err("Editable document count exceeds bound".into());
    }
    let digest = hash(&input);
    let folder = format!(
        "attempt_{:03}_{}_editable_v1_{}_{}",
        native.attempt,
        if word { "word" } else { "rtf" },
        language.code(),
        &digest[..12]
    );
    let output = recovery_disk.join(folder);
    let mut files = Vec::new();
    let mut records = Vec::new();
    let mut refusals = Vec::new();
    let mut total = 0;
    for (i, doc) in docs.iter().enumerate() {
        crate::cancellation::check()?;
        let parent = if word {
            &doc["parent"]
        } else {
            &doc["parent"]["record"]
        };
        let name = parent["path"]
            .as_str()
            .unwrap_or("unknown original filename");
        match blocks(directory, doc, word, language).and_then(|b| docx(name, &b, language)) {
            Ok(bytes) => {
                total += bytes.len();
                if total > MAX {
                    return Err("Editable publication exceeds combined size bound".into());
                }
                let sha = hash(&bytes);
                let path = format!("editable_{i:03}_{}.docx", &sha[..12]);
                records.push(json!({"parent":parent,"editable_edition":{"path":path,"bytes":bytes.len(),"sha256":sha},"source_extents":doc["source_extents"],"segments":doc["segments"],"source_record":doc,
                    "confidence":"verified_decoded_segments_in_new_editable_container_not_original_container_repair","repaired_original":false,"customer_delivery_certified":false}));
                files.push((path, bytes));
            }
            Err(error) => refusals.push(json!({"original_path":name,"reason":error})),
        }
    }
    let record = json!({"schema_version":1,"disk":native.disk,"attempt":native.attempt,"source_sha256":native.source_sha256,
        "customer_language":language.code(),"source_text_report":report_path.strip_prefix(recovery_disk).map_err(|_|"Foreign editable source report")?.to_string_lossy().replace('\\',"/"),"source_text_report_sha256":digest,
        "documents":records,"refusals":refusals,"repaired_originals":0,"customer_delivery_certified":false,
        "limits":"New editable DOCX carries only hash-bound verified decoded text and separately labelled gap/editorial notices. Original container, formatting, embedded objects and semantic completeness are not repaired/certified. No host tools or media reads; original images/text/HTML remain unchanged. Editions do not increase whole-file yield or ordinary conversion totals."});
    let serialized = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
    if serialized.len() > MAX {
        return Err("Editable provenance exceeds bound".into());
    }
    let reused = output.try_exists().map_err(|e| e.to_string())?;
    if reused {
        sector_inspection::regular(&output, true)?;
        if output.canonicalize().map_err(|e| e.to_string())?.parent() != Some(recovery_disk)
            || read(&output.join("editable-documents.json"))? != serialized
            || fs::read_dir(&output).map_err(|e| e.to_string())?.count() != files.len() + 1
        {
            return Err("Editable edition report/inventory changed; preserved".into());
        }
        for (name, b) in &files {
            if read(&output.join(name))? != *b {
                return Err("Editable edition changed; preserved".into());
            }
        }
    } else {
        let staging = recovery_disk.join(format!(
            ".fluxvault-editable-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        for (name, b) in &files {
            write_new(&staging.join(name), b)?;
        }
        write_new(&staging.join("editable-documents.json"), &serialized)?;
        if read(report_path)? != input || hash(&read(source)?) != native.source_sha256 {
            return Err("Editable source changed; staging retained".into());
        }
        fs::rename(&staging, &output).map_err(|e| format!("Editable staging retained: {e}"))?;
    }
    if read(report_path)? != input || hash(&read(source)?) != native.source_sha256 {
        return Err("Editable source bindings changed during publication/reuse".into());
    }
    Ok(Editions {
        report_path: output.join("editable-documents.json"),
        paths: files.iter().map(|(p, _)| output.join(p)).collect(),
        refused_documents: refusals.len(),
        reused,
        repaired_originals: 0,
        customer_delivery_certified: false,
    })
}
fn write_new(path: &Path, b: &[u8]) -> Result<(), String> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(b)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_editable_package_preserves_source_text_and_localizes_only_notices() {
        let blocks = vec![
            (false, "Árvíztűrő <&>\n\tfragment".into()),
            (true, "Explicit gap".into()),
        ];
        let hu = docx("original.rtf", &blocks, Language::Hungarian).unwrap();
        assert_eq!(
            hu,
            docx("original.rtf", &blocks, Language::Hungarian).unwrap()
        );
        let mut z = zip::ZipArchive::new(Cursor::new(hu)).unwrap();
        assert_eq!(z.len(), 5);
        let mut xml = String::new();
        z.by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("Árvíztűrő &lt;&amp;&gt;"));
        assert!(xml.contains("<w:br/>") && xml.contains("<w:tab/>"));
        assert!(xml.contains("Kimentett szerkeszthető szöveg"));
        assert!(!xml.contains("TargetMode=\"External\""));
        assert!(docx("source", &[(false, "bad\0".into())], Language::English).is_err());
        for name in [
            "CON.txt",
            "nul",
            "LPT1.txt",
            "COM².txt",
            "../x.txt",
            "text.txt:stream",
        ] {
            assert!(safe_leaf(name).is_err());
        }
    }
}
