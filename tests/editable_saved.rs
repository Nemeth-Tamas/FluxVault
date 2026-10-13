//! Opt-in source-preserving saved-capture replay; never opens floppy hardware.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
fn hash(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    format!("{:x}", digest.finalize())
}
fn rebase(v: &mut Value, old: &[String], new: &str) {
    match v {
        Value::String(s) => {
            for o in old {
                if s.contains(o) {
                    *s = s.replace(o, new);
                    break;
                }
            }
        }
        Value::Array(a) => {
            for v in a {
                rebase(v, old, new);
            }
        }
        Value::Object(o) => {
            for v in o.values_mut() {
                rebase(v, old, new);
            }
        }
        _ => {}
    }
}
#[test]
#[ignore = "requires FV_EDITABLE_SOURCE and fresh FV_EDITABLE_OUTPUT; copies selected saved images/logs, no media or recapture"]
fn saved_damaged_word_and_rtf_create_editable_editions_preserve_sources_and_refuse_tamper() {
    let source = PathBuf::from(std::env::var_os("FV_EDITABLE_SOURCE").unwrap());
    let output = PathBuf::from(std::env::var_os("FV_EDITABLE_OUTPUT").unwrap());
    assert!(!output.exists());
    let target = fluxvault::project::ProjectState::create_without_session(output.clone()).unwrap();
    let old = [
        source
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        source.to_string_lossy().into_owned(),
    ];
    let new = target
        .root()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut sources = BTreeMap::new();
    // Keep raw/packed capture, decode and publication proof bytes EXACTLY as
    // recorded, including historical command arguments. Only catalog locations
    // are rebound below. Never relabel GW images or strip their verification.
    for folder in ["Flux", "Flux/Derived", "Flux/Recovery"] {
        let destination = output.join(folder);
        fs::create_dir_all(&destination).unwrap();
        for entry in fs::read_dir(source.join(folder)).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if ![17, 22, 24, 27, 52, 128]
                .iter()
                .any(|d| name.starts_with(&format!("{d:03}_")))
            {
                continue;
            }
            sources.insert(entry.path(), hash(&entry.path()));
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
    for disk in [17, 22, 24, 27, 52, 128] {
        for entry in fs::read_dir(source.join("Images")).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy();
            if !name.starts_with(&format!("{disk:03}_attempt_"))
                || !name.ends_with(".json")
                || name.contains(".partial.")
            {
                continue;
            }
            let mut metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            // A derived image needs its complete multi-source recipe; retain
            // original physical attempts only in this minimal test copy.
            if metadata.get("offline_provenance").is_some() {
                continue;
            }
            sources.insert(path.clone(), hash(&path));
            let image = Path::new(metadata["image_file"].as_str().unwrap())
                .file_name()
                .unwrap();
            let image_source = source.join("Images").join(image);
            sources.insert(image_source.clone(), hash(&image_source));
            fs::copy(&image_source, target.images_dir().join(image)).unwrap();
            let log = Path::new(metadata["log_file"].as_str().unwrap())
                .file_name()
                .unwrap();
            let log_source = source.join("Logs").join(log);
            sources.insert(log_source.clone(), hash(&log_source));
            fs::copy(&log_source, target.logs_dir().join(log)).unwrap();
            rebase(&mut metadata, &old, &new);
            fs::write(
                target.images_dir().join(path.file_name().unwrap()),
                serde_json::to_vec_pretty(&metadata).unwrap(),
            )
            .unwrap();
        }
    }
    let mut editions = Vec::new();
    for disk in [17, 22, 24, 27, 52, 128] {
        let invoke = |language: &str| {
            Command::new(env!("CARGO_BIN_EXE_fluxvault"))
                .args([
                    "recovery",
                    "documents",
                    &disk.to_string(),
                    "--language",
                    language,
                    "--json",
                ])
                .current_dir(&output)
                .output()
                .unwrap()
        };
        for language in ["hu", "en"] {
            let r = invoke(language);
            assert_eq!(
                r.status.code(),
                Some(3),
                "{}",
                String::from_utf8_lossy(&r.stdout)
            );
            let result: Value = serde_json::from_slice(&r.stdout).unwrap();
            let selected = result["native_recovery"]["editable_editions"]
                .as_array()
                .unwrap();
            for e in selected {
                for p in e["paths"].as_array().unwrap() {
                    let p = PathBuf::from(p.as_str().unwrap());
                    let expected = hash(&p);
                    let mut z = zip::ZipArchive::new(fs::File::open(&p).unwrap()).unwrap();
                    assert!(z.by_name("word/document.xml").is_ok());
                    assert!(z.by_name("_rels/.rels").is_ok());
                    editions.push((p, expected));
                }
            }
            let r = invoke(language);
            assert_eq!(r.status.code(), Some(3));
        }
    }
    assert!(
        editions.len() >= 4,
        "No meaningful damaged-text replay editions"
    );
    for (p, expected) in &editions {
        assert_eq!(hash(p), *expected);
    }
    let mut office_conversion = None;
    if std::env::var("FV_EDITABLE_OFFICE").as_deref() == Ok("1") {
        // Product integration, not a visual/layout acceptance test: run the
        // existing audited, timeout-bounded conversion service on copies.
        let interop = output.join("Interoperability");
        let project =
            fluxvault::project::ProjectState::create_without_session(interop.clone()).unwrap();
        let directory = project.extracted_dir().join("001");
        fs::create_dir(&directory).unwrap();
        for (i, (path, _)) in editions.iter().enumerate() {
            // The normal planner intentionally mirrors modern DOCX without
            // reconversion. A fixture-only .doc alias exercises LibreOffice's
            // content sniffing through that existing legacy conversion path;
            // these are not customer filenames or acquisition metadata.
            fs::copy(path, directory.join(format!("edition_{i:03}.doc"))).unwrap();
        }
        let converted = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
            .args(["conversion", "run", "--conversion-workers", "2", "--json"])
            .current_dir(&interop)
            .output()
            .unwrap();
        assert!(
            converted.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&converted.stdout),
            String::from_utf8_lossy(&converted.stderr)
        );
        let pdfs = fs::read_dir(project.converted_dir().join("001"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|e| e == "pdf"))
            .count();
        assert_eq!(pdfs, editions.len());
        office_conversion = Some(
            serde_json::json!({"verified_pdf_outputs":pdfs,"visual_layout_acceptance":false,"receipt":serde_json::from_slice::<Value>(&converted.stdout).unwrap()}),
        );
    }
    // A changed editable derivative must be preserved and must block reuse.
    let changed = &editions[0].0;
    fs::write(changed, b"operator-changed editable copy").unwrap();
    let disk = changed
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .file_name()
        .unwrap();
    let language = if changed
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("_hu_")
    {
        "hu"
    } else {
        "en"
    };
    let rejected = Command::new(env!("CARGO_BIN_EXE_fluxvault"))
        .args([
            "recovery",
            "documents",
            disk.to_str().unwrap(),
            "--language",
            language,
            "--json",
        ])
        .current_dir(&output)
        .output()
        .unwrap();
    assert_ne!(rejected.status.code(), Some(3));
    assert_eq!(
        fs::read(changed).unwrap(),
        b"operator-changed editable copy"
    );
    for (p, expected) in &sources {
        assert_eq!(hash(p), *expected, "Source changed: {}", p.display());
    }
    fs::write(output.join("Reports/EditableValidation.json"),serde_json::to_vec_pretty(&serde_json::json!({"physical_media_access":false,"source_files_unchanged":sources.len(),"editions":editions,"tampered_derivative_preserved_and_reuse_refused":true,"office_conversion":office_conversion,"repaired_originals":0,"customer_delivery_certified":false})).unwrap()).unwrap();
    // Leave artifacts for optional independent installed-tool interoperability.
}
