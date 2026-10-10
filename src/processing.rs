//! Durable, coalesced image-only processing. Exactly one workstation writer owns
//! a project; physical acquisition publishes through a separate short snapshot gate.
use crate::{
    pipeline::{self, PipelineRequest, PipelineResult},
    project::ProjectState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const CONTROL: &str = ".fluxvault-processing";
const STATUS: &str = "ProcessingStatus.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    schema_version: u32,
    disk: u32,
    attempt: u32,
    image_sha256: String,
    status: String,
    detail: String,
}

impl Job {
    fn name(&self) -> String {
        format!("{:03}_attempt_{:03}.json", self.disk, self.attempt)
    }
    fn pending(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "processing" | "failed")
    }
    fn validate(&self, path: &Path) -> Result<(), String> {
        if self.schema_version != 1
            || self.disk == 0
            || self.attempt == 0
            || self.image_sha256.len() != 64
            || !self.image_sha256.bytes().all(|c| c.is_ascii_hexdigit())
            || path.file_name().and_then(|n| n.to_str()) != Some(self.name().as_str())
            || !matches!(
                self.status.as_str(),
                "queued" | "processing" | "failed" | "processed" | "attention"
            )
        {
            return Err("Invalid processing job identity/binding/status".into());
        }
        Ok(())
    }
}

fn read_control(path: &Path) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Processing control must be a regular file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(131073)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 131072 {
        return Err("Oversized processing control".into());
    }
    Ok(bytes)
}

fn save(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if bytes.len() > 131072 {
        return Err("Oversized processing control; prior state retained".into());
    }
    if path.exists() {
        read_control(path)?;
    }
    let temp = path.with_file_name(format!(
        ".processing-{}-{}-{}.partial",
        std::process::id(),
        crate::external_tools::current_unix_ms(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(temp, path).map_err(|e| e.to_string())
}

fn contained_directory(parent: &Path, child: &Path) -> Result<(), String> {
    if child.canonicalize().map_err(|e| e.to_string())?.parent()
        != Some(parent.canonicalize().map_err(|e| e.to_string())?.as_path())
    {
        return Err("Processing directory escapes its workstation parent".into());
    }
    Ok(())
}

pub(crate) fn validate_workspace(project: &ProjectState) -> Result<(), String> {
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let name = root.to_string_lossy().to_ascii_uppercase();
    if ["A:", "B:", "\\\\?\\A:", "\\\\?\\B:"]
        .iter()
        .any(|p| name.starts_with(p))
    {
        return Err("Processing must use workstation files, not floppy drives".into());
    }
    for name in [
        "Images",
        "Logs",
        "Extracted",
        "Converted",
        "Recovery",
        "Reports",
        "Flux",
    ] {
        contained_directory(&root, &root.join(name))?;
    }
    Ok(())
}

fn jobs_dir(project: &ProjectState) -> Result<PathBuf, String> {
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let directory = root.join(CONTROL);
    if !directory.exists() {
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
    }
    if directory
        .canonicalize()
        .map_err(|e| e.to_string())?
        .parent()
        != Some(root.as_path())
    {
        return Err("Processing queue escapes project".into());
    }
    let jobs = directory.join("jobs");
    if !jobs.exists() {
        fs::create_dir(&jobs).map_err(|e| e.to_string())?;
    }
    if jobs.canonicalize().map_err(|e| e.to_string())?.parent() != Some(directory.as_path()) {
        return Err("Processing jobs escape queue".into());
    }
    Ok(jobs)
}

fn jobs(directory: &Path) -> Result<Vec<(PathBuf, Job)>, String> {
    let mut result = Vec::new();
    for item in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = item.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "json") {
            let job: Job =
                serde_json::from_slice(&read_control(&path)?).map_err(|e| e.to_string())?;
            job.validate(&path)?;
            result.push((path, job));
            if result.len() > 4096 {
                return Err("Processing queue exceeds 4096 jobs".into());
            }
        }
    }
    result.sort_by_key(|a| (a.1.disk, a.1.attempt));
    Ok(result)
}

fn verify(project: &ProjectState, job: &Job) -> Result<(), String> {
    let attempts = crate::imaging::load_attempts_for_disk(&project.images_dir(), job.disk)?;
    let attempt = attempts
        .iter()
        .find(|a| a.attempt_number == job.attempt)
        .ok_or("Queued acquisition attempt is missing")?;
    let image =
        crate::recovery_plan::resolve_image_path(&project.images_dir(), &attempt.image_file)?;
    if attempt.sha256 != job.image_sha256 {
        return Err("Queued acquisition binding changed".into());
    }
    let mut bytes = Vec::new();
    File::open(&image)
        .map_err(|e| e.to_string())?
        .take(crate::fat12::MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > crate::fat12::MAX_IMAGE_BYTES
        || !bytes.len().is_multiple_of(512)
        || format!("{:x}", Sha256::digest(&bytes)) != job.image_sha256
    {
        return Err("Queued image size/hash changed; evidence preserved".into());
    }
    crate::fat12_recovery::validate_sector_evidence(attempt, bytes.len() / 512, &job.image_sha256)
}

pub(crate) fn summary(result: &PipelineResult) -> Value {
    let attention = result.audit.attention_disks > 0
        || result.raw_format_exceptions > 0
        || result.conversion.failed > 0
        || result.conversion.partial > 0
        || result.declined_composites > 0
        || result.declined_recovery_publications > 0;
    json!({"exit_code":if attention {3} else {0},"disks":result.extraction.total_disks,"verified_disks":result.audit.verified_disks,"attention_disks":result.audit.attention_disks,
        "conversion_ok":result.conversion.ok,"conversion_failed":result.conversion.failed,"conversion_partial":result.conversion.partial,
        "extracted":result.extraction.extracted_disks,"reused_extractions":result.extraction.reused_disks,"recovery_queue":result.extraction.recovery_disks,
        "converted_ok":result.conversion.ok,"converted_failed":result.conversion.failed,"converted_partial":result.conversion.partial,
        "evidence_verified":result.audit.verified_disks,"evidence_attention":result.audit.attention_disks,"customer_delivery_certified":false,
        "disk_verification":result.audit.disk_verification,"raw_format_exceptions":result.raw_format_exceptions,"workbook":result.workbook_path,"recovery_decisions":result.recovery_decisions_path,
        "published_recovery_images":result.published_recovery_images,"reused_recovery_images":result.reused_recovery_images,
        "declined_recovery_publications":result.declined_recovery_publications,"physical_media_access":false})
}

#[derive(Default, Serialize)]
pub struct Outcome {
    pub runs: usize,
    pub processed_jobs: usize,
    pub errors: Vec<String>,
    pub last_result: Option<Value>,
    pub processing_elapsed_ms: u64,
}

pub(crate) struct Queue {
    // Keep ownership in the producer too: a failed/exited worker must not let
    // another report writer race a still-feeding scan.
    _owner: Arc<File>,
    directory: PathBuf,
    wake: Option<mpsc::SyncSender<()>>,
    drain: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Outcome>>,
}

impl Queue {
    pub(crate) fn start(request: PipelineRequest) -> Result<Self, String> {
        let owner = crate::project_work::reserve(request.project.root())?;
        Self::start_owned(request, owner)
    }

    fn start_owned(request: PipelineRequest, owner: File) -> Result<Self, String> {
        Self::start_shared(request, Arc::new(owner))
    }

    pub(crate) fn start_shared(
        mut request: PipelineRequest,
        owner: Arc<File>,
    ) -> Result<Self, String> {
        if !(1..=16).contains(&request.conversion_workers) {
            return Err("Conversion workers must be from 1 to 16".into());
        }
        // Leave capacity for the live decoder/acquisition and terminal.
        let cpus = thread::available_parallelism().map_or(4, usize::from);
        let requested_workers = request.conversion_workers;
        request.conversion_workers = requested_workers.min(cpus.saturating_sub(2).max(1));
        Self::start_shared_reserved(request, requested_workers, owner, |request, stage| {
            pipeline::run_pipeline_incremental(request, &|message| stage(message))
                .map(|r| summary(&r))
        })
    }

    #[cfg(test)]
    fn start_reserved<F>(
        request: PipelineRequest,
        requested_workers: usize,
        owner: File,
        run: F,
    ) -> Result<Self, String>
    where
        F: Fn(&PipelineRequest, &dyn Fn(&str)) -> Result<Value, String> + Send + 'static,
    {
        Self::start_shared_reserved(request, requested_workers, Arc::new(owner), run)
    }

    fn start_shared_reserved<F>(
        request: PipelineRequest,
        requested_workers: usize,
        owner: Arc<File>,
        run: F,
    ) -> Result<Self, String>
    where
        F: Fn(&PipelineRequest, &dyn Fn(&str)) -> Result<Value, String> + Send + 'static,
    {
        validate_workspace(&request.project)?;
        let directory = jobs_dir(&request.project)?;
        let events_path = request.project.logs_dir().join("ProcessingEvents.jsonl");
        if events_path.exists()
            && !fs::symlink_metadata(&events_path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_file()
        {
            return Err("Unsafe processing events log".into());
        }
        let events = Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(events_path)
                .map_err(|e| e.to_string())?,
        );
        let (wake, receiver) = mpsc::sync_channel(1);
        let drain = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let ending = drain.clone();
        let cancelled = cancel.clone();
        let tasks = directory.clone();
        let worker_owner = Arc::clone(&owner);
        let worker = crate::cancellation::spawn(move || {
            let _owner = worker_owner;
            let status_path = request.project.reports_dir().join(STATUS);
            let mut outcome = Outcome::default();
            let worker_started = std::time::Instant::now();
            let mut attempted = std::collections::BTreeSet::new();
            let publish = |stage: &str, disks: &[u32], runs: usize| -> Result<(), String> {
                let event = json!({"schema_version":1,"updated_unix_ms":crate::external_tools::current_unix_ms(),"worker_elapsed_ms":crate::benchmark::milliseconds(worker_started.elapsed()),"stage":stage,"active_disks":disks,"runs":runs,"requested_conversion_workers":requested_workers,"effective_conversion_workers":request.conversion_workers,"resource_budget":crate::resource_budget::snapshot(),"physical_media_access":false});
                save(&status_path, &event)?;
                writeln!(events.lock().map_err(|e| e.to_string())?, "{event}")
                    .map_err(|e| e.to_string())
            };
            loop {
                if cancelled.load(Ordering::Acquire) || crate::cancellation::requested() {
                    break;
                }
                let pending = match jobs(&tasks) {
                    Ok(all) => all
                        .into_iter()
                        .filter(|(path, job)| job.pending() && !attempted.contains(path))
                        .collect::<Vec<_>>(),
                    Err(error) => {
                        outcome.errors.push(error);
                        break;
                    }
                };
                if pending.is_empty() {
                    if let Err(e) = publish("idle", &[], outcome.runs) {
                        outcome.errors.push(e);
                        break;
                    }
                    if ending.load(Ordering::Acquire) {
                        // The initial snapshot may predate the producer's last
                        // atomic publication. Acquire the drain flag first,
                        // then re-read before deciding the queue is empty.
                        match jobs(&tasks) {
                            Ok(all)
                                if all.iter().any(|(path, job)| {
                                    job.pending() && !attempted.contains(path)
                                }) =>
                            {
                                continue;
                            }
                            Ok(_) => break,
                            Err(error) => {
                                outcome.errors.push(error);
                                break;
                            }
                        }
                    }
                    match receiver.recv_timeout(std::time::Duration::from_secs(5)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    continue;
                }
                let mut batch = Vec::new();
                // Validate all source bindings against one committed-image view.
                let validation = (|| {
                    let _gate = crate::project_work::snapshot(request.project.root())?;
                    for (path, mut job) in pending {
                        attempted.insert(path.clone());
                        match verify(&request.project, &job) {
                            Ok(()) => {
                                job.status = "processing".into();
                                job.detail.clear();
                                save(&path, &job)?;
                                batch.push((path, job));
                            }
                            Err(error) => {
                                job.status = "failed".into();
                                job.detail = error.clone();
                                save(&path, &job)?;
                                outcome
                                    .errors
                                    .push(format!("Disk {:03}: {error}; task retained", job.disk));
                            }
                        }
                    }
                    Ok::<(), String>(())
                })();
                if let Err(error) = validation {
                    outcome.errors.push(error);
                    break;
                }
                if batch.is_empty() {
                    continue;
                }
                let disks = batch.iter().map(|(_, j)| j.disk).collect::<Vec<_>>();
                let stage_errors = Mutex::new(Vec::new());
                let stage = |message: &str| {
                    if let Err(error) = publish(message, &disks, outcome.runs) {
                        stage_errors.lock().unwrap().push(error);
                    }
                };
                let batch_started = std::time::Instant::now();
                let result = run(&request, &stage);
                outcome.processing_elapsed_ms = outcome
                    .processing_elapsed_ms
                    .saturating_add(crate::benchmark::milliseconds(batch_started.elapsed()));
                outcome.runs += 1;
                let publication_errors = stage_errors.into_inner().unwrap();
                let (state, detail) = if !publication_errors.is_empty() {
                    let detail = format!(
                        "Processing visibility publication failed: {}",
                        publication_errors.join("; ")
                    );
                    outcome.errors.push(detail.clone());
                    ("failed", detail)
                } else {
                    match &result {
                        Ok(value) => {
                            outcome.last_result = Some(value.clone());
                            outcome.processed_jobs += batch.len();
                            (
                                if value["exit_code"] == 0 {
                                    "processed"
                                } else {
                                    "attention"
                                },
                                String::new(),
                            )
                        }
                        Err(error) => {
                            outcome.errors.push(format!(
                            "Processing batch failed: {error}; source evidence and tasks retained"
                        ));
                            ("failed", error.clone())
                        }
                    }
                };
                for (path, mut job) in batch {
                    job.status = if state != "failed" {
                        result
                            .as_ref()
                            .ok()
                            .and_then(|v| {
                                v["disk_verification"][format!("{:03}", job.disk)].as_bool()
                            })
                            .map(|verified| if verified { "processed" } else { "attention" })
                            .unwrap_or(state)
                            .into()
                    } else {
                        state.into()
                    };
                    job.detail = detail.clone();
                    if let Err(error) = save(&path, &job) {
                        outcome.errors.push(error);
                    }
                }
            }
            if let Err(e) = publish("stopped", &[], outcome.runs) {
                outcome.errors.push(e);
            }
            if let Err(e) = save(
                &status_path,
                &json!({"schema_version":1,"stage":"stopped","updated_unix_ms":crate::external_tools::current_unix_ms(),"outcome":outcome,"requested_conversion_workers":requested_workers,"effective_conversion_workers":request.conversion_workers,"resource_budget":crate::resource_budget::snapshot(),"physical_media_access":false}),
            ) {
                outcome.errors.push(e);
            }
            outcome
        });
        Ok(Self {
            _owner: owner,
            directory,
            wake: Some(wake),
            drain,
            cancel,
            worker: Some(worker),
        })
    }

    pub(crate) fn enqueue(
        &self,
        result: &crate::flux_recovery::RecoveryResult,
    ) -> Result<(), String> {
        let name = result
            .image
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Result image has no filename")?;
        let prefix = format!("{:03}_attempt_", result.disk);
        let attempt = name
            .strip_prefix(&prefix)
            .and_then(|n| n.strip_suffix(".img"))
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or("Result image is not a numbered acquisition")?;
        self.enqueue_attempt(result.disk, attempt, &result.image_sha256)
    }

    pub(crate) fn enqueue_attempt(
        &self,
        disk: u32,
        attempt: u32,
        sha256: &str,
    ) -> Result<(), String> {
        let job = Job {
            schema_version: 1,
            disk,
            attempt,
            image_sha256: sha256.to_owned(),
            status: "queued".into(),
            detail: String::new(),
        };
        let path = self.directory.join(job.name());
        job.validate(&path)?;
        if path.exists() {
            let old: Job =
                serde_json::from_slice(&read_control(&path)?).map_err(|e| e.to_string())?;
            old.validate(&path)?;
            if old.image_sha256 != job.image_sha256 {
                return Err("Processing task source hash changed; prior task retained".into());
            }
        } else {
            save(&path, &job)?;
        }
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Outcome {
        self.drain.store(true, Ordering::Release);
        self.wake.take();
        self.join()
    }
    fn join(&mut self) -> Outcome {
        self.worker
            .take()
            .map(|w| {
                w.join().unwrap_or_else(|_| Outcome {
                    errors: vec![
                        "Processing worker panicked; durable tasks/evidence retained".into(),
                    ],
                    ..Outcome::default()
                })
            })
            .unwrap_or_default()
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.wake.take();
        self.join();
    }
}

/// Read-only status: no owner, external tools or physical drive needed.
pub fn status(project: &ProjectState) -> Result<Value, String> {
    validate_workspace(project)?;
    let directory = project.root().join(CONTROL).join("jobs");
    let all = if directory.exists() {
        contained_directory(project.root(), &project.root().join(CONTROL))?;
        contained_directory(&project.root().join(CONTROL), &directory)?;
        jobs(&directory)?
    } else {
        Vec::new()
    };
    let path = project.reports_dir().join(STATUS);
    let worker: Value = if path.exists() {
        serde_json::from_slice(&read_control(&path)?).map_err(|e| e.to_string())?
    } else {
        json!({"stage":"not_started"})
    };
    let active = crate::project_work::active(project.root())?;
    Ok(
        json!({"worker":worker,"owner_active":active,"stale_worker_status":!active && worker["stage"] != "stopped" && worker["stage"] != "not_started","jobs":all.iter().map(|(_, j)| json!({"disk":j.disk,"attempt":j.attempt,"status":j.status,"detail":j.detail,"image_sha256":j.image_sha256})).collect::<Vec<_>>(),"pending":all.iter().filter(|(_, j)| j.pending()).count(),"attention":all.iter().filter(|(_, j)| j.status == "attention").count(),"physical_media_access":false}),
    )
}

/// Refresh successfully processed jobs from the authoritative end-of-session
/// audit; failed/pending tasks remain retryable rather than being hidden.
pub(crate) fn record_final(project: &ProjectState, result: &Value) -> Result<(), String> {
    let directory = project.root().join(CONTROL).join("jobs");
    if directory.exists() {
        contained_directory(project.root(), &project.root().join(CONTROL))?;
        contained_directory(&project.root().join(CONTROL), &directory)?;
        for (path, mut job) in jobs(&directory)? {
            if matches!(job.status.as_str(), "processed" | "attention")
                && let Some(verified) =
                    result["disk_verification"][format!("{:03}", job.disk)].as_bool()
            {
                job.status = if verified { "processed" } else { "attention" }.into();
                save(&path, &job)?;
            }
        }
    }
    let path = project.reports_dir().join(STATUS);
    let mut value: Value = if path.exists() {
        serde_json::from_slice(&read_control(&path)?).map_err(|e| e.to_string())?
    } else {
        json!({"schema_version":1,"stage":"stopped","physical_media_access":false})
    };
    value["reconciled_result"] = result.clone();
    value["updated_unix_ms"] = json!(crate::external_tools::current_unix_ms());
    save(&path, &value)
}

pub(crate) fn resume(project: &ProjectState, workers: usize) -> Result<Outcome, String> {
    validate_workspace(project)?;
    if !(1..=16).contains(&workers) {
        return Err("Conversion workers must be from 1 to 16".into());
    }
    let owner = crate::project_work::reserve(project.root())?;
    let settings = project.tool_settings()?;
    let audit = project.logs_dir().join("external-tools.jsonl");
    let seven = crate::external_tools::ToolKind::SevenZip;
    let office = crate::external_tools::ToolKind::LibreOffice;
    let request = PipelineRequest {
        project: project.clone(),
        seven_zip_executable: crate::external_tools::find_ready_tool(
            seven,
            settings.path(seven),
            &audit,
        )?,
        libreoffice_executable: crate::external_tools::find_ready_tool(
            office,
            settings.path(office),
            &audit,
        )?,
        command_audit_path: audit,
        conversion_workers: workers,
    };
    let mut outcome = Queue::start_owned(request.clone(), owner)?.finish();
    // A resumed queue can already be drained, or contain attention-only work.
    // Reconcile the whole saved project and explicitly retry prior failed outputs
    // once; never return a false all-clear merely because no new jobs arrived.
    let _owner = crate::project_work::reserve(project.root())?;
    let reconcile_started = std::time::Instant::now();
    match pipeline::run_pipeline(&request, &|stage| eprintln!("{stage}")) {
        Ok(result) => {
            outcome.runs += 1;
            outcome.last_result = Some(summary(&result));
        }
        Err(error) => outcome
            .errors
            .push(format!("Final offline reconciliation failed: {error}")),
    }
    outcome.processing_elapsed_ms = outcome.processing_elapsed_ms.saturating_add(
        reconcile_started
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64,
    );
    save(
        &project.reports_dir().join(STATUS),
        &json!({"schema_version":1,"stage":"stopped","updated_unix_ms":crate::external_tools::current_unix_ms(),"outcome":outcome,"resource_budget":crate::resource_budget::snapshot(),"physical_media_access":false}),
    )?;
    if let Some(value) = &outcome.last_result {
        record_final(project, value)?;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flux_recovery::RecoveryResult;
    use std::time::Duration;

    fn project() -> ProjectState {
        ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-processing-{}-{}-{}",
            std::process::id(),
            crate::external_tools::current_unix_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )))
        .unwrap()
    }

    fn request(project: &ProjectState) -> PipelineRequest {
        PipelineRequest {
            project: project.clone(),
            seven_zip_executable: project.root().join("never-7z.exe"),
            libreoffice_executable: project.root().join("never-office.exe"),
            command_audit_path: project.logs_dir().join("audit.jsonl"),
            conversion_workers: 4,
        }
    }

    fn acquisition(project: &ProjectState, disk: u32) -> RecoveryResult {
        let bytes = vec![(disk % 255) as u8; 512];
        let sha = format!("{:x}", Sha256::digest(&bytes));
        let image = project
            .images_dir()
            .join(format!("{disk:03}_attempt_001.img"));
        let log = project
            .logs_dir()
            .join(format!("{disk:03}_attempt_001.log"));
        fs::write(&image, &bytes).unwrap();
        fs::write(&log, format!("GEOMETRY | cylinders=1 | heads=1 | sectors_per_track=1 | bytes_per_sector=512 | total_sectors=1\nEND | status=OK | bytes=512 | sha256={sha}\n")).unwrap();
        fs::write(project.images_dir().join(format!("{disk:03}_attempt_001.json")), serde_json::to_vec(&json!({
            "fluxvault_version":"fixture", "status":"OK", "disk_number":disk,"attempt_number":1,
            "source_backend":"synthetic-test", "source_device":"none", "image_file":image.file_name().unwrap().to_str().unwrap(),
            "log_file":log, "timestamp_unix_ms":1,
            "geometry":{"cylinders":1,"heads":1,"sectors_per_track":1,"bytes_per_sector":512,"total_bytes":512,"format_guess":"fixture"},
            "sector_retries":0,"total_sectors":1,"bytes_written":512,"retry_recovered_sectors":0,
            "bad_sector_count":0,"bad_sectors":[],"sha256":sha
        })).unwrap()).unwrap();
        RecoveryResult {
            disk,
            selected_profile: None,
            status: "acquired".into(),
            stop_reason: "synthetic".into(),
            capture_attempts: vec![],
            image,
            image_sha256: sha,
            provenance: PathBuf::new(),
            provenance_sha256: String::new(),
            missing_lbas: vec![],
            conflicting_lbas: vec![],
            read_conflict_lbas: vec![],
            corroborated_sectors: 0,
            single_capture_sectors: 1,
            physical_reads_this_run: 0,
            resumed: false,
            format_exception: None,
        }
    }

    fn injected(
        project: &ProjectState,
        run: impl Fn(&PipelineRequest, &dyn Fn(&str)) -> Result<Value, String> + Send + 'static,
    ) -> Queue {
        Queue::start_reserved(
            request(project),
            4,
            crate::project_work::reserve(project.root()).unwrap(),
            run,
        )
        .unwrap()
    }

    #[test]
    fn coalesced_136_jobs_leave_publication_and_read_only_status_available() {
        let project = project();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let worker = injected(&project, move |_, stage| {
            stage("converting saved files");
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            Ok(json!({"exit_code":0}))
        });
        worker.enqueue(&acquisition(&project, 1)).unwrap();
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(crate::project_work::reserve(project.root()).is_err());
        assert!(status(&project).unwrap()["owner_active"].as_bool().unwrap());
        for args in [
            ["audit"].as_slice(),
            ["conversion", "plan"].as_slice(),
            ["disk", "next"].as_slice(),
        ] {
            let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            assert!(
                crate::cli::run(&args, project.root())
                    .unwrap_err()
                    .contains("processing owner")
            );
        }
        let read = crate::cli::run(
            &["processing".into(), "status".into(), "--json".into()],
            project.root(),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&read.output).unwrap()["physical_media_access"],
            false
        );
        // Simulate publication while a long Office job is running. The worker
        // owns reports, not the short publication gate or the physical drive.
        for disk in 2..=136 {
            let _publication = crate::project_work::snapshot(project.root()).unwrap();
            let result = acquisition(&project, disk);
            worker.enqueue(&result).unwrap();
            worker.enqueue(&result).unwrap();
        }
        release_tx.send(()).unwrap();
        let outcome = worker.finish();
        assert_eq!(outcome.runs, 2);
        assert_eq!(outcome.processed_jobs, 136);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        let current = status(&project).unwrap();
        assert_eq!(current["pending"], 0);
        assert_eq!(current["jobs"].as_array().unwrap().len(), 136);
        assert_eq!(current["owner_active"], false);
        assert_eq!(project.current_disk_number(), 1);
        assert!(!project.logs_dir().join("audit.jsonl").exists());
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn failed_136_job_cohort_reopens_drains_once_and_keeps_source_bindings() {
        let project = project();
        let root = project.root().to_path_buf();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = std::sync::atomic::AtomicBool::new(true);
        let worker = injected(&project, move |_, _| {
            if first.swap(false, Ordering::SeqCst) {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            Err("simulated downstream worker failure; evidence retained".into())
        });
        let mut sources = vec![];
        let first_disk = acquisition(&project, 1);
        sources.push((first_disk.image.clone(), first_disk.image_sha256.clone()));
        worker.enqueue(&first_disk).unwrap();
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for disk in 2..=136 {
            let result = acquisition(&project, disk);
            sources.push((result.image.clone(), result.image_sha256.clone()));
            worker.enqueue(&result).unwrap();
            worker.enqueue(&result).unwrap();
        }
        release_tx.send(()).unwrap();
        let failed = worker.finish();
        assert!((1..=2).contains(&failed.runs));
        assert!(!failed.errors.is_empty());
        assert_eq!(status(&project).unwrap()["pending"], 136);
        assert_eq!(
            status(&project).unwrap()["jobs"].as_array().unwrap().len(),
            136
        );
        drop(project);
        let reopened = ProjectState::open_without_session(root).unwrap();
        let resumed = injected(&reopened, |_, _| Ok(json!({"exit_code":3}))).finish();
        assert_eq!(resumed.runs, 1);
        assert_eq!(resumed.processed_jobs, 136);
        assert!(resumed.errors.is_empty());
        let saved = status(&reopened).unwrap();
        assert_eq!(saved["pending"], 0);
        assert_eq!(saved["jobs"].as_array().unwrap().len(), 136);
        // A mock pipeline is not a real integrity audit: no false all-clear.
        assert_eq!(saved["attention"], 136);
        assert_eq!(
            injected(&reopened, |_, _| panic!("completed jobs must not loop"))
                .finish()
                .runs,
            0
        );
        for (image, expected) in sources {
            assert_eq!(
                format!("{:x}", Sha256::digest(fs::read(image).unwrap())),
                expected
            );
        }
        assert_eq!(reopened.current_disk_number(), 1);
        fs::remove_dir_all(reopened.root()).unwrap();
    }

    #[test]
    fn changed_image_is_retained_failed_and_resumes_only_after_binding_restored() {
        let project = project();
        let result = acquisition(&project, 1);
        let original = fs::read(&result.image).unwrap();
        fs::write(&result.image, vec![99; 512]).unwrap();
        let worker = injected(&project, |_, _| {
            panic!("changed evidence cannot reach pipeline")
        });
        worker.enqueue(&result).unwrap();
        let first = worker.finish();
        assert_eq!(first.runs, 0);
        assert_eq!(first.errors.len(), 1);
        assert_eq!(status(&project).unwrap()["jobs"][0]["status"], "failed");
        fs::write(&result.image, original).unwrap();
        let second = injected(&project, |_, _| Ok(json!({"exit_code":3}))).finish();
        assert_eq!(second.processed_jobs, 1);
        assert!(second.errors.is_empty());
        assert_eq!(status(&project).unwrap()["attention"], 1);
        let third = injected(&project, |_, _| {
            panic!("completed attention job must not loop")
        })
        .finish();
        assert_eq!(third.runs, 0);
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn interrupted_processing_restarts_and_failed_pipeline_is_bounded_per_session() {
        let project = project();
        let result = acquisition(&project, 1);
        let directory = jobs_dir(&project).unwrap();
        let job = Job {
            schema_version: 1,
            disk: 1,
            attempt: 1,
            image_sha256: result.image_sha256.clone(),
            status: "processing".into(),
            detail: "interrupted".into(),
        };
        save(&directory.join(job.name()), &job).unwrap();
        fs::write(
            directory.join(".processing-crash.partial"),
            b"truncated private data",
        )
        .unwrap();
        let first = injected(&project, |_, stage| {
            stage("test failure");
            Err("disposable pipeline failure".into())
        })
        .finish();
        assert_eq!(first.runs, 1);
        assert_eq!(first.errors.len(), 1);
        assert_eq!(status(&project).unwrap()["pending"], 1);
        let second = injected(&project, |_, _| Ok(json!({"exit_code":0}))).finish();
        assert_eq!(second.runs, 1);
        assert_eq!(status(&project).unwrap()["pending"], 0);
        assert!(directory.join(".processing-crash.partial").is_file());
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn invalid_controls_and_oversized_publication_preserve_previous_state() {
        let project = project();
        let path = project.reports_dir().join(STATUS);
        save(&path, &json!({"stage":"converting"})).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(save(&path, &"X".repeat(131073)).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(status(&project).unwrap()["stale_worker_status"], true);
        let directory = jobs_dir(&project).unwrap();
        fs::write(
            directory.join("001_attempt_001.json"),
            b"{\"schema_version\":999}",
        )
        .unwrap();
        let outcome = injected(&project, |_, _| panic!("invalid control cannot run")).finish();
        assert_eq!(outcome.runs, 0);
        assert_eq!(outcome.errors.len(), 1);
        assert!(
            fs::read(directory.join("001_attempt_001.json"))
                .unwrap()
                .starts_with(b"{\"schema_version\":999}")
        );
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn status_publication_failure_keeps_the_job_retryable_not_silently_processed() {
        let project = project();
        let result = acquisition(&project, 1);
        let worker = injected(&project, |request, stage| {
            stage("starting");
            let path = request.project.reports_dir().join(STATUS);
            fs::remove_file(&path).unwrap();
            fs::create_dir(&path).unwrap();
            stage("could not publish visibility");
            Ok(json!({"exit_code":0}))
        });
        worker.enqueue(&result).unwrap();
        let first = worker.finish();
        assert!(!first.errors.is_empty());
        let directory = jobs_dir(&project).unwrap();
        let job = jobs(&directory).unwrap().remove(0).1;
        assert_eq!(job.status, "failed");
        assert!(job.detail.contains("publication failed"));
        fs::remove_dir(project.reports_dir().join(STATUS)).unwrap();
        let resumed = injected(&project, |_, _| Ok(json!({"exit_code":0}))).finish();
        assert_eq!(resumed.processed_jobs, 1);
        assert!(resumed.errors.is_empty());
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn global_attention_does_not_mark_clean_disks_and_final_audit_refreshes_only_completed_jobs() {
        let project = project();
        let worker = injected(&project, |_, _| {
            Ok(json!({"exit_code":3,"disk_verification":{"001":true,"002":false}}))
        });
        worker.enqueue(&acquisition(&project, 1)).unwrap();
        worker.enqueue(&acquisition(&project, 2)).unwrap();
        let first = worker.finish();
        assert_eq!(first.processed_jobs, 2);
        let current = status(&project).unwrap();
        assert_eq!(current["jobs"][0]["status"], "processed");
        assert_eq!(current["jobs"][1]["status"], "attention");
        assert_eq!(current["attention"], 1);
        let _owner = crate::project_work::reserve(project.root()).unwrap();
        record_final(
            &project,
            &json!({"exit_code":0,"disk_verification":{"001":true,"002":true}}),
        )
        .unwrap();
        assert_eq!(status(&project).unwrap()["attention"], 0);
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    fn exited_worker_does_not_release_a_still_feeding_producers_project() {
        let project = project();
        let directory = jobs_dir(&project).unwrap();
        fs::write(directory.join("001_attempt_001.json"), b"broken task").unwrap();
        let worker = injected(&project, |_, _| panic!("invalid queue must not run"));
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !worker.worker.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(crate::project_work::reserve(project.root()).is_err());
        assert!(crate::project_work::active(project.root()).unwrap());
        assert_eq!(worker.finish().errors.len(), 1);
        drop(crate::project_work::reserve(project.root()).unwrap());
        fs::remove_dir_all(project.root()).unwrap();
    }

    #[test]
    #[ignore = "requires FLUXVAULT_BACKGROUND_TEST_PROJECT, FLUXVAULT_BACKGROUND_TEST_OUTPUT, FLUXVAULT_TEST_7Z and FLUXVAULT_TEST_LIBREOFFICE; saved images only"]
    fn real_twenty_disk_background_chain_on_separate_saved_evidence_copy() {
        let source = ProjectState::open_without_session(PathBuf::from(
            std::env::var("FLUXVAULT_BACKGROUND_TEST_PROJECT").unwrap(),
        ))
        .unwrap();
        let output = PathBuf::from(std::env::var("FLUXVAULT_BACKGROUND_TEST_OUTPUT").unwrap());
        assert!(
            !output.exists(),
            "Validation needs a new project; never overwrite existing work"
        );
        let project = ProjectState::create_without_session(output).unwrap();
        let request = PipelineRequest {
            project: project.clone(),
            seven_zip_executable: PathBuf::from(std::env::var("FLUXVAULT_TEST_7Z").unwrap()),
            libreoffice_executable: PathBuf::from(
                std::env::var("FLUXVAULT_TEST_LIBREOFFICE").unwrap(),
            ),
            command_audit_path: project.logs_dir().join("external-tools.jsonl"),
            conversion_workers: 4,
        };
        let worker = Queue::start(request.clone()).unwrap();
        let mut bindings = Vec::new();
        for disk in 1..=20 {
            let attempts =
                crate::imaging::load_attempts_for_disk(&source.images_dir(), disk).unwrap();
            let attempt = attempts.iter().find(|a| a.attempt_number == 1).unwrap();
            let original =
                crate::recovery_plan::resolve_image_path(&source.images_dir(), &attempt.image_file)
                    .unwrap();
            let bytes = fs::read(&original).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), attempt.sha256);
            bindings.push((original, attempt.sha256.clone()));
            let _gate = crate::project_work::snapshot(project.root()).unwrap();
            let image = project
                .images_dir()
                .join(format!("{disk:03}_attempt_001.img"));
            let log = project
                .logs_dir()
                .join(format!("{disk:03}_attempt_001.log"));
            fs::write(&image, &bytes).unwrap();
            fs::copy(&attempt.log_file, &log).unwrap();
            let mut metadata: Value =
                serde_json::from_slice(&fs::read(&attempt.metadata_path).unwrap()).unwrap();
            metadata["image_file"] = json!(image);
            metadata["log_file"] = json!(log);
            // Keep the original flux provenance reference and explicitly record
            // the offline copy, rather than inventing a new physical read.
            metadata["offline_validation_source_metadata"] = json!(attempt.metadata_path);
            fs::write(
                project
                    .images_dir()
                    .join(format!("{disk:03}_attempt_001.json")),
                serde_json::to_vec_pretty(&metadata).unwrap(),
            )
            .unwrap();
            let result = RecoveryResult {
                disk,
                selected_profile: None,
                status: if attempt.bad_sectors.is_empty() {
                    "acquired"
                } else {
                    "partial"
                }
                .into(),
                stop_reason: "offline validation copy".into(),
                capture_attempts: vec![],
                image,
                image_sha256: attempt.sha256.clone(),
                provenance: PathBuf::new(),
                provenance_sha256: String::new(),
                missing_lbas: attempt.bad_sectors.clone(),
                conflicting_lbas: vec![],
                read_conflict_lbas: vec![],
                corroborated_sectors: 0,
                single_capture_sectors: attempt.total_sectors - attempt.bad_sectors.len(),
                physical_reads_this_run: 0,
                resumed: true,
                format_exception: None,
            };
            worker.enqueue(&result).unwrap();
        }
        eprintln!("Offline validation: all 20 copied acquisition bindings queued; no drive access");
        let outcome = worker.finish();
        assert_eq!(outcome.processed_jobs, 20);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        let _owner = crate::project_work::reserve(project.root()).unwrap();
        let final_result =
            pipeline::run_pipeline_incremental(&request, &|s| eprintln!("{s}")).unwrap();
        assert_eq!(final_result.extraction.total_disks, 20);
        assert_eq!(final_result.conversion.ok, 172);
        assert_eq!(final_result.conversion.failed, 0);
        assert_eq!(final_result.conversion.partial, 0);
        assert_eq!(final_result.audit.verified_disks, 17);
        assert_eq!(final_result.audit.attention_disks, 3);
        assert_eq!(status(&project).unwrap()["pending"], 0);
        for (path, sha) in bindings {
            assert_eq!(crate::conversion::sha256_file(&path).unwrap(), sha);
        }
        let audit = fs::read_to_string(&request.command_audit_path).unwrap();
        assert!(!audit.contains("Greaseweazle"));
        eprintln!(
            "OFFLINE BACKGROUND VALIDATION: {} coalesced runs; 20 jobs; 172 successful conversions; 17 verified / 3 attention. Preserved: {}",
            outcome.runs,
            project.root().display()
        );
    }
}
