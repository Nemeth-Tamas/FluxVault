//! USB feeding shares sealed production custody and the saved-file queue.
use super::{
    CliResponse, acquire,
    audible::{self, Cues, Station},
    terminal::{self, Cue},
};
use crate::{
    production::{Coordinator, Station as PhysicalStation, Ticket},
    project::ProjectState,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, Write},
};

pub(super) struct Options {
    pub json: bool,
    pub drive: String,
    pub retries: usize,
    pub count: Option<usize>,
    pub last: Option<u32>,
    pub protected: bool,
    pub workers: usize,
    pub acquisition_only: bool,
    pub sound: bool,
    pub color: bool,
}
impl Options {
    fn validate(&self) -> Result<(), String> {
        if !self.protected {
            return Err("Scan is blocked until the drive/write blocker has been independently verified with a known-good disposable disk. Do not validate using customer media.".into());
        }
        if self.count == Some(0)
            || self.retries > 10
            || !(1..=16).contains(&self.workers)
            || self.last.is_some_and(|n| n == 0 || n == u32::MAX)
        {
            return Err("Invalid USB scan count, endpoint, retry cap or worker count".into());
        }
        Ok(())
    }
}

pub(super) fn run(project: &mut ProjectState, options: Options) -> Result<CliResponse, String> {
    options.validate()?;
    crate::processing::validate_workspace(project)?;
    let _control = crate::run_control::Session::start(project, "usb_scan")?;
    let _device = super::media_reservation::UsbReservation::acquire(&options.drive)?;
    // Preflight before insertion; acquisition-only explicitly opts out.
    let request = if options.acquisition_only {
        None
    } else {
        let settings = project.tool_settings()?;
        let audit = project.logs_dir().join("external-tools.jsonl");
        let mut paths = Vec::new();
        for kind in [
            crate::external_tools::ToolKind::SevenZip,
            crate::external_tools::ToolKind::LibreOffice,
        ] {
            eprintln!(
                "Before feeding USB disks: checking {}...",
                kind.display_name()
            );
            paths.push(crate::external_tools::find_ready_tool(
                kind,
                settings.path(kind),
                &audit,
            )?);
        }
        Some(crate::pipeline::PipelineRequest {
            project: project.clone(),
            seven_zip_executable: paths.remove(0),
            libreoffice_executable: paths.remove(0),
            command_audit_path: audit,
            conversion_workers: options.workers,
        })
    };
    let last = options.last.or(crate::production::status(project)?["last"]
        .as_u64()
        .map(|n| n as u32));
    let mut coordinator = Coordinator::open(project.clone(), last, false)?;
    if coordinator.held(PhysicalStation::Greaseweazle).is_some() {
        return Err(
            "GW has recorded custody; resume dual mode to resolve it before USB-only feeding"
                .into(),
        );
    }
    let queue = request
        .as_ref()
        .map(|r| crate::processing::Queue::start_shared(r.clone(), coordinator.owner()))
        .transpose()?;
    if let Some(queue) = &queue {
        for (disk, attempt) in coordinator.saved_attempts() {
            enqueue(project, queue, disk, attempt)?;
        }
        eprintln!(
            "USB BACKGROUND PROCESSING ON / swap cues do not wait for conversions. `fv processing status` shows work."
        );
    }
    let mut response = feed(
        project,
        &mut coordinator,
        &options,
        crate::run_control::Input::stdin(),
        &mut io::stderr(),
        |p, t| {
            if t.retry {
                let attempts = crate::imaging::load_attempts_for_disk(&p.images_dir(), t.disk)?;
                if let Some(a) = attempts
                    .iter()
                    .rev()
                    .find(|a| !a.legacy_image && matches!(a.status.as_str(), "OK" | "PARTIAL"))
                {
                    let meta: Value = serde_json::from_slice(
                        &fs::read(&a.metadata_path).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    if meta["source_backend"] != "windows-raw-sector" {
                        return Err(
                            "Interrupted USB label has another backend; no reread started".into(),
                        );
                    }
                    return Ok(a.attempt_number); // complete() verifies all artifacts.
                }
            }
            acquire::image_reserved(p, &options.drive, t.disk, options.retries)
                .map(|r| r.attempt_number)
        },
        |p, disk, attempt| {
            if let Some(q) = &queue {
                enqueue(p, q, disk, attempt)?;
            }
            Ok(())
        },
    )?;
    let background = queue.map(|q| q.finish());
    let mut value: Value = serde_json::from_str(&response.output).map_err(|e| e.to_string())?;
    if let Some(request) = &request
        && !coordinator.saved_attempts().is_empty()
    {
        eprintln!(
            "USB FEEDING FINISHED / offline recovery, audit and reconciliation; no insertion requested."
        );
        let result = crate::pipeline::run_pipeline_incremental(request, &|s| eprintln!("{s}"))?;
        let summary = crate::processing::summary(&result);
        crate::processing::record_final(project, &summary)?;
        if summary["exit_code"] != 0 {
            response.exit_code = 3;
        }
        value["processing"] = summary;
    }
    if let Some(background) = background {
        if !background.errors.is_empty() {
            response.exit_code = 3;
        }
        value["background_processing"] =
            serde_json::to_value(background).map_err(|e| e.to_string())?;
    }
    response.output = if options.json {
        value.to_string()
    } else {
        format!(
            "USB scan stopped: {} saved this session. Next disk: {:03}. Pending GW transfers: {}.\nSaved custody/removal stays explicit; `fv status` shows the next safe action. Files process automatically unless --acquisition-only was selected.",
            value["scanned"],
            project.current_disk_number(),
            value["recovery_queue"]
        )
    };
    Ok(response)
}

fn enqueue(
    project: &ProjectState,
    queue: &crate::processing::Queue,
    disk: u32,
    attempt: u32,
) -> Result<(), String> {
    let attempts = crate::imaging::load_attempts_for_disk(&project.images_dir(), disk)?;
    let source = attempts
        .iter()
        .find(|a| a.attempt_number == attempt)
        .ok_or("Sealed USB acquisition missing")?;
    queue.enqueue_attempt(disk, attempt, &source.sha256)
}

fn sync_cursor(project: &mut ProjectState, coordinator: &Coordinator) -> Result<(), String> {
    let _snapshot = crate::project_work::snapshot(project.root())?;
    *project = ProjectState::open_without_session(project.root().to_owned())?;
    let next = coordinator.resume_cursor();
    let held = coordinator.held(PhysicalStation::Usb).map(|(t, _)| t.disk);
    if project.current_disk_number() != next && held != Some(project.current_disk_number()) {
        return Err("Project cursor was externally changed; sealed USB evidence retained, no cursor overwritten".into());
    }
    if project.current_disk_number() != next {
        project.set_current_disk_number_without_session(next)?;
    }
    Ok(())
}

// Boundary injects physical read/publication callbacks without touching media in tests.
#[allow(clippy::too_many_arguments)]
fn feed<R: BufRead, W: Write, F, E>(
    project: &mut ProjectState,
    coordinator: &mut Coordinator,
    options: &Options,
    mut input: R,
    output: &mut W,
    mut read: F,
    mut enqueued: E,
) -> Result<CliResponse, String>
where
    F: FnMut(&ProjectState, &Ticket) -> Result<u32, String>,
    E: FnMut(&ProjectState, u32, u32) -> Result<(), String>,
{
    options.validate()?;
    let cues = Cues::start(options.sound);
    let mut scanned = 0;
    let mut partial = 0;
    loop {
        crate::cancellation::check()?;
        if let Some((ticket, phase)) = coordinator.held(PhysicalStation::Usb)
            && phase == "saved"
        {
            let state = coordinator.status();
            let attempt = state["disks"][ticket.disk.to_string()]["usb"]["attempt"]
                .as_u64()
                .ok_or("Saved USB receipt missing")? as u32;
            enqueued(project, ticket.disk, attempt)?;
            sync_cursor(project, coordinator)?;
        }
        if options.count.is_some_and(|n| scanned >= n) {
            break;
        }
        let held = coordinator.held(PhysicalStation::Usb);
        let pending = held
            .as_ref()
            .filter(|(_, p)| p != "saved")
            .map(|(t, _)| t.disk);
        let offered = pending.or_else(|| coordinator.next_fresh_disk());
        if offered.is_none() && held.is_none() {
            break;
        }
        let removal = held
            .as_ref()
            .filter(|(_, p)| p == "saved")
            .map(|(t, _)| t.disk);
        if let Some(disk) = offered {
            writeln!(output, "Insert floppy {disk:03} in USB {} with its write-protect hole OPEN. Type {disk:03} to confirm its label (legacy READ accepted), OUT after prior removal, PAUSE/RESUME, STATUS, QUIT to drain, or STOP:", options.drive).map_err(|e|e.to_string())?;
            if let Some(old) = removal {
                writeln!(output, "Confirming {disk:03} also confirms REMOVE {old:03}; partial disks must be set aside for GW.").map_err(|e|e.to_string())?;
            }
        } else {
            writeln!(output, "USB BATCH FINISHED / REMOVE {:03}. Type OUT after removal, or QUIT (custody stays saved). No next insertion.", removal.unwrap()).map_err(|e|e.to_string())?;
        }
        output.flush().map_err(|e| e.to_string())?;
        let mut answer = String::new();
        if input.read_line(&mut answer).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let answer = answer.trim().to_ascii_uppercase();
        match answer.as_str() {
            "Q" | "QUIT" => break,
            "STOP" => {
                crate::cancellation::current().request();
                crate::cancellation::check()?;
            }
            "STATUS" => {
                writeln!(output, "{}", coordinator.status()).map_err(|e| e.to_string())?;
                continue;
            }
            "PAUSE" | "RESUME" => {
                coordinator.set_paused(answer == "PAUSE")?;
                writeln!(output,"USB feeding {}; saved-file processing continues. STATUS, OUT, RESUME or QUIT remain available.",if coordinator.paused(){"paused"}else{"resumed"}).map_err(|e|e.to_string())?;
                continue;
            }
            "OUT" => {
                if let Some((t, p)) = &held
                    && p == "saved"
                {
                    coordinator.removed(t, true)?;
                } else {
                    writeln!(output, "No saved USB disk can be removed; do not move interrupted/reading media until stopped.").map_err(|e|e.to_string())?;
                }
                continue;
            }
            _ => {}
        }
        let Some(disk) =
            offered.filter(|n| answer == "READ" || answer.parse::<u32>().ok() == Some(*n))
        else {
            writeln!(
                output,
                "No read started. Confirm the displayed exact label or QUIT."
            )
            .map_err(|e| e.to_string())?;
            continue;
        };
        if coordinator.paused() {
            writeln!(
                output,
                "Feeding is paused. Type RESUME before confirming the next label."
            )
            .map_err(|e| e.to_string())?;
            continue;
        }
        if let Some((t, p)) = &held
            && p == "saved"
        {
            coordinator.removed(t, true)?;
        }
        let ticket = coordinator.claim(PhysicalStation::Usb, disk)?;
        coordinator.confirm(&ticket, &disk.to_string(), true)?;
        let attempt = match read(project, &ticket) {
            Ok(a) => a,
            Err(e) => {
                coordinator.failed(&ticket, &e.chars().take(1000).collect::<String>())?;
                terminal::banner(
                    output,
                    options.color,
                    Cue::Error,
                    &format!("USB READ FAILED {disk:03} / NUMBER NOT ADVANCED"),
                    "Evidence retained. Resume/reconfirm this SAME label; wait for STOPPED and drive idle on cancellation.",
                )?;
                if !crate::cancellation::requested() {
                    cues.notify(Station::Usb, audible::Outcome::Failed);
                }
                return Err(e);
            }
        };
        if let Err(error) = coordinator.complete(&ticket, attempt) {
            coordinator.failed(&ticket, &error.chars().take(1000).collect::<String>())?;
            return Err(format!(
                "USB evidence verification failed; label {disk:03} NOT advanced: {error}"
            ));
        }
        enqueued(project, disk, attempt)?; // Durable handoff BEFORE numbering.
        sync_cursor(project, coordinator)?;
        let state = coordinator.status();
        let bad = state["disks"][disk.to_string()]["usb"]["bad"]
            .as_array()
            .ok_or("USB bad-sector map missing")?
            .len();
        partial += usize::from(bad > 0);
        scanned += 1;
        terminal::banner(
            output,
            options.color,
            if bad == 0 { Cue::Success } else { Cue::Error },
            &format!(
                "USB / {} SAVED {disk:03} / REMOVE {disk:03}",
                if bad == 0 { "OK" } else { "PARTIAL" }
            ),
            if bad == 0 {
                "Durable image/log/map/metadata verified. Safe to swap after the written cue."
            } else {
                "Saved partial / SET ASIDE FOR GW. USB may continue. Transfer queue is durable; exact label required at GW."
            },
        )?;
        cues.notify(Station::Usb, audible::saved_outcome(Station::Usb, bad > 0));
    }
    let state = coordinator.status();
    let queue = state["usb_transfer_pending"].clone();
    let attention = partial > 0 || queue.as_array().is_some_and(|a| !a.is_empty());
    Ok(CliResponse { output: json!({"project":project.root(),"scanned":scanned,"partial_this_session":partial,
        "next_disk":project.current_disk_number(),"recovery_queue":queue,"custody":state,
        "retry_policy":{"maximum_usb_retry_passes":options.retries,"first_retry":"backward","second_retry":"forward","default":"speed_first_to_gw","missing_bytes":"image_only_zero_fill_with_explicit_map"},
        "source_media_access":"read_only"}).to_string(), exit_code: if attention {3} else {0} })
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
