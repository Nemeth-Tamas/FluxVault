use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use chrono::Local;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::{
    conversion::{self, ConversionJob, ConversionPlanningRequest, ConversionPlanningResult},
    external_tools::{self, CommandAudit},
};

#[derive(Debug, Clone)]
pub struct ConversionRequest {
    pub planning: ConversionPlanningRequest,
    pub libreoffice_executable: PathBuf,
    pub command_audit_path: PathBuf,
    pub timeout_seconds: u64,
    pub workers: usize,
    pub selected_sources: Option<Vec<PathBuf>>,
    pub previous_result: Option<Box<ConversionResult>>,
}

pub const DEFAULT_CONVERSION_WORKERS: usize = 4;
const CONVERSION_SNAPSHOT_FILE: &str = "ConversionState.json";

pub(crate) fn snapshot_path(reports_directory: &Path) -> PathBuf {
    reports_directory.join(CONVERSION_SNAPSHOT_FILE)
}
const CONVERSION_RETRY_DELAY: Duration = Duration::from_millis(250);
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub enum ConversionEvent {
    Stage(String),
    Progress { completed: usize, total: usize },
    Finished(Box<Result<ConversionResult, String>>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionResult {
    pub planning: ConversionPlanningResult,
    pub ok: usize,
    pub partial: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub reused_outputs: usize,
    pub retried_outputs: usize,
    pub issues: Vec<ConversionIssue>,
    pub summary_path: PathBuf,
    pub failures_path: PathBuf,
    pub(crate) rows: Vec<JobResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionIssue {
    pub source_path: PathBuf,
    pub floppy: String,
    pub forensic_path: String,
    pub status: String,
    pub modern_result: String,
    pub modern_detail: String,
    pub pdf_result: String,
    pub pdf_detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum OutputState {
    Ok,
    Reused,
    Failed,
    Timeout,
}

impl OutputState {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Reused => "REUSED",
            Self::Failed => "FAILED",
            Self::Timeout => "TIMEOUT",
        }
    }

    pub(crate) fn successful(self) -> bool {
        matches!(self, Self::Ok | Self::Reused)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OutputResult {
    pub(crate) state: OutputState,
    pub(crate) detail: String,
    retryable: bool,
    retry_count: u8,
    #[serde(default)]
    pub(crate) output_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JobResult {
    pub(crate) job: ConversionJob,
    pub(crate) modern: OutputResult,
    pub(crate) pdf: OutputResult,
    pub(crate) duration_seconds: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConversionSnapshot {
    schema_version: u32,
    project_root: PathBuf,
    result: ConversionResult,
}

pub(crate) fn save_snapshot(
    reports_directory: &Path,
    project_root: &Path,
    result: &ConversionResult,
) -> Result<PathBuf, String> {
    let path = snapshot_path(reports_directory);
    let snapshot = ConversionSnapshot {
        schema_version: 1,
        project_root: project_root
            .canonicalize()
            .map_err(|error| format!("Cannot resolve project root: {error}"))?,
        result: result.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("Cannot serialize conversion state: {error}"))?;
    // Keep every completed state immutable before replacing the mutable cursor.
    // A rejected reuse must not erase the only earlier source/output hash binding.
    let history = reports_directory.join("ConversionHistory");
    fs::create_dir_all(&history).map_err(|e| e.to_string())?;
    if history.canonicalize().map_err(|e| e.to_string())?.parent()
        != Some(
            reports_directory
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Conversion history escapes Reports".to_owned());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let record = history.join(format!(
        "ConversionState-{nonce}-{}.json",
        std::process::id()
    ));
    if path.exists() {
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        {
            return Err("Unsafe conversion state path".to_owned());
        }
        let prior = fs::read(&path).map_err(|e| e.to_string())?;
        let mut prior_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(history.join(format!(
                "PreviousConversionState-{nonce}-{}.json",
                std::process::id()
            )))
            .map_err(|e| e.to_string())?;
        prior_file
            .write_all(&prior)
            .and_then(|_| prior_file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    let mut history_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(record)
        .map_err(|e| e.to_string())?;
    history_file
        .write_all(&bytes)
        .and_then(|_| history_file.sync_all())
        .map_err(|e| e.to_string())?;
    let temporary = path.with_file_name(format!(
        ".fluxvault-conversion-state-{}-{nonce}-{}.partial.json",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("Cannot reserve conversion state: {error}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Cannot save conversion state {}: {error}", path.display()))?;
    drop(file);
    fs::rename(&temporary, &path)
        .map_err(|error| format!("Cannot commit conversion state: {error}"))?;
    Ok(path)
}

pub(crate) fn load_snapshot(
    reports_directory: &Path,
    project_root: &Path,
) -> Result<ConversionResult, String> {
    let path = snapshot_path(reports_directory);
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err("Saved conversion state must be a bounded regular file".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Saved conversion state is a reparse point".into());
        }
    }
    let mut bytes = Vec::new();
    File::open(&path)
        .and_then(|f| f.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|error| {
            format!(
                "No saved conversion state {}: {error}; run conversion run first",
                path.display()
            )
        })?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Saved conversion state exceeds 32 MiB".into());
    }
    let snapshot: ConversionSnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid saved conversion state {}: {error}", path.display()))?;
    if snapshot.schema_version != 1
        || snapshot.project_root
            != project_root
                .canonicalize()
                .map_err(|error| format!("Cannot resolve project root: {error}"))?
    {
        return Err("Saved conversion state belongs to a different project or schema".to_owned());
    }
    Ok(snapshot.result)
}

impl JobResult {
    pub(crate) fn status(&self) -> &'static str {
        match (self.modern.state.successful(), self.pdf.state.successful()) {
            (true, true) => "OK",
            (true, false) | (false, true) => "PARTIAL",
            (false, false) => "FAILED",
        }
    }

    fn reason(&self) -> String {
        if self.status() == "OK" {
            String::new()
        } else {
            format!(
                "Modern={}; PDF={}",
                self.modern.state.label(),
                self.pdf.state.label()
            )
        }
    }
}

pub fn spawn_conversion(request: ConversionRequest) -> Receiver<ConversionEvent> {
    let (sender, receiver) = mpsc::channel();
    crate::cancellation::spawn(move || {
        let result = run_conversion(
            &request,
            &|stage| {
                let _ = sender.send(ConversionEvent::Stage(stage.to_owned()));
            },
            &|completed, total| {
                let _ = sender.send(ConversionEvent::Progress { completed, total });
            },
        );
        let _ = sender.send(ConversionEvent::Finished(Box::new(result)));
    });
    receiver
}

pub(crate) fn run_conversion(
    request: &ConversionRequest,
    send_stage: &impl Fn(&str),
    send_progress: &impl Fn(usize, usize),
) -> Result<ConversionResult, String> {
    run_conversion_mode(request, send_stage, send_progress, false)
}

pub(crate) fn run_conversion_mode(
    request: &ConversionRequest,
    send_stage: &impl Fn(&str),
    send_progress: &impl Fn(usize, usize),
    incremental: bool,
) -> Result<ConversionResult, String> {
    if !request.libreoffice_executable.is_file() {
        return Err(format!(
            "A LibreOffice futtatható fájl nem található: {}",
            request.libreoffice_executable.display()
        ));
    }
    if !(10..=600).contains(&request.timeout_seconds) {
        return Err("A konverziós időkorlát 10 és 600 másodperc között lehet.".to_owned());
    }
    if !(1..=16).contains(&request.workers) {
        return Err("A konverziós munkaszálak száma 1 és 16 között lehet.".to_owned());
    }
    if let Some(sources) = &request.selected_sources
        && (sources.is_empty() || request.previous_result.is_none())
    {
        return Err(
            "A kiválasztott újrapróbáláshoz korábbi eredmény és legalább egy forrás szükséges."
                .to_owned(),
        );
    }

    let _reservation = crate::conversion_lock::reserve(&request.planning.reports_directory)?;
    send_stage("Delivery eredetik és friss conversion plan készítése...");
    let planning = {
        let root = request
            .planning
            .reports_directory
            .parent()
            .ok_or("Reports has no project parent")?;
        let _snapshot = crate::project_work::snapshot(root)?;
        conversion::build_conversion_plan_reserved(&request.planning, send_stage)?
    };
    let selected_sources = request.selected_sources.as_ref().map(|sources| {
        sources
            .iter()
            .map(|path| path_identity(path))
            .collect::<HashSet<_>>()
    });
    if let Some(selected) = &selected_sources {
        let matched = planning
            .jobs
            .iter()
            .filter(|job| selected.contains(&path_identity(&job.source_path)))
            .count();
        if matched != selected.len() {
            return Err("Egy kiválasztott fájl már nincs a friss konverziós tervben; futtassa újra a teljes sort.".to_owned());
        }
    }
    let project_root = request
        .planning
        .reports_directory
        .parent()
        .ok_or("Conversion reports directory has no project parent")?;
    let saved_result = if request.previous_result.is_none()
        && snapshot_path(&request.planning.reports_directory).exists()
    {
        Some(load_snapshot(
            &request.planning.reports_directory,
            project_root,
        )?)
    } else {
        None
    };
    let previous_rows = compatible_previous(
        &planning.jobs,
        request.previous_result.as_deref().or(saved_result.as_ref()),
        &request.planning.reports_directory,
        project_root,
    )?;
    if let Some(selected) = &selected_sources
        && planning.jobs.iter().any(|job| {
            selected.contains(&path_identity(&job.source_path))
                && previous_rows
                    .get(&path_identity(&job.source_path))
                    .is_none_or(|previous| {
                        previous.job.source_sha256 != job.source_sha256
                            || !same_path(&previous.job.modern_path, &job.modern_path)
                            || !same_path(&previous.job.pdf_path, &job.pdf_path)
                    })
        })
    {
        return Err("A kiválasztott fájl forrása vagy delivery útvonala megváltozott; a korábbi konverziós eredmény nem használható biztonságos újrapróbáláshoz.".to_owned());
    }
    let total = planning.jobs.len();
    let estimates = planning
        .jobs
        .iter()
        .map(|job| {
            previous_rows
                .get(&path_identity(&job.source_path))
                .filter(|row| row.job.source_sha256 == job.source_sha256 && row.status() == "OK")
                .map(|row| row.duration_seconds)
                .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
                .unwrap_or_else(|| {
                    1.0 + fs::metadata(&job.source_path).map_or(0, |m| m.len()) as f64 / 104_857.6
                })
        })
        .collect::<Vec<_>>();
    let order = balanced_job_order(&estimates);
    send_stage(&format!(
        "Régi Office fájlok átalakítása: {total} fájl, legfeljebb {} párhuzamos munkaszál...",
        request.workers
    ));
    let rows = run_bounded_events(
        &planning.jobs,
        request.workers,
        &order,
        |job, notice| {
            let started = Instant::now();
            let selected = selected_sources
                .as_ref()
                .is_none_or(|sources| sources.contains(&path_identity(&job.source_path)));
            let previous = previous_rows
                .get(&path_identity(&job.source_path))
                .filter(|row| {
                    row.job.source_sha256 == job.source_sha256
                        && same_path(&row.job.modern_path, &job.modern_path)
                        && same_path(&row.job.pdf_path, &job.pdf_path)
                });
            // During continuous processing an unchanged previously attempted job
            // is inspected, not repeatedly relaunched for every following disk.
            // Explicit conversion run/retry retains its existing retry behavior.
            let (modern, pdf) = if selected && !(incremental && previous.is_some()) {
                (
                    retry_transient_failure(|| {
                        convert_output(
                            request,
                            job,
                            &job.modern_path,
                            &job.modern_format.to_ascii_lowercase(),
                            &job.modern_filter,
                            previous.map(|row| &row.modern),
                            &notice,
                        )
                    }),
                    retry_transient_failure(|| {
                        convert_output(
                            request,
                            job,
                            &job.pdf_path,
                            "pdf",
                            &job.pdf_filter,
                            previous.map(|row| &row.pdf),
                            &notice,
                        )
                    }),
                )
            } else {
                (
                    inspect_without_conversion(
                        previous.map(|row| &row.modern),
                        &job.modern_path,
                        &job.modern_format.to_ascii_lowercase(),
                    ),
                    inspect_without_conversion(previous.map(|row| &row.pdf), &job.pdf_path, "pdf"),
                )
            };
            JobResult {
                job: job.clone(),
                modern,
                pdf,
                duration_seconds: started.elapsed().as_secs_f64(),
            }
        },
        |completed| send_progress(completed, total),
        send_stage,
    )?;

    let summary_path = request
        .planning
        .reports_directory
        .join("ConversionSummary.csv");
    let failures_path = request
        .planning
        .reports_directory
        .join("ConversionFailures.txt");
    write_summary(&summary_path, &rows, &request.planning.converted_root)?;
    write_failures(&failures_path, &rows)?;

    let issues = rows
        .iter()
        .filter(|row| row.status() != "OK")
        .map(|row| ConversionIssue {
            source_path: row.job.source_path.clone(),
            floppy: row.job.floppy.clone(),
            forensic_path: row.job.original_forensic_path.clone(),
            status: row.status().to_owned(),
            modern_result: row.modern.state.label().to_owned(),
            modern_detail: row.modern.detail.clone(),
            pdf_result: row.pdf.state.label().to_owned(),
            pdf_detail: row.pdf.detail.clone(),
        })
        .collect();

    let result = ConversionResult {
        ok: rows.iter().filter(|row| row.status() == "OK").count(),
        partial: rows.iter().filter(|row| row.status() == "PARTIAL").count(),
        failed: rows.iter().filter(|row| row.status() == "FAILED").count(),
        timed_out: rows
            .iter()
            .filter(|row| {
                row.modern.state == OutputState::Timeout || row.pdf.state == OutputState::Timeout
            })
            .count(),
        reused_outputs: rows
            .iter()
            .map(|row| {
                usize::from(row.modern.state == OutputState::Reused)
                    + usize::from(row.pdf.state == OutputState::Reused)
            })
            .sum(),
        retried_outputs: rows
            .iter()
            .map(|row| usize::from(row.modern.retry_count + row.pdf.retry_count))
            .sum(),
        issues,
        planning,
        summary_path,
        failures_path,
        rows,
    };
    save_snapshot(&request.planning.reports_directory, project_root, &result)?;
    Ok(result)
}

fn path_identity(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

fn same_path(first: &Path, second: &Path) -> bool {
    first == second
        || match (first.canonicalize(), second.canonicalize()) {
            (Ok(first), Ok(second)) => first == second,
            _ => false,
        }
}

fn same_conversion_binding(old: &ConversionJob, current: &ConversionJob) -> bool {
    // Extraction attempt paths can change after USB -> GW or generation upgrade.
    // The content, label, relative identity, filters and delivery paths cannot.
    old.source_sha256 == current.source_sha256
        && old.floppy == current.floppy
        && old.original_forensic_path == current.original_forensic_path
        && old.delivery_original_path == current.delivery_original_path
        && old.source_type == current.source_type
        && old.modern_format == current.modern_format
        && old.modern_filter == current.modern_filter
        && old.pdf_filter == current.pdf_filter
        && same_path(&old.modern_path, &current.modern_path)
        && same_path(&old.pdf_path, &current.pdf_path)
}

fn compatible_previous(
    jobs: &[ConversionJob],
    previous: Option<&ConversionResult>,
    reports: &Path,
    root: &Path,
) -> Result<HashMap<PathBuf, JobResult>, String> {
    let mut bound = HashMap::<PathBuf, JobResult>::new();
    let merge = |bound: &mut HashMap<PathBuf, JobResult>, rows: &[JobResult]| {
        for job in jobs {
            let key = path_identity(&job.source_path);
            for donor in rows.iter().filter(|r| same_conversion_binding(&r.job, job)) {
                let target = bound.entry(key.clone()).or_insert_with(|| donor.clone());
                // Do not erase a hash that would detect an edited output. Fill
                // missing bindings only from recorded successful conversions.
                for (out, old, path) in [
                    (&mut target.modern, &donor.modern, &job.modern_path),
                    (&mut target.pdf, &donor.pdf, &job.pdf_path),
                ] {
                    if out.output_sha256.is_none()
                        && old.state.successful()
                        && old
                            .output_sha256
                            .as_ref()
                            .is_some_and(|h| conversion::sha256_file(path).as_ref() == Ok(h))
                    {
                        *out = old.clone();
                    }
                }
            }
        }
    };
    if let Some(previous) = previous {
        merge(&mut bound, &previous.rows);
    }
    let needs_history = |bound: &HashMap<PathBuf, JobResult>| {
        jobs.iter().any(|job| {
            let old = bound.get(&path_identity(&job.source_path));
            (job.modern_path.exists() && old.is_none_or(|r| r.modern.output_sha256.is_none()))
                || (job.pdf_path.exists() && old.is_none_or(|r| r.pdf.output_sha256.is_none()))
        })
    };
    let history = reports.join("ConversionHistory");
    if !needs_history(&bound) || !history.exists() {
        return Ok(bound);
    }
    if history.canonicalize().map_err(|e| e.to_string())?.parent()
        != Some(reports.canonicalize().map_err(|e| e.to_string())?.as_path())
    {
        return Err("Conversion history escapes Reports".into());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&history).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if files.len() >= 4096 {
            return Err("Conversion history exceeds 4096-record reuse bound".into());
        }
        files.push(entry.path());
    }
    files.sort();
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut read_bytes = 0;
    for file in files.into_iter().rev() {
        if !needs_history(&bound) {
            break;
        }
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        if !name.starts_with("ConversionState-") && !name.starts_with("PreviousConversionState-") {
            continue;
        }
        if file.extension().is_none_or(|s| s != "json") {
            continue;
        }
        let meta = fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Conversion history reparse point refused".into());
            }
        }
        if !meta.is_file() || meta.len() > 8 * 1024 * 1024 {
            return Err("Invalid bounded conversion history record".into());
        }
        let mut bytes = Vec::new();
        File::open(&file)
            .map_err(|e| e.to_string())?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        read_bytes += bytes.len();
        if bytes.len() > 8 * 1024 * 1024 || read_bytes > 128 * 1024 * 1024 {
            return Err("Conversion history exceeds bounded reuse search".into());
        }
        let snapshot: ConversionSnapshot = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid conversion history record: {e}"))?;
        if snapshot.schema_version != 1 || snapshot.project_root != root {
            return Err("Conversion history belongs to a different project/schema".into());
        }
        merge(&mut bound, &snapshot.result.rows);
    }
    Ok(bound)
}

fn inspect_without_conversion(
    previous: Option<&OutputResult>,
    target: &Path,
    extension: &str,
) -> OutputResult {
    match validate_output(target, extension) {
        Ok(true) => reuse_if_bound(previous, target),
        Ok(false) if target.exists() => failure(format!(
            "Existing output failed integrity validation; preserved without overwrite: {}",
            target.display()
        )),
        Ok(false) => match previous {
            Some(result) if !result.state.successful() => {
                let mut result = result.clone();
                result.retryable = false;
                result.retry_count = 0;
                result
            }
            _ => failure("Not selected for this run; output is missing".to_owned()),
        },
        Err(error) => failure(error),
    }
}

fn reuse_if_bound(previous: Option<&OutputResult>, target: &Path) -> OutputResult {
    let Some(expected_hash) = previous
        .filter(|result| result.state.successful())
        .and_then(|result| result.output_sha256.as_deref())
    else {
        return failure(format!(
            "Valid-looking output has no saved source/output hash binding; preserved without reuse: {}",
            target.display()
        ));
    };
    match conversion::sha256_file(target) {
        Ok(actual_hash) if actual_hash == expected_hash => OutputResult {
            state: OutputState::Reused,
            detail: "Existing output matched saved source and output hashes".to_owned(),
            retryable: false,
            retry_count: 0,
            output_sha256: Some(actual_hash),
        },
        Ok(_) => failure(format!(
            "Existing output hash changed; preserved without reuse: {}",
            target.display()
        )),
        Err(error) => failure(error),
    }
}

fn retry_transient_failure(mut attempt: impl FnMut() -> OutputResult) -> OutputResult {
    let first = attempt();
    if !first.retryable || crate::cancellation::requested() {
        return first;
    }
    thread::sleep(CONVERSION_RETRY_DELAY);
    let mut second = attempt();
    second.detail = format!("Attempt 1: {}; attempt 2: {}", first.detail, second.detail);
    second.retryable = false;
    second.retry_count = 1;
    second
}

// Results arrive in completion order, but reports must retain the plan's stable order.
/// Interleave one expensive job with three inexpensive jobs; idle workers claim from
/// the shared queue, keeping long jobs running without hiding all small jobs behind them.
fn balanced_job_order(estimates: &[f64]) -> Vec<usize> {
    let mut ranked: Vec<_> = (0..estimates.len()).collect();
    ranked.sort_by(|a, b| {
        estimates[*b]
            .total_cmp(&estimates[*a])
            .then_with(|| a.cmp(b))
    });
    let mut order = Vec::with_capacity(ranked.len());
    let (mut front, mut end) = (0, ranked.len());
    while front < end {
        order.push(ranked[front]);
        front += 1;
        for _ in 0..3 {
            if front == end {
                break;
            }
            end -= 1;
            order.push(ranked[end]);
        }
    }
    order
}

#[cfg(test)]
fn run_bounded<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    order: &[usize],
    work: impl Fn(&T) -> R + Sync,
    on_complete: impl Fn(usize),
) -> Result<Vec<R>, String> {
    run_bounded_events(
        items,
        workers,
        order,
        |item, _| work(item),
        on_complete,
        |_| {},
    )
}

enum WorkerEvent<R> {
    Complete(usize, R),
    Stage(String),
}

fn run_bounded_events<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    order: &[usize],
    work: impl Fn(&T, &dyn Fn(&str)) -> R + Sync,
    on_complete: impl Fn(usize),
    on_stage: impl Fn(&str),
) -> Result<Vec<R>, String> {
    if !(1..=16).contains(&workers) {
        return Err("Conversion workers must be from 1 to 16".to_owned());
    }
    let mut seen = vec![false; items.len()];
    if order.len() != items.len()
        || order.iter().any(|index| {
            seen.get_mut(*index)
                .is_none_or(|claimed| std::mem::replace(claimed, true))
        })
    {
        return Err("Conversion schedule must claim every job exactly once".to_owned());
    }
    let mut results: Vec<Option<R>> = std::iter::repeat_with(|| None).take(items.len()).collect();
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..workers.min(items.len()) {
            let sender = sender.clone();
            let next = &next;
            let work = &work;
            let stop_token = crate::cancellation::current();
            scope.spawn(move || {
                let _stop_scope = crate::cancellation::enter(stop_token);
                loop {
                    let slot = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&index) = order.get(slot) else { break };
                    let item = &items[index];
                    let notice = |message: &str| {
                        let _ = sender.send(WorkerEvent::Stage(message.to_owned()));
                    };
                    if sender
                        .send(WorkerEvent::Complete(index, work(item, &notice)))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut completed = 0;
        for event in receiver {
            match event {
                WorkerEvent::Complete(index, result) => {
                    results[index] = Some(result);
                    completed += 1;
                    on_complete(completed);
                }
                WorkerEvent::Stage(message) => on_stage(&message),
            }
        }
    });
    results
        .into_iter()
        .map(|result| {
            result.ok_or_else(|| "A konverziós munkaszál eredmény nélkül leállt.".to_owned())
        })
        .collect()
}

fn convert_output(
    request: &ConversionRequest,
    job: &ConversionJob,
    target: &Path,
    extension: &str,
    filter: &str,
    previous: Option<&OutputResult>,
    stage: &impl Fn(&str),
) -> OutputResult {
    match validate_output(target, extension) {
        Ok(true) => return reuse_if_bound(previous, target),
        Ok(false) if target.exists() => {
            return failure(format!(
                "Existing output failed integrity validation; preserved without overwrite: {}",
                target.display()
            ));
        }
        Err(error) => return failure(error),
        Ok(false) => {}
    }

    let source_bytes = match fs::metadata(&job.source_path) {
        Ok(info) => info.len(),
        Err(error) => return failure(format!("Cannot estimate conversion source: {error}")),
    };
    let reserve = source_bytes
        .saturating_mul(4)
        .saturating_add(64 * crate::resource_budget::MIB);
    let temp = std::env::temp_dir();
    // Reuse needs no admission. Wait before scratch creation and before the
    // host-process timeout starts; resource pressure is not an Office timeout.
    let _budget = match crate::resource_budget::background(
        crate::resource_budget::Kind::Office,
        (512 * crate::resource_budget::MIB).saturating_add(
            source_bytes
                .saturating_mul(4)
                .min(512 * crate::resource_budget::MIB),
        ),
        &[
            (
                target.parent().unwrap_or(&request.planning.converted_root),
                reserve,
            ),
            (&temp, reserve),
        ],
        stage,
    ) {
        Ok(permit) => permit,
        Err(error) => return failure(error),
    };
    let root = temp.join(format!(
        "fluxvault-office-{}-{}-{}-{}",
        std::process::id(),
        unix_ms(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        extension
    ));
    let output_directory = root.join("out");
    let profile_directory = root.join("profile");
    if let Err(error) = fs::create_dir(&root) {
        return failure(format!("Ideiglenes konverziós mappa hiba: {error}"));
    }
    let result = (|| {
        fs::create_dir(&output_directory)
            .map_err(|error| format!("Ideiglenes output mappa hiba: {error}"))?;
        fs::create_dir(&profile_directory)
            .map_err(|error| format!("Ideiglenes LibreOffice profil hiba: {error}"))?;
        let stdout_path = root.join("stdout.txt");
        let stderr_path = root.join("stderr.txt");
        let stdout = File::create(&stdout_path)
            .map_err(|error| format!("LibreOffice stdout fájl hiba: {error}"))?;
        let stderr = File::create(&stderr_path)
            .map_err(|error| format!("LibreOffice stderr fájl hiba: {error}"))?;
        let arguments = vec![
            "--headless".to_owned(),
            "--nologo".to_owned(),
            "--nodefault".to_owned(),
            "--norestore".to_owned(),
            "--nofirststartwizard".to_owned(),
            format!("-env:UserInstallation={}", file_uri(&profile_directory)?),
            "--convert-to".to_owned(),
            format!("{extension}:{filter}"),
            "--outdir".to_owned(),
            output_directory.display().to_string(),
            job.source_path.display().to_string(),
        ];
        let executable = external_tools::libreoffice_console_host(&request.libreoffice_executable);
        let started_unix_ms = unix_ms();
        let started = Instant::now();
        let mut child = crate::process_supervision::spawn(
            Command::new(&executable)
                .args(&arguments)
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr)),
        )
        .map_err(|error| format!("LibreOffice indítási hiba: {error}"))?;
        let mut timed_out = false;
        let mut termination_issue = None;
        let exit_status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None)
                    if started.elapsed() >= Duration::from_secs(request.timeout_seconds)
                        || crate::cancellation::requested() =>
                {
                    timed_out = !crate::cancellation::requested();
                    termination_issue = child.terminate_tree().err();
                    break child.wait().map_err(|error| {
                        format!("LibreOffice timeout utáni wait hiba: {error}")
                    })?;
                }
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    let termination_issue = child.terminate_tree().err();
                    let _ = child.wait();
                    return Err(format!(
                        "LibreOffice wait hiba: {error}{}",
                        termination_issue
                            .map(|issue| format!("; process-tree termination unverified: {issue}"))
                            .unwrap_or_default()
                    ));
                }
            }
        };
        let stdout_text = fs::read(&stdout_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_default();
        let stderr_text = fs::read(&stderr_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_default();
        let audit = CommandAudit {
            tool: "LibreOffice conversion".to_owned(),
            executable,
            arguments,
            started_unix_ms,
            duration_ms: started.elapsed().as_millis(),
            success: exit_status.success() && !timed_out && !crate::cancellation::requested(),
            exit_code: exit_status.code(),
            stdout: stdout_text.clone(),
            stderr: match &termination_issue {
                Some(issue) => {
                    format!("{stderr_text}\nProcess-tree termination unverified: {issue}")
                }
                None => stderr_text.clone(),
            },
            version: None,
            controller_supervision: crate::process_supervision::audit_mode(),
        };
        external_tools::append_audit(&request.command_audit_path, &audit)?;
        crate::cancellation::check()?;
        if timed_out {
            return Ok(OutputResult {
                state: OutputState::Timeout,
                detail: match termination_issue {
                    Some(ref issue) => format!(
                        "LibreOffice exceeded {} seconds; process-tree termination unverified: {issue}",
                        request.timeout_seconds
                    ),
                    None => format!(
                        "LibreOffice exceeded {} seconds; process tree stopped",
                        request.timeout_seconds
                    ),
                },
                retryable: termination_issue.is_none(),
                retry_count: 0,
                output_sha256: None,
            });
        }
        if !exit_status.success() {
            return Ok(retryable_failure(format!(
                "LibreOffice exit {:?}: {} {}",
                exit_status.code(),
                stdout_text,
                stderr_text
            )));
        }
        let made = libreoffice_output_path(&output_directory, &job.source_path, extension)?;
        if !validate_output(&made, extension)? {
            return Ok(retryable_failure(format!(
                "LibreOffice output missing or invalid: {} | {} {}",
                made.display(),
                stdout_text,
                stderr_text
            )));
        }
        if target.exists() {
            return Ok(failure(format!(
                "A cél közben létrejött; nincs felülírás: {}",
                target.display()
            )));
        }
        let output_sha256 = conversion::sha256_file(&made)?;
        fs::rename(&made, target).map_err(|error| {
            format!(
                "Érvényes konverziós output előléptetési hiba {}: {error}",
                target.display()
            )
        })?;
        Ok(OutputResult {
            state: OutputState::Ok,
            detail: "Integrity validation passed".to_owned(),
            retryable: false,
            retry_count: 0,
            output_sha256: Some(output_sha256),
        })
    })();
    if !crate::cancellation::requested() {
        let _ = fs::remove_dir_all(&root);
    }
    match result {
        Ok(result) => result,
        Err(error) => failure(if crate::cancellation::requested() {
            format!(
                "{error}; temporary conversion evidence retained at {}",
                root.display()
            )
        } else {
            error
        }),
    }
}

fn libreoffice_output_path(
    output_directory: &Path,
    source: &Path,
    extension: &str,
) -> Result<PathBuf, String> {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "A forrásfájlnak nincs alapneve.".to_owned())?;
    // `with_extension` would treat the last dot inside "Dr. Anka" as an extension
    // and incorrectly look for "Dr.docx" instead of LibreOffice's "Dr. Anka.docx".
    Ok(output_directory.join(format!("{stem}.{extension}")))
}

fn failure(detail: String) -> OutputResult {
    OutputResult {
        state: OutputState::Failed,
        detail,
        retryable: false,
        retry_count: 0,
        output_sha256: None,
    }
}

fn retryable_failure(detail: String) -> OutputResult {
    OutputResult {
        state: OutputState::Failed,
        detail,
        retryable: true,
        retry_count: 0,
        output_sha256: None,
    }
}

#[cfg(test)]
fn terminate_process_tree(child: &mut std::process::Child) -> Result<(), String> {
    external_tools::terminate_process_tree(child)
}

fn file_uri(path: &Path) -> Result<String, String> {
    let absolute = path
        .canonicalize()
        .map_err(|error| format!("LibreOffice profilútvonal-hiba: {error}"))?;
    let absolute_text = absolute.to_string_lossy();
    let text = absolute_text
        .strip_prefix(r"\\?\")
        .unwrap_or(&absolute_text)
        .replace('\\', "/");
    let encoded = text
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect::<String>();
    Ok(format!("file:///{encoded}"))
}

pub(crate) fn validate_output(path: &Path, extension: &str) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    if extension.eq_ignore_ascii_case("pdf") {
        return validate_pdf(path);
    }
    let required = match extension.to_ascii_lowercase().as_str() {
        "docx" => "word/document.xml",
        "xlsx" => "xl/workbook.xml",
        "pptx" => "ppt/presentation.xml",
        _ => return Err(format!("Nem támogatott modern formátum: {extension}")),
    };
    let file = File::open(path)
        .map_err(|error| format!("OOXML fájl nem nyitható {}: {error}", path.display()))?;
    let mut archive = match ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(_) => return Ok(false),
    };
    let mut has_content_types = false;
    let mut has_required = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("OOXML ZIP bejegyzés hiba: {error}"))?;
        has_content_types |= entry.name() == "[Content_Types].xml";
        has_required |= entry.name() == required;
    }
    Ok(has_content_types && has_required)
}

fn validate_pdf(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("PDF nem olvasható {}: {error}", path.display()))?;
    let size = file
        .metadata()
        .map_err(|error| format!("PDF metadata hiba {}: {error}", path.display()))?
        .len();
    if size < 10 {
        return Ok(false);
    }
    let mut header = [0_u8; 5];
    file.read_exact(&mut header)
        .map_err(|error| format!("PDF fejléc olvasási hiba: {error}"))?;
    if &header != b"%PDF-" {
        return Ok(false);
    }
    let tail_length = size.min(8192) as usize;
    file.seek(SeekFrom::End(-(tail_length as i64)))
        .map_err(|error| format!("PDF végének keresési hibája: {error}"))?;
    let mut tail = vec![0_u8; tail_length];
    file.read_exact(&mut tail)
        .map_err(|error| format!("PDF végének olvasási hibája: {error}"))?;
    Ok(tail.windows(5).any(|window| window == b"%%EOF"))
}

fn write_summary(path: &Path, rows: &[JobResult], converted_root: &Path) -> Result<(), String> {
    let mut file = File::create(path)
        .map_err(|error| format!("ConversionSummary nem írható {}: {error}", path.display()))?;
    file.write_all(b"\xEF\xBB\xBF")
        .map_err(|error| format!("ConversionSummary BOM hiba: {error}"))?;
    writeln!(file, "\"Floppy\",\"OriginalForensicPath\",\"DeliveryOriginalPath\",\"RecoveryMethod\",\"SourceType\",\"SourceSHA256\",\"ModernFormat\",\"ModernOK\",\"ModernResult\",\"ModernDetail\",\"ModernPath\",\"PDFOK\",\"PDFResult\",\"PDFDetail\",\"PDFPath\",\"Status\",\"PartialReason\",\"DurationSec\",\"ModernRetries\",\"PDFRetries\"")
        .map_err(|error| format!("ConversionSummary fejléc hiba: {error}"))?;
    for row in rows {
        let modern_path = if row.modern.state.successful() {
            relative_output(&row.job.modern_path, converted_root)?
        } else {
            String::new()
        };
        let pdf_path = if row.pdf.state.successful() {
            relative_output(&row.job.pdf_path, converted_root)?
        } else {
            String::new()
        };
        let values = [
            row.job.floppy.clone(),
            row.job.original_forensic_path.clone(),
            row.job.delivery_original_path.clone(),
            row.job.recovery_method.clone(),
            row.job.source_type.clone(),
            row.job.source_sha256.clone(),
            row.job.modern_format.clone(),
            row.modern.state.successful().to_string(),
            row.modern.state.label().to_owned(),
            row.modern.detail.clone(),
            modern_path,
            row.pdf.state.successful().to_string(),
            row.pdf.state.label().to_owned(),
            row.pdf.detail.clone(),
            pdf_path,
            row.status().to_owned(),
            row.reason(),
            format!("{:.2}", row.duration_seconds),
            row.modern.retry_count.to_string(),
            row.pdf.retry_count.to_string(),
        ];
        writeln!(
            file,
            "{}",
            values
                .iter()
                .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(",")
        )
        .map_err(|error| format!("ConversionSummary sorhiba: {error}"))?;
    }
    Ok(())
}

fn write_failures(path: &Path, rows: &[JobResult]) -> Result<(), String> {
    let mut text = format!(
        "\u{feff}LEGACY OFFICE CONVERSION EXCEPTIONS\r\nGenerated: {}\r\n\r\n",
        Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    for row in rows.iter().filter(|row| row.status() != "OK") {
        text.push_str(&format!(
            "{} | {} | {} | {} | {} | {}\r\n",
            row.job.floppy,
            row.job.original_forensic_path,
            row.status(),
            row.reason(),
            row.modern.detail,
            row.pdf.detail
        ));
    }
    fs::write(path, text)
        .map_err(|error| format!("ConversionFailures nem írható {}: {error}", path.display()))
}

fn relative_output(path: &Path, root: &Path) -> Result<String, String> {
    // Planning and reopened project paths can use different Windows spellings
    // (DOS versus verbatim). Resolve both; never fall back to an absolute CSV
    // path, and never strip a textual prefix without checking confinement.
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot resolve conversion output root: {e}"))?;
    let output = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve conversion output {}: {e}", path.display()))?;
    let relative = output.strip_prefix(&root).map_err(|_| {
        format!(
            "Conversion output escapes Converted folder: {}",
            path.display()
        )
    })?;
    if relative.as_os_str().is_empty() || !output.is_file() {
        return Err(format!(
            "Conversion output is not a file: {}",
            path.display()
        ));
    }
    Ok(relative.to_string_lossy().replace('/', "\\"))
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn worker_wait_notices_reach_controller_before_job_completion() {
        let rendezvous = std::sync::Barrier::new(2);
        let notices = std::cell::RefCell::new(Vec::new());
        let result = run_bounded_events(
            &[7],
            1,
            &[0],
            |value, notice| {
                notice("waiting for simulated RAM");
                rendezvous.wait();
                *value
            },
            |_| {},
            |notice| {
                // This callback intentionally is not Sync; status publication remains
                // on the controller, not concurrently inside Office worker threads.
                notices.borrow_mut().push(notice.to_owned());
                rendezvous.wait();
            },
        )
        .unwrap();
        assert_eq!(result, vec![7]);
        assert_eq!(*notices.borrow(), vec!["waiting for simulated RAM"]);
    }

    #[test]
    fn summary_output_paths_resolve_equivalent_roots_and_refuse_escape() {
        let temp = std::env::temp_dir().join(format!(
            "fv-summary-path-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let converted = temp.join("Converted");
        let output = converted.join("007/Dr. Anka [from DOC].docx");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(&output, b"path fixture").unwrap();
        let canonical = output.canonicalize().unwrap();
        for (file, root) in [
            (output.clone(), converted.canonicalize().unwrap()),
            (canonical, converted.clone()),
        ] {
            assert_eq!(
                relative_output(&file, &root).unwrap(),
                "007\\Dr. Anka [from DOC].docx"
            );
        }
        let sibling = temp.join("Converted-other");
        fs::create_dir_all(&sibling).unwrap();
        let outside = sibling.join("outside.docx");
        fs::write(&outside, b"outside fixture").unwrap();
        assert!(
            relative_output(&outside, &converted)
                .unwrap_err()
                .contains("escapes")
        );
        assert!(relative_output(&converted, &converted).is_err());
        assert!(relative_output(&converted.join("missing.docx"), &converted).is_err());
        assert!(
            relative_output(
                &converted.join("../Converted-other/outside.docx"),
                &converted
            )
            .is_err()
        );
        assert_eq!(fs::read(&outside).unwrap(), b"outside fixture");
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn conversion_state_survives_restart_but_is_scoped_to_its_project() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-conversion-state-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let first = root.join("first");
        let second = root.join("second");
        let reports = first.join("Reports");
        fs::create_dir_all(&reports).unwrap();
        fs::create_dir_all(&second).unwrap();
        let result = ConversionResult {
            planning: ConversionPlanningResult {
                disk_count: 0,
                mirrored_files: 0,
                reused_files: 0,
                conversion_candidates: 0,
                path_map: reports.join("PathMap.csv"),
                conversion_plan: reports.join("ConversionPlan.csv"),
                jobs: Vec::new(),
                retired_mirrors: 0,
                preserved_obsolete_mirrors: 0,
                cleanup_reports: Vec::new(),
            },
            ok: 0,
            partial: 0,
            failed: 0,
            timed_out: 0,
            reused_outputs: 0,
            retried_outputs: 0,
            issues: Vec::new(),
            summary_path: reports.join("ConversionSummary.csv"),
            failures_path: reports.join("ConversionFailures.csv"),
            rows: Vec::new(),
        };
        let path = save_snapshot(&reports, &first, &result).unwrap();
        assert!(path.is_file());
        assert_eq!(load_snapshot(&reports, &first).unwrap().ok, 0);
        assert!(load_snapshot(&reports, &second).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transient_failure_is_retried_once_and_both_attempts_are_recorded() {
        let mut calls = 0;
        let result = retry_transient_failure(|| {
            calls += 1;
            if calls == 1 {
                retryable_failure("LibreOffice exit 1".to_owned())
            } else {
                OutputResult {
                    state: OutputState::Ok,
                    detail: "Integrity validation passed".to_owned(),
                    retryable: false,
                    retry_count: 0,
                    output_sha256: None,
                }
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(result.state, OutputState::Ok);
        assert_eq!(result.retry_count, 1);
        assert!(result.detail.contains("Attempt 1: LibreOffice exit 1"));
        assert!(
            result
                .detail
                .contains("attempt 2: Integrity validation passed")
        );
    }

    #[test]
    fn equivalent_project_paths_reuse_bound_outputs_and_preserve_prior_snapshots_without_a_tool() {
        let root = std::env::temp_dir().join(format!(
            "fv-bound-path-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("Extracted/001")).unwrap();
        fs::write(
            root.join("Extracted/001/sample.rtf"),
            b"{\\rtf1 bound test}",
        )
        .unwrap();
        let planning_request = ConversionPlanningRequest {
            extracted_root: root.join("Extracted"),
            converted_root: root.join("Converted"),
            reports_directory: root.join("Reports"),
        };
        let planning = conversion::build_conversion_plan(&planning_request, &|_| {}).unwrap();
        let job = planning.jobs[0].clone();
        let mut zip = zip::ZipWriter::new(File::create(&job.modern_path).unwrap());
        for name in ["[Content_Types].xml", "word/document.xml"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"<test/>").unwrap();
        }
        zip.finish().unwrap();
        fs::write(&job.pdf_path, b"%PDF-1.7\ntest\n%%EOF\n").unwrap();
        let bound = |path: &Path| OutputResult {
            state: OutputState::Ok,
            detail: "Synthetic hash-bound fixture".to_owned(),
            retryable: false,
            retry_count: 0,
            output_sha256: Some(conversion::sha256_file(path).unwrap()),
        };
        let previous = ConversionResult {
            rows: vec![JobResult {
                job: job.clone(),
                modern: bound(&job.modern_path),
                pdf: bound(&job.pdf_path),
                duration_seconds: 1.0,
            }],
            planning,
            ok: 1,
            partial: 0,
            failed: 0,
            timed_out: 0,
            reused_outputs: 0,
            retried_outputs: 0,
            issues: Vec::new(),
            summary_path: root.join("Reports/ConversionSummary.csv"),
            failures_path: root.join("Reports/ConversionFailures.txt"),
        };
        let state = save_snapshot(&planning_request.reports_directory, &root, &previous).unwrap();
        let initial_state = fs::read(&state).unwrap();
        let request = ConversionRequest {
            planning: ConversionPlanningRequest {
                extracted_root: planning_request.extracted_root.canonicalize().unwrap(),
                converted_root: planning_request.converted_root.canonicalize().unwrap(),
                reports_directory: planning_request.reports_directory.canonicalize().unwrap(),
            },
            libreoffice_executable: std::env::current_exe().unwrap(),
            command_audit_path: root.join("audit.jsonl"),
            timeout_seconds: 45,
            workers: 12,
            selected_sources: None,
            previous_result: None,
        };
        let reused = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(reused.ok, 1);
        assert_eq!(reused.reused_outputs, 2);
        assert!(!request.command_audit_path.exists());
        // The job outputs are canonical/verbatim while the planning result's
        // Converted root may retain its original DOS spelling.
        let summary = fs::read_to_string(&reused.summary_path).unwrap();
        assert!(summary.contains("\"001\\sample [from RTF].docx\""));
        assert!(summary.contains("\"001\\sample [from RTF].pdf\""));
        assert!(!summary.contains(&root.to_string_lossy().to_string()));
        assert!(
            fs::read_dir(root.join("Reports/ConversionHistory"))
                .unwrap()
                .any(|entry| fs::read(entry.unwrap().path()).unwrap() == initial_state)
        );
        // Same bytes/identity delivered from a different extraction attempt.
        let mut relocated = previous.clone();
        relocated.rows[0].job.source_path = root.join("OldEvidence/previous-attempt/sample.rtf");
        fs::create_dir_all(relocated.rows[0].job.source_path.parent().unwrap()).unwrap();
        fs::copy(&job.source_path, &relocated.rows[0].job.source_path).unwrap();
        save_snapshot(&planning_request.reports_directory, &root, &relocated).unwrap();
        let transferred = run_conversion_mode(&request, &|_| {}, &|_, _| {}, true).unwrap();
        assert_eq!(transferred.reused_outputs, 2);
        assert_eq!(transferred.ok, 1);
        assert!(!request.command_audit_path.exists());
        // Already affected pilots retain the successful historical bindings.
        let mut rejected = transferred.clone();
        rejected.rows[0].modern = failure("old source-path-only rejection".into());
        rejected.rows[0].pdf = failure("old source-path-only rejection".into());
        save_snapshot(&planning_request.reports_directory, &root, &rejected).unwrap();
        let restored = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(restored.reused_outputs, 2);
        assert!(!request.command_audit_path.exists());
        // Different source content/filter/label cannot inherit those bindings.
        for change in 0..3 {
            let mut incompatible = job.clone();
            match change {
                0 => incompatible.source_sha256 = "different content".into(),
                1 => incompatible.pdf_filter = "different filter".into(),
                _ => incompatible.floppy = "002".into(),
            }
            let bound = compatible_previous(
                &[incompatible],
                Some(&relocated),
                &planning_request.reports_directory,
                &root,
            )
            .unwrap();
            assert!(bound.is_empty());
        }
        // Historical fallback refuses malformed, foreign or oversized records,
        // just as the current snapshot does. Only disposable fixture files here.
        let bad_history = root.join("Reports/ConversionHistory/PreviousConversionState-zz.json");
        let mut unmatched = job.clone();
        unmatched.source_sha256 = "no compatible donor".into();
        fs::write(&bad_history, b"invalid history").unwrap();
        assert!(
            compatible_previous(
                &[unmatched.clone()],
                None,
                &planning_request.reports_directory,
                &root
            )
            .unwrap_err()
            .contains("Invalid conversion history")
        );
        let mut foreign: serde_json::Value = serde_json::from_slice(&initial_state).unwrap();
        foreign["project_root"] = serde_json::json!(root.join("another-project"));
        fs::write(&bad_history, serde_json::to_vec(&foreign).unwrap()).unwrap();
        assert!(
            compatible_previous(
                &[unmatched.clone()],
                None,
                &planning_request.reports_directory,
                &root
            )
            .unwrap_err()
            .contains("different project")
        );
        File::create(&bad_history)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        assert!(
            compatible_previous(
                &[unmatched],
                None,
                &planning_request.reports_directory,
                &root
            )
            .unwrap_err()
            .contains("bounded conversion history")
        );
        fs::remove_file(&bad_history).unwrap();
        // A valid-looking edited output is still refused; history is not a way
        // to overwrite/adopt it. Its original recorded hash remains authoritative.
        fs::write(&job.pdf_path, b"%PDF-1.7\nchanged output\n%%EOF\n").unwrap();
        let changed = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(changed.partial, 1);
        assert!(changed.rows[0].pdf.detail.contains("hash changed"));
        assert_eq!(
            fs::read(&job.pdf_path).unwrap(),
            b"%PDF-1.7\nchanged output\n%%EOF\n"
        );
        assert!(!request.command_audit_path.exists());
        fs::write(&state, b"broken state").unwrap();
        assert!(
            run_conversion(&request, &|_| {}, &|_, _| {})
                .unwrap_err()
                .contains("Invalid saved conversion state")
        );
        assert_eq!(fs::read(&state).unwrap(), b"broken state");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsafe_timeout_and_permanent_failure_are_not_retried() {
        for first in [
            OutputResult {
                state: OutputState::Timeout,
                detail: "process-tree termination unverified".to_owned(),
                retryable: false,
                retry_count: 0,
                output_sha256: None,
            },
            failure("Existing invalid output was preserved".to_owned()),
        ] {
            let mut calls = 0;
            let result = retry_transient_failure(|| {
                calls += 1;
                if calls == 1 {
                    OutputResult {
                        state: first.state,
                        detail: first.detail.clone(),
                        retryable: first.retryable,
                        retry_count: first.retry_count,
                        output_sha256: first.output_sha256.clone(),
                    }
                } else {
                    panic!("a permanent failure must not be retried")
                }
            });
            assert_eq!(calls, 1);
            assert_eq!(result.state, first.state);
        }
    }

    #[test]
    fn incremental_processing_preserves_failed_jobs_but_explicit_run_retries_them() {
        let root = std::env::temp_dir().join(format!(
            "fv-incremental-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("Extracted/001")).unwrap();
        fs::write(root.join("Extracted/001/sample.rtf"), b"{\\rtf1 test}").unwrap();
        let planning_request = ConversionPlanningRequest {
            extracted_root: root.join("Extracted"),
            converted_root: root.join("Converted"),
            reports_directory: root.join("Reports"),
        };
        let planning = conversion::build_conversion_plan(&planning_request, &|_| {}).unwrap();
        let job = planning.jobs[0].clone();
        let previous = ConversionResult {
            rows: vec![JobResult {
                job,
                modern: failure("prior bounded modern failure".into()),
                pdf: failure("prior bounded PDF failure".into()),
                duration_seconds: 1.0,
            }],
            planning,
            ok: 0,
            partial: 0,
            failed: 1,
            timed_out: 0,
            reused_outputs: 0,
            retried_outputs: 0,
            issues: vec![],
            summary_path: root.join("Reports/ConversionSummary.csv"),
            failures_path: root.join("Reports/ConversionFailures.txt"),
        };
        save_snapshot(&planning_request.reports_directory, &root, &previous).unwrap();
        // A regular empty fixture cannot execute. The incremental path must not
        // even try it; the explicit retry must surface its launch failure.
        let executable = root.join("invalid-fixture.exe");
        fs::write(&executable, []).unwrap();
        let request = ConversionRequest {
            planning: planning_request,
            libreoffice_executable: executable,
            command_audit_path: root.join("audit.jsonl"),
            timeout_seconds: 10,
            workers: 2,
            selected_sources: None,
            previous_result: None,
        };
        for _ in 0..3 {
            let result = run_conversion_mode(&request, &|_| {}, &|_, _| {}, true).unwrap();
            assert_eq!(result.failed, 1);
            assert_eq!(result.rows[0].modern.detail, "prior bounded modern failure");
            assert_eq!(result.rows[0].pdf.detail, "prior bounded PDF failure");
            assert!(!request.command_audit_path.exists());
        }
        let retry = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(retry.failed, 1);
        assert!(retry.rows[0].modern.detail.contains("indítási hiba"));
        assert!(retry.rows[0].pdf.detail.contains("indítási hiba"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_second_transient_failure_stops_after_two_attempts() {
        let mut calls = 0;
        let result = retry_transient_failure(|| {
            calls += 1;
            retryable_failure(format!("LibreOffice exit {calls}"))
        });
        assert_eq!(calls, 2);
        assert_eq!(result.state, OutputState::Failed);
        assert!(!result.retryable);
        assert_eq!(result.retry_count, 1);
        assert!(result.detail.contains("Attempt 1: LibreOffice exit 1"));
        assert!(result.detail.contains("attempt 2: LibreOffice exit 2"));
    }

    #[test]
    fn unselected_missing_output_cannot_inherit_an_old_success() {
        let missing = std::env::temp_dir().join(format!(
            "fluxvault-missing-output-{}-{}.pdf",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let previous_ok = OutputResult {
            state: OutputState::Ok,
            detail: "Previously valid".to_owned(),
            retryable: false,
            retry_count: 0,
            output_sha256: None,
        };
        let result = inspect_without_conversion(Some(&previous_ok), &missing, "pdf");
        assert_eq!(result.state, OutputState::Failed);
        assert!(result.detail.contains("missing"));

        let previous_timeout = OutputResult {
            state: OutputState::Timeout,
            detail: "Previous timeout".to_owned(),
            retryable: false,
            retry_count: 1,
            output_sha256: None,
        };
        let result = inspect_without_conversion(Some(&previous_timeout), &missing, "pdf");
        assert_eq!(result.state, OutputState::Timeout);
        assert_eq!(result.detail, "Previous timeout");
        assert_eq!(result.retry_count, 0);
    }

    #[test]
    fn unselected_valid_output_requires_matching_prior_source_evidence() {
        let path = std::env::temp_dir().join(format!(
            "fluxvault-unattributed-output-{}-{}.pdf",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, b"%PDF-1.7\nbody\n%%EOF\n").unwrap();
        let result = inspect_without_conversion(None, &path, "pdf");
        assert_eq!(result.state, OutputState::Failed);
        assert!(
            result
                .detail
                .contains("no saved source/output hash binding")
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn structurally_valid_output_cannot_be_reused_after_its_hash_changes() {
        let path = std::env::temp_dir().join(format!(
            "fluxvault-changed-output-{}-{}.pdf",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, b"%PDF-1.7\noriginal\n%%EOF\n").unwrap();
        let previous = OutputResult {
            state: OutputState::Ok,
            detail: "Previously valid".to_owned(),
            retryable: false,
            retry_count: 0,
            output_sha256: Some(conversion::sha256_file(&path).unwrap()),
        };
        assert_eq!(
            inspect_without_conversion(Some(&previous), &path, "pdf").state,
            OutputState::Reused
        );
        fs::write(&path, b"%PDF-1.7\nchanged\n%%EOF\n").unwrap();
        let changed = inspect_without_conversion(Some(&previous), &path, "pdf");
        assert_eq!(changed.state, OutputState::Failed);
        assert!(changed.detail.contains("hash changed"));
        fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires permission to terminate a disposable Windows process tree"]
    fn timeout_terminates_a_spawned_child_process_too() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-tree-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let parent_script = root.join("parent.ps1");
        let child_script = root.join("child.ps1");
        let ready = root.join("ready.txt");
        let child_started = root.join("child-started.txt");
        let orphan_marker = root.join("orphan.txt");
        let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
        fs::write(
            &child_script,
            format!(
                "[IO.File]::WriteAllText('{}', 'started')\nStart-Sleep -Seconds 3\n[IO.File]::WriteAllText('{}', 'orphan')\n",
                quote(&child_started),
                quote(&orphan_marker)
            ),
        )
        .unwrap();
        fs::write(
            &parent_script,
            format!(
                "$child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -NonInteractive -File \"{}\"' -PassThru\n[IO.File]::WriteAllText('{}', $child.Id.ToString())\nStart-Sleep -Seconds 30\n",
                quote(&child_script),
                quote(&ready)
            ),
        )
        .unwrap();
        let mut parent = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(&parent_script)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !(ready.exists() && child_started.exists()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if !(ready.exists() && child_started.exists()) {
            let _ = terminate_process_tree(&mut parent);
            let _ = parent.wait();
            let _ = fs::remove_dir_all(&root);
            panic!("the disposable test process or its child did not start");
        }
        let child_pid = fs::read_to_string(&ready).unwrap();
        let termination = terminate_process_tree(&mut parent);
        let _ = parent.wait().unwrap();
        thread::sleep(Duration::from_millis(3500));
        let survived = orphan_marker.exists();
        if survived {
            let _ = Command::new("taskkill")
                .args(["/PID", child_pid.trim(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        fs::remove_dir_all(root).unwrap();
        assert!(termination.is_ok(), "{termination:?}");
        assert!(!survived, "a child process survived the tree kill");
    }

    #[test]
    fn bounded_workers_preserve_plan_order_and_report_completion() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);
        let jobs: Vec<usize> = (0..12).collect();
        let results = run_bounded(
            &jobs,
            4,
            &(0..jobs.len()).collect::<Vec<_>>(),
            |job| {
                let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(running, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(if *job == 0 { 80 } else { 10 }));
                active.fetch_sub(1, Ordering::SeqCst);
                job * 2
            },
            |count| {
                assert_eq!(count, completed.fetch_add(1, Ordering::SeqCst) + 1);
            },
        )
        .unwrap();
        assert_eq!(results, jobs.iter().map(|job| job * 2).collect::<Vec<_>>());
        assert_eq!(completed.load(Ordering::SeqCst), jobs.len());
        assert!(peak.load(Ordering::SeqCst) <= 4);
        assert!(peak.load(Ordering::SeqCst) > 1);
    }

    #[test]
    fn libreoffice_output_keeps_dots_inside_source_stem() {
        assert_eq!(
            libreoffice_output_path(Path::new("out"), Path::new("Dr. Anka.doc"), "docx").unwrap(),
            PathBuf::from("out").join("Dr. Anka.docx")
        );
    }

    #[test]
    fn balanced_schedule_interleaves_slow_and_small_jobs_and_claims_each_once() {
        let estimates = [1000.0, 900.0, 800.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let order = balanced_job_order(&estimates);
        assert_eq!(&order[..5], &[0, 3, 4, 5, 1]);
        let jobs: Vec<_> = (0..estimates.len()).collect();
        let calls: Vec<_> = jobs.iter().map(|_| AtomicUsize::new(0)).collect();
        let results = run_bounded(
            &jobs,
            12,
            &order,
            |job| {
                assert_eq!(calls[*job].fetch_add(1, Ordering::SeqCst), 0);
                *job
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(results, jobs);
        assert!(calls.iter().all(|calls| calls.load(Ordering::SeqCst) == 1));
        assert!(balanced_job_order(&[]).is_empty());
        for invalid in [vec![0; jobs.len()], vec![0], vec![usize::MAX; jobs.len()]] {
            assert!(
                run_bounded(
                    &jobs,
                    12,
                    &invalid,
                    |_| panic!("invalid schedule ran"),
                    |_| {}
                )
                .is_err()
            );
        }
        for workers in [0, 17] {
            assert!(
                run_bounded(
                    &jobs,
                    workers,
                    &order,
                    |_| panic!("invalid workers ran"),
                    |_| {}
                )
                .is_err()
            );
        }
    }

    #[test]
    fn balanced_queue_keeps_short_jobs_flowing_while_a_long_job_is_running() {
        use std::sync::{Arc, Barrier};
        let gate = Arc::new(Barrier::new(2));
        let order = balanced_job_order(&[1000.0, 1.0, 2.0, 3.0, 4.0]);
        let jobs = [0, 1, 2, 3, 4];
        let results = run_bounded(
            &jobs,
            2,
            &order,
            |job| {
                if *job == 0 || *job == 4 {
                    // Long job cannot complete until the other worker finishes three small jobs.
                    gate.wait();
                }
                *job
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(results, jobs);
    }

    #[test]
    fn pdf_integrity_checks_header_and_tail() {
        let path = std::env::temp_dir().join(format!(
            "fluxvault-pdf-check-{}-{}.pdf",
            std::process::id(),
            unix_ms()
        ));
        fs::write(&path, b"%PDF-1.7\nbody\n%%EOF\n").unwrap();
        assert!(validate_pdf(&path).unwrap());
        fs::write(&path, b"%PDF-1.7\nmissing end\n").unwrap();
        assert!(!validate_pdf(&path).unwrap());
        fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires FLUXVAULT_TEST_LIBREOFFICE"]
    fn converts_a_disposable_rtf_and_reuses_valid_outputs() {
        let executable = PathBuf::from(
            std::env::var("FLUXVAULT_TEST_LIBREOFFICE")
                .expect("FLUXVAULT_TEST_LIBREOFFICE is required"),
        );
        let root = std::env::temp_dir().join(format!(
            "fluxvault-office-integration-{}-{}",
            std::process::id(),
            unix_ms()
        ));
        let source_directory = root.join("Extracted").join("001");
        fs::create_dir_all(&source_directory).unwrap();
        for index in 0..4 {
            fs::write(
                source_directory.join(format!("sample{index}.rtf")),
                b"{\\rtf1\\ansi FluxVault conversion test.}",
            )
            .unwrap();
        }
        let request = ConversionRequest {
            planning: ConversionPlanningRequest {
                extracted_root: root.join("Extracted"),
                converted_root: root.join("Converted"),
                reports_directory: root.join("Reports"),
            },
            libreoffice_executable: executable,
            command_audit_path: root.join("Logs").join("external-tools.jsonl"),
            timeout_seconds: 45,
            workers: DEFAULT_CONVERSION_WORKERS,
            selected_sources: None,
            previous_result: None,
        };

        let first = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(first.ok, 4);
        assert_eq!(first.reused_outputs, 0);
        for job in &first.planning.jobs {
            assert!(job.modern_path.is_file());
            assert!(job.pdf_path.is_file());
        }
        assert!(first.summary_path.is_file());
        let summary = fs::read_to_string(&first.summary_path).unwrap();
        assert!(summary.lines().next().unwrap().contains("ModernRetries"));
        assert!(summary.lines().next().unwrap().contains("PDFRetries"));
        let audit = fs::read_to_string(&request.command_audit_path).unwrap();
        assert_eq!(audit.lines().count(), 8);
        for line in audit.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["tool"], "LibreOffice conversion");
        }

        let second = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(second.ok, 4);
        assert_eq!(second.reused_outputs, 8);
        let mut canonical_request = request.clone();
        canonical_request.planning.extracted_root =
            request.planning.extracted_root.canonicalize().unwrap();
        canonical_request.planning.converted_root =
            request.planning.converted_root.canonicalize().unwrap();
        canonical_request.planning.reports_directory =
            request.planning.reports_directory.canonicalize().unwrap();
        let canonical = run_conversion(&canonical_request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(canonical.ok, 4);
        assert_eq!(canonical.reused_outputs, 8);
        assert_eq!(
            fs::read_to_string(&request.command_audit_path)
                .unwrap()
                .lines()
                .count(),
            8
        );
        assert!(
            fs::read_dir(request.planning.reports_directory.join("ConversionHistory"))
                .unwrap()
                .count()
                >= 3
        );

        let selected_job = second.planning.jobs[0].clone();
        let unselected_job = second.planning.jobs[1].clone();
        fs::remove_file(&selected_job.pdf_path).unwrap();
        fs::remove_file(&unselected_job.pdf_path).unwrap();
        let mut selected_request = request.clone();
        selected_request.selected_sources = Some(vec![selected_job.source_path.clone()]);
        selected_request.previous_result = Some(Box::new(second));
        let selected = run_conversion(&selected_request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(selected.ok, 3);
        assert_eq!(selected.partial, 1);
        assert_eq!(selected.issues.len(), 1);
        assert_eq!(selected.issues[0].source_path, unselected_job.source_path);
        assert!(selected_job.pdf_path.is_file());
        assert!(!unselected_job.pdf_path.exists());
        assert_eq!(
            fs::read_to_string(&selected.summary_path)
                .unwrap()
                .lines()
                .count(),
            5
        );
        assert_eq!(
            fs::read_to_string(&request.command_audit_path)
                .unwrap()
                .lines()
                .count(),
            9
        );

        let retry_failed = run_conversion(&request, &|_| {}, &|_, _| {}).unwrap();
        assert_eq!(retry_failed.ok, 4);
        assert!(retry_failed.issues.is_empty());
        assert!(unselected_job.pdf_path.is_file());

        fs::write(
            &selected_job.source_path,
            b"{\\rtf1\\ansi Changed test source.}",
        )
        .unwrap();
        let mut stale_request = request.clone();
        stale_request.selected_sources = Some(vec![selected_job.source_path.clone()]);
        stale_request.previous_result = Some(Box::new(retry_failed));
        let stale_error = run_conversion(&stale_request, &|_| {}, &|_, _| {}).unwrap_err();
        assert!(stale_error.contains("megváltozott"));
        assert_eq!(
            fs::read_to_string(&request.command_audit_path)
                .unwrap()
                .lines()
                .count(),
            10
        );
        fs::write(
            &selected_job.source_path,
            b"{\\rtf1\\ansi FluxVault conversion test.}",
        )
        .unwrap();
        for index in 0..4 {
            assert_eq!(
                fs::read(source_directory.join(format!("sample{index}.rtf"))).unwrap(),
                b"{\\rtf1\\ansi FluxVault conversion test.}"
            );
        }

        fs::remove_dir_all(root).unwrap();
    }
}
