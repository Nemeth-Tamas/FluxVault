//! Durable, source-bound Office publication. Unsealed scratch is never adopted.
use crate::{
    conversion::{self, ConversionJob},
    conversion_run::validate_output,
    sector_inspection,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub(crate) const LIMIT: u64 = 128 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    project: PathBuf,
    source: PathBuf,
    source_sha256: String,
    target: PathBuf,
    extension: String,
    filter: String,
    source_type: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    binding: Binding,
    output_sha256: String,
    bytes: u64,
}
pub(crate) struct Context {
    binding: Binding,
    directory: PathBuf,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn regular(path: &Path, dir: bool) -> Result<(), String> {
    sector_inspection::regular(path, dir)
}
fn bounded_hash(path: &Path) -> Result<String, String> {
    regular(path, false)?;
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > LIMIT {
        return Err("Office publication payload exceeds 128 MiB".into());
    }
    conversion::sha256_file(path)
}
pub(crate) fn context(
    reports: &Path,
    job: &ConversionJob,
    target: &Path,
    extension: &str,
    filter: &str,
) -> Result<Context, String> {
    regular(reports, true)?;
    let project = reports
        .canonicalize()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("Office reports have no project")?
        .to_path_buf();
    regular(&job.source_path, false)?;
    let source = job.source_path.canonicalize().map_err(|e| e.to_string())?;
    regular(target.parent().ok_or("Office output has no parent")?, true)?;
    let target = target
        .parent()
        .unwrap()
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(target.file_name().ok_or("Office output has no filename")?);
    if fs::symlink_metadata(&target).is_ok() {
        regular(&target, false)?;
    }
    if !source.starts_with(project.join("Extracted"))
        || !target.starts_with(project.join("Converted"))
    {
        return Err("Office publication input/output escapes the project".into());
    }
    if !matches!(extension, "docx" | "xlsx" | "pptx" | "pdf")
        || job.source_sha256.len() != 64
        || !job.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid Office publication binding".into());
    }
    let binding = Binding {
        project: project.clone(),
        source,
        source_sha256: job.source_sha256.to_ascii_lowercase(),
        target,
        extension: extension.into(),
        filter: filter.into(),
        source_type: job.source_type.clone(),
    };
    let key = hash(&serde_json::to_vec(&binding).map_err(|e| e.to_string())?);
    Ok(Context {
        binding,
        directory: project
            .join("Reports/.fluxvault-conversion-output")
            .join(key),
    })
}
impl Context {
    fn source_unchanged(&self) -> Result<(), String> {
        if bounded_hash(&self.binding.source)? != self.binding.source_sha256 {
            return Err("Office source changed from its conversion-plan binding; no output published/reused".into());
        }
        Ok(())
    }
    pub(crate) fn snapshot_source(&self, target: &Path) -> Result<(), String> {
        self.source_unchanged()?;
        copy_new(&self.binding.source, target)?;
        if bounded_hash(target)? != self.binding.source_sha256 {
            return Err("Office input changed during private snapshot copy".into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn replay(&self) -> Result<Option<String>, String> {
        self.replay_with_stage(&|_| {})
    }
    pub(crate) fn replay_with_stage(
        &self,
        stage: &impl Fn(&str),
    ) -> Result<Option<String>, String> {
        self.source_unchanged()?;
        if !self.directory.try_exists().map_err(|e| e.to_string())? {
            return Ok(None);
        }
        regular(&self.directory, true)?;
        let mut candidates = Vec::new();
        let mut entries = 0;
        for entry in fs::read_dir(&self.directory).map_err(|e| e.to_string())? {
            crate::cancellation::check()?;
            entries += 1;
            if entries > 4096 {
                return Err("Office publication receipt search exceeds bound".into());
            }
            let dir = entry.map_err(|e| e.to_string())?.path();
            regular(&dir, true)?;
            let receipt = dir.join("receipt.json");
            if !receipt.try_exists().map_err(|e| e.to_string())? {
                continue;
            }
            regular(&receipt, false)?;
            let mut bytes = Vec::new();
            File::open(&receipt)
                .map_err(|e| e.to_string())?
                .take(65537)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 65536 {
                return Err("Oversized Office publication receipt".into());
            }
            let r: Receipt = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if r.schema != 1
                || r.binding != self.binding
                || r.bytes > LIMIT
                || r.output_sha256.len() != 64
                || !r.output_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Office publication receipt binding differs from current job".into());
            }
            let payload = dir.join("payload");
            if fs::metadata(&payload).map_err(|e| e.to_string())?.len() != r.bytes
                || bounded_hash(&payload)? != r.output_sha256
                || !validate_output(&payload, &self.binding.extension)?
            {
                return Err("Prepared Office output changed; preserved without reuse".into());
            }
            candidates.push((r.output_sha256, payload));
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let expected = &candidates[0].0;
        if candidates.iter().any(|(h, _)| h != expected) {
            return Err("Competing sealed Office outputs; no automatic byte winner".into());
        }
        self.source_unchanged()?;
        if self
            .binding
            .target
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            if bounded_hash(&self.binding.target)? != *expected
                || !validate_output(&self.binding.target, &self.binding.extension)?
            {
                return Err("Published Office output changed; no replacement allowed".into());
            }
        } else {
            // A prepared output still consumes workstation storage when restored.
            // Admission is needed only for that copy, not an already-published reuse.
            let _budget = crate::resource_budget::background(
                crate::resource_budget::Kind::Office,
                64 * crate::resource_budget::MIB,
                &[(
                    self.binding.target.parent().unwrap(),
                    LIMIT + 64 * crate::resource_budget::MIB,
                )],
                stage,
            )?;
            promote(&candidates[0].1, &self.binding.target, expected)?;
        }
        self.source_unchanged()?;
        Ok(Some(expected.clone()))
    }
    pub(crate) fn publish(
        &self,
        made: &Path,
        cut: &impl Fn(&str) -> Result<(), String>,
    ) -> Result<String, String> {
        self.source_unchanged()?;
        if self
            .binding
            .target
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err("Office output appeared during conversion; nothing overwritten".into());
        }
        if !validate_output(made, &self.binding.extension)? {
            return Err("Office output is not structurally valid".into());
        }
        let sha = bounded_hash(made)?;
        let private = self
            .directory
            .parent()
            .ok_or("Office publication has no private parent")?;
        ensure_directory(
            private,
            private
                .parent()
                .ok_or("Office publication has no reports parent")?,
        )?;
        ensure_directory(&self.directory, private)?;
        let dir = self.directory.join(format!(
            "{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).map_err(|e| e.to_string())?;
        let payload = dir.join("payload");
        copy_new(made, &payload)?;
        if bounded_hash(&payload)? != sha {
            return Err("Office output changed while staging".into());
        }
        cut("payload")?;
        self.source_unchanged()?;
        let bytes = serde_json::to_vec(&Receipt {
            schema: 1,
            binding: self.binding.clone(),
            output_sha256: sha.clone(),
            bytes: fs::metadata(&payload).map_err(|e| e.to_string())?.len(),
        })
        .map_err(|e| e.to_string())?;
        let partial = dir.join("receipt.partial.json");
        write_new(&partial, &bytes)?;
        fs::hard_link(&partial, dir.join("receipt.json")).map_err(|e| e.to_string())?;
        cut("sealed")?;
        crate::cancellation::check()?;
        self.source_unchanged()?;
        promote(&payload, &self.binding.target, &sha)?;
        cut("published")?;
        self.source_unchanged()?;
        Ok(sha)
    }
}

#[cfg(test)]
#[path = "conversion_publication_tests.rs"]
mod tests;
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
fn ensure_directory(path: &Path, parent: &Path) -> Result<(), String> {
    regular(parent, true)?;
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    regular(path, true)?;
    if path.canonicalize().map_err(|e| e.to_string())?.parent()
        != Some(parent.canonicalize().map_err(|e| e.to_string())?.as_path())
    {
        return Err("Office publication directory escapes its parent".into());
    }
    Ok(())
}
fn copy_new(source: &Path, target: &Path) -> Result<(), String> {
    regular(source, false)?;
    let input = File::open(source).map_err(|e| e.to_string())?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(|e| e.to_string())?;
    let count =
        std::io::copy(&mut input.take(LIMIT + 1), &mut output).map_err(|e| e.to_string())?;
    if count > LIMIT {
        return Err("Office snapshot exceeds bound; partial evidence retained".into());
    }
    output.sync_all().map_err(|e| e.to_string())
}
fn promote(payload: &Path, target: &Path, expected: &str) -> Result<(), String> {
    let stage = target.with_file_name(format!(
        ".fluxvault-office-{}-{}.partial",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    copy_new(payload, &stage)?;
    if bounded_hash(&stage)? != expected {
        return Err("Office publication stage hash changed".into());
    }
    crate::cancellation::check()?;
    fs::hard_link(&stage, target)
        .map_err(|e| format!("Office publication refused to replace existing output: {e}"))?;
    if bounded_hash(target)? != expected {
        return Err("Office output changed during publication".into());
    }
    fs::remove_file(stage).map_err(|e| e.to_string())
}
