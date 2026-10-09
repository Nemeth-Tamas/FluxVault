//! Admission control, not OS CPU affinity, bandwidth throttling or a hard RAM limit.
//! Shared by queues within this controller process. Never wait while holding a
//! project snapshot/capture lock, or hold a permit while requesting another.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub(crate) const MIB: u64 = 1024 * 1024;
const MEMORY_HEADROOM: u64 = 1024 * MIB;
const DISK_HEADROOM: u64 = 512 * MIB;
const PRESSURE_TIMEOUT: Duration = Duration::from_secs(30);
const SLOT_TIMEOUT: Duration = Duration::from_secs(900);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Packing,
    Recovery,
    Office,
}

#[derive(Default)]
struct State {
    next: u64,
    foreground: usize,
    background: usize,
    bulk_io: usize,
    memory: u64,
    disks: BTreeMap<String, u64>,
    waiting: BTreeMap<u64, Kind>,
    admissions: u64,
    deferrals: u64,
    last_reason: Option<String>,
}

struct Governor {
    state: Mutex<State>,
    changed: Condvar,
}
impl Governor {
    fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        }
    }
}
fn governor() -> &'static Governor {
    static GOVERNOR: OnceLock<Governor> = OnceLock::new();
    GOVERNOR.get_or_init(Governor::new)
}

#[derive(Clone)]
struct Storage {
    volume: String,
    bytes: u64,
    available: u64,
}
struct Request {
    kind: Kind,
    memory: u64,
    storage: Vec<Storage>,
}
struct Host {
    cpus: usize,
    available_memory: u64,
}

fn slots(cpus: usize, foreground: usize) -> usize {
    cpus.saturating_sub(if foreground > 0 { 2 } else { 1 })
        .max(1)
}
fn storage_reason(state: &State, storage: &[Storage]) -> Option<String> {
    for item in storage {
        let needed = state
            .disks
            .get(&item.volume)
            .copied()
            .unwrap_or(0)
            .saturating_add(item.bytes)
            .saturating_add(DISK_HEADROOM);
        if item.available < needed {
            return Some(format!(
                "workstation storage pressure: {} needs {needed} bytes including reservations/headroom; {} available",
                item.volume, item.available
            ));
        }
    }
    None
}
fn pressure(state: &State, request: &Request, host: &Host) -> Option<String> {
    if host.available_memory
        < state
            .memory
            .saturating_add(request.memory)
            .saturating_add(MEMORY_HEADROOM)
    {
        return Some(format!(
            "RAM pressure: {} bytes available; {} estimated reserved plus {} requested and {} headroom",
            host.available_memory, state.memory, request.memory, MEMORY_HEADROOM
        ));
    }
    storage_reason(state, &request.storage)
}
fn slot_reason(state: &State, request: &Request, host: &Host, ticket: u64) -> Option<String> {
    if state.background >= slots(host.cpus, state.foreground) {
        return Some("shared background CPU slots busy; acquisition has priority".into());
    }
    if request.kind != Kind::Office && state.bulk_io >= 1 {
        return Some("saved-image/packing bulk-I/O slot busy".into());
    }
    if state
        .waiting
        .iter()
        .any(|(other, kind)| (*kind, *other) < (request.kind, ticket))
    {
        return Some("earlier/higher-priority background work waiting".into());
    }
    None
}

pub(crate) struct Permit<'a> {
    governor: &'a Governor,
    foreground: bool,
    memory: u64,
    bulk: bool,
    storage: Vec<Storage>,
}
fn reserve(state: &mut State, storage: &[Storage]) {
    for item in storage {
        *state.disks.entry(item.volume.clone()).or_default() += item.bytes;
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = self
            .governor
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.foreground {
            state.foreground -= 1;
        } else {
            state.background -= 1;
            state.memory -= self.memory;
            if self.bulk {
                state.bulk_io -= 1;
            }
        }
        for item in &self.storage {
            if let Some(bytes) = state.disks.get_mut(&item.volume) {
                *bytes -= item.bytes;
                if *bytes == 0 {
                    state.disks.remove(&item.volume);
                }
            }
        }
        drop(state);
        self.governor.changed.notify_all();
    }
}
struct Waiting<'a> {
    governor: &'a Governor,
    ticket: u64,
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.governor
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiting
            .remove(&self.ticket);
        self.governor.changed.notify_all();
    }
}

impl Governor {
    fn background<'a>(
        &'a self,
        kind: Kind,
        memory: u64,
        probe: impl Fn() -> Result<(Host, Vec<Storage>), String>,
        stage: &impl Fn(&str),
        pressure_timeout: Duration,
        slot_timeout: Duration,
    ) -> Result<Permit<'a>, String> {
        let ticket = {
            let mut state = self.state.lock().map_err(|e| e.to_string())?;
            let ticket = state.next;
            state.next = state
                .next
                .checked_add(1)
                .ok_or("Resource admission sequence exhausted")?;
            state.waiting.insert(ticket, kind);
            ticket
        };
        let waiting = Waiting {
            governor: self,
            ticket,
        };
        let started = Instant::now();
        let mut pressured_since = None;
        let mut notice = None;
        loop {
            crate::cancellation::check()?;
            let (host, storage) = probe()?;
            let request = Request {
                kind,
                memory,
                storage,
            };
            let mut state = self.state.lock().map_err(|e| e.to_string())?;
            let resource_reason = pressure(&state, &request, &host);
            if resource_reason.is_some() {
                pressured_since.get_or_insert_with(Instant::now);
            } else {
                pressured_since = None;
            }
            let reason = resource_reason.or_else(|| slot_reason(&state, &request, &host, ticket));
            if let Some(reason) = reason {
                state.deferrals += 1;
                state.last_reason = Some(reason.clone());
                let timed_out = pressured_since
                    .is_some_and(|time| time.elapsed() >= pressure_timeout)
                    || started.elapsed() >= slot_timeout;
                if timed_out {
                    return Err(format!(
                        "Resource admission deferred {kind:?}: {reason}. Evidence retained; free resources and resume saved processing/storage work."
                    ));
                }
                if notice.is_none_or(|time: Instant| time.elapsed() >= Duration::from_secs(5)) {
                    drop(state);
                    stage(&format!("Waiting for resources ({kind:?}): {reason}"));
                    notice = Some(Instant::now());
                    state = self.state.lock().map_err(|e| e.to_string())?;
                }
                let (guard, _) = self
                    .changed
                    .wait_timeout(state, Duration::from_millis(250))
                    .map_err(|e| e.to_string())?;
                drop(guard);
                continue;
            }
            state.waiting.remove(&ticket);
            state.background += 1;
            state.memory += memory;
            let bulk = kind != Kind::Office;
            if bulk {
                state.bulk_io += 1;
            }
            reserve(&mut state, &request.storage);
            state.admissions += 1;
            drop(state);
            drop(waiting);
            return Ok(Permit {
                governor: self,
                foreground: false,
                memory,
                bulk,
                storage: request.storage,
            });
        }
    }
    fn foreground(&self, storage: Vec<Storage>) -> Result<Permit<'_>, String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        if let Some(reason) = storage_reason(&state, &storage) {
            state.last_reason = Some(reason.clone());
            return Err(format!(
                "Read/decode not started: {reason}; no source media accessed by resource preflight"
            ));
        }
        state.foreground += 1;
        reserve(&mut state, &storage);
        drop(state);
        self.changed.notify_all();
        Ok(Permit {
            governor: self,
            foreground: true,
            memory: 0,
            bulk: false,
            storage,
        })
    }
}

// Inspect workstation destinations only, never A:/B: or raw source handles.
fn workstation_path(path: &Path) -> Result<std::path::PathBuf, String> {
    fn reject(path: &Path) -> Result<(), String> {
        let text = path
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_uppercase();
        let drive = text.strip_prefix("\\\\?\\").unwrap_or(&text);
        if !path.is_absolute()
            || drive.starts_with("A:")
            || drive.starts_with("B:")
            || text.starts_with("\\\\.\\")
            || text.starts_with("\\\\?\\GLOBALROOT")
            || text.starts_with("\\\\?\\VOLUME{")
            || text.starts_with("\\\\?\\UNC\\")
        {
            return Err("Resource preflight requires an absolute workstation destination, never a source device".into());
        }
        Ok(())
    }
    reject(path)?;
    let mut existing = path;
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or("No workstation destination ancestor")?;
            }
            Err(e) => return Err(format!("Cannot inspect workstation destination: {e}")),
        }
    }
    let canonical = existing
        .canonicalize()
        .map_err(|e| format!("Cannot resolve workstation destination: {e}"))?;
    reject(&canonical)?;
    Ok(canonical)
}

#[cfg(windows)]
fn disk(path: &Path, bytes: u64) -> Result<Storage, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            GetDiskFreeSpaceExW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
        },
        core::PCWSTR,
    };
    let path = workstation_path(path)?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut mount = vec![0u16; 32768];
    let mut volume = vec![0u16; 64];
    let mut available = 0;
    unsafe {
        GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut mount)
            .map_err(|e| format!("Cannot locate workstation volume: {e}"))?;
        GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None)
            .map_err(|e| format!("Cannot check workstation free space: {e}"))?;
        GetVolumeNameForVolumeMountPointW(PCWSTR(mount.as_ptr()), &mut volume)
            .map_err(|e| format!("Cannot identify workstation volume: {e}"))?;
    }
    let end = volume
        .iter()
        .position(|v| *v == 0)
        .ok_or("Invalid workstation volume name")?;
    Ok(Storage {
        volume: String::from_utf16_lossy(&volume[..end]).to_ascii_lowercase(),
        bytes,
        available,
    })
}
#[cfg(not(windows))]
fn disk(path: &Path, bytes: u64) -> Result<Storage, String> {
    workstation_path(path)?;
    Err(format!(
        "Resource free-space preflight is currently Windows-only ({} requested bytes)",
        bytes
    ))
}
fn storage(paths: &[(&Path, u64)]) -> Result<Vec<Storage>, String> {
    let mut volumes: BTreeMap<String, Storage> = BTreeMap::new();
    for (path, bytes) in paths {
        let item = disk(path, *bytes)?;
        if let Some(existing) = volumes.get_mut(&item.volume) {
            existing.bytes = existing
                .bytes
                .checked_add(item.bytes)
                .ok_or("Storage reservation overflow")?;
            existing.available = existing.available.min(item.available);
        } else {
            volumes.insert(item.volume.clone(), item);
        }
    }
    Ok(volumes.into_values().collect())
}
fn host() -> Result<Host, String> {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut memory = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut memory) }
            .map_err(|e| format!("Cannot inspect workstation RAM: {e}"))?;
        Ok(Host {
            cpus: std::thread::available_parallelism().map_or(1, usize::from),
            available_memory: memory.ullAvailPhys,
        })
    }
    #[cfg(not(windows))]
    {
        Err("Resource RAM preflight is currently Windows-only".into())
    }
}
pub(crate) fn background(
    kind: Kind,
    memory: u64,
    paths: &[(&Path, u64)],
    stage: &impl Fn(&str),
) -> Result<Permit<'static>, String> {
    governor().background(
        kind,
        memory,
        || Ok((host()?, storage(paths)?)),
        stage,
        PRESSURE_TIMEOUT,
        SLOT_TIMEOUT,
    )
}
pub(crate) fn foreground(paths: &[(&Path, u64)]) -> Result<Permit<'static>, String> {
    governor().foreground(storage(paths)?)
}
#[derive(Serialize)]
pub(crate) struct Snapshot {
    scope: &'static str,
    available_memory_bytes: Option<u64>,
    foreground_jobs: usize,
    background_jobs: usize,
    bulk_io_jobs: usize,
    waiting_jobs: usize,
    estimated_memory_reserved_bytes: u64,
    storage_reserved_bytes: u64,
    background_cpu_slots: usize,
    memory_headroom_bytes: u64,
    disk_headroom_bytes: u64,
    admissions: u64,
    deferral_samples: u64,
    last_deferral_reason: Option<String>,
}
pub(crate) fn snapshot() -> Snapshot {
    let available_memory_bytes = host().ok().map(|host| host.available_memory);
    let state = governor().state.lock().unwrap_or_else(|e| e.into_inner());
    Snapshot {
        scope: "controller-process admission estimates; not OS hard limits",
        available_memory_bytes,
        foreground_jobs: state.foreground,
        background_jobs: state.background,
        bulk_io_jobs: state.bulk_io,
        waiting_jobs: state.waiting.len(),
        estimated_memory_reserved_bytes: state.memory,
        storage_reserved_bytes: state.disks.values().copied().sum(),
        background_cpu_slots: slots(
            std::thread::available_parallelism().map_or(1, usize::from),
            state.foreground,
        ),
        memory_headroom_bytes: MEMORY_HEADROOM,
        disk_headroom_bytes: DISK_HEADROOM,
        admissions: state.admissions,
        deferral_samples: state.deferrals,
        last_deferral_reason: state.last_reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host_snapshot(cpus: usize) -> Host {
        Host {
            cpus,
            available_memory: 8 * 1024 * MIB,
        }
    }
    fn store(bytes: u64, available: u64) -> Storage {
        Storage {
            volume: "fixture-volume".into(),
            bytes,
            available,
        }
    }
    fn admit(g: &Governor, kind: Kind) -> Permit<'_> {
        g.background(
            kind,
            128 * MIB,
            || Ok((host_snapshot(4), vec![store(MIB, 10000 * MIB)])),
            &|_| {},
            Duration::from_millis(5),
            Duration::from_secs(1),
        )
        .unwrap()
    }
    #[test]
    fn foreground_never_waits_for_cpu_and_reduces_new_admission() {
        let g = Governor::new();
        let a = admit(&g, Kind::Office);
        let b = admit(&g, Kind::Office);
        let c = admit(&g, Kind::Office);
        let read = g.foreground(vec![store(MIB, 10000 * MIB)]).unwrap();
        let state = g.state.lock().unwrap();
        assert_eq!(slots(4, state.foreground), 2);
        assert!(
            slot_reason(
                &state,
                &Request {
                    kind: Kind::Office,
                    memory: 0,
                    storage: vec![]
                },
                &host_snapshot(4),
                9
            )
            .is_some()
        );
        drop(state);
        drop(read);
        drop((a, b, c));
        let state = g.state.lock().unwrap();
        assert_eq!(
            (state.foreground, state.background, state.memory),
            (0, 0, 0)
        );
        assert!(state.disks.is_empty());
    }
    #[test]
    fn physical_and_background_reservations_share_volume() {
        let g = Governor::new();
        let read = g.foreground(vec![store(200 * MIB, 1000 * MIB)]).unwrap();
        assert!(g.foreground(vec![store(400 * MIB, 1000 * MIB)]).is_err());
        drop(read);
        assert!(g.foreground(vec![store(400 * MIB, 1000 * MIB)]).is_ok());
    }
    #[test]
    fn ram_pressure_is_bounded_observable_and_releases_waiter() {
        let g = Governor::new();
        let notices = Mutex::new(Vec::new());
        assert!(
            g.background(
                Kind::Office,
                512 * MIB,
                || Ok((
                    Host {
                        cpus: 8,
                        available_memory: MIB
                    },
                    vec![]
                )),
                &|s| notices.lock().unwrap().push(s.to_owned()),
                Duration::from_millis(1),
                Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(!notices.lock().unwrap().is_empty());
        assert!(g.state.lock().unwrap().waiting.is_empty());
        assert_eq!(g.state.lock().unwrap().background, 0);
        drop(admit(&g, Kind::Office));
    }
    #[test]
    fn bulk_io_serializes_recovery_and_packing_not_office() {
        let g = Governor::new();
        let pack = admit(&g, Kind::Packing);
        let office = admit(&g, Kind::Office);
        let state = g.state.lock().unwrap();
        assert!(
            slot_reason(
                &state,
                &Request {
                    kind: Kind::Recovery,
                    memory: 0,
                    storage: vec![]
                },
                &host_snapshot(4),
                9
            )
            .is_some()
        );
        drop(state);
        drop((pack, office));
        assert_eq!(g.state.lock().unwrap().bulk_io, 0);
    }
    #[test]
    fn priority_and_fifo_are_deterministic() {
        let mut state = State::default();
        state.waiting.insert(1, Kind::Office);
        state.waiting.insert(2, Kind::Packing);
        let request = |kind| Request {
            kind,
            memory: 0,
            storage: vec![],
        };
        assert!(slot_reason(&state, &request(Kind::Office), &host_snapshot(8), 1).is_some());
        assert!(slot_reason(&state, &request(Kind::Packing), &host_snapshot(8), 2).is_none());
        state.waiting.insert(3, Kind::Packing);
        assert!(slot_reason(&state, &request(Kind::Packing), &host_snapshot(8), 3).is_some());
    }
    #[test]
    fn failed_probe_and_slot_timeout_leave_no_claims() {
        let g = Governor::new();
        assert!(
            g.background(
                Kind::Office,
                MIB,
                || Err("probe unavailable".into()),
                &|_| {},
                Duration::ZERO,
                Duration::ZERO
            )
            .is_err()
        );
        let pack = admit(&g, Kind::Packing);
        assert!(
            g.background(
                Kind::Recovery,
                MIB,
                || Ok((host_snapshot(4), vec![])),
                &|_| {},
                Duration::ZERO,
                Duration::ZERO
            )
            .is_err()
        );
        assert!(g.state.lock().unwrap().waiting.is_empty());
        drop(pack);
    }
    #[test]
    fn single_cpu_allows_one_background_job() {
        assert_eq!(slots(1, 0), 1);
        assert_eq!(slots(1, 2), 1);
    }
    #[test]
    fn source_device_paths_rejected_before_probe() {
        for name in [
            "A:\\Images",
            "B:\\Logs",
            "\\\\.\\A:",
            "\\\\?\\A:\\Images",
            "\\\\?\\Volume{fixture}\\Images",
            "\\\\?\\UNC\\server\\share\\Images",
            "relative",
        ] {
            assert!(workstation_path(Path::new(name)).is_err(), "{name}");
        }
    }
    #[cfg(windows)]
    #[test]
    fn same_volume_destinations_are_added_and_host_probes_work() {
        let root = std::env::temp_dir();
        let items =
            storage(&[(&root, 100), (&root.join("not-yet-created-fv-budget"), 200)]).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].bytes, 300);
        assert!(items[0].available > 0);
        assert!(host().unwrap().available_memory > 0);
    }
    #[test]
    fn permit_unwind_releases_claims() {
        let g = Governor::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = admit(&g, Kind::Office);
            panic!("fixture");
        }));
        let state = g.state.lock().unwrap();
        assert_eq!(state.background, 0);
        assert!(state.disks.is_empty());
    }

    #[test]
    fn pressure_clearing_admits_without_retry_or_leaked_waiter() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let g = Governor::new();
        let available = AtomicU64::new(MIB);
        let (notice, received) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                g.background(
                    Kind::Office,
                    512 * MIB,
                    || {
                        Ok((
                            Host {
                                cpus: 4,
                                available_memory: available.load(Ordering::Acquire),
                            },
                            vec![],
                        ))
                    },
                    &|_| {
                        // The observer is permitted to inspect the governor: callbacks
                        // must never execute while its mutex is held.
                        assert_eq!(g.state.lock().unwrap().waiting.len(), 1);
                        notice.send(()).unwrap();
                    },
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                )
                .unwrap()
            });
            received.recv_timeout(Duration::from_secs(2)).unwrap();
            available.store(8 * 1024 * MIB, Ordering::Release);
            g.changed.notify_all();
            drop(worker.join().unwrap());
        });
        let state = g.state.lock().unwrap();
        assert!(state.waiting.is_empty());
        assert_eq!(
            (state.admissions, state.background, state.memory),
            (1, 0, 0)
        );
    }

    #[test]
    fn concurrent_workers_obey_shared_limit_and_all_complete() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let g = Governor::new();
        let read = g.foreground(vec![]).unwrap();
        let running = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| {
                    let permit = admit(&g, Kind::Office);
                    let active = running.fetch_add(1, Ordering::AcqRel) + 1;
                    peak.fetch_max(active, Ordering::AcqRel);
                    std::thread::sleep(Duration::from_millis(10));
                    running.fetch_sub(1, Ordering::AcqRel);
                    drop(permit);
                    done.fetch_add(1, Ordering::AcqRel);
                });
            }
        });
        assert_eq!(done.load(Ordering::Acquire), 16);
        assert!(peak.load(Ordering::Acquire) <= 2);
        drop(read);
        let state = g.state.lock().unwrap();
        assert_eq!(
            (
                state.foreground,
                state.background,
                state.bulk_io,
                state.memory
            ),
            (0, 0, 0, 0)
        );
        assert!(state.waiting.is_empty());
        assert!(state.disks.is_empty());
    }

    #[test]
    fn observer_panic_releases_queued_claim() {
        let g = Governor::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = g.background(
                Kind::Office,
                MIB,
                || {
                    Ok((
                        Host {
                            cpus: 4,
                            available_memory: 0,
                        },
                        vec![],
                    ))
                },
                &|_| panic!("observer failed"),
                Duration::from_secs(1),
                Duration::from_secs(1),
            );
        }));
        assert!(g.state.lock().unwrap().waiting.is_empty());
        drop(admit(&g, Kind::Office));
    }
}
