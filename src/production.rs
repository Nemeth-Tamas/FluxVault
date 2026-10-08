//! First dual-station coordinator slice. No physical backend is launched here.
//! One owner serializes durable transitions; station work runs outside its lock.
use crate::{imaging, project::ProjectState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

const JOURNAL: &str = ".fluxvault-production.json";
const MAX_CONTROL: usize = 8 * 1024 * 1024;
const MAX_DISKS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Station {
    Usb,
    Greaseweazle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Work {
    Fresh,
    UsbRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub disk: u32,
    pub station: Station,
    pub work: Work,
    pub generation: u64,
    pub retry: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Reserved,
    Reading,
    Interrupted,
    Saved,
    AwaitGw,
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    station: Station,
    attempt: u32,
    image: String,
    sha256: String,
    metadata_sha256: String,
    log_sha256: String,
    sectors: usize,
    bad: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Disk {
    phase: Phase,
    ticket: Option<Ticket>,
    usb: Option<Receipt>,
    gw: Option<Receipt>,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    #[serde(default)]
    paused: bool,
    first: u32,
    last: Option<u32>,
    generation: u64,
    occupied: BTreeSet<u32>,
    disks: BTreeMap<u32, Disk>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Production evidence/control is a reparse point".into());
        }
    }
    if !metadata.file_type().is_file() || metadata.len() > limit as u64 {
        return Err("Production evidence/control is not a bounded regular file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Production evidence/control grew beyond its bound".into());
    }
    Ok(bytes)
}

fn next(j: &Journal) -> Option<u32> {
    let mut number = j.first;
    loop {
        if number == u32::MAX || j.last.is_some_and(|last| number > last) {
            return None;
        }
        if !j.occupied.contains(&number) && !j.disks.contains_key(&number) {
            return Some(number);
        }
        number = number.checked_add(1)?;
    }
}

fn validate(j: &Journal) -> Result<(), String> {
    if j.schema != 1
        || j.first == 0
        || j.first == u32::MAX
        || j.last.is_some_and(|n| n < j.first || n == u32::MAX)
        || j.disks.len() > MAX_DISKS
        || j.occupied.len() > MAX_DISKS
        || j.occupied.contains(&0)
        || j.occupied.contains(&u32::MAX)
    {
        return Err("Invalid/bounded production journal".into());
    }
    let mut held = BTreeSet::new();
    let mut generations = BTreeSet::new();
    for (number, disk) in &j.disks {
        if *number == 0 || *number == u32::MAX || j.last.is_some_and(|n| *number > n) {
            return Err("Invalid production disk identity".into());
        }
        let needs_ticket = matches!(
            disk.phase,
            Phase::Reserved | Phase::Reading | Phase::Interrupted | Phase::Saved
        );
        if needs_ticket != disk.ticket.is_some() {
            return Err("Production custody/ticket mismatch".into());
        }
        if let Some(t) = &disk.ticket {
            if t.disk != *number
                || t.generation == 0
                || t.generation > j.generation
                || !held.insert(t.station as u8)
                || !generations.insert(t.generation)
                || (t.work == Work::UsbRecovery
                    && (t.station != Station::Greaseweazle
                        || disk.usb.as_ref().is_none_or(|r| r.bad.is_empty())))
            {
                return Err("Duplicated or invalid production custody".into());
            }
            let other = match t.station {
                Station::Usb => &disk.gw,
                Station::Greaseweazle => &disk.usb,
            };
            if (t.work == Work::Fresh
                && (other.is_some()
                    || (disk.phase != Phase::Saved && (disk.usb.is_some() || disk.gw.is_some()))))
                || (t.work == Work::UsbRecovery && disk.phase != Phase::Saved && disk.gw.is_some())
            {
                return Err("Production ticket route disagrees with prior receipts".into());
            }
            if disk.phase == Phase::Saved
                && match t.station {
                    Station::Usb => disk.usb.is_none(),
                    Station::Greaseweazle => disk.gw.is_none(),
                }
            {
                return Err("Saved station lacks its evidence receipt".into());
            }
        }
        for (station, receipt) in [(Station::Usb, &disk.usb), (Station::Greaseweazle, &disk.gw)] {
            if let Some(r) = receipt {
                if r.station != station
                    || r.attempt == 0
                    || r.sectors == 0
                    || r.sectors > crate::fat12::MAX_IMAGE_BYTES / 512
                    || r.bad.len() > r.sectors
                    || r.bad.windows(2).any(|p| p[0] >= p[1])
                    || r.bad.iter().any(|l| *l >= r.sectors as u64)
                    || [
                        r.sha256.as_str(),
                        r.metadata_sha256.as_str(),
                        r.log_sha256.as_str(),
                    ]
                    .iter()
                    .any(|h| h.len() != 64 || !h.bytes().all(|c| c.is_ascii_hexdigit()))
                    || Path::new(&r.image).file_name().and_then(|s| s.to_str())
                        != Some(r.image.as_str())
                {
                    return Err("Invalid production evidence receipt".into());
                }
            }
        }
        let expected = if disk.gw.is_some() {
            if disk.gw.as_ref().unwrap().bad.is_empty() {
                Phase::Complete
            } else {
                Phase::Partial
            }
        } else if let Some(usb) = &disk.usb {
            if usb.bad.is_empty() {
                Phase::Complete
            } else {
                Phase::AwaitGw
            }
        } else {
            Phase::Reserved
        };
        if !needs_ticket && disk.phase != expected {
            return Err("Production routing disagrees with evidence".into());
        }
    }
    Ok(())
}

fn receipt(
    project: &ProjectState,
    disk: u32,
    attempt: u32,
    station: Station,
) -> Result<(Receipt, Vec<u8>), String> {
    let attempts = imaging::load_attempts_for_disk(&project.images_dir(), disk)?;
    let a = attempts
        .iter()
        .find(|a| a.attempt_number == attempt)
        .ok_or("Completed station attempt is missing")?;
    if !matches!(a.status.as_str(), "OK" | "PARTIAL") || a.metadata_path.as_os_str().is_empty() {
        return Err("Station completion requires an original completed acquisition, not DERIVED/legacy evidence".into());
    }
    let meta = read(&a.metadata_path, 1024 * 1024)?;
    let value: Value = serde_json::from_slice(&meta).map_err(|e| e.to_string())?;
    let backend = match station {
        Station::Usb => "windows-raw-sector",
        Station::Greaseweazle => "greaseweazle-derived",
    };
    if value["source_backend"] != backend
        || value["disk_number"] != disk
        || value["attempt_number"] != attempt
    {
        return Err("Completion came from the wrong disk/station backend".into());
    }
    let image_path =
        crate::recovery_plan::resolve_image_path(&project.images_dir(), &a.image_file)?;
    let image = read(&image_path, crate::fat12::MAX_IMAGE_BYTES)?;
    if !image.len().is_multiple_of(512) || digest(&image) != a.sha256 {
        return Err("Completed production image size/hash changed".into());
    }
    crate::fat12_recovery::validate_sector_evidence(a, image.len() / 512, &a.sha256)?;
    if a.parsed_log.as_ref().is_none_or(|log| {
        log.disk_number != Some(disk)
            || log.attempt_number != Some(attempt)
            || log.status.label() != a.status
    }) {
        return Err("Production log disk/attempt/status identity differs".into());
    }
    let log_path = Path::new(&a.log_file)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if log_path.parent()
        != Some(
            project
                .logs_dir()
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Production acquisition log escapes project".into());
    }
    let log = read(&log_path, MAX_CONTROL)?;
    let mut bad = a.bad_sectors.clone();
    bad.sort_unstable();
    let r = Receipt {
        station,
        attempt,
        image: image_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid image filename")?
            .to_owned(),
        sha256: a.sha256.clone(),
        metadata_sha256: digest(&meta),
        log_sha256: digest(&log),
        sectors: a.total_sectors,
        bad,
    };
    Ok((r, image))
}

fn consistent(old: &Receipt, usb: &[u8], r: &Receipt, bytes: &[u8]) -> Result<(), String> {
    if r.sectors != old.sectors {
        return Err("USB/GW source geometry differs; queue unchanged".into());
    }
    let old_bad = old.bad.iter().copied().collect::<BTreeSet<_>>();
    let gw_bad = r.bad.iter().copied().collect::<BTreeSet<_>>();
    let mut shared = 0;
    for lba in 0..r.sectors as u64 {
        if old_bad.contains(&lba) || gw_bad.contains(&lba) {
            continue;
        }
        shared += 1;
        let range = lba as usize * 512..(lba as usize + 1) * 512;
        if usb[range.clone()] != bytes[range] {
            return Err("USB/GW readable bytes disagree; check disk identity. Queue unchanged, evidence preserved".into());
        }
    }
    if shared == 0 {
        return Err("No mutually readable sectors establish cross-station consistency".into());
    }
    Ok(())
}

fn verify_receipts(project: &ProjectState, j: &Journal) -> Result<(), String> {
    for (number, disk) in &j.disks {
        let mut verified = Vec::new();
        for r in [&disk.usb, &disk.gw].into_iter().flatten() {
            let (checked, bytes) = receipt(project, *number, r.attempt, r.station)?;
            if checked != *r {
                return Err("Production evidence receipt changed; prior queue preserved".into());
            }
            verified.push((r, bytes));
        }
        if verified.len() == 2 {
            consistent(verified[0].0, &verified[0].1, verified[1].0, &verified[1].1)?;
        }
    }
    Ok(())
}

/// Backend-independent coordinator. It does not open drives, spawn readers, or
/// run downstream processing. Future adapters must preserve this single owner.
pub struct Coordinator {
    pub(crate) project: ProjectState,
    journal: Journal,
    _owner: Arc<File>,
    control_sha256: Option<String>,
}

impl Coordinator {
    pub fn open(project: ProjectState, last: Option<u32>, no_verify: bool) -> Result<Self, String> {
        if no_verify {
            return Err("Dual scan requires label verification; --no-verify cannot select earlier USB recovery disks".into());
        }
        crate::processing::validate_workspace(&project)?;
        let owner = crate::project_work::reserve(project.root())?;
        let path = project.root().join(JOURNAL);
        let control = if path.try_exists().map_err(|e| e.to_string())? {
            Some(read(&path, MAX_CONTROL)?)
        } else {
            None
        };
        let mut journal = if path.try_exists().map_err(|e| e.to_string())? {
            let j: Journal =
                serde_json::from_slice(&read(&path, MAX_CONTROL)?).map_err(|e| e.to_string())?;
            validate(&j)?;
            if j.last != last {
                return Err("Resume requires the same production endpoint".into());
            }
            verify_receipts(&project, &j)?;
            j
        } else {
            let stats = imaging::load_project_statistics(&project.images_dir())?;
            Journal {
                schema: 1,
                paused: false,
                first: project.current_disk_number(),
                last,
                generation: 0,
                occupied: stats.disks.iter().map(|d| d.disk_number).collect(),
                disks: BTreeMap::new(),
            }
        };
        // Never infer that a process restart means a physical swap happened.
        for disk in journal.disks.values_mut() {
            if matches!(disk.phase, Phase::Reserved | Phase::Reading) {
                disk.phase = Phase::Interrupted;
            }
        }
        // Other commands may have acquired labels while this coordinator was
        // stopped. Never offer those identities as unreserved new media.
        journal.occupied.extend(
            imaging::load_project_statistics(&project.images_dir())?
                .disks
                .iter()
                .map(|d| d.disk_number),
        );
        let mut coordinator = Self {
            project,
            journal: journal.clone(),
            _owner: Arc::new(owner),
            control_sha256: control.as_ref().map(|bytes| digest(bytes)),
        };
        coordinator.commit(journal)?;
        Ok(coordinator)
    }

    fn commit(&mut self, candidate: Journal) -> Result<(), String> {
        validate(&candidate)?;
        let bytes = serde_json::to_vec_pretty(&candidate).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_CONTROL {
            return Err("Production journal exceeds bound; previous queue retained".into());
        }
        let root = self.project.root();
        let destination = root.join(JOURNAL);
        if let Some(expected) = &self.control_sha256 {
            if digest(&read(&destination, MAX_CONTROL)?) != *expected {
                return Err(
                    "Production control was externally edited; queue commit refused".into(),
                );
            }
        } else if destination.try_exists().map_err(|e| e.to_string())? {
            return Err("Another production control appeared; queue commit refused".into());
        }
        let temp = root.join(format!(
            ".production-{}-{}.partial.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
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
        fs::rename(temp, root.join(JOURNAL)).map_err(|e| e.to_string())?;
        self.journal = candidate;
        self.control_sha256 = Some(digest(&bytes));
        Ok(())
    }

    pub fn next_fresh_disk(&self) -> Option<u32> {
        next(&self.journal)
    }

    pub fn paused(&self) -> bool {
        self.journal.paused
    }

    /// Pause authorizes no new physical reads. Existing workers may finish and
    /// publish their receipts; removal confirmations and downstream work remain
    /// usable. Persist the intent so a restart cannot silently undo it.
    pub fn set_paused(&mut self, paused: bool) -> Result<(), String> {
        let mut candidate = self.journal.clone();
        candidate.paused = paused;
        self.commit(candidate)
    }

    fn require_feeding(&self) -> Result<(), String> {
        if self.paused() {
            Err("Feeding is PAUSED; type RESUME before confirming any new physical read".into())
        } else {
            Ok(())
        }
    }

    pub(crate) fn owner(&self) -> Arc<File> {
        self._owner.clone()
    }

    pub(crate) fn held(&self, station: Station) -> Option<(Ticket, String)> {
        self.journal.disks.values().find_map(|d| {
            d.ticket
                .as_ref()
                .filter(|t| t.station == station)
                .map(|t| (t.clone(), format!("{:?}", d.phase).to_ascii_lowercase()))
        })
    }

    pub(crate) fn transfer_guard(&self, ticket: &Ticket) -> Result<Option<TransferGuard>, String> {
        let mut j = self.journal.clone();
        Self::current(&mut j, ticket, Phase::Reading)?;
        if ticket.work != Work::UsbRecovery {
            return Ok(None);
        }
        let usb = j.disks[&ticket.disk]
            .usb
            .clone()
            .ok_or("Missing transfer receipt")?;
        Ok(Some(TransferGuard {
            project: self.project.clone(),
            disk: ticket.disk,
            usb,
        }))
    }

    pub(crate) fn saved_attempts(&self) -> Vec<(u32, u32)> {
        self.journal
            .disks
            .iter()
            .flat_map(|(disk, d)| {
                [&d.usb, &d.gw]
                    .into_iter()
                    .flatten()
                    .map(move |r| (*disk, r.attempt))
            })
            .collect()
    }

    pub(crate) fn resume_cursor(&self) -> u32 {
        self.journal
            .disks
            .iter()
            .find(|(_, d)| {
                matches!(
                    d.phase,
                    Phase::Reserved | Phase::Reading | Phase::Interrupted
                )
            })
            .map(|(n, _)| *n)
            .or_else(|| self.next_fresh_disk())
            .unwrap_or_else(|| self.journal.last.map_or(self.journal.first, |n| n + 1))
    }

    /// A GUI/terminal must ask the operator for this ticket's exact label and
    /// protection assertion before confirm; claim alone never authorizes a read.
    pub fn claim(&mut self, station: Station, disk: u32) -> Result<Ticket, String> {
        self.require_feeding()?;
        if disk == 0 {
            return Err("Disk label must be positive".into());
        }
        let held = self
            .journal
            .disks
            .iter()
            .find(|(_, d)| d.ticket.as_ref().is_some_and(|t| t.station == station));
        let (work, retry) = if let Some((number, d)) = held {
            if *number != disk || d.phase != Phase::Interrupted {
                return Err("Station still holds a disk; finish/remove it or reconfirm its interrupted label".into());
            }
            (d.ticket.as_ref().unwrap().work, true)
        } else if let Some(d) = self.journal.disks.get(&disk) {
            if station != Station::Greaseweazle || d.phase != Phase::AwaitGw {
                return Err("That disk is not a queued USB partial available for GW".into());
            }
            (Work::UsbRecovery, false)
        } else {
            if self.next_fresh_disk() != Some(disk) {
                return Err("Fresh label is not the next unreserved disk; do not duplicate or skip identity".into());
            }
            (Work::Fresh, false)
        };
        let mut j = self.journal.clone();
        j.generation = j
            .generation
            .checked_add(1)
            .ok_or("Production ticket generation exhausted")?;
        let ticket = Ticket {
            disk,
            station,
            work,
            generation: j.generation,
            retry,
        };
        let d = j.disks.entry(disk).or_insert(Disk {
            phase: Phase::Reserved,
            ticket: None,
            usb: None,
            gw: None,
            error: None,
        });
        d.phase = Phase::Reserved;
        d.ticket = Some(ticket.clone());
        d.error = None;
        self.commit(j)?;
        Ok(ticket)
    }

    /// Replace an unread GW fresh-label offer with the queued USB number the
    /// operator actually inserted. A started/interrupted read cannot be abandoned
    /// through this convenience; no evidence-bearing record is removed.
    pub fn select_queued_instead(&mut self, offer: &Ticket, disk: u32) -> Result<Ticket, String> {
        self.require_feeding()?;
        let mut j = self.journal.clone();
        Self::current(&mut j, offer, Phase::Reserved)?;
        if offer.station != Station::Greaseweazle
            || offer.work != Work::Fresh
            || offer.retry
            || j.occupied.contains(&offer.disk)
            || !imaging::load_attempts_for_disk(&self.project.images_dir(), offer.disk)?.is_empty()
        {
            return Err("Only an unread fresh GW offer can switch to a queued USB label".into());
        }
        if j.disks.get(&disk).is_none_or(|d| d.phase != Phase::AwaitGw) {
            return Err("That earlier label is not an available USB recovery disk".into());
        }
        j.generation = j
            .generation
            .checked_add(1)
            .ok_or("Production ticket generation exhausted")?;
        let ticket = Ticket {
            disk,
            station: Station::Greaseweazle,
            work: Work::UsbRecovery,
            generation: j.generation,
            retry: false,
        };
        j.disks.remove(&offer.disk);
        let selected = j.disks.get_mut(&disk).unwrap();
        selected.phase = Phase::Reserved;
        selected.ticket = Some(ticket.clone());
        selected.error = None;
        self.commit(j)?;
        Ok(ticket)
    }

    fn current<'a>(
        j: &'a mut Journal,
        ticket: &Ticket,
        phase: Phase,
    ) -> Result<&'a mut Disk, String> {
        let disk = j
            .disks
            .get_mut(&ticket.disk)
            .ok_or("Unknown production ticket")?;
        if disk.ticket.as_ref() != Some(ticket) || disk.phase != phase {
            return Err("Stale, wrong-station or wrong-phase production ticket".into());
        }
        Ok(disk)
    }

    pub fn confirm(&mut self, ticket: &Ticket, label: &str, protected: bool) -> Result<(), String> {
        self.require_feeding()?;
        if !protected || label.trim().parse::<u32>().ok() != Some(ticket.disk) {
            return Err("Exact disk label and physical write-protection confirmation required; no read authorized".into());
        }
        if ticket.work == Work::UsbRecovery {
            let old = self
                .journal
                .disks
                .get(&ticket.disk)
                .and_then(|d| d.usb.as_ref())
                .ok_or("Missing USB transfer receipt")?;
            if receipt(&self.project, ticket.disk, old.attempt, Station::Usb)?.0 != *old {
                return Err(
                    "USB transfer evidence changed before confirmation; no read authorized".into(),
                );
            }
        }
        let mut j = self.journal.clone();
        Self::current(&mut j, ticket, Phase::Reserved)?.phase = Phase::Reading;
        self.commit(j)
    }

    pub fn failed(&mut self, ticket: &Ticket, detail: &str) -> Result<(), String> {
        if detail.is_empty() || detail.len() > 4096 {
            return Err("Invalid bounded production failure detail".into());
        }
        let mut j = self.journal.clone();
        let d = Self::current(&mut j, ticket, Phase::Reading)?;
        d.phase = Phase::Interrupted;
        d.error = Some(detail.to_owned());
        self.commit(j)
    }

    /// Only completed, hash/map/log-verified saved acquisitions enter routing.
    /// Hardware adapters must stage/validate cross-station identity BEFORE they
    /// expose new artifacts to downstream processing; this core launches none.
    pub fn complete(&mut self, ticket: &Ticket, attempt: u32) -> Result<(), String> {
        let mut j = self.journal.clone();
        Self::current(&mut j, ticket, Phase::Reading)?;
        let (r, bytes) = receipt(&self.project, ticket.disk, attempt, ticket.station)?;
        if ticket.work == Work::UsbRecovery {
            let old = j.disks[&ticket.disk]
                .usb
                .as_ref()
                .ok_or("USB recovery lacks its prior receipt")?;
            let (checked, usb) = receipt(&self.project, ticket.disk, old.attempt, Station::Usb)?;
            if &checked != old || r.sectors != old.sectors {
                return Err("USB/GW source binding or geometry differs; queue unchanged".into());
            }
            consistent(old, &usb, &r, &bytes)?;
        }
        let d = Self::current(&mut j, ticket, Phase::Reading)?;
        match ticket.station {
            Station::Usb => d.usb = Some(r),
            Station::Greaseweazle => d.gw = Some(r),
        }
        d.phase = Phase::Saved;
        self.commit(j)
    }

    /// Source cannot be claimed at GW while still held in USB. A station-client
    /// swap confirmation may combine old-disk removal and new-label assertion.
    pub fn removed(&mut self, ticket: &Ticket, confirmed: bool) -> Result<(), String> {
        if !confirmed {
            return Err("Physical removal/transfer confirmation required".into());
        }
        let mut j = self.journal.clone();
        let d = Self::current(&mut j, ticket, Phase::Saved)?;
        d.phase = match ticket.station {
            Station::Usb if !d.usb.as_ref().unwrap().bad.is_empty() => Phase::AwaitGw,
            Station::Usb => Phase::Complete,
            Station::Greaseweazle if !d.gw.as_ref().unwrap().bad.is_empty() => Phase::Partial,
            Station::Greaseweazle => Phase::Complete,
        };
        d.ticket = None;
        self.commit(j)
    }

    pub fn status(&self) -> Value {
        status_value(&self.journal)
    }
}

/// Immutable source binding passed to a physical worker. It never holds the
/// coordinator mutation lock across capture/decode or image publication.
pub(crate) struct TransferGuard {
    project: ProjectState,
    disk: u32,
    usb: Receipt,
}

impl TransferGuard {
    pub(crate) fn check(&self, bytes: &[u8], bad: &[u64]) -> Result<(), String> {
        if bytes.len() != self.usb.sectors * 512
            || bad.iter().any(|n| *n >= self.usb.sectors as u64)
        {
            return Err("USB/GW candidate geometry/map differs; no image published".into());
        }
        let (sealed, usb) = receipt(&self.project, self.disk, self.usb.attempt, Station::Usb)?;
        if sealed != self.usb {
            return Err("USB transfer evidence changed; no image published".into());
        }
        let mut candidate = self.usb.clone();
        candidate.bad = bad.to_vec();
        consistent(&self.usb, &usb, &candidate, bytes)
    }
}

fn status_value(j: &Journal) -> Value {
    json!({"schema":1,"paused":j.paused,"next_fresh_disk":next(j),"first":j.first,"last":j.last,
        "usb_recovery_queue":j.disks.iter().filter(|(_, d)| d.phase==Phase::AwaitGw).map(|(n, _)| *n).collect::<Vec<_>>(),
        // A saved partial still held in USB is transferable by gNNN, but not
        // yet in the removal-confirmed queue. Keep those two custody facts
        // distinct while making all actionable transfers visible.
        "usb_transfer_pending":j.disks.iter().filter(|(_, d)| d.phase==Phase::AwaitGw
            || (d.phase==Phase::Saved && d.ticket.as_ref().is_some_and(|t| t.station==Station::Usb)
                && d.usb.as_ref().is_some_and(|r| !r.bad.is_empty())))
            .map(|(n, _)| *n).collect::<Vec<_>>(),
        "disks":j.disks,"live_dual_adapter_ready":true,"physical_media_access":false})
}

/// Read-only inspection: no lock/control creation, backend probing or media read.
pub fn status(project: &ProjectState) -> Result<Value, String> {
    crate::processing::validate_workspace(project)?;
    let path = project.root().join(JOURNAL);
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(
            json!({"initialized":false,"live_dual_adapter_ready":true,"physical_media_access":false}),
        );
    }
    let j: Journal =
        serde_json::from_slice(&read(&path, MAX_CONTROL)?).map_err(|e| e.to_string())?;
    validate(&j)?;
    verify_receipts(project, &j)?;
    let mut value = status_value(&j);
    value["initialized"] = json!(true);
    value["coordinator_owner_active"] = json!(crate::project_work::active(project.root())?);
    // Ownership probe cannot identify the process or promise a current reader.
    value["requires_reconfirmation_if_owner_absent"] =
        json!(value["coordinator_owner_active"] == false);
    Ok(value)
}

pub(crate) fn preview(project: &ProjectState, last: Option<u32>) -> Result<Value, String> {
    crate::processing::validate_workspace(project)?;
    let stats = imaging::load_project_statistics(&project.images_dir())?;
    let existing = status(project)?;
    let mut j = if existing["initialized"] == true {
        let j: Journal = serde_json::from_slice(&read(&project.root().join(JOURNAL), MAX_CONTROL)?)
            .map_err(|e| e.to_string())?;
        validate(&j)?;
        verify_receipts(project, &j)?;
        if last.is_some() && last != j.last {
            return Err("Dual preview must use the saved production endpoint".into());
        }
        j
    } else {
        Journal {
            schema: 1,
            paused: false,
            first: project.current_disk_number(),
            last,
            generation: 0,
            occupied: BTreeSet::new(),
            disks: BTreeMap::new(),
        }
    };
    j.occupied.extend(stats.disks.iter().map(|d| d.disk_number));
    validate(&j)?;
    let mut value = status_value(&j);
    value["preview_only"] = json!(true);
    value["existing_coordinator"] = existing;
    value["stations"] = json!([
        "usb: fresh first-pass images; partials set aside for GW",
        "gw: fresh automatic scan/recovery, or typed queued USB label"
    ]);
    Ok(value)
}

#[cfg(test)]
#[path = "production_tests.rs"]
mod tests;
