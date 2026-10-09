//! Guided Greaseweazle custody loop. Each disk requires a numbered or opt-in Enter confirmation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, BufRead, Read, Write},
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    benchmark,
    flux_recovery::{self, RecoveryPolicy, RecoveryResult},
    greaseweazle::GreaseweazleProfile,
    project::ProjectState,
};

use super::{
    CliResponse, flux,
    media_reservation::GreaseweazleReservation,
    terminal::{self, Cue},
};

const JOURNAL: &str = ".fluxvault-gw-scan.json";
const LOCK: &str = ".fluxvault-gw-scan.lock";
const MAX_NO_INDEX_RESEATS: usize = 2;

pub(super) struct ScanOptions {
    pub profile: GreaseweazleProfile,
    pub automatic_format: bool,
    pub packed_captures: bool,
    pub background_processing: bool,
    pub profile_map: BTreeMap<u32, String>,
    pub drive: char,
    pub protected: bool,
    pub policy: RecoveryPolicy,
    pub count: Option<usize>,
    pub last_disk: Option<u32>,
    pub acquisition_only: bool,
    pub json_output: bool,
    pub no_verify: bool,
    pub color: bool,
    pub conversion_workers: usize,
}

impl ScanOptions {
    fn automatic_for(&self, disk: u32) -> bool {
        self.automatic_format && !self.profile_map.contains_key(&disk)
    }

    fn format_label(&self, disk: u32) -> Result<&str, String> {
        if self.automatic_for(disk) {
            Ok("auto (720 KB / 1.44 MB)")
        } else {
            Ok(self.profile_for(disk)?.argument())
        }
    }
    fn profile_for(&self, disk: u32) -> Result<GreaseweazleProfile, String> {
        self.profile_map
            .get(&disk)
            .map_or(Ok(self.profile), |name| GreaseweazleProfile::parse(name))
    }

    fn validate(&self) -> Result<(), String> {
        validate_workers(self.conversion_workers)?;
        if !self.protected {
            return Err(
                "Scan requires --source-write-protected; check the physical tab on every disk"
                    .to_owned(),
            );
        }
        if !matches!(self.drive, 'A' | 'B') || self.count == Some(0) {
            return Err("Scan requires drive A or B and a positive --count".to_owned());
        }
        if self.last_disk.is_some_and(|n| n == 0 || n == u32::MAX) {
            return Err(
                "--last-disk must be positive and leave room for the next number".to_owned(),
            );
        }
        validate_profiles(&self.profile_map)?;
        self.policy.validate()
    }
}

fn default_conversion_workers() -> usize {
    crate::conversion_run::DEFAULT_CONVERSION_WORKERS
}

fn validate_workers(workers: usize) -> Result<(), String> {
    if !(1..=16).contains(&workers) {
        return Err("Scan conversion workers must be from 1 to 16".to_owned());
    }
    Ok(())
}

fn validate_profiles(profiles: &BTreeMap<u32, String>) -> Result<(), String> {
    for (disk, name) in profiles {
        if *disk == 0 || *disk == u32::MAX {
            return Err(
                "Profile-map disk numbers must be positive and leave room for the next disk"
                    .to_owned(),
            );
        }
        if GreaseweazleProfile::parse(name)?.argument() != name {
            return Err("Profile-map formats must be ibm.1440 or ibm.720".to_owned());
        }
    }
    Ok(())
}

/// An explicit, bounded list of known formats, not an automatic format detector.
pub(super) fn load_profile_map(path: &Path) -> Result<BTreeMap<u32, String>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        disk: u32,
        profile: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct MapFile {
        schema_version: u32,
        profiles: Vec<Entry>,
    }
    let text = path.to_string_lossy().to_ascii_uppercase();
    if ["A:", "B:", "\\\\?\\A:", "\\\\?\\B:"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
    {
        return Err("Profile maps must be saved on the workstation, not a floppy drive".to_owned());
    }
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Profile map must be a regular file".to_owned());
    }
    let resolved = path.canonicalize().map_err(|e| e.to_string())?;
    let text = resolved.to_string_lossy().to_ascii_uppercase();
    if ["A:", "B:", "\\\\?\\A:", "\\\\?\\B:"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
    {
        return Err("Profile map resolves to a floppy drive".to_owned());
    }
    let mut bytes = Vec::new();
    File::open(resolved)
        .map_err(|e| e.to_string())?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("Profile map exceeds 64 KiB".to_owned());
    }
    let list: MapFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid profile map: {e}"))?;
    if list.schema_version != 1 {
        return Err("Unsupported profile-map schema".to_owned());
    }
    let mut profiles = BTreeMap::new();
    for entry in list.profiles {
        if profiles.insert(entry.disk, entry.profile).is_some() {
            return Err(format!("Duplicate disk {:03} in profile map", entry.disk));
        }
    }
    validate_profiles(&profiles)?;
    Ok(profiles)
}

fn preflight_downstream(
    acquisition_only: bool,
    mut check: impl FnMut(crate::external_tools::ToolKind) -> Result<(), String>,
) -> Result<(), String> {
    if acquisition_only {
        return Ok(());
    }
    for tool in [
        crate::external_tools::ToolKind::SevenZip,
        crate::external_tools::ToolKind::LibreOffice,
    ] {
        check(tool).map_err(|e| format!("Scan processing preflight failed before any media read: {e}. Configure the tool or explicitly use --acquisition-only to process saved images later."))?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Pending {
    disk: u32,
    result: Option<RecoveryResult>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    schema_version: u32,
    profile: String,
    #[serde(default)]
    automatic_format: bool,
    #[serde(default)]
    packed_captures: bool,
    #[serde(default)]
    background_processing: bool,
    #[serde(default)]
    profile_map: BTreeMap<u32, String>,
    drive: char,
    policy: RecoveryPolicy,
    #[serde(default)]
    last_disk: Option<u32>,
    #[serde(default = "default_conversion_workers")]
    conversion_workers: usize,
    pending: Option<Pending>,
    completed: Vec<RecoveryResult>,
}

pub(super) struct SavedDefaults {
    pub profile: GreaseweazleProfile,
    pub automatic_format: bool,
    pub packed_captures: bool,
    pub background_processing: bool,
    pub profile_map: BTreeMap<u32, String>,
    pub drive: char,
    pub policy: RecoveryPolicy,
    pub last_disk: Option<u32>,
    pub conversion_workers: usize,
}

pub(super) fn saved_defaults(project: &ProjectState) -> Result<Option<SavedDefaults>, String> {
    let path = project.root().join(JOURNAL);
    regular_or_missing(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let journal: Journal = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid saved scan settings: {e}"))?;
    if journal.schema_version != 1 || !matches!(journal.drive, 'A' | 'B') {
        return Err("Unsupported saved scan settings".to_owned());
    }
    validate_profiles(&journal.profile_map)?;
    journal.policy.validate()?;
    validate_workers(journal.conversion_workers)?;
    Ok(Some(SavedDefaults {
        profile: GreaseweazleProfile::parse(&journal.profile)?,
        automatic_format: journal.automatic_format,
        packed_captures: journal.packed_captures,
        background_processing: journal.background_processing,
        profile_map: journal.profile_map,
        drive: journal.drive,
        policy: journal.policy,
        last_disk: journal.last_disk,
        conversion_workers: journal.conversion_workers,
    }))
}

pub(super) fn run(mut project: ProjectState, options: ScanOptions) -> Result<CliResponse, String> {
    options.validate()?;
    let _control = crate::run_control::Session::start(&project, "scan")?;
    crate::flux_capture::project_flux_dir(&project)?;
    let reservation = GreaseweazleReservation::acquire()?;
    let settings = crate::external_tools::load_settings()?;
    let audit = project.logs_dir().join("external-tools.jsonl");
    let mut seven_zip = None;
    let mut libreoffice = None;
    preflight_downstream(options.acquisition_only, |tool| {
        eprintln!(
            "Before feeding disks: checking {} for saved-file processing...",
            tool.display_name()
        );
        let path = crate::external_tools::find_ready_tool(tool, settings.path(tool), &audit)?;
        match tool {
            crate::external_tools::ToolKind::SevenZip => seven_zip = Some(path),
            crate::external_tools::ToolKind::LibreOffice => libreoffice = Some(path),
            _ => {}
        }
        Ok(())
    })?;
    let saved = load(&project.root().join(JOURNAL), &options)?;
    let request = if options.background_processing && !options.acquisition_only {
        Some(crate::pipeline::PipelineRequest {
            project: project.clone(),
            seven_zip_executable: seven_zip.unwrap(),
            libreoffice_executable: libreoffice.unwrap(),
            command_audit_path: audit.clone(),
            conversion_workers: options.conversion_workers,
        })
    } else {
        None
    };
    let processing = std::cell::RefCell::new(match &request {
        Some(request) => Some(crate::processing::Queue::start(request.clone())?),
        None => None,
    });
    let processing_errors = std::cell::RefCell::new(Vec::<String>::new());
    let feeding_owner = std::cell::RefCell::new(if request.is_none() {
        Some(crate::project_work::reserve(project.root())?)
    } else {
        None
    });
    if let Some(worker) = processing.borrow().as_ref() {
        for result in &saved.completed {
            if result.format_exception.is_some() {
                continue;
            }
            if let Err(error) = worker.enqueue(result) {
                processing_errors.borrow_mut().push(error);
            }
        }
        eprintln!(
            "BACKGROUND PROCESSING ON: saved-file work continues between swaps. Worker details: `fv processing status`; swap banners stay unobscured."
        );
    }
    let mut stderr = io::stderr();
    let mut queue = if options.packed_captures
        && project
            .root()
            .join("Flux/.fluxvault-storage-queue")
            .is_dir()
    {
        Some(crate::flux_archive::Queue::start(&project)?)
    } else {
        None
    };
    let mut storage_errors = Vec::new();
    let response = run_with_io(
        &mut project,
        &options,
        crate::run_control::Input::stdin(),
        &mut stderr,
        |project, disk| {
            let response = flux::recover_reserved(
                project,
                flux::RecoveryOptions {
                    disk,
                    profile: options.profile_for(disk)?,
                    automatic_format: options.automatic_for(disk),
                    drive: options.drive,
                    protected: true,
                    policy: options.policy.clone(),
                    acquisition_only: true,
                    json_output: true,
                },
                &reservation,
            )?;
            if !matches!(response.exit_code, 0 | 3) {
                return Err(format!("Recovery for disk {disk:03} did not complete"));
            }
            let value: serde_json::Value = serde_json::from_str(&response.output)
                .map_err(|e| format!("Invalid recovery response: {e}"))?;
            serde_json::from_value(value["recovery"].clone())
                .map_err(|e| format!("Invalid recovery result: {e}"))
        },
        |project, result| {
            flux_recovery::verify_completed_result(project, result)?;
            if let Some(worker) = processing.borrow().as_ref() {
                if result.format_exception.is_none()
                    && let Err(error) = worker.enqueue(result)
                {
                    processing_errors.borrow_mut().push(error);
                }
                match crate::processing::status(project) {
                    Ok(state) => eprintln!(
                        "Background saved-file work: {} pending/failed; {}. Next swap cue follows.",
                        state["pending"],
                        state["worker"]["stage"].as_str().unwrap_or("starting")
                    ),
                    Err(error) => processing_errors.borrow_mut().push(error),
                }
            }
            if options.packed_captures && queue.is_none() {
                match crate::flux_archive::Queue::start(project) {
                    Ok(worker) => queue = Some(worker),
                    Err(error) => storage_errors.push(error),
                }
            }
            if let Some(queue) = &queue {
                for attempt in &result.capture_attempts {
                    if let Err(error) = queue.enqueue(result.disk, *attempt) {
                        storage_errors.push(error);
                    }
                }
            }
            Ok(())
        },
        |project| {
            // Tail processing acquires its own owner; release only after all
            // physical feeding/publication is finished, never during a read.
            drop(feeding_owner.borrow_mut().take());
            if let Some(request) = &request {
                let outcome = processing.borrow_mut().take().unwrap().finish();
                let _owner = crate::project_work::reserve(project.root())?;
                let final_result =
                    crate::pipeline::run_pipeline_incremental(request, &|s| eprintln!("{s}"))?;
                let mut value = crate::processing::summary(&final_result);
                crate::processing::record_final(project, &value)?;
                let attention = value["exit_code"] != 0
                    || !outcome.errors.is_empty()
                    || !processing_errors.borrow().is_empty();
                value["background_processing"] =
                    serde_json::to_value(outcome).map_err(|e| e.to_string())?;
                value["enqueue_errors"] = json!(*processing_errors.borrow());
                Ok(CliResponse {
                    output: value.to_string(),
                    exit_code: if attention { 3 } else { 0 },
                })
            } else {
                flux::process_saved_with_workers(project, options.conversion_workers)
            }
        },
    );
    if let Some(queue) = queue {
        eprintln!("Finishing queued lossless capture storage; no physical drive access...");
        storage_errors.extend(queue.finish());
    }
    for error in &storage_errors {
        eprintln!("Capture storage attention: {error}");
    }
    let mut response = response?;
    if !storage_errors.is_empty() {
        response.exit_code = 3;
    }
    if options.json_output {
        let mut value: serde_json::Value =
            serde_json::from_str(&response.output).map_err(|e| e.to_string())?;
        value["capture_storage"] =
            json!({"managed_packing":options.packed_captures,"errors":storage_errors});
        response.output = value.to_string();
    } else if options.packed_captures {
        response.output.push_str(&format!("\nManaged capture storage: {}. Verified ZIP captures retain the original SCP bytes/hash; failed tasks remain resumable.", if storage_errors.is_empty() { "completed" } else { "needs attention" }));
    }
    Ok(response)
}

fn regular_or_missing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(format!("Unsafe scan control path: {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn reserve(root: &Path) -> Result<File, String> {
    let path = root.join(LOCK);
    regular_or_missing(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|_| "This project already has a running Greaseweazle scan".to_owned())?;
    Ok(file)
}

fn save(path: &Path, journal: &Journal) -> Result<(), String> {
    regular_or_missing(path)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temporary = path.with_file_name(format!(
        ".fluxvault-gw-scan-{}-{nonce}.partial.json",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(journal).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|e| format!("Cannot commit scan progress: {e}"))
}

fn load(path: &Path, options: &ScanOptions) -> Result<Journal, String> {
    regular_or_missing(path)?;
    let mut journal: Journal = if path.exists() {
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid scan journal: {e}"))?
    } else {
        Journal {
            schema_version: 1,
            profile: options.profile.argument().to_owned(),
            automatic_format: options.automatic_format,
            packed_captures: options.packed_captures,
            background_processing: options.background_processing,
            profile_map: options.profile_map.clone(),
            drive: options.drive,
            policy: options.policy.clone(),
            last_disk: options.last_disk,
            conversion_workers: options.conversion_workers,
            pending: None,
            completed: Vec::new(),
        }
    };
    validate_profiles(&journal.profile_map)?;
    validate_workers(journal.conversion_workers)?;
    let ids = journal
        .completed
        .iter()
        .map(|r| r.disk)
        .collect::<BTreeSet<_>>();
    if journal.schema_version != 1
        || ids.contains(&0)
        || ids.len() != journal.completed.len()
        || journal
            .pending
            .as_ref()
            .is_some_and(|p| p.disk == 0 || ids.contains(&p.disk))
    {
        return Err("Invalid or duplicated disk identities in scan journal".to_owned());
    }
    if journal.pending.is_some()
        && (journal.profile != options.profile.argument()
            || journal.automatic_format != options.automatic_format
            || journal.profile_map != options.profile_map
            || journal.drive != options.drive
            || journal.policy != options.policy)
    {
        return Err("Resume the pending disk with the same profile, drive, and policy".to_owned());
    }
    if journal.pending.is_some() && journal.last_disk != options.last_disk {
        return Err("Resume the pending disk with the same --last-disk target".to_owned());
    }
    journal.profile = options.profile.argument().to_owned();
    journal.automatic_format = options.automatic_format;
    journal.packed_captures = options.packed_captures;
    journal.background_processing = options.background_processing;
    journal.profile_map = options.profile_map.clone();
    journal.drive = options.drive;
    journal.policy = options.policy.clone();
    journal.last_disk = options.last_disk;
    journal.conversion_workers = options.conversion_workers;
    Ok(journal)
}

// Callbacks isolate custody/journal tests from both hardware and external tools.
fn run_with_io<R, W, F, V, P>(
    project: &mut ProjectState,
    options: &ScanOptions,
    mut input: R,
    output: &mut W,
    mut recover_disk: F,
    mut verify: V,
    mut process: P,
) -> Result<CliResponse, String>
where
    R: BufRead,
    W: Write,
    F: FnMut(&ProjectState, u32) -> Result<RecoveryResult, String>,
    V: FnMut(&ProjectState, &RecoveryResult) -> Result<(), String>,
    P: FnMut(&ProjectState) -> Result<CliResponse, String>,
{
    options.validate()?;
    crate::flux_capture::project_flux_dir(project)?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    let _lock = reserve(&root)?; // Includes the downstream tail, not just physical work.
    *project = ProjectState::open_without_session(root.clone())?;
    let path = root.join(JOURNAL);
    let mut journal = load(&path, options)?;
    let mut telemetry = benchmark::Session::start(
        project,
        json!({"profile":options.profile.argument(),
        "automatic_format":options.automatic_format,
        "packed_captures":options.packed_captures,
        "background_processing":options.background_processing,
        "profile_map":options.profile_map,
        "drive":options.drive,"policy":options.policy,"count":options.count,
        "acquisition_only":options.acquisition_only,"start_disk":project.current_disk_number(),
        "last_disk":options.last_disk,"conversion_workers":options.conversion_workers,
        "identity_confirmation":if options.no_verify {"enter_only"} else {"numbered"}}),
    )?;
    let mut results = Vec::new();
    let mut resumed_advances = 0usize;
    let mut waiting: Option<(u32, Instant)> = None;
    let mut no_index_reseats = 0usize;
    let mut reseat_error: Option<String> = None;
    loop {
        crate::cancellation::check()?;
        if options.count.is_some_and(|limit| results.len() >= limit) {
            break;
        }
        let disk = journal
            .pending
            .as_ref()
            .map_or(project.current_disk_number(), |p| p.disk);
        if options.last_disk.is_some_and(|last| disk > last) {
            if journal.pending.is_some() {
                return Err("End-disk target excludes a pending job; no read started".to_owned());
            }
            break;
        }
        let next = disk.checked_add(1).ok_or("Disk number overflow")?;
        let profile = options.profile_for(disk)?;
        if journal.completed.iter().any(|r| r.disk == disk) {
            return Err(format!(
                "Disk {disk:03} is already completed in this scan; select the next unscanned disk"
            ));
        }
        let current = project.current_disk_number();
        let saved_result = journal.pending.as_ref().and_then(|p| p.result.clone());
        if current != disk && !(saved_result.is_some() && current == next) {
            return Err(format!(
                "Pending disk {disk:03} disagrees with project numbering; no read started"
            ));
        }
        let numbering_resumed = saved_result.is_some();
        let mut recovery_elapsed_ms = None;
        let result = if let Some(mut result) = saved_result {
            resumed_advances += 1;
            writeln!(
                output,
                "Resuming saved disk {disk:03}; verifying evidence without reading media."
            )
            .map_err(|e| e.to_string())?;
            result.physical_reads_this_run = 0;
            result.resumed = true;
            result
        } else {
            if waiting.as_ref().is_none_or(|(number, _)| *number != disk) {
                waiting = Some((disk, Instant::now()));
            }
            if options.no_verify {
                terminal::banner(
                    output,
                    options.color,
                    Cue::Attention,
                    "WARNING: --no-verify / LABEL CONFIRMATION SKIPPED",
                    "Enter assigns the displayed number. Check the label and OPEN protection hole.\nRead-only access and all image/hash/provenance verification remain ON.",
                )?;
            }
            if let Some(error) = &reseat_error {
                terminal::banner(
                    output,
                    options.color,
                    Cue::Error,
                    &format!(
                        "NO INDEX {disk:03} / REMOVE AND REINSERT SAME DISK / RETRY {no_index_reseats} OF {MAX_NO_INDEX_RESEATS}"
                    ),
                    &format!(
                        "Physical read has stopped; safe to remove the floppy.\nRemove and fully reinsert disk {disk:03}, check label/protection, drive power and closed door/lever.\nDo NOT insert the next disk. No read starts until you confirm again; QUIT stops safely.\nAttempt evidence retained.\n{error}"
                    ),
                )?;
            } else {
                terminal::banner(
                    output,
                    options.color,
                    Cue::Action,
                    &format!("GW {} / WAITING FOR YOU / INSERT {disk:03}", options.drive),
                    &format!(
                        "Check disk label {disk:03} and OPEN write-protect hole. Format: {}",
                        options.format_label(disk)?
                    ),
                )?;
            }
            if options.no_verify {
                writeln!(
                    output,
                    "Press Enter after inserting {disk:03}, QUIT to drain, or STOP to cancel:"
                )
                .map_err(|e| e.to_string())?;
            } else {
                writeln!(
                    output,
                    "Type {disk:03} to confirm and read it, QUIT to drain, or STOP to cancel: [format {}]",
                    options.format_label(disk)?
                )
                .map_err(|e| e.to_string())?;
            }
            output.flush().map_err(|e| e.to_string())?;
            let mut answer = String::new();
            if input.read_line(&mut answer).map_err(|e| e.to_string())? == 0 {
                break;
            }
            let words: Vec<_> = answer.split_whitespace().collect();
            if words.len() == 1
                && (words[0].eq_ignore_ascii_case("QUIT") || words[0].eq_ignore_ascii_case("Q"))
            {
                break;
            }
            let number = match words.as_slice() {
                [number] => number.parse::<u32>().ok(),
                [verb, number] if verb.eq_ignore_ascii_case("READ") => number.parse::<u32>().ok(),
                _ => None,
            };
            if number != Some(disk) && !(options.no_verify && words.is_empty()) {
                writeln!(
                    output,
                    "No read started. Confirm the displayed disk number {disk:03}, or QUIT."
                )
                .map_err(|e| e.to_string())?;
                continue;
            }
            let fresh = ProjectState::open_without_session(root.clone())?;
            if fresh.current_disk_number() != disk {
                return Err(
                    "Project disk selection changed while awaiting confirmation; no read started"
                        .to_owned(),
                );
            }
            *project = fresh;
            let reseat_confirmed = reseat_error.take().is_some();
            journal.pending = Some(Pending { disk, result: None });
            save(&path, &journal)?; // Custody persists before any physical operation.
            telemetry.record(
                "read_confirmed",
                json!({"disk":disk,"profile":profile.argument(),
                "reseat_after_no_index":reseat_confirmed,
                "identity_confirmation":if options.no_verify && words.is_empty() {"enter_only"} else {"numbered"},
                "operator_wait_ms":benchmark::milliseconds(waiting.take().unwrap().1.elapsed())}),
            )?;
            terminal::banner(
                output,
                options.color,
                Cue::Action,
                &format!("READING {disk:03} / DO NOT REMOVE"),
                "READ ONLY. Swap only after the saved-result cue.",
            )?;
            let started = Instant::now();
            match recover_disk(project, disk) {
                Ok(result) => {
                    recovery_elapsed_ms = Some(benchmark::milliseconds(started.elapsed()));
                    result
                }
                Err(error) => {
                    let retryable = crate::flux_capture::is_no_index_capture_failure(&error)
                        && no_index_reseats < MAX_NO_INDEX_RESEATS;
                    telemetry.record(
                        "recovery_failed",
                        json!({"disk":disk,"phase":"acquisition",
                        "cancelled":crate::cancellation::requested(),
                        "elapsed_ms":benchmark::milliseconds(started.elapsed()),"error":error,
                        "reseat_retry_available":retryable,"reseat_retries_used":no_index_reseats}),
                    )?;
                    if retryable {
                        no_index_reseats += 1;
                        reseat_error = Some(error);
                        // Pending custody stays on this disk. A new confirmation
                        // is required, even in Enter-only mode; no blind retry.
                        continue;
                    }
                    terminal::banner(
                        output,
                        options.color,
                        Cue::Error,
                        &format!(
                            "{} {disk:03} / NUMBER NOT ADVANCED",
                            if crate::cancellation::requested() {
                                "READ CANCELLED"
                            } else {
                                "READ FAILED"
                            }
                        ),
                        if crate::cancellation::requested() {
                            "Stop requested. Keep the disk seated until the final STOPPED cue and drive activity has stopped. Pending label and partial evidence retained."
                        } else if crate::flux_capture::is_no_index_capture_failure(&error) {
                            "No Index persisted after two confirmed reseat retries. Evidence retained; check disk seating/drive/power before manually resuming this same project. No next-disk read started."
                        } else {
                            "Evidence retained. No automatic next-disk read; inspect the error before retrying."
                        },
                    )?;
                    return Err(error);
                }
            }
        };
        if result.disk != disk
            || !matches!(
                result.status.as_str(),
                "acquired" | "partial" | "unrecoverable_within_policy" | "raw_format_exception"
            )
        {
            return Err("Recovery returned a different disk or invalid terminal state".to_owned());
        }
        let verification = Instant::now();
        if let Err(error) = verify(project, &result) {
            telemetry.record(
                "recovery_failed",
                json!({"disk":disk,"phase":"verification",
                "elapsed_ms":benchmark::milliseconds(verification.elapsed()),"error":error}),
            )?;
            terminal::banner(
                output,
                options.color,
                Cue::Error,
                &format!("VERIFICATION FAILED {disk:03} / NUMBER NOT ADVANCED"),
                "Saved evidence did not pass integrity checks. No next-disk read started.",
            )?;
            return Err(error);
        }
        let selected_profile = result
            .selected_profile
            .as_deref()
            .unwrap_or(profile.argument());
        if let Some(elapsed_ms) = recovery_elapsed_ms {
            telemetry.record("recovery_finished", json!({"disk":disk,"profile":selected_profile,"elapsed_ms":elapsed_ms,
                "verification_ms":benchmark::milliseconds(verification.elapsed()),"outcome":benchmark::outcome(&result)}))?;
        }
        journal.pending = Some(Pending {
            disk,
            result: Some(result.clone()),
        });
        save(&path, &journal)?; // Publication precedes the numbering commit.
        let fresh = ProjectState::open_without_session(root.clone())?;
        if fresh.current_disk_number() != disk && fresh.current_disk_number() != next {
            return Err(
                "Project disk selection changed during recovery; saved evidence retained"
                    .to_owned(),
            );
        }
        *project = fresh;
        if project.current_disk_number() == disk {
            project.set_current_disk_number_without_session(next)?;
        }
        journal.completed.push(result.clone());
        journal.pending = None;
        save(&path, &journal)?;
        telemetry.record(
            "disk_committed",
            json!({"disk":disk,"profile":selected_profile,"next_disk":next,
            "numbering_resumed":numbering_resumed,"outcome":benchmark::outcome(&result)}),
        )?;
        let capped = options.last_disk.is_some_and(|last| disk >= last)
            || options
                .count
                .is_some_and(|limit| results.len() + 1 >= limit);
        let action = if capped {
            format!("REMOVE {disk:03} / BATCH FINISHED / NO NEXT INSERTION")
        } else {
            format!("REMOVE {disk:03} / INSERT {next:03}")
        };
        terminal::banner(
            output,
            options.color,
            if result.status == "acquired" {
                Cue::Success
            } else if result.format_exception.is_some() {
                Cue::Attention
            } else {
                Cue::Error
            },
            &format!(
                "GW SWAP / {} {disk:03} / {action}",
                if result.status == "acquired" {
                    "DONE"
                } else if result.format_exception.is_some() {
                    "RAW-ONLY FORMAT EXCEPTION SAVED"
                } else {
                    "PARTIAL SAVED"
                }
            ),
            &format!(
                "{}; {} missing, {} conflicting. {}. {} verification passed. Safe to swap; proceed with the displayed action.",
                result.status,
                if result.format_exception.is_some() {
                    "unknown".into()
                } else {
                    result.missing_lbas.len().to_string()
                },
                if result.format_exception.is_some() {
                    "unknown".into()
                } else {
                    result.conflicting_lbas.len().to_string()
                },
                recovery_elapsed_ms
                    .map(|ms| format!("Read/decode: {:.1}s", ms as f64 / 1000.0))
                    .unwrap_or_else(|| "Resumed saved evidence".to_owned()),
                if result.format_exception.is_some() {
                    "Raw capture/format report; no sector geometry or image claimed. Evidence"
                } else {
                    "Image and evidence"
                }
            ),
        )?;
        results.push(result);
        no_index_reseats = 0;
        reseat_error = None;
    }
    let mut attention =
        journal.pending.is_some() || journal.completed.iter().any(|r| r.status != "acquired");
    let processing = if options.acquisition_only
        || journal.completed.is_empty()
        || journal
            .completed
            .iter()
            .all(|r| r.format_exception.is_some())
    {
        json!({"skipped":true})
    } else {
        terminal::banner(
            output,
            options.color,
            Cue::Action,
            "FEEDING FINISHED / REMOVE THE FLOPPY",
            &format!(
                "{} saved-file work: extraction, conversion ({} requested workers), audit and workbook. No more insertions requested.",
                if options.background_processing {
                    "Draining and reconciling background"
                } else {
                    "Starting"
                },
                options.conversion_workers
            ),
        )?;
        telemetry.record("downstream_started", json!({}))?;
        let started = Instant::now();
        match process(project) {
            Ok(response) => {
                attention |= response.exit_code != 0;
                let detail = serde_json::from_str(&response.output)
                    .unwrap_or_else(|_| json!({"detail":response.output}));
                telemetry.record(
                    "downstream_finished",
                    json!({"elapsed_ms":benchmark::milliseconds(started.elapsed()),
                    "exit_code":response.exit_code,"summary":detail}),
                )?;
                detail
            }
            Err(error) => {
                attention = true;
                telemetry.record(
                    "downstream_finished",
                    json!({"elapsed_ms":benchmark::milliseconds(started.elapsed()),
                    "exit_code":2,"error":error}),
                )?;
                json!({"error":error,"acquisition_preserved":true})
            }
        }
    };
    let partial = results.iter().filter(|r| r.status != "acquired").count();
    terminal::banner(
        output,
        options.color,
        if attention {
            Cue::Attention
        } else {
            Cue::Success
        },
        if attention {
            "SCAN FINISHED / PARTIAL OR ATTENTION RESULTS"
        } else {
            "SCAN FINISHED / CLEAN RESULTS"
        },
        &format!(
            "This session: {} saved, {partial} partial. Project: {} saved. See Reports for results.\nNo floppy insertion requested; resume cursor is {:03}.",
            results.len(),
            journal.completed.len(),
            project.current_disk_number()
        ),
    )?;
    telemetry.finish(results.len(), project.current_disk_number())?;
    let measured = benchmark::report(project)?;
    let (benchmark_json, benchmark_csv) = benchmark::export(project, &measured)?;
    Ok(CliResponse {
        output: if options.json_output {
            json!({"project":root,"scanned":results.len(),"partial_this_session":partial,
                "next_disk":project.current_disk_number(),"total_scanned":journal.completed.len(),
                "resumed_advances":resumed_advances,"pending_disk":journal.pending.as_ref().map(|p| p.disk),
                "disks":results,"processing":processing,
                "identity_confirmation":if options.no_verify {"enter_only"} else {"numbered"},
                "conversion_workers":options.conversion_workers,
                "benchmark":{"events":telemetry.path(),"summary":benchmark_json,"disks_csv":benchmark_csv,
                    "unique_disks":measured.unique_committed_disks,"projected_136_feed_hours":measured.projected_136_feed_hours},
                "source_media_access":"read_only","customer_delivery_certified":false})
            .to_string()
        } else {
            let downstream = if processing["skipped"] == true {
                "Skipped.".to_owned()
            } else if let Some(error) = processing["error"].as_str() {
                format!("Needs attention: {error}; acquisitions retained.")
            } else {
                format!(
                    "Extracted: {}; converted OK: {}; failed: {}; workbook: {}.",
                    processing["extracted"],
                    processing["converted_ok"],
                    processing["converted_failed"],
                    processing["workbook"].as_str().unwrap_or("See Reports")
                )
            };
            format!(
                "Greaseweazle feeding finished: {} disk(s) saved this session ({partial} partial). Project total: {}.\nResume cursor: {:03} (not an insertion request).\nDownstream: {downstream}\nPilot benchmark: {}\nPer-disk CSV: {}",
                results.len(),
                journal.completed.len(),
                project.current_disk_number(),
                benchmark_json.display(),
                benchmark_csv.display()
            )
        },
        exit_code: if attention { 3 } else { 0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(
                std::env::temp_dir()
                    .join(format!("fluxvault-gw-scan-{}-{nonce}", std::process::id())),
            )
        }
        fn project(&self) -> ProjectState {
            ProjectState::create_without_session(self.0.clone()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn options() -> ScanOptions {
        ScanOptions {
            profile: GreaseweazleProfile::Ibm1440,
            automatic_format: false,
            packed_captures: false,
            background_processing: false,
            profile_map: BTreeMap::new(),
            drive: 'B',
            protected: true,
            policy: RecoveryPolicy::default(),
            count: None,
            last_disk: None,
            acquisition_only: true,
            json_output: true,
            no_verify: false,
            color: false,
            conversion_workers: default_conversion_workers(),
        }
    }

    #[test]
    fn enter_only_is_explicit_audited_not_persisted_and_still_verifies_every_result() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let mut opts = options();
        opts.no_verify = true;
        opts.color = true;
        opts.last_disk = Some(2);
        opts.conversion_workers = 12;
        let mut reads = Vec::new();
        let mut verified = Vec::new();
        let mut output = Vec::new();
        let response = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"999\n\n\r\n\n"),
            &mut output,
            |p, disk| {
                reads.push(disk);
                Ok(result(p, disk, disk == 2))
            },
            |_, result| {
                verified.push(result.disk);
                Ok(())
            },
            skipped,
        )
        .unwrap();
        assert_eq!(reads, [1, 2]);
        assert_eq!(verified, [1, 2]);
        assert_eq!(project.current_disk_number(), 3);
        let summary: serde_json::Value = serde_json::from_str(&response.output).unwrap();
        assert_eq!(summary["identity_confirmation"], "enter_only");
        assert_eq!(summary["conversion_workers"], 12);
        assert!(!response.output.contains('\x1b'));
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("WARNING: --no-verify"));
        assert!(output.contains("WAITING FOR YOU / INSERT 001"));
        assert!(output.contains("PARTIAL SAVED 002"));
        assert!(output.contains("\x1b[1;31m============================================================\nGW SWAP / PARTIAL SAVED 002"));
        assert!(output.contains("Safe to swap; proceed"));
        assert!(output.contains("REMOVE 002 / BATCH FINISHED / NO NEXT INSERTION"));
        assert!(!output.contains("INSERT 003"));
        assert!(output.contains('\x1b'));
        let saved = saved_defaults(&project).unwrap().unwrap();
        assert_eq!(saved.conversion_workers, 12);
        let journal = fs::read_to_string(fixture.0.join(JOURNAL)).unwrap();
        assert!(!journal.contains("no_verify"));
        let measured = benchmark::report(&project).unwrap();
        assert_eq!(
            measured.session_configurations[0]["data"]["configuration"]["identity_confirmation"],
            "enter_only"
        );
        assert_eq!(
            measured.session_configurations[0]["data"]["configuration"]["conversion_workers"],
            12
        );
    }

    #[test]
    fn eof_never_confirms_and_default_mode_rejects_blank_input() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        for enter_only in [false, true] {
            let mut opts = options();
            opts.no_verify = enter_only;
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b""),
                &mut Vec::new(),
                no_read,
                |_, _| panic!("nothing to verify"),
                skipped,
            )
            .unwrap();
        }
        run_with_io(
            &mut project,
            &options(),
            Cursor::new(b"\n \r\nQUIT\n"),
            &mut Vec::new(),
            no_read,
            |_, _| panic!("blank input is not custody by default"),
            skipped,
        )
        .unwrap();
        assert_eq!(project.current_disk_number(), 1);
        assert!(!fixture.0.join(JOURNAL).exists());
    }

    #[test]
    fn enter_only_never_bypasses_evidence_verification_or_advances_a_failed_result() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let mut opts = options();
        opts.no_verify = true;
        let error = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"\n\n"),
            &mut Vec::new(),
            |p, disk| Ok(result(p, disk, false)),
            |_, _| Err("Changed image hash".to_owned()),
            skipped,
        )
        .unwrap_err();
        assert!(error.contains("Changed image hash"));
        assert_eq!(project.current_disk_number(), 1);
        assert!(
            load(&fixture.0.join(JOURNAL), &opts)
                .unwrap()
                .pending
                .is_some()
        );
        assert!(
            load(&fixture.0.join(JOURNAL), &opts)
                .unwrap()
                .completed
                .is_empty()
        );
    }

    #[test]
    fn legacy_scan_settings_default_workers_and_invalid_saved_workers_are_refused() {
        let fixture = Fixture::new();
        let project = fixture.project();
        let path = fixture.0.join(JOURNAL);
        let journal = load(&path, &options()).unwrap();
        let mut value = serde_json::to_value(journal).unwrap();
        value.as_object_mut().unwrap().remove("conversion_workers");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            saved_defaults(&project)
                .unwrap()
                .unwrap()
                .conversion_workers,
            4
        );
        for workers in [0, 17] {
            value["conversion_workers"] = json!(workers);
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(saved_defaults(&project).is_err());
            assert!(load(&path, &options()).is_err());
        }
    }
    fn result(project: &ProjectState, disk: u32, partial: bool) -> RecoveryResult {
        RecoveryResult {
            disk,
            selected_profile: None,
            status: if partial { "partial" } else { "acquired" }.to_owned(),
            stop_reason: "synthetic".to_owned(),
            capture_attempts: vec![1],
            image: project.images_dir().join(format!("{disk:03}.img")),
            image_sha256: "test".to_owned(),
            provenance: project.root().join("Flux").join(format!("{disk:03}.json")),
            provenance_sha256: "test".to_owned(),
            missing_lbas: if partial { vec![24] } else { vec![] },
            conflicting_lbas: vec![],
            corroborated_sectors: 0,
            single_capture_sectors: 0,
            physical_reads_this_run: 1,
            resumed: false,
            format_exception: None,
        }
    }
    fn skipped(_: &ProjectState) -> Result<CliResponse, String> {
        panic!("processing must be skipped")
    }
    fn no_read(_: &ProjectState, _: u32) -> Result<RecoveryResult, String> {
        panic!("no physical read authorized")
    }

    #[test]
    fn profile_map_is_bounded_strict_and_selects_only_listed_disks() {
        let fixture = Fixture::new();
        let _project = fixture.project();
        let path = fixture.0.join("profiles.json");
        for invalid in [
            r#"{"schema_version":2,"profiles":[]}"#,
            r#"{"schema_version":1,"profiles":[{"disk":0,"profile":"ibm.720"}]}"#,
            r#"{"schema_version":1,"profiles":[{"disk":9,"profile":"ibm.720"},{"disk":9,"profile":"ibm.1440"}]}"#,
            r#"{"schema_version":1,"profiles":[{"disk":9,"profile":"720"}]}"#,
            r#"{"schema_version":1,"profiles":[],"typo":true}"#,
            r#"{"schema_version":1,"profiles":[{"disk":9,"profile":"ibm.scan"}]}"#,
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(load_profile_map(&path).is_err(), "accepted {invalid}");
        }
        fs::write(&path, vec![b' '; 65_537]).unwrap();
        assert!(load_profile_map(&path).unwrap_err().contains("64 KiB"));
        fs::write(
            &path,
            include_str!("../../policies/customer-first-20-profiles.json"),
        )
        .unwrap();
        let mut opts = options();
        opts.profile_map = load_profile_map(&path).unwrap();
        opts.validate().unwrap();
        assert_eq!(opts.profile_for(8).unwrap(), GreaseweazleProfile::Ibm1440);
        assert_eq!(opts.profile_for(9).unwrap(), GreaseweazleProfile::Ibm720);
        assert_eq!(opts.profile_for(10).unwrap(), GreaseweazleProfile::Ibm1440);
        let policy: RecoveryPolicy =
            serde_json::from_str(include_str!("../../policies/pilot-short.json")).unwrap();
        policy.validate().unwrap();
        assert_eq!(policy.max_seconds, 180);
        assert!(load_profile_map(Path::new("A:\\profiles.json")).is_err());
    }

    #[test]
    fn pending_mixed_format_job_refuses_changed_map_and_older_journals_still_load() {
        let fixture = Fixture::new();
        let project = fixture.project();
        let mut opts = options();
        opts.profile_map.insert(9, "ibm.720".to_owned());
        let path = fixture.0.join(JOURNAL);
        let mut journal = load(&path, &opts).unwrap();
        journal.pending = Some(Pending {
            disk: 9,
            result: None,
        });
        save(&path, &journal).unwrap();
        let mut changed = options();
        changed.profile_map.insert(9, "ibm.1440".to_owned());
        assert!(
            load(&path, &changed)
                .err()
                .unwrap()
                .contains("same profile")
        );
        assert_eq!(load(&path, &opts).unwrap().profile_map[&9], "ibm.720");
        let mut legacy = serde_json::to_value(journal).unwrap();
        legacy.as_object_mut().unwrap().remove("profile_map");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(load(&path, &options()).unwrap().profile_map.is_empty());
        assert_eq!(project.current_disk_number(), 1);
    }

    #[test]
    fn downstream_preflight_checks_both_tools_and_explicit_capture_only_skips_it() {
        use crate::external_tools::ToolKind;
        let mut checked = Vec::new();
        let error = preflight_downstream(false, |tool| {
            checked.push(tool);
            if tool == ToolKind::LibreOffice {
                Err("LibreOffice unavailable".to_owned())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(checked, [ToolKind::SevenZip, ToolKind::LibreOffice]);
        assert!(error.contains("before any media read"));
        assert!(error.contains("--acquisition-only"));
        preflight_downstream(true, |_| {
            panic!("capture-only must not require downstream tools")
        })
        .unwrap();
    }

    #[test]
    fn twenty_disk_mixed_format_pilot_stops_at_target_and_runs_one_tail() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let mut opts = options();
        opts.profile_map.insert(9, "ibm.720".to_owned());
        opts.last_disk = Some(20);
        opts.acquisition_only = false;
        let mut processed = 0;
        let mut output = Vec::new();
        let confirmations = (1..=21)
            .map(|n| format!("READ {n:03}\n"))
            .collect::<String>();
        let response = run_with_io(
            &mut project,
            &opts,
            Cursor::new(confirmations),
            &mut output,
            |p, n| {
                assert!(n <= 20);
                Ok(result(p, n, n % 6 == 0))
            },
            |_, _| Ok(()),
            |_| {
                processed += 1;
                Ok(CliResponse {
                    output: "{}".to_owned(),
                    exit_code: 0,
                })
            },
        )
        .unwrap();
        assert_eq!(response.exit_code, 3);
        assert_eq!(processed, 1);
        assert_eq!(project.current_disk_number(), 21);
        let output = String::from_utf8(output).unwrap();
        assert!(
            output
                .lines()
                .any(|line| line.contains("Type 009") && line.contains("ibm.720"))
        );
        assert!(
            output
                .lines()
                .any(|line| line.contains("Type 010") && line.contains("ibm.1440"))
        );
        assert!(!output.contains("Type 021"));
        let measured = benchmark::report(&project).unwrap();
        assert_eq!(measured.unique_committed_disks, 20);
        assert_eq!(measured.status_counts["partial"], 3);
    }

    #[test]
    fn numbered_custody_partial_continuation_and_one_downstream_tail() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let mut opts = options();
        opts.count = Some(2);
        opts.acquisition_only = false;
        let mut output = Vec::new();
        let mut reads = Vec::new();
        let mut processes = 0;
        let response = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"READ\nREAD 999\nread 001\nREAD 2\nREAD 3\n"),
            &mut output,
            |p, disk| {
                reads.push(disk);
                Ok(result(p, disk, disk == 1))
            },
            |_, _| Ok(()),
            |_| {
                processes += 1;
                Ok(CliResponse {
                    output: "{}".to_owned(),
                    exit_code: 0,
                })
            },
        )
        .unwrap();
        assert_eq!(reads, vec![1, 2]);
        assert_eq!(processes, 1);
        assert_eq!(response.exit_code, 3);
        let json: serde_json::Value = serde_json::from_str(&response.output).unwrap();
        assert_eq!(json["scanned"], 2);
        assert_eq!(json["partial_this_session"], 1);
        assert_eq!(json["next_disk"], 3);
        assert_eq!(
            ProjectState::open_without_session(fixture.0.clone())
                .unwrap()
                .current_disk_number(),
            3
        );
        assert!(String::from_utf8(output).unwrap().contains("GW SWAP"));
    }

    #[test]
    fn safety_and_policy_gates_precede_reading_input_or_creating_progress() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        for kind in 0..4 {
            let mut opts = options();
            match kind {
                0 => opts.protected = false,
                1 => opts.count = Some(0),
                2 => opts.drive = 'C',
                _ => opts.policy.max_seconds = 0,
            }
            assert!(
                run_with_io(
                    &mut project,
                    &opts,
                    Cursor::new(b"READ 1\n"),
                    &mut Vec::new(),
                    no_read,
                    |_, _| panic!("no verification"),
                    skipped
                )
                .is_err()
            );
            assert!(!fixture.0.join(JOURNAL).exists());
        }
    }

    #[test]
    fn failed_read_keeps_number_and_requires_new_custody_confirmation() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let opts = options();
        let error = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"READ 1\n"),
            &mut Vec::new(),
            |_, _| Err("host stopped".to_owned()),
            |_, _| Ok(()),
            skipped,
        )
        .unwrap_err();
        assert!(error.contains("host stopped"));
        assert_eq!(project.current_disk_number(), 1);
        let journal = load(&fixture.0.join(JOURNAL), &opts).unwrap();
        assert_eq!(journal.pending.unwrap().disk, 1);
        let quit = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"QUIT\n"),
            &mut Vec::new(),
            no_read,
            |_, _| Ok(()),
            skipped,
        )
        .unwrap();
        assert_eq!(quit.exit_code, 3);
        let mut bounded = options();
        bounded.count = Some(1);
        run_with_io(
            &mut project,
            &bounded,
            Cursor::new(b"READ 1\n"),
            &mut Vec::new(),
            |p, disk| Ok(result(p, disk, false)),
            |_, _| Ok(()),
            skipped,
        )
        .unwrap();
        assert_eq!(project.current_disk_number(), 2);
    }

    fn no_index_error() -> String {
        format!(
            "{}Command Failed: GetFluxStatus: No Index",
            crate::flux_capture::NO_INDEX_ERROR_PREFIX
        )
    }

    #[test]
    fn no_index_reseat_rejects_next_number_and_advances_once_after_same_disk_confirmation() {
        for enter_only in [false, true] {
            let fixture = Fixture::new();
            let mut project = fixture.project();
            let mut opts = options();
            opts.no_verify = enter_only;
            opts.last_disk = Some(1);
            opts.color = true;
            let input: &[u8] = if enter_only { b"\n2\n\n" } else { b"1\n2\n1\n" };
            let mut reads = 0;
            let mut verified = 0;
            let mut output = Vec::new();
            let response = run_with_io(
                &mut project,
                &opts,
                Cursor::new(input),
                &mut output,
                |p, disk| {
                    assert_eq!(disk, 1);
                    assert_eq!(p.current_disk_number(), 1);
                    reads += 1;
                    if reads == 1 {
                        Err(no_index_error())
                    } else {
                        Ok(result(p, disk, false))
                    }
                },
                |_, _| {
                    verified += 1;
                    Ok(())
                },
                skipped,
            )
            .unwrap();
            assert_eq!(response.exit_code, 0);
            assert_eq!((reads, verified, project.current_disk_number()), (2, 1, 2));
            let text = String::from_utf8(output).unwrap();
            assert!(text.contains("NO INDEX 001 / REMOVE AND REINSERT SAME DISK / RETRY 1 OF 2"));
            assert!(text.contains("safe to remove"));
            assert!(text.contains("No read started"));
            assert!(!text.contains("INSERT 002"));
            assert!(text.contains("\x1b[1;31m"));
            let stats = benchmark::report(&project).unwrap();
            assert_eq!(stats.unique_committed_disks, 1);
            assert_eq!(stats.recovery_errors, 1);
            assert_eq!(stats.confirmed_insertions, 2);
            assert_eq!(
                load(&fixture.0.join(JOURNAL), &opts)
                    .unwrap()
                    .completed
                    .len(),
                1
            );
        }
    }

    #[test]
    fn no_index_eof_quit_or_wrong_label_never_blindly_retries_and_resume_keeps_custody() {
        for input in [b"1\n".as_slice(), b"1\nQUIT\n", b"1\n2\nQUIT\n"] {
            let fixture = Fixture::new();
            let mut project = fixture.project();
            let mut opts = options();
            opts.last_disk = Some(1);
            let mut reads = 0;
            let response = run_with_io(
                &mut project,
                &opts,
                Cursor::new(input),
                &mut Vec::new(),
                |_, _| {
                    reads += 1;
                    Err(no_index_error())
                },
                |_, _| panic!("no image to verify"),
                skipped,
            )
            .unwrap();
            assert_eq!(
                (response.exit_code, reads, project.current_disk_number()),
                (3, 1, 1)
            );
            let pending = load(&fixture.0.join(JOURNAL), &opts)
                .unwrap()
                .pending
                .unwrap();
            assert_eq!(pending.disk, 1);
            assert!(pending.result.is_none());
            let resumed = run_with_io(
                &mut project,
                &opts,
                Cursor::new(b"1\n"),
                &mut Vec::new(),
                |p, disk| Ok(result(p, disk, false)),
                |_, _| Ok(()),
                skipped,
            )
            .unwrap();
            assert_eq!(resumed.exit_code, 0);
            assert_eq!(project.current_disk_number(), 2);
        }
    }

    #[test]
    fn repeated_no_index_stops_after_two_reseats_without_advancing_number() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let opts = options();
        let mut reads = 0;
        let mut output = Vec::new();
        let error = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"1\n1\n1\n1\n"),
            &mut output,
            |_, _| {
                reads += 1;
                Err(no_index_error())
            },
            |_, _| panic!("no image"),
            skipped,
        )
        .unwrap_err();
        assert!(error.contains("No Index"));
        assert_eq!(reads, 3);
        assert_eq!(project.current_disk_number(), 1);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("after two confirmed reseat retries")
        );
        assert_eq!(benchmark::report(&project).unwrap().recovery_errors, 3);
    }

    #[test]
    fn misleading_no_index_text_and_verification_errors_still_stop_immediately() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let opts = options();
        let mut reads = 0;
        assert!(
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b"1\n1\n"),
                &mut Vec::new(),
                |_, _| {
                    reads += 1;
                    Err("Decode failed for No Index.scp".into())
                },
                |_, _| Ok(()),
                skipped
            )
            .is_err()
        );
        assert_eq!(reads, 1);
        assert!(
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b"1\n1\n"),
                &mut Vec::new(),
                |p, disk| Ok(result(p, disk, false)),
                |_, _| Err(no_index_error()),
                skipped
            )
            .is_err()
        );
        assert_eq!(project.current_disk_number(), 1);
    }

    #[test]
    fn restart_at_either_numbering_boundary_advances_exactly_once_without_a_read() {
        for number_already_saved in [false, true] {
            let fixture = Fixture::new();
            let mut project = fixture.project();
            let mut opts = options();
            opts.count = Some(1);
            let path = fixture.0.join(JOURNAL);
            let mut journal = load(&path, &opts).unwrap();
            journal.pending = Some(Pending {
                disk: 1,
                result: Some(result(&project, 1, false)),
            });
            save(&path, &journal).unwrap();
            if number_already_saved {
                project.set_current_disk_number_without_session(2).unwrap();
            }
            let mut verifications = 0;
            let response = run_with_io(
                &mut project,
                &opts,
                Cursor::new(b""),
                &mut Vec::new(),
                no_read,
                |_, _| {
                    verifications += 1;
                    Ok(())
                },
                skipped,
            )
            .unwrap();
            assert_eq!(verifications, 1);
            assert_eq!(project.current_disk_number(), 2);
            let json: serde_json::Value = serde_json::from_str(&response.output).unwrap();
            assert_eq!(json["resumed_advances"], 1);
            assert_eq!(json["total_scanned"], 1);
            assert_eq!(json["disks"][0]["physical_reads_this_run"], 0);
            assert_eq!(json["disks"][0]["resumed"], true);
            let response = run_with_io(
                &mut project,
                &opts,
                Cursor::new(b"QUIT\n"),
                &mut Vec::new(),
                no_read,
                |_, _| panic!("no repeat verification"),
                skipped,
            )
            .unwrap();
            assert_eq!(project.current_disk_number(), 2);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&response.output).unwrap()["scanned"],
                0
            );
        }
    }

    #[test]
    fn changed_settings_bad_hashes_and_number_mismatch_do_not_advance() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let opts = options();
        let path = fixture.0.join(JOURNAL);
        let mut journal = load(&path, &opts).unwrap();
        journal.pending = Some(Pending {
            disk: 1,
            result: Some(result(&project, 1, false)),
        });
        save(&path, &journal).unwrap();
        let mut changed = options();
        changed.profile = GreaseweazleProfile::Ibm720;
        assert!(
            run_with_io(
                &mut project,
                &changed,
                Cursor::new(b"READ 1\n"),
                &mut Vec::new(),
                no_read,
                |_, _| Ok(()),
                skipped
            )
            .unwrap_err()
            .contains("same profile")
        );
        assert!(
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b""),
                &mut Vec::new(),
                no_read,
                |_, _| Err("bad hashes".to_owned()),
                skipped
            )
            .unwrap_err()
            .contains("bad hashes")
        );
        assert_eq!(project.current_disk_number(), 1);
        project.set_current_disk_number_without_session(7).unwrap();
        assert!(
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b""),
                &mut Vec::new(),
                no_read,
                |_, _| panic!("mismatch precedes validation"),
                skipped
            )
            .unwrap_err()
            .contains("numbering")
        );
        assert_eq!(project.current_disk_number(), 7);
    }

    #[test]
    fn duplicate_scan_lock_and_existing_disk_identity_are_refused() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let opts = options();
        let lock = reserve(&fixture.0).unwrap();
        assert!(
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b"READ 1\n"),
                &mut Vec::new(),
                no_read,
                |_, _| Ok(()),
                skipped
            )
            .unwrap_err()
            .contains("running")
        );
        drop(lock);
        let mut journal = load(&fixture.0.join(JOURNAL), &opts).unwrap();
        journal.completed.push(result(&project, 1, false));
        save(&fixture.0.join(JOURNAL), &journal).unwrap();
        assert!(
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b"READ 1\n"),
                &mut Vec::new(),
                no_read,
                |_, _| Ok(()),
                skipped
            )
            .unwrap_err()
            .contains("already completed")
        );
    }

    #[test]
    fn downstream_failure_retains_completed_acquisitions_and_numbering() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let mut opts = options();
        opts.count = Some(1);
        opts.acquisition_only = false;
        let response = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"READ 1\n"),
            &mut Vec::new(),
            |p, disk| Ok(result(p, disk, false)),
            |_, _| Ok(()),
            |_| Err("converter missing".to_owned()),
        )
        .unwrap();
        assert_eq!(response.exit_code, 3);
        assert_eq!(project.current_disk_number(), 2);
        assert_eq!(
            load(&fixture.0.join(JOURNAL), &opts)
                .unwrap()
                .completed
                .len(),
            1
        );
        let json: serde_json::Value = serde_json::from_str(&response.output).unwrap();
        assert_eq!(json["processing"]["acquisition_preserved"], true);
    }

    #[test]
    fn cursor_change_while_waiting_for_custody_refuses_the_read() {
        struct SelectionChange(ProjectState);
        impl Write for SelectionChange {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.0
                    .set_current_disk_number_without_session(7)
                    .map_err(io::Error::other)
            }
        }
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let mut output = SelectionChange(project.clone());
        let error = run_with_io(
            &mut project,
            &options(),
            Cursor::new(b"READ 1\n"),
            &mut output,
            no_read,
            |_, _| panic!("no result to verify"),
            skipped,
        )
        .unwrap_err();
        assert!(error.contains("changed while awaiting confirmation"));
        assert!(!fixture.0.join(JOURNAL).exists());
        assert_eq!(
            ProjectState::open_without_session(fixture.0.clone())
                .unwrap()
                .current_disk_number(),
            7
        );
    }

    #[test]
    fn pilot_136_disk_soak_records_yield_failure_and_resume_without_number_drift() {
        let fixture = Fixture::new();
        let mut project = fixture.project();
        let mut opts = options();
        opts.count = Some(60);
        opts.last_disk = Some(136);
        let confirmations = |first, last| {
            (first..=last)
                .map(|disk| format!("READ {disk:03}\n"))
                .collect::<String>()
        };
        run_with_io(
            &mut project,
            &opts,
            Cursor::new(confirmations(1, 60)),
            &mut Vec::new(),
            |p, disk| Ok(result(p, disk, disk % 11 == 0)),
            |_, _| Ok(()),
            skipped,
        )
        .unwrap();
        assert_eq!(project.current_disk_number(), 61);
        drop(project);
        let mut project = ProjectState::open_without_session(fixture.0.clone()).unwrap();
        assert_eq!(project.current_disk_number(), 61);
        assert!(
            run_with_io(
                &mut project,
                &opts,
                Cursor::new(b"READ 061\n"),
                &mut Vec::new(),
                |_, _| Err("simulated disconnected board".to_owned()),
                |_, _| Ok(()),
                skipped
            )
            .is_err()
        );
        assert_eq!(project.current_disk_number(), 61);
        opts.count = None;
        drop(project);
        let mut project = ProjectState::open_without_session(fixture.0.clone()).unwrap();
        assert_eq!(project.current_disk_number(), 61);
        run_with_io(
            &mut project,
            &opts,
            Cursor::new(confirmations(61, 136)),
            &mut Vec::new(),
            |p, disk| Ok(result(p, disk, disk % 11 == 0)),
            |_, _| Ok(()),
            skipped,
        )
        .unwrap();
        assert_eq!(project.current_disk_number(), 137);
        assert_eq!(
            load(&fixture.0.join(JOURNAL), &opts)
                .unwrap()
                .completed
                .len(),
            136
        );
        let measured = benchmark::report(&project).unwrap();
        assert_eq!(measured.unique_committed_disks, 136);
        assert_eq!(measured.confirmed_insertions, 137);
        assert_eq!(measured.reported_physical_reads, 136);
        assert_eq!(measured.timed_physical_jobs, 136);
        assert_eq!(measured.status_counts["partial"], 12);
        assert_eq!(measured.status_counts["acquired"], 124);
        assert_eq!(measured.recovery_errors, 1);
        assert_eq!(measured.incomplete_sessions, 1);
        assert_eq!(measured.finished_sessions, 2);
        let (_, csv) = benchmark::export(&project, &measured).unwrap();
        assert_eq!(fs::read_to_string(csv).unwrap().lines().count(), 137);
        let after = run_with_io(
            &mut project,
            &opts,
            Cursor::new(b"READ 137\n"),
            &mut Vec::new(),
            no_read,
            |_, _| panic!("target already reached"),
            skipped,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&after.output).unwrap()["scanned"],
            0
        );
    }

    #[test]
    fn older_journals_remain_compatible_but_a_pending_end_target_cannot_change() {
        let fixture = Fixture::new();
        let project = fixture.project();
        let mut opts = options();
        let path = fixture.0.join(JOURNAL);
        let mut journal = load(&path, &opts).unwrap();
        let mut old = serde_json::to_value(&journal).unwrap();
        old.as_object_mut().unwrap().remove("last_disk");
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(load(&path, &opts).unwrap().last_disk, None);
        opts.last_disk = Some(136);
        journal.last_disk = Some(136);
        journal.pending = Some(Pending {
            disk: 1,
            result: Some(result(&project, 1, false)),
        });
        save(&path, &journal).unwrap();
        assert_eq!(load(&path, &opts).unwrap().last_disk, Some(136));
        opts.last_disk = Some(100);
        assert!(matches!(load(&path, &opts), Err(e) if e.contains("same --last-disk")));
    }
}
