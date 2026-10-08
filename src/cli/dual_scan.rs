//! One input/event pump, two physical workers, one durable project owner.
use super::{
    CliResponse,
    media_reservation::{GreaseweazleReservation, UsbReservation},
    terminal::{self, Cue},
};
use crate::{
    external_tools::{self, ToolKind},
    flux_recovery::{self, RecoveryResult},
    greaseweazle::ProcessGreaseweazleBackend,
    imaging::{self, ImagingEvent},
    production::{Coordinator, Station, Ticket, TransferGuard},
    project::ProjectState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, BufRead, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

const SETTINGS: &str = ".fluxvault-dual-settings.json";

pub(super) struct Options {
    pub usb: Option<String>,
    pub gw: Option<char>,
    pub last: Option<u32>,
    pub workers: usize,
    pub verified: bool,
    pub acquisition_only: bool,
    pub json: bool,
    pub color: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    schema: u32,
    usb: String,
    gw: char,
    last: Option<u32>,
}

fn settings(project: &ProjectState, options: &Options) -> Result<Settings, String> {
    if !options.verified {
        return Err("Dual USB scanning requires --write-blocker-verified; existing hardware/protection gates are unchanged".into());
    }
    if !(1..=16).contains(&options.workers) {
        return Err("Conversion workers must be 1-16".into());
    }
    let path = project.root().join(SETTINGS);
    let old = if path.try_exists().map_err(|e| e.to_string())? {
        let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err("Unsafe dual settings reparse point".into());
            }
        }
        if !m.is_file() || m.len() > 65536 {
            return Err("Invalid bounded dual settings".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Dual settings exceeded bound".into());
        }
        Some(serde_json::from_slice::<Settings>(&bytes).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let selected = Settings {
        schema: 1,
        usb: options
            .usb
            .as_deref()
            .map(|s| s.trim().to_ascii_uppercase())
            .unwrap_or_else(|| old.as_ref().map_or("A:".into(), |s| s.usb.clone())),
        gw: options
            .gw
            .unwrap_or_else(|| old.as_ref().map_or('B', |s| s.gw)),
        last: options.last.or_else(|| old.as_ref().and_then(|s| s.last)),
    };
    if selected.usb.len() != 2
        || !selected.usb.as_bytes()[0].is_ascii_uppercase()
        || !selected.usb.ends_with(':')
        || !matches!(selected.gw, 'A' | 'B')
        || selected.last.is_some_and(|n| n == 0 || n == u32::MAX)
        || old.as_ref().is_some_and(|s| *s != selected)
    {
        return Err(
            "Invalid/changed dual drive selectors or endpoint; use the same settings to resume"
                .into(),
        );
    }
    Ok(selected)
}

fn save_settings(project: &ProjectState, selected: &Settings) -> Result<(), String> {
    let path = project.root().join(SETTINGS);
    if path.exists() {
        return Ok(());
    }
    let temp = project.root().join(format!(
        ".dual-settings-{}.partial.json",
        std::process::id()
    ));
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    f.write_all(&serde_json::to_vec_pretty(selected).map_err(|e| e.to_string())?)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    crate::flux_recovery::publish_image_no_replace(&temp, &path)
}

struct ReadDone {
    attempt: u32,
    flux: Option<RecoveryResult>,
}
enum Event {
    Input(Option<String>),
    Progress(Station, u32, String),
    Done(Ticket, Result<ReadDone, String>),
}
type Reader = Arc<
    dyn Fn(Ticket, Option<TransferGuard>, SyncSender<Event>) -> Result<ReadDone, String>
        + Send
        + Sync,
>;

fn station_name(station: Station) -> &'static str {
    match station {
        Station::Usb => "USB",
        Station::Greaseweazle => "GW",
    }
}

fn usb_read(
    project: &ProjectState,
    ticket: &Ticket,
    drive: &str,
    events: &SyncSender<Event>,
) -> Result<ReadDone, String> {
    // Restart can adopt a completed, original acquisition left before receipt commit.
    if ticket.retry {
        let attempts = imaging::load_attempts_for_disk(&project.images_dir(), ticket.disk)?;
        if let Some(a) = attempts.iter().rev().find(|a| {
            !a.metadata_path.as_os_str().is_empty() && matches!(a.status.as_str(), "OK" | "PARTIAL")
        }) {
            let value: Value =
                serde_json::from_slice(&fs::read(&a.metadata_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if value["source_backend"] != "windows-raw-sector" {
                return Err(
                    "Interrupted USB identity contains a different backend; no reread started"
                        .into(),
                );
            }
            return Ok(ReadDone {
                attempt: a.attempt_number,
                flux: None,
            });
        }
    }
    let drives = crate::floppy::enumerate_removable_drives()?;
    let drive = super::select_removable_drive(&drives, drive)?;
    let geometry =
        super::acquire::acquisition_eligible(&crate::floppy::geometry_read_only(&drive)?)?;
    if geometry.bytes_per_sector != 512 {
        return Err("Dual scan currently supports 512-byte USB sectors only".into());
    }
    let receiver = imaging::start_imaging_publication(
        drive,
        geometry,
        project.images_dir(),
        project.logs_dir(),
        ticket.disk,
        0,
        Some(project.root().into()),
    );
    let mut bucket = 0;
    for event in receiver {
        match event {
            ImagingEvent::Progress { completed, total } if total > 0 => {
                let current = completed * 10 / total;
                if current > bucket {
                    bucket = current;
                    let _ = events.try_send(Event::Progress(
                        Station::Usb,
                        ticket.disk,
                        format!("{}% first pass", current * 10),
                    ));
                }
            }
            ImagingEvent::Completed(result) => {
                return Ok(ReadDone {
                    attempt: result.attempt_number,
                    flux: None,
                });
            }
            ImagingEvent::Failed(error) => return Err(error),
            _ => {}
        }
    }
    Err("USB worker stopped without a completed image; partial evidence retained".into())
}

fn gw_read(
    project: &ProjectState,
    ticket: &Ticket,
    guard: Option<TransferGuard>,
    drive: char,
    executable: &std::path::Path,
    events: &SyncSender<Event>,
) -> Result<ReadDone, String> {
    let mut backend = ProcessGreaseweazleBackend::new(
        executable.into(),
        project.logs_dir().join("external-tools.jsonl"),
    )?
    .with_stream_to_stderr(false);
    let tx = events.clone();
    let disk = ticket.disk;
    let mut meter = super::read_progress::PassMeter::default();
    backend.set_progress_callback(Some(Box::new(move |event| {
        if let Some(text) = meter.event(event) {
            let _ = tx.try_send(Event::Progress(Station::Greaseweazle, disk, text));
        }
    })));
    let result = flux_recovery::recover_auto_checked(
        project,
        disk,
        drive,
        Default::default(),
        &mut backend,
        &|s| {
            if let Some(text) = concise_stage(s) {
                let _ = events.try_send(Event::Progress(Station::Greaseweazle, disk, text));
            }
        },
        &|bytes, bad| guard.as_ref().map_or(Ok(()), |g| g.check(bytes, bad)),
    )?;
    flux_recovery::verify_completed_result(project, &result)?;
    if result.format_exception.is_some() {
        return Err("Raw-only format exception preserved; no sector image to route. USB can continue; stop and inspect this GW identity".into());
    }
    let attempt = result
        .image
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.rsplit('_').next())
        .and_then(|s| s.parse().ok())
        .ok_or("GW result lacks an acquisition attempt")?;
    Ok(ReadDone {
        attempt,
        flux: Some(result),
    })
}

fn concise_stage(message: &str) -> Option<String> {
    if let Some((stage, detail)) = message.split_once(" pass: ") {
        if detail.starts_with("reading") || detail.starts_with("rereading") {
            return Some(format!("{stage} read pass"));
        }
        return Some(format!(
            "{stage}: {}",
            detail.split(" [").next().unwrap_or(detail)
        ));
    }
    if message.starts_with("Identifying") {
        return Some("Checking disk format (offline)".into());
    }
    if let Some(rest) = message.strip_prefix("Detected ") {
        return Some(format!(
            "Format: {}",
            rest.split(':').next().unwrap_or(rest)
        ));
    }
    // Keep explicit stage-limit/stop/conflict diagnostics, without long evidence
    // paths. Full host output and command arguments remain in the durable audit.
    (!message.contains("Evidence:")).then(|| message.chars().take(140).collect())
}

enum Command {
    Read(Station, u32),
    Out(Station),
    Status,
    Pause,
    Resume,
    Quit,
}
fn command(text: &str) -> Result<Command, String> {
    let text = text.trim().to_ascii_uppercase();
    if matches!(text.as_str(), "QUIT" | "Q") {
        return Ok(Command::Quit);
    }
    if matches!(text.as_str(), "STATUS" | "S" | "?") {
        return Ok(Command::Status);
    }
    if matches!(text.as_str(), "PAUSE" | "P") {
        return Ok(Command::Pause);
    }
    if matches!(text.as_str(), "RESUME" | "R") {
        return Ok(Command::Resume);
    }
    let mut chars = text.chars();
    let station = match chars.next() {
        Some('U') => Station::Usb,
        Some('G') => Station::Greaseweazle,
        _ => {
            return Err(
                "Use u001 or g002 (check each label/open tab), u out / g out, STATUS, PAUSE, RESUME or QUIT"
                    .into(),
            );
        }
    };
    let suffix = chars.as_str().trim();
    if suffix == "OUT" {
        return Ok(Command::Out(station));
    }
    if !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Use digits after u/g, e.g. u1 / g2".into());
    }
    let disk = suffix.parse::<u32>().ok().filter(|n| *n > 0 && *n < u32::MAX).ok_or("Use a station plus exact positive disk number, e.g. u1 / g2; blank Enter does not read")?;
    Ok(Command::Read(station, disk))
}

fn begin(c: &mut Coordinator, station: Station, disk: u32) -> Result<Ticket, String> {
    // Check before implicitly confirming an old SAVED disk's removal.
    if c.paused() {
        return Err(
            "Feeding is PAUSED; type RESUME first. No custody changed and no read started".into(),
        );
    }
    let state = c.status();
    let queued = state["usb_recovery_queue"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_u64() == Some(disk as u64));
    let usb_saved = c.held(Station::Usb).filter(|(t, phase)| {
        t.disk == disk
            && phase == "saved"
            && state["disks"][disk.to_string()]["usb"]["bad"]
                .as_array()
                .is_some_and(|b| !b.is_empty())
    });
    let target_valid = c.next_fresh_disk() == Some(disk)
        || (station == Station::Greaseweazle && (queued || usb_saved.is_some()));
    if let Some((held, phase)) = c.held(station) {
        match phase.as_str() {
            "saved" if target_valid => c.removed(&held, true)?,
            "interrupted" if held.disk == disk => {}
            "reserved" if held.disk == disk => {
                c.confirm(&held, &disk.to_string(), true)?;
                return Ok(held);
            }
            _ => {
                return Err(format!(
                    "{} still holds {:03} ({phase}); finish it, confirm OUT when saved, or retry its interrupted exact label",
                    station_name(station),
                    held.disk
                ));
            }
        }
    }
    // gNNN on a USB saved partial explicitly asserts its physical transfer.
    if station == Station::Greaseweazle
        && let Some((old, _)) = usb_saved
    {
        c.removed(&old, true)?;
    }
    let ticket = c.claim(station, disk)?;
    c.confirm(&ticket, &disk.to_string(), true)?;
    Ok(ticket)
}

fn next_action(state: &Value, station: Station, held: Option<(u32, &str)>) -> String {
    let name = station_name(station);
    let prefix = if station == Station::Usb { "u" } else { "g" };
    let paused = state["paused"] == true;
    if paused && held.is_none_or(|(_, phase)| !matches!(phase, "saved" | "reading")) {
        return "PAUSED / no new reads; RESUME enables numbered confirmation, QUIT drains".into();
    }
    if let Some((disk, phase)) = held {
        match phase {
            "reading" => return format!("WAIT / DO NOT REMOVE {disk:03}"),
            "reserved" | "interrupted" => {
                return format!(
                    "CHECK/RESEAT SAME {disk:03}, open tab, then {prefix}{disk}; QUIT stops new reads"
                );
            }
            "saved" => {
                let followup = if paused {
                    "RESUME required before another read"
                } else {
                    "or feed NEXT FRESH"
                };
                let partial = station == Station::Usb
                    && state["usb_transfer_pending"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|n| n.as_u64() == Some(disk as u64)));
                return if partial {
                    format!(
                        "REMOVE {disk:03} / SET ASIDE FOR GW (g{disk} when GW free); {prefix} out confirms removal; {followup}"
                    )
                } else {
                    format!("REMOVE {disk:03}; {prefix} out confirms removal; {followup}")
                };
            }
            _ => {}
        }
    }
    let fresh = state["next_fresh_disk"].as_u64();
    let transfer = (station == Station::Greaseweazle)
        .then(|| state["usb_transfer_pending"].as_array()?.first()?.as_u64())
        .flatten();
    match (fresh, transfer) {
        (Some(n), Some(old)) => format!(
            "INSERT fresh {n:03} in {name} ({prefix}{n}), or MOVE USB partial {old:03} to GW (g{old}); open tab"
        ),
        (Some(n), None) => format!("INSERT fresh {n:03} in {name}, open tab, then {prefix}{n}"),
        (None, Some(old)) => format!("MOVE USB partial {old:03} to GW, open tab, then g{old}"),
        (None, None) => "NO FRESH DISKS / station ready".into(),
    }
}

fn held_in_state(state: &Value, station: Station) -> Option<(u32, &str)> {
    let station_key = match station {
        Station::Usb => "usb",
        Station::Greaseweazle => "greaseweazle",
    };
    state["disks"]
        .as_object()?
        .iter()
        .find_map(|(number, disk)| {
            if disk["ticket"]["station"] == station_key {
                Some((number.parse().ok()?, disk["phase"].as_str()?))
            } else {
                None
            }
        })
}

// Offline status never claims a recorded READING phase is an active reader.
pub(super) fn saved_status(state: &Value) -> String {
    if state["initialized"] != true {
        return "No dual scan started in this project. No physical media accessed.".into();
    }
    let mut lines = vec!["Dual coordinator (saved state; not a live reader probe)".into()];
    lines.push(format!(
        "Feeding: {}",
        if state["paused"] == true {
            "PAUSED (resume scan, then type RESUME)"
        } else {
            "enabled; each read still needs a numbered confirmation"
        }
    ));
    for station in [Station::Usb, Station::Greaseweazle] {
        let held = held_in_state(state, station);
        let text = held.map_or("ready".into(), |(n, p)| format!("{p} {n:03}"));
        lines.push(format!("{}: {text}", station_name(station)));
        if held.is_some_and(|(_, p)| p == "reading") {
            lines.push("  Recorded read: check the original console; if it ended, resume and reconfirm the same label. Do not move a potentially active disk.".into());
        } else {
            lines.push(format!("  ACTION: {}", next_action(state, station, held)));
        }
    }
    lines.push(format!("USB -> GW pending (including held saved partial): {}\nRemoval-confirmed queue: {}\nNo physical media accessed.", state["usb_transfer_pending"], state["usb_recovery_queue"]));
    lines.join("\n")
}

fn display(
    session: &Session,
    output: &mut impl Write,
    color: bool,
    draining: bool,
) -> Result<(), String> {
    let state = session.coordinator.status();
    if state["paused"] == true {
        writeln!(output, "FEEDING PAUSED / no new reads / RESUME or QUIT; active reads and saved-file work may finish").map_err(|e| e.to_string())?;
    }
    for station in [Station::Usb, Station::Greaseweazle] {
        let held = held_in_state(&state, station);
        let mut text = held.map_or("ready".into(), |(n, p)| format!("{p} {n:03}"));
        if let Some(start) = session.started.get(&(station as u8)) {
            text.push_str(&format!(" / {:.0}s elapsed", start.elapsed().as_secs_f64()));
        }
        if let Some(progress) = session.progress.get(&(station as u8)) {
            text.push_str(&format!(" / {progress}"));
        }
        writeln!(output, "{}: {text}", station_name(station)).map_err(|e| e.to_string())?;
        let action = if draining {
            if held.is_some_and(|(_, p)| p == "reading") {
                "DRAINING / WAIT / DO NOT REMOVE reading disk".into()
            } else {
                "DRAINING / no new reads; remove saved disks, resume later if work remains".into()
            }
        } else {
            next_action(&state, station, held)
        };
        let (escape, reset) = if color {
            ("\x1b[1;36m", "\x1b[0m")
        } else {
            ("", "")
        };
        writeln!(output, "{escape}  ACTION: {action}{reset}").map_err(|e| e.to_string())?;
    }
    writeln!(
        output,
        "NEXT FRESH: {}{}",
        session
            .coordinator
            .next_fresh_disk()
            .map_or("finished".into(), |n| format!("{n:03}")),
        if state["usb_transfer_pending"]
            .as_array()
            .is_some_and(|q| !q.is_empty())
        {
            format!(" | USB -> GW pending: {}", state["usb_transfer_pending"])
        } else {
            String::new()
        }
    )
    .map_err(|e| e.to_string())?;
    writeln!(output, "{}", session.pace.text(&state))
        .and_then(|_| output.flush())
        .map_err(|e| e.to_string())
}

struct Session {
    coordinator: Coordinator,
    workers: BTreeMap<u8, thread::JoinHandle<()>>,
    started: BTreeMap<u8, Instant>,
    progress: BTreeMap<u8, String>,
    pace: crate::dual_benchmark::Live,
    closing: Arc<AtomicBool>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::Release);
        for (_, worker) in std::mem::take(&mut self.workers) {
            let _ = worker.join();
        }
    }
}

fn deliver(tx: &SyncSender<Event>, mut event: Event, closing: &AtomicBool) {
    while !closing.load(Ordering::Acquire) {
        match tx.try_send(event) {
            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => return,
            Err(mpsc::TrySendError::Full(pending)) => {
                event = pending;
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn feed(
    mut session: Session,
    receiver: Receiver<Event>,
    tx: SyncSender<Event>,
    reader: Reader,
    processing: Option<&crate::processing::Queue>,
    packing: Option<&crate::flux_archive::Queue>,
    output: &mut impl Write,
    color: bool,
    mut telemetry: Option<&mut crate::benchmark::Session>,
) -> Result<Value, String> {
    let feeding_started = Instant::now();
    let mut draining = false;
    let mut errors = Vec::new();
    let mut completed = 0;
    let mut measurements = Vec::new();
    display(&session, output, color, draining)?;
    let mut refresh = Instant::now();
    loop {
        if draining && session.workers.is_empty() {
            break;
        }
        let event = match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !session.workers.is_empty() {
                    if refresh.elapsed() >= Duration::from_secs(10) {
                        display(&session, output, color, draining)?;
                        refresh = Instant::now();
                    }
                }
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        match event {
            Event::Input(None) => {
                draining = true;
                writeln!(
                    output,
                    "Input closed; finishing active reads, no new reads."
                )
                .map_err(|e| e.to_string())?;
            }
            Event::Input(Some(line)) if !draining => {
                let result = match command(&line) {
                    Ok(Command::Quit) => {
                        draining = true;
                        writeln!(output, "QUIT: finishing active bounded reads and saved-file work; do not remove a READING disk.").map_err(|e| e.to_string())?;
                        Ok(())
                    }
                    Ok(Command::Status) => display(&session, output, color, draining),
                    Ok(command @ (Command::Pause | Command::Resume)) => {
                        let paused = matches!(command, Command::Pause);
                        let result = session.coordinator.set_paused(paused);
                        if result.is_ok()
                            && let Some(log) = telemetry.as_deref_mut()
                        {
                            crate::dual_benchmark::record(
                                log,
                                if paused { "dual_pause" } else { "dual_resume" },
                                json!({}),
                            )?;
                        }
                        result
                            .and_then(|_| terminal::banner(output, color, Cue::Action,
                                if paused { "FEEDING PAUSED" } else { "FEEDING RESUMED / NO AUTOMATIC READ" },
                                if paused { "No new reads. Active reads finish; wait for SAVED before removal. Background files continue. RESUME or QUIT." }
                                    else { "Check the disk label and open tab, then type uN / gN. RESUME itself starts no read." }))
                            .and_then(|_| display(&session, output, color, draining))
                    }
                    Ok(Command::Out(station)) => {
                        let held = session
                            .coordinator
                            .held(station)
                            .ok_or("That station has no held disk".to_string());
                        let result = held.and_then(|(t, _)| {
                            session.coordinator.removed(&t, true)?;
                            Ok(t)
                        });
                        if let Ok(ticket) = &result
                            && let Some(log) = telemetry.as_deref_mut()
                        {
                            crate::dual_benchmark::record(
                                log,
                                "dual_removal_confirmed",
                                json!({"disk":ticket.disk,"station":station_name(station)}),
                            )?;
                        }
                        result.and_then(|_| display(&session, output, color, draining))
                    }
                    Ok(Command::Read(station, disk)) => {
                        match begin(&mut session.coordinator, station, disk) {
                            Err(e) => Err(e),
                            Ok(ticket) => {
                                let guard = session.coordinator.transfer_guard(&ticket)?;
                                if let Some(log) = telemetry.as_deref_mut() {
                                    crate::dual_benchmark::record(
                                        log,
                                        "dual_read_started",
                                        json!({"disk":ticket.disk,"station":station_name(ticket.station),"generation":ticket.generation,"work":ticket.work,"retry":ticket.retry}),
                                    )?;
                                }
                                let read = reader.clone();
                                let tx = tx.clone();
                                let closing = session.closing.clone();
                                let owner = session.coordinator.owner();
                                let t = ticket.clone();
                                let worker = thread::spawn(move || {
                                    let _owner = owner;
                                    let result = std::panic::catch_unwind(
                                        std::panic::AssertUnwindSafe(|| {
                                            read(t.clone(), guard, tx.clone())
                                        }),
                                    )
                                    .unwrap_or_else(|_| {
                                        Err("Physical worker panicked; evidence retained".into())
                                    });
                                    deliver(&tx, Event::Done(t, result), &closing);
                                });
                                session.workers.insert(station as u8, worker);
                                session.started.insert(station as u8, Instant::now());
                                session.progress.remove(&(station as u8));
                                writeln!(output, "[{} {disk:03}] READ ONLY started. Do not move this disk while READING.", station_name(station)).map_err(|e| e.to_string())?;
                                display(&session, output, color, draining)
                            }
                        }
                    }
                    Err(e) => Err(e),
                };
                if let Err(error) = result {
                    terminal::banner(output, color, Cue::Error, "NO NEW READ", &error)?;
                }
                // Commands already refreshed actions (or printed a refusal).
                // Do not immediately duplicate that display on the heartbeat.
                refresh = Instant::now();
            }
            Event::Input(_) => {}
            Event::Progress(station, disk, text) => {
                session
                    .progress
                    .insert(station as u8, text.chars().take(160).collect());
                if text.starts_with("gw warning:") || text.starts_with("gw error:") {
                    terminal::banner(
                        output,
                        color,
                        Cue::Error,
                        &format!("{} {disk:03}", station_name(station)),
                        &text,
                    )?;
                }
            }
            Event::Done(ticket, result) => {
                session.progress.remove(&(ticket.station as u8));
                let read_ms = session
                    .started
                    .remove(&(ticket.station as u8))
                    .map_or(0, |start| crate::benchmark::milliseconds(start.elapsed()));
                if let Some(worker) = session.workers.remove(&(ticket.station as u8)) {
                    let _ = worker.join();
                }
                let result = result.and_then(|done| {
                    session.coordinator.complete(&ticket, done.attempt)?;
                    Ok(done)
                });
                match result {
                    Ok(done) => {
                        completed += 1;
                        let attempts = imaging::load_attempts_for_disk(
                            &session.coordinator.project.images_dir(),
                            ticket.disk,
                        )?;
                        let a = attempts
                            .iter()
                            .find(|a| a.attempt_number == done.attempt)
                            .ok_or("Saved receipt vanished")?;
                        session
                            .pace
                            .saved(ticket.disk, ticket.work == crate::production::Work::Fresh);
                        if let Some(log) = telemetry.as_deref_mut() {
                            crate::dual_benchmark::record(
                                log,
                                "dual_receipt_saved",
                                json!({"disk":ticket.disk,"station":station_name(ticket.station),"generation":ticket.generation,
                                "attempt":done.attempt,"image_sha256":a.sha256,"bad_sectors":a.bad_sectors.len(),"read_decode_ms":read_ms,
                                "gw_physical_reads_reported":done.flux.as_ref().map(|f| f.physical_reads_this_run)}),
                            )?;
                        }
                        if let Some(queue) = processing
                            && let Err(e) =
                                queue.enqueue_attempt(ticket.disk, done.attempt, &a.sha256)
                        {
                            errors.push(e);
                        }
                        if let Some(queue) = packing
                            && let Some(flux) = done.flux
                        {
                            for attempt in flux.capture_attempts {
                                if let Err(e) = queue.enqueue(ticket.disk, attempt) {
                                    errors.push(e);
                                }
                            }
                        }
                        let partial = !a.bad_sectors.is_empty();
                        measurements.push(json!({"disk":ticket.disk,"station":station_name(ticket.station),"attempt":done.attempt,
                            "read_decode_ms":read_ms,"bad_sectors":a.bad_sectors.len(),"image_sha256":a.sha256}));
                        let detail = if ticket.station == Station::Usb && partial {
                            "SET ASIDE FOR GW, or transfer with gNNN. USB can continue with uNEXT."
                        } else {
                            "Remove this disk. Feed the next label, or confirm u out / g out. Saved files process in background."
                        };
                        terminal::banner(
                            output,
                            color,
                            if partial { Cue::Error } else { Cue::Success },
                            &format!(
                                "{} / {} SAVED {:03} / REMOVE {:03}",
                                station_name(ticket.station),
                                if partial { "PARTIAL" } else { "OK" },
                                ticket.disk,
                                ticket.disk
                            ),
                            &format!(
                                "{} missing sectors. Read/decode: {:.1}s. {detail}",
                                a.bad_sectors.len(),
                                read_ms as f64 / 1000.0
                            ),
                        )?;
                    }
                    Err(error) => {
                        session
                            .coordinator
                            .failed(&ticket, &error.chars().take(2000).collect::<String>())?;
                        if let Some(log) = telemetry.as_deref_mut() {
                            crate::dual_benchmark::record(
                                log,
                                "dual_read_failed",
                                json!({"disk":ticket.disk,"station":station_name(ticket.station),"generation":ticket.generation,
                                "read_decode_ms":read_ms,"error":error.chars().take(2000).collect::<String>()}),
                            )?;
                        }
                        terminal::banner(
                            output,
                            color,
                            Cue::Error,
                            &format!(
                                "{} {:03} NEEDS ATTENTION",
                                station_name(ticket.station),
                                ticket.disk
                            ),
                            &format!(
                                "{error}\nPhysical read stopped. Reseat/check SAME disk, then type {}{} to retry, or QUIT. Other station remains usable.",
                                if ticket.station == Station::Usb {
                                    "u"
                                } else {
                                    "g"
                                },
                                ticket.disk
                            ),
                        )?;
                        errors.push(error);
                    }
                }
                display(&session, output, color, draining)?;
                refresh = Instant::now();
            }
        }
        // Progress can arrive more often than recv_timeout: a busy reader must
        // not starve the recurring swap/action cue for the other station.
        if !session.workers.is_empty() && refresh.elapsed() >= Duration::from_secs(10) {
            display(&session, output, color, draining)?;
            refresh = Instant::now();
        }
        if !draining
            && session.coordinator.next_fresh_disk().is_none()
            && [Station::Usb, Station::Greaseweazle]
                .iter()
                .all(|s| session.coordinator.held(*s).is_none())
            && session.coordinator.status()["usb_recovery_queue"]
                .as_array()
                .unwrap()
                .is_empty()
        {
            draining = true;
            writeln!(
                output,
                "Requested range and removal confirmations finished; draining saved-file work."
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let mut project = session.coordinator.project.clone();
    project.set_current_disk_number_without_session(session.coordinator.resume_cursor())?;
    if let Some(log) = telemetry.as_deref_mut() {
        crate::dual_benchmark::record(
            log,
            "dual_feeding_finished",
            json!({"saved_results":completed,"next_disk":session.coordinator.resume_cursor()}),
        )?;
    }
    Ok(
        json!({"completed_this_session":completed,"feeding_elapsed_ms":crate::benchmark::milliseconds(feeding_started.elapsed()),"measurements":measurements,"state":session.coordinator.status(),"errors":errors,"source_media_access":"read_only"}),
    )
}

pub(super) fn run(project: ProjectState, options: Options) -> Result<CliResponse, String> {
    let run_started = Instant::now();
    crate::processing::validate_workspace(&project)?;
    let selected = settings(&project, &options)?;
    let coordinator = Coordinator::open(project.clone(), selected.last, false)?;
    let mut telemetry = crate::benchmark::Session::start_dual(
        &project,
        json!({
            "mode":"dual","project_root":project.root().canonicalize().map_err(|e|e.to_string())?,
            "selectors":selected,"conversion_workers":options.workers,"acquisition_only":options.acquisition_only,"paused":coordinator.paused()
        }),
    )?;
    let usb_reservation = UsbReservation::acquire(&selected.usb)?;
    let gw_reservation = GreaseweazleReservation::acquire()?;
    let tools = external_tools::load_settings()?;
    let audit = project.logs_dir().join("external-tools.jsonl");
    let gw = external_tools::find_ready_tool(
        ToolKind::Greaseweazle,
        tools.path(ToolKind::Greaseweazle),
        &audit,
    )?;
    let request = if options.acquisition_only {
        None
    } else {
        Some(crate::pipeline::PipelineRequest {
            project: project.clone(),
            seven_zip_executable: external_tools::find_ready_tool(
                ToolKind::SevenZip,
                tools.path(ToolKind::SevenZip),
                &audit,
            )?,
            libreoffice_executable: external_tools::find_ready_tool(
                ToolKind::LibreOffice,
                tools.path(ToolKind::LibreOffice),
                &audit,
            )?,
            command_audit_path: audit,
            conversion_workers: options.workers,
        })
    };
    save_settings(&project, &selected)?;
    let processing = request
        .as_ref()
        .map(|r| crate::processing::Queue::start_shared(r.clone(), coordinator.owner()))
        .transpose()?;
    if let Some(queue) = &processing {
        for (disk, attempt) in coordinator.saved_attempts() {
            let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk)?;
            let a = attempts
                .iter()
                .find(|a| a.attempt_number == attempt)
                .ok_or("Saved receipt missing")?;
            queue.enqueue_attempt(disk, attempt, &a.sha256)?;
        }
    }
    let packing = crate::flux_archive::Queue::start(&project)?;
    let (tx, receiver) = mpsc::sync_channel(64);
    let input_tx = tx.clone();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        loop {
            let mut bytes = Vec::new();
            match input.by_ref().take(4097).read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => {
                    let _ = input_tx.send(Event::Input(None));
                    break;
                }
                Ok(_) if bytes.len() > 4096 => {
                    let _ = input_tx.send(Event::Input(Some("INVALID_OVERSIZED_COMMAND".into())));
                    let _ = input_tx.send(Event::Input(None));
                    break;
                }
                Ok(_) => {
                    if input_tx
                        .send(Event::Input(Some(String::from_utf8_lossy(&bytes).into())))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    let read_project = project.clone();
    let read_settings = selected.clone();
    let reader: Reader = Arc::new(move |ticket, guard, events| {
        let _keep_devices = (&usb_reservation, &gw_reservation);
        match ticket.station {
            Station::Usb => usb_read(&read_project, &ticket, &read_settings.usb, &events),
            Station::Greaseweazle => gw_read(
                &read_project,
                &ticket,
                guard,
                read_settings.gw,
                &gw,
                &events,
            ),
        }
    });
    let mut stderr = io::stderr();
    terminal::banner(
        &mut stderr,
        options.color,
        Cue::Action,
        "DUAL READ-ONLY PILOT / EXACT LABELS REQUIRED",
        &format!(
            "USB {} / GW {}. Type u1 for USB 001, g2 for GW 002. Each command asserts the label and OPEN protection tab; replacing a SAVED disk asserts its removal. gNNN transfers a saved USB partial. STATUS / u out / g out / PAUSE / RESUME / QUIT. QUIT drains reads; never move a READING disk.",
            selected.usb, selected.gw
        ),
    )?;
    let session = Session {
        coordinator,
        workers: BTreeMap::new(),
        started: BTreeMap::new(),
        progress: BTreeMap::new(),
        pace: crate::dual_benchmark::Live::new(),
        closing: Arc::new(AtomicBool::new(false)),
    };
    // Keep the owner across final processing too, even after the producer ends.
    let owner = session.coordinator.owner();
    let result = feed(
        session,
        receiver,
        tx,
        reader,
        processing.as_ref(),
        Some(&packing),
        &mut stderr,
        options.color,
        Some(&mut telemetry),
    );
    let outcome = processing.map(|q| q.finish());
    let storage_errors = packing.finish();
    let mut value = result?;
    value["background_processing"] = json!(outcome);
    value["capture_storage_errors"] = json!(storage_errors);
    if let Some(request) = request {
        match crate::pipeline::run_pipeline_incremental(&request, &|s| eprintln!("[FILES] {s}")) {
            Ok(final_result) => {
                let summary = crate::processing::summary(&final_result);
                crate::processing::record_final(&project, &summary)?;
                value["processing"] = summary;
            }
            Err(e) => value["processing"] = json!({"error":e,"exit_code":3}),
        }
    }
    value["report_schema"] = json!(3);
    value["session_elapsed_ms"] = json!(crate::benchmark::milliseconds(run_started.elapsed()));
    crate::dual_benchmark::record(
        &mut telemetry,
        "dual_session_finished",
        json!({"completed_this_session":value["completed_this_session"],
        "processing_exit_code":value["processing"]["exit_code"],
        "background_error_count":value["background_processing"]["errors"].as_array().map_or(0,|a|a.len()),
        "storage_error_count":value["capture_storage_errors"].as_array().map_or(0,|a|a.len())}),
    )?;
    value["dual_telemetry"] = json!(telemetry.path());
    value["dual_benchmark"] = crate::dual_benchmark::report(&project)?;
    let (summary, csv) = crate::dual_benchmark::export(&project, &value["dual_benchmark"])?;
    value["dual_benchmark_export"] = json!({"summary":summary,"receipts_csv":csv});
    let reports = project
        .reports_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if reports.parent()
        != Some(
            project
                .root()
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Dual measurement directory escapes project".into());
    }
    let report = reports.join(format!(
        "DualScan-{}-{}.json",
        external_tools::current_unix_ms(),
        std::process::id()
    ));
    let pending_report = report.with_extension("partial.json");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending_report)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    crate::flux_recovery::publish_image_no_replace(&pending_report, &report)?;
    value["dual_run_report"] = json!(report);
    drop(owner);
    let state = &value["state"];
    let attention = !value["errors"].as_array().unwrap().is_empty()
        || !value["capture_storage_errors"]
            .as_array()
            .unwrap()
            .is_empty()
        || value["background_processing"]["errors"]
            .as_array()
            .is_some_and(|e| !e.is_empty())
        || value["processing"]["exit_code"]
            .as_i64()
            .is_some_and(|n| n != 0)
        || !state["usb_recovery_queue"].as_array().unwrap().is_empty()
        || state["disks"]
            .as_object()
            .unwrap()
            .values()
            .any(|d| d["phase"] != "complete");
    Ok(CliResponse {
        output: if options.json {
            value.to_string()
        } else {
            format!(
                "Dual scan stopped: {} station results saved. USB/GW state and queue are durable. {}\nUse fv production status, fv processing status, or resume fv scan --double --write-blocker-verified.\nDual measurements: {}",
                value["completed_this_session"],
                if attention {
                    "Attention remains (not customer-delivery certification)."
                } else {
                    "Acquisition results complete; inspect reports before delivery."
                },
                report.display()
            )
        },
        exit_code: if attention { 3 } else { 0 },
    })
}

#[cfg(test)]
#[path = "dual_scan_tests.rs"]
mod tests;
