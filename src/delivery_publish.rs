//! Root-level, immutable handoff copies from a hash-bound verified archive.
//! The caller holds project ownership/snapshot. No source images or media opened.
use crate::{final_report, project::ProjectState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use zip::ZipArchive;

const HISTORY: &str = ".fluxvault-delivery";
const READY: &str = "DELIVERY_READY.json";
const REPORT_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    bytes: u64,
    sha256: String,
    member: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    project: PathBuf,
    source_zip: PathBuf,
    archive_sha256: String,
    archive: String,
    reports: Vec<Artifact>,
    archive_bytes: u64,
    customer_delivery_certified: bool,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())
}
fn name(n: &str) -> Result<(), String> {
    if n.is_empty()
        || n.len() > 240
        || n.contains(['\\', '/', ':'])
        || n.ends_with(['.', ' '])
        || !n
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        || Path::new(n)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || matches!(
            n.split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
    {
        return Err("Unsafe handoff filename".into());
    }
    Ok(())
}
fn checked(path: &Path, directory: bool) -> Result<(), String> {
    crate::safety::workstation_path(path)?;
    let mut ancestor = Some(path);
    while let Some(p) = ancestor {
        match fs::symlink_metadata(p) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err("Linked handoff path refused".into());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err("Reparse handoff path refused".into());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        ancestor = p.parent();
    }
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err("Nonregular handoff path refused".into());
    }
    crate::safety::workstation_path(&path.canonicalize().map_err(|e| e.to_string())?)
}
fn read_small(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    checked(path, false)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Oversized handoff record".into());
    }
    Ok(bytes)
}
fn member(zip: &mut ZipArchive<File>, n: &str, limit: u64) -> Result<Vec<u8>, String> {
    crate::cancellation::check()?;
    let mut f = zip
        .by_name(n)
        .map_err(|e| format!("Missing archived report {n}: {e}"))?;
    if f.is_dir() || f.size() > limit {
        return Err("Oversized/nonregular archived report".into());
    }
    let size = f.size();
    let mut bytes = Vec::new();
    f.by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != size {
        return Err("Archived report size mismatch".into());
    }
    Ok(bytes)
}
fn load(path: &Path, root: &Path) -> Result<Receipt, String> {
    let r: Receipt =
        serde_json::from_slice(&read_small(path, 64 * 1024)?).map_err(|e| e.to_string())?;
    name(&r.archive)?;
    if r.schema != 1
        || r.project != root
        || r.customer_delivery_certified
        || !valid_hash(&r.archive_sha256)
        || !r.source_zip.is_absolute()
        || r.reports.is_empty()
        || r.reports.len() > 16
        || !r.archive.ends_with(".zip")
    {
        return Err("Invalid/foreign handoff receipt".into());
    }
    crate::safety::workstation_path(&r.source_zip)?;
    let mut names = BTreeSet::from([r.archive.to_ascii_lowercase()]);
    for a in &r.reports {
        name(&a.name)?;
        if !valid_hash(&a.sha256)
            || a.bytes > REPORT_LIMIT
            || !names.insert(a.name.to_ascii_lowercase())
        {
            return Err("Invalid handoff artifact binding".into());
        }
    }
    Ok(r)
}
fn verify(path: &Path, bytes: u64, sha: &str) -> Result<(), String> {
    checked(path, false)?;
    if fs::metadata(path).map_err(|e| e.to_string())?.len() != bytes
        || crate::conversion::sha256_file(path)? != sha
    {
        return Err(format!(
            "Handoff copy changed; refusing replacement: {}",
            path.display()
        ));
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
fn promote(stage: &Path, target: &Path) -> Result<(), String> {
    crate::cancellation::check()?;
    // Unlike rename on Unix, hard_link never replaces an existing target.
    // Stage is a separately streamed copy, NOT a link to source evidence.
    fs::hard_link(stage, target)
        .map_err(|e| format!("Cannot publish handoff without replacement: {e}"))?;
    fs::remove_file(stage).map_err(|e| e.to_string())
}
fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

/// Publish only copies. The external ZIP remains the durable original receipt.
pub(crate) fn publish(project: &ProjectState, package: &mut Value) -> Result<(), String> {
    crate::cancellation::check()?;
    crate::processing::validate_workspace(project)?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    checked(&root, true)?;
    let source = PathBuf::from(
        package["zip"]
            .as_str()
            .ok_or("Archive receipt has no ZIP")?,
    );
    let sha = package["sha256"]
        .as_str()
        .filter(|h| valid_hash(h))
        .ok_or("Archive receipt has no valid hash")?
        .to_ascii_lowercase();
    checked(&source, false)?;
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    eprintln!("HANDOFF / verifying the finished archive hash...");
    if crate::conversion::sha256_file(&source)? != sha {
        return Err("Archive hash changed before handoff".into());
    }
    let archive_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid archive name")?
        .to_owned();
    name(&archive_name)?;
    if !archive_name.ends_with(".zip") {
        return Err("Handoff requires a verified ZIP".into());
    }
    let mut zip = ZipArchive::new(File::open(&source).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if zip.len() > 100_000 {
        return Err("Handoff archive entry limit exceeded".into());
    }
    let mut names = BTreeSet::new();
    for i in 0..zip.len() {
        crate::cancellation::check()?;
        let f = zip.by_index(i).map_err(|e| e.to_string())?;
        if f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err("Linked archived handoff member refused".into());
        }
        if !names.insert(f.name().to_ascii_lowercase()) {
            return Err("Duplicate/case-colliding archive members".into());
        }
    }
    let mut contents = Vec::new();
    let short = &sha[..16];
    if names.contains("reports/finalreportlatest.json") {
        let binding_bytes = member(&mut zip, "Reports/FinalReportLatest.json", 64 * 1024)?;
        let binding = final_report::Binding::parse(&binding_bytes)?;
        for (file, expected) in &binding.artifacts_sha256 {
            let path = format!("Reports/{}/{file}", binding.directory);
            let bytes = member(&mut zip, &path, REPORT_LIMIT)?;
            if hash(&bytes) != expected.to_ascii_lowercase() {
                return Err("Archived final-report hash mismatch".into());
            }
            let (stem, ext) = file.rsplit_once('.').ok_or("Report extension missing")?;
            contents.push((format!("FINAL_{stem}-{short}.{ext}"), bytes, Some(path)));
        }
        contents.push((
            format!("FINAL_ReportBinding-{short}.json"),
            binding_bytes,
            Some("Reports/FinalReportLatest.json".into()),
        ));
    } else {
        // Raw-only production has no fabricated image/file/Office report.
        let mut raw = names
            .iter()
            .filter(|n| n.starts_with("reports/rawonlyendpoint-") && n.ends_with(".json"));
        let n = raw
            .next()
            .ok_or("Archive has no final report; refresh reports before packaging")?;
        if raw.next().is_some() {
            return Err("Ambiguous raw-only report generation".into());
        }
        let actual = (0..zip.len())
            .find_map(|i| {
                zip.by_index(i).ok().and_then(|f| {
                    f.name()
                        .eq_ignore_ascii_case(n)
                        .then(|| f.name().to_owned())
                })
            })
            .ok_or("Missing raw-only report")?;
        let bytes = member(&mut zip, &actual, REPORT_LIMIT)?;
        let _: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        contents.push((
            format!("FINAL_RawEvidence-{short}.json"),
            bytes,
            Some(actual),
        ));
    }
    let checksum = format!("{sha}  {archive_name}\n");
    contents.push((
        format!("{archive_name}.sha256"),
        checksum.into_bytes(),
        None,
    ));
    let r = Receipt {
        schema: 1,
        project: root.clone(),
        source_zip: source.clone(),
        archive_sha256: sha.clone(),
        archive: archive_name,
        archive_bytes: fs::metadata(&source).map_err(|e| e.to_string())?.len(),
        reports: contents
            .iter()
            .map(|(name, bytes, member)| Artifact {
                name: name.clone(),
                bytes: bytes.len() as u64,
                sha256: hash(bytes),
                member: member.clone(),
            })
            .collect(),
        customer_delivery_certified: false,
    };
    // Validate an existing ready pointer before producing any new output.
    let pointer = root.join(READY);
    if fs::symlink_metadata(&pointer).is_ok() {
        let old = load(&pointer, &root)?;
        let history = root
            .join(HISTORY)
            .join(format!("{}.json", old.archive_sha256));
        if old != load(&history, &root)? {
            return Err("Unowned/changed ready pointer; preserve it and inspect manually".into());
        }
    }
    let history = root.join(HISTORY);
    match fs::create_dir(&history) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    checked(&history, true)?;
    let intent = history.join(format!("{sha}.json"));
    if fs::symlink_metadata(&intent).is_ok() {
        if load(&intent, &root)? != r {
            return Err("Handoff intent binding changed".into());
        }
    } else {
        for n in std::iter::once(&r.archive).chain(r.reports.iter().map(|a| &a.name)) {
            name(n)?;
            if fs::symlink_metadata(root.join(n)).is_ok() {
                return Err(format!("Unowned handoff filename already exists: {n}"));
            }
        }
        let stage = history.join(format!("{}.partial.json", nonce()));
        write_new(
            &stage,
            &serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?,
        )?;
        promote(&stage, &intent)?;
    }
    let archive_target = root.join(&r.archive);
    eprintln!("HANDOFF / checking or copying the ready ZIP into the project root...");
    if fs::symlink_metadata(&archive_target).is_ok() {
        verify(&archive_target, r.archive_bytes, &sha)?;
    } else {
        let stage = history.join(format!("{}.partial.zip", nonce()));
        let mut input = File::open(&source).map_err(|e| e.to_string())?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stage)
            .map_err(|e| e.to_string())?;
        let mut buffer = [0; 256 * 1024];
        let mut copied = 0_u64;
        let mut progress = std::time::Instant::now();
        loop {
            crate::cancellation::check()?;
            let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            copied += count as u64;
            if progress.elapsed().as_secs() >= 2 {
                eprintln!(
                    "HANDOFF / ZIP copy {}%",
                    copied.saturating_mul(100) / r.archive_bytes.max(1)
                );
                progress = std::time::Instant::now();
            }
        }
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        verify(&stage, r.archive_bytes, &sha)?;
        promote(&stage, &archive_target)?;
    }
    for ((_, bytes, _), artifact) in contents.iter().zip(&r.reports) {
        crate::cancellation::check()?;
        let target = root.join(&artifact.name);
        if fs::symlink_metadata(&target).is_ok() {
            verify(&target, artifact.bytes, &artifact.sha256)?;
        } else {
            let stage = history.join(format!("{}.partial.report", nonce()));
            write_new(&stage, bytes)?;
            verify(&stage, artifact.bytes, &artifact.sha256)?;
            promote(&stage, &target)?;
        }
    }
    // A ready pointer is the commit marker, never publish it before all copies.
    eprintln!("HANDOFF / final hash checks and ready receipt...");
    verify(&source, r.archive_bytes, &sha)?;
    verify(&archive_target, r.archive_bytes, &sha)?;
    for a in &r.reports {
        crate::cancellation::check()?;
        verify(&root.join(&a.name), a.bytes, &a.sha256)?;
    }
    let stage = history.join(format!("{}.partial.ready", nonce()));
    write_new(
        &stage,
        &serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?,
    )?;
    crate::cancellation::check()?;
    if fs::symlink_metadata(&pointer).is_ok() {
        let old = load(&pointer, &root)?;
        if old != load(&history.join(format!("{}.json", old.archive_sha256)), &root)? {
            return Err("Ready pointer changed before publication".into());
        }
        fs::rename(&stage, &pointer).map_err(|e| e.to_string())?;
    } else {
        promote(&stage, &pointer)?;
    }
    package["delivery"] = json!({"zip":archive_target,"sha256_file":root.join(format!("{}.sha256",r.archive)),
        "ready":pointer,"reports":r.reports.iter().filter(|a| a.member.is_some()).map(|a|root.join(&a.name)).collect::<Vec<_>>(),
        "source_zip":source,"sha256":sha,"customer_delivery_certified":false});
    Ok(())
}

#[cfg(test)]
#[path = "delivery_publish_tests.rs"]
mod tests;
