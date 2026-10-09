//! Bounded, project-bound finalization receipts. Never trust a receipt as proof
//! that saved inputs or a finished package are still valid today.
use crate::project::ProjectState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const LATEST: &str = ".fluxvault-finalize.json";
const HISTORY: &str = ".fluxvault-finalizations";
const LIMIT: usize = 65536;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Options {
    pub destination: PathBuf,
    pub workers: usize,
    pub allow_attention: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Processing,
    Packaging,
    BlockedByAttention,
    CompleteClean,
    CompleteAttention,
    Interrupted,
    Failed,
}
impl Phase {
    pub(super) fn complete(self) -> bool {
        matches!(self, Self::CompleteClean | Self::CompleteAttention)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub schema: u32,
    pub project: PathBuf,
    pub run: String,
    pub started_ms: u64,
    pub updated_ms: u64,
    pub options: Options,
    pub phase: Phase,
    pub processing: Option<Value>,
    pub package: Option<Value>,
    pub error: Option<String>,
    pub customer_delivery_certified: bool,
}

pub(super) fn workstation(path: &Path) -> Result<(), String> {
    crate::safety::workstation_path(path)
}

pub(super) fn destination(cwd: &Path, root: &Path, path: PathBuf) -> Result<PathBuf, String> {
    workstation(&path)?;
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    workstation(&path)?;
    let resolved = path.canonicalize().map_err(|e| {
        format!(
            "Package destination must be an existing directory: {} ({e})",
            path.display()
        )
    })?;
    workstation(&resolved)?;
    if !resolved.is_dir() || resolved.starts_with(root) {
        return Err("Package destination must be an existing folder outside the project".into());
    }
    Ok(resolved)
}

fn regular(path: &Path) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Linked finalization receipt refused".into());
        }
    }
    if !m.file_type().is_file() {
        return Err("Finalization receipt must be a regular file".into());
    }
    Ok(())
}

pub(super) fn load(project: &ProjectState) -> Result<Option<Record>, String> {
    crate::processing::validate_workspace(project)?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let path = root.join(LATEST);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        _ => {}
    }
    regular(&path)?;
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|e| e.to_string())?
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("Oversized finalization receipt".into());
    }
    let record: Record =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid finalization receipt: {e}"))?;
    if record.schema != 1
        || record.project != root
        || record.customer_delivery_certified
        || record.run.is_empty()
        || record.run.len() > 100
        || !record.run.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        || !(1..=16).contains(&record.options.workers)
        || record.updated_ms < record.started_ms
        || !record.options.destination.is_absolute()
    {
        return Err("Invalid finalization receipt identity/settings".into());
    }
    // Reject device destinations without opening the saved path. Resume resolves
    // and checks the workstation destination again before starting any tools.
    workstation(&record.options.destination)?;
    Ok(Some(record))
}

impl Record {
    pub(super) fn new(project: &ProjectState, options: Options) -> Result<Self, String> {
        if !(1..=16).contains(&options.workers) {
            return Err("Conversion workers must be 1-16".into());
        }
        crate::processing::validate_workspace(project)?;
        let now = crate::external_tools::current_unix_ms();
        Ok(Self {
            schema: 1,
            project: project.root().canonicalize().map_err(|e| e.to_string())?,
            run: format!(
                "{now}-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            started_ms: now,
            updated_ms: now,
            options,
            phase: Phase::Processing,
            processing: None,
            package: None,
            error: None,
            customer_delivery_certified: false,
        })
    }

    pub(super) fn save(&mut self, project: &ProjectState) -> Result<(), String> {
        // Caller owns the long-lived project writer. Save receipts even when
        // cancellation is set so an interrupted operation remains explainable.
        load(project)?;
        if self.project != project.root().canonicalize().map_err(|e| e.to_string())? {
            return Err("Finalization receipt escapes project".into());
        }
        self.updated_ms = crate::external_tools::current_unix_ms().max(self.started_ms);
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Oversized finalization receipt; prior state retained".into());
        }
        let history = self.project.join(HISTORY);
        match fs::create_dir(&history) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        if history.canonicalize().map_err(|e| e.to_string())?.parent()
            != Some(self.project.as_path())
        {
            return Err("Finalization history escapes project".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if fs::symlink_metadata(&history)
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("Linked finalization history refused".into());
            }
        }
        if fs::symlink_metadata(&history)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Linked finalization history refused".into());
        }
        let suffix = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let receipt = history.join(format!("{}-{suffix}.json", self.run));
        write_new(&receipt, &bytes)?;
        let temporary = self.project.join(format!(
            ".fluxvault-finalize-{}-{suffix}.partial.json",
            self.run
        ));
        write_new(&temporary, &bytes)?;
        // Recheck the pointer kind immediately before replacing it.
        load(project)?;
        fs::rename(temporary, self.project.join(LATEST)).map_err(|e| e.to_string())
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
