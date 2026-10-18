//! Offline customer entry page, bound to the same archive manifest/report.
//! Only current VALID, hash-matching generated Office/PDF outputs are linked.
use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

fn escaped(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn href(path: &str) -> Result<String, String> {
    let path = path.replace('\\', "/");
    if path.is_empty()
        || path
            .split('/')
            .any(|s| s.is_empty() || matches!(s, "." | "..") || s.contains(':'))
    {
        return Err("Unsafe customer index path".into());
    }
    Ok(path
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect())
}
fn read_json(file: &PackageFile, rows: &[ManifestRow]) -> Result<Option<Value>, String> {
    let row = rows
        .iter()
        .find(|r| r.archive_path == file.archive_path)
        .ok_or("Index input not inventoried")?;
    if row.bytes > 16 * 1024 * 1024 {
        // The archive still carries the full report; presentation is optional.
        return Ok(None);
    }
    let b = crate::document_salvage::regular_read(&file.source)?;
    if format!("{:x}", Sha256::digest(&b)) != row.sha256 {
        return Err("Customer index report changed after archival copy".into());
    }
    Ok(serde_json::from_slice(&b).ok())
}

pub(super) fn build(
    files: &[PackageFile],
    rows: &[ManifestRow],
    project: &str,
    lang: crate::final_report::Language,
) -> Result<String, String> {
    let tr = |hu, en| lang.text(hu, en);
    let mut report = None;
    let mut report_path = None;
    if let Some(pointer) = files
        .iter()
        .find(|f| f.archive_path == "Reports/FinalReportLatest.json")
        && let Some(binding) = read_json(pointer, rows)?
        && let Some(directory) = binding["directory"].as_str()
    {
        let path = format!("Reports/{directory}/FloppyFinalReport.json");
        href(&path)?;
        if let Some(file) = files.iter().find(|f| f.archive_path == path) {
            report = read_json(file, rows)?;
            report_path = Some(path);
        }
    }
    let empty = Value::Null;
    let data = report
        .as_ref()
        .filter(|r| r["schema_version"] == 1)
        .unwrap_or(&empty);
    let members = rows
        .iter()
        .map(|r| (r.archive_path.as_str(), r))
        .collect::<BTreeMap<_, _>>();
    let mut disks = BTreeMap::<String, (bool, Vec<(String, String)>)>::new();
    for d in data["floppies"].as_array().into_iter().flatten() {
        if let Some(n) = d["Floppy"]
            .as_str()
            .filter(|n| !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit()))
        {
            disks.insert(n.into(), (d["AuditStatus"] != "OK", vec![]));
        }
    }
    let mut seen = BTreeSet::new();
    let mut missing = 0;
    for d in data["delivery_files"].as_array().into_iter().flatten() {
        if d["FileRole"] != "Generated conversion" || d["Integrity"] != "VALID" {
            continue;
        }
        let Some(path) = d["DeliveryPath"].as_str() else {
            continue;
        };
        let path = format!("Converted/{}", path.replace('\\', "/"));
        let extension = Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "pdf" | "docx" | "xlsx" | "pptx") {
            continue;
        }
        let Some(row) = members.get(path.as_str()).filter(|r| {
            d["SHA256"]
                .as_str()
                .is_some_and(|h| h.eq_ignore_ascii_case(&r.sha256))
        }) else {
            missing += 1;
            continue;
        };
        let Some(n) = d["Floppy"].as_str() else {
            continue;
        };
        if let Some((_, links)) = disks.get_mut(n)
            && seen.insert(path.clone())
        {
            let name = Path::new(&row.archive_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            links.push((path, name));
        }
    }
    let attention = disks.values().filter(|(a, _)| *a).count();
    let mut html = format!(
        r#"<!doctype html><html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"><title>{} — FluxVault</title><style>
:root{{color-scheme:light;font:16px/1.6 system-ui,Segoe UI,sans-serif;color:#17283d;background:#f2f5f8}}*{{box-sizing:border-box}}body{{margin:0}}main{{max-width:1100px;margin:auto;padding:36px 24px 60px}}header,.notice,details{{background:white;border:1px solid #dce3eb;border-radius:16px;padding:22px;margin:18px 0}}.brand{{color:#256678;letter-spacing:.12em;font-size:12px;font-weight:700}}h1{{font-size:clamp(28px,5vw,44px);margin:8px 0}}h2{{margin:0;font-size:22px}}p{{margin:8px 0}}.stats{{display:flex;gap:16px;flex-wrap:wrap}}.stat{{flex:1;min-width:130px;background:#e8eff5;border-radius:12px;padding:16px}}.stat strong{{font-size:28px;display:block}}.notice{{border-left:5px solid #b97708;background:#fff9ee}}a{{color:#125d88;overflow-wrap:anywhere}}nav{{display:flex;gap:8px;flex-wrap:wrap;margin:20px 0}}nav a,.folder{{display:inline-block;border:1px solid #c9d7e3;border-radius:8px;padding:5px 12px;text-decoration:none}}summary{{cursor:pointer;font-size:18px;font-weight:650}}.pill{{display:inline-block;margin:0 10px;font-size:12px;padding:3px 9px;border-radius:20px;background:#e3f2e9;color:#155d35}}.attention{{background:#ffead8;color:#8b4017}}.files{{list-style:none;padding:0;columns:2;column-gap:28px}}.files li{{break-inside:avoid;padding:6px 0;border-bottom:1px solid #e8eef3}}small,footer{{color:#526579}}footer{{margin-top:28px}}@media(max-width:650px){{main{{padding:18px 12px}}.files{{columns:1}}header,.notice,details{{padding:16px}}}}
</style></head><body><main><header><div class="brand">FLUXVAULT · {}</div><h1>{}</h1><p>{}</p></header>"#,
        lang.code(),
        escaped(project),
        tr("LEMEZARCHÍVUM", "DISK ARCHIVE"),
        escaped(project),
        tr(
            "Dokumentumok és megőrzött lemeztartalmak, lemezenként rendezve.",
            "Documents and preserved disk contents, organized by disk."
        )
    );
    write!(html,"<div class=\"stats\"><div class=\"stat\"><strong>{}</strong>{}</div><div class=\"stat\"><strong>{}</strong>{}</div><div class=\"stat\"><strong>{}</strong>{}</div></div>",disks.len(),tr("jelentésben szereplő lemez","reported disks"),seen.len(),tr("elérhető PDF / szerkeszthető kiadás","available PDF / editable editions"),attention,tr("figyelmet igénylő lemez","disks needing attention")).unwrap();
    write!(html,"<aside class=\"notice\"><h2>{}</h2><p>{}</p><p>{}</p></aside>",tr("Olvassa el először","Read this first"),tr("A ZIP-et előbb bontsa ki, majd nyissa meg a START_HERE.html fájlt. Ez egy archivált pillanatkép, nem a hibátlan vagy teljes adatmentés tanúsítványa.","Extract the ZIP first, then open START_HERE.html. This is an archival snapshot, not a certificate of complete or flawless recovery."),tr("A figyelmeztetéseket a jelentések őrzik. A külön helyreállítási szövegek jelöltek: az eredeti formázás, hiányzó tartalom és jelentés nem igazolt. Eredeti programokat és makrókat ne indítson el automatikusan.","Reports retain the warnings. Separate recovered text editions are candidates: original formatting, missing content and meaning remain unverified. Do not automatically run original programs or macros.")).unwrap();
    if data.is_null() {
        write!(html,"<p class=\"notice\">{}</p>",tr("Ehhez a csomaghoz nem áll rendelkezésre értelmezhető végleges fájljelentés. A fájlok épségére vagy teljes kinyerésére nem teszünk állítást; nézze meg a Reports mappát.","No readable final file report is available for this package. No file-completeness claim is made; see Reports.")).unwrap();
    }
    if missing > 0 {
        write!(
            html,
            "<p class=\"notice\">{}: {missing}</p>",
            tr(
                "Nem elérhető vagy eltérő ellenőrzőösszegű hivatkozások kihagyva",
                "Unavailable or hash-mismatching links omitted"
            )
        )
        .unwrap();
    }
    if let Some(path) = report_path {
        let workbook = path.replace("FloppyFinalReport.json", "FloppyFinalReport.xlsx");
        if members.contains_key(workbook.as_str()) {
            write!(
                html,
                "<p><a class=\"folder\" href=\"{}\">{}</a></p>",
                href(&workbook)?,
                tr(
                    "Részletes végleges jelentés (Excel)",
                    "Detailed final report (Excel)"
                )
            )
            .unwrap();
        }
    }
    html.push_str("<nav>");
    for n in disks.keys() {
        write!(html, "<a href=\"#disk-{n}\">{n}</a>").unwrap();
    }
    html.push_str("</nav>");
    for (n, (a, links)) in disks {
        write!(html,"<details id=\"disk-{n}\"><summary>{} {n}<span class=\"pill{}\">{}</span><small>{} {}</small></summary>",tr("Lemez","Disk"),if a{" attention"}else{""},if a{tr("Figyelmet igényel","Needs attention")}else{tr("A jelentés szerint ellenőrizve","Checked within report scope")},links.len(),tr("kiadás","editions")).unwrap();
        html.push_str("<ul class=\"files\">");
        for (path, name) in links {
            write!(
                html,
                "<li><a href=\"{}\">{}</a></li>",
                href(&path)?,
                escaped(&name)
            )
            .unwrap();
        }
        html.push_str("</ul>");
        for (prefix, label) in [
            (
                "Converted",
                tr(
                    "Eredetik és kiadások mappája",
                    "Originals and editions folder",
                ),
            ),
            (
                "Recovery",
                tr(
                    "Külön helyreállítási bizonyítékok és szövegjelöltek",
                    "Separate recovery evidence and text candidates",
                ),
            ),
        ] {
            let path = format!("{prefix}/{n}");
            if rows
                .iter()
                .any(|r| r.archive_path.starts_with(&format!("{path}/")))
            {
                write!(
                    html,
                    "<p><a class=\"folder\" href=\"{}\">{label}</a></p>",
                    href(&path)?
                )
                .unwrap();
            }
        }
        html.push_str("</details>");
    }
    write!(html,"<footer>{}<br>FluxVault {}</footer></main></body></html>",tr("A PACKAGE_MANIFEST.csv tartalmazza az archivált fájlok SHA-256 ellenőrzőösszegeit. A munkapéldány melletti oldal is ehhez az archív kiadáshoz tartozik; újabb módosításokat nem tanúsít.","PACKAGE_MANIFEST.csv records SHA-256 hashes of archived files. A copy beside the working project also describes this archived edition; it does not certify later changes."),env!("CARGO_PKG_VERSION")).unwrap();
    if html.len() > 8 * 1024 * 1024 {
        return Err("Customer index exceeds publication bound".into());
    }
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::final_report::Language;
    use serde_json::json;

    #[test]
    fn local_links_encode_names_and_reject_traversal_schemes_and_ads() {
        assert_eq!(
            href("Converted\\001\\Á & #.pdf").unwrap(),
            "Converted/001/%C3%81%20%26%20%23.pdf"
        );
        for p in [
            "",
            "/file",
            "../a",
            "a/../b",
            "a//b",
            "javascript:evil",
            "a/b:stream",
            "C:\\file",
        ] {
            assert!(href(p).is_err(), "{p}");
        }
        assert_eq!(escaped("<&\"'>"), "&lt;&amp;&quot;&#39;&gt;");
    }

    #[test]
    fn index_is_bilingual_and_links_only_current_valid_manifest_bound_editions() {
        let root = std::env::temp_dir().join(format!(
            "fv-customer-index-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let mut files = vec![];
        let mut rows = vec![];
        let mut add = |name: &str, bytes: Vec<u8>| {
            let source = root.join(format!("{}.json", files.len()));
            fs::write(&source, &bytes).unwrap();
            files.push(PackageFile {
                source,
                archive_path: name.into(),
                expected_sha256: None,
            });
            rows.push(ManifestRow {
                archive_path: name.into(),
                bytes: bytes.len() as u64,
                modified_utc: String::new(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            });
        };
        let payload_hash = format!("{:x}", Sha256::digest(b"pdf"));
        add(
            "Reports/FinalReportLatest.json",
            json!({"directory":"FinalReports/current"})
                .to_string()
                .into_bytes(),
        );
        add("Reports/FinalReports/current/FloppyFinalReport.json", json!({
            "schema_version":1,
            "floppies":[{"Floppy":"001","AuditStatus":"OK"},{"Floppy":"002","AuditStatus":"ATTENTION"},{"Floppy":"","AuditStatus":"OK"},{"Floppy":"<script>","AuditStatus":"OK"}],
            "delivery_files":[
                {"Floppy":"001","FileRole":"Generated conversion","Integrity":"VALID","DeliveryPath":"001/Á & #.pdf","SHA256":payload_hash},
                {"Floppy":"001","FileRole":"Generated conversion","Integrity":"VALID","DeliveryPath":"001/stale.pdf","SHA256":"wrong"},
                {"Floppy":"001","FileRole":"Generated conversion","Integrity":"INVALID","DeliveryPath":"001/invalid.pdf","SHA256":payload_hash},
                {"Floppy":"001","FileRole":"Recovered source","Integrity":"VALID","DeliveryPath":"001/program.exe","SHA256":payload_hash}
            ]
        }).to_string().into_bytes());
        add("Converted/001/Á & #.pdf", b"pdf".to_vec());
        add("Converted/001/stale.pdf", b"pdf".to_vec());
        add("Converted/001/invalid.pdf", b"pdf".to_vec());
        add("Converted/001/program.exe", b"pdf".to_vec());
        for (lang, title) in [
            (Language::Hungarian, "Olvassa el először"),
            (Language::English, "Read this first"),
        ] {
            let h = build(&files, &rows, "<Customer & test>", lang).unwrap();
            assert!(h.contains(title));
            assert!(h.contains("&lt;Customer &amp; test&gt;"));
            assert!(h.contains("href=\"Converted/001/%C3%81%20%26%20%23.pdf\""));
            assert!(h.contains("Á &amp; #.pdf"));
            assert!(h.contains("disk-002"));
            for forbidden in [
                "stale.pdf",
                "invalid.pdf",
                "program.exe",
                "<script>",
                "id=\"disk-\"",
            ] {
                assert!(!h.contains(forbidden), "{forbidden}");
            }
            assert!(h.contains("default-src 'none'"));
        }
        fs::write(&files[1].source, b"changed after archival copy").unwrap();
        assert!(
            build(&files, &rows, "test", Language::Hungarian)
                .unwrap_err()
                .contains("changed")
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn historical_or_raw_only_package_has_explicit_unknown_report_notice() {
        let h = build(&[], &[], "raw-only", Language::English).unwrap();
        assert!(h.contains("No readable final file report"));
        assert!(!h.contains("<details"));
        assert!(h.contains("not a certificate"));
    }
}
