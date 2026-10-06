//! Guided Greaseweazle custody loop. Only an explicit numbered confirmation can read media.

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

use super::{CliResponse, flux, media_reservation::GreaseweazleReservation};

const JOURNAL: &str = ".fluxvault-gw-scan.json";
const LOCK: &str = ".fluxvault-gw-scan.lock";

pub(super) struct ScanOptions {
    pub profile: GreaseweazleProfile,
    pub profile_map: BTreeMap<u32, String>,
    pub drive: char,
    pub protected: bool,
    pub policy: RecoveryPolicy,
    pub count: Option<usize>,
    pub last_disk: Option<u32>,
    pub acquisition_only: bool,
    pub json_output: bool,
}

impl ScanOptions {
    fn profile_for(&self, disk: u32) -> Result<GreaseweazleProfile, String> {
        self.profile_map
            .get(&disk)
            .map_or(Ok(self.profile), |name| GreaseweazleProfile::parse(name))
    }

    fn validate(&self) -> Result<(), String> {
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
    profile_map: BTreeMap<u32, String>,
    drive: char,
    policy: RecoveryPolicy,
    #[serde(default)]
    last_disk: Option<u32>,
    pending: Option<Pending>,
    completed: Vec<RecoveryResult>,
}

pub(super) struct SavedDefaults {
    pub profile: GreaseweazleProfile,
    pub profile_map: BTreeMap<u32, String>,
    pub drive: char,
    pub policy: RecoveryPolicy,
    pub last_disk: Option<u32>,
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
    Ok(Some(SavedDefaults {
        profile: GreaseweazleProfile::parse(&journal.profile)?,
        profile_map: journal.profile_map,
        drive: journal.drive,
        policy: journal.policy,
        last_disk: journal.last_disk,
    }))
}

pub(super) fn run(mut project: ProjectState, options: ScanOptions) -> Result<CliResponse, String> {
    options.validate()?;
    crate::flux_capture::project_flux_dir(&project)?;
    let reservation = GreaseweazleReservation::acquire()?;
    let settings = crate::external_tools::load_settings()?;
    let audit = project.logs_dir().join("external-tools.jsonl");
    preflight_downstream(options.acquisition_only, |tool| {
        eprintln!(
            "Before feeding disks: checking {} for saved-file processing...",
            tool.display_name()
        );
        crate::external_tools::find_ready_tool(tool, settings.path(tool), &audit).map(|_| ())
    })?;
    let stdin = io::stdin();
    let mut stderr = io::stderr();
    run_with_io(
        &mut project,
        &options,
        stdin.lock(),
        &mut stderr,
        |project, disk| {
            let response = flux::recover_reserved(
                project,
                flux::RecoveryOptions {
                    disk,
                    profile: options.profile_for(disk)?,
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
        flux_recovery::verify_completed_result,
        flux::process_saved,
    )
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
            profile_map: options.profile_map.clone(),
            drive: options.drive,
            policy: options.policy.clone(),
            last_disk: options.last_disk,
            pending: None,
            completed: Vec::new(),
        }
    };
    validate_profiles(&journal.profile_map)?;
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
    journal.profile_map = options.profile_map.clone();
    journal.drive = options.drive;
    journal.policy = options.policy.clone();
    journal.last_disk = options.last_disk;
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
        "profile_map":options.profile_map,
        "drive":options.drive,"policy":options.policy,"count":options.count,
        "acquisition_only":options.acquisition_only,"start_disk":project.current_disk_number(),
        "last_disk":options.last_disk}),
    )?;
    let mut results = Vec::new();
    let mut resumed_advances = 0usize;
    let mut waiting: Option<(u32, Instant)> = None;
    loop {
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
            writeln!(output,
                "GW {}: insert floppy {disk:03}, check its write-protect hole is OPEN. Type {disk:03} to confirm and read it, or QUIT: [format {}]", options.drive, profile.argument()
            ).map_err(|e| e.to_string())?;
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
            if number != Some(disk) {
                writeln!(
                    output,
                    "No read started. Type the displayed disk number {disk:03}, or QUIT."
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
            journal.pending = Some(Pending { disk, result: None });
            save(&path, &journal)?; // Custody persists before any physical operation.
            telemetry.record(
                "read_confirmed",
                json!({"disk":disk,"profile":profile.argument(),
                "operator_wait_ms":benchmark::milliseconds(waiting.take().unwrap().1.elapsed())}),
            )?;
            let started = Instant::now();
            match recover_disk(project, disk) {
                Ok(result) => {
                    recovery_elapsed_ms = Some(benchmark::milliseconds(started.elapsed()));
                    result
                }
                Err(error) => {
                    telemetry.record(
                        "recovery_failed",
                        json!({"disk":disk,"phase":"acquisition",
                        "elapsed_ms":benchmark::milliseconds(started.elapsed()),"error":error}),
                    )?;
                    return Err(error);
                }
            }
        };
        if result.disk != disk
            || !matches!(
                result.status.as_str(),
                "acquired" | "partial" | "unrecoverable_within_policy"
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
            return Err(error);
        }
        if let Some(elapsed_ms) = recovery_elapsed_ms {
            telemetry.record("recovery_finished", json!({"disk":disk,"profile":profile.argument(),"elapsed_ms":elapsed_ms,
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
            json!({"disk":disk,"profile":profile.argument(),"next_disk":next,
            "numbering_resumed":numbering_resumed,"outcome":benchmark::outcome(&result)}),
        )?;
        writeln!(output,
            "GW SWAP: disk {disk:03} saved ({}; {} missing, {} conflicting). Remove it. Next disk: {next:03}.",
            result.status, result.missing_lbas.len(), result.conflicting_lbas.len()
        ).map_err(|e| e.to_string())?;
        results.push(result);
    }
    let mut attention =
        journal.pending.is_some() || journal.completed.iter().any(|r| r.status != "acquired");
    let processing = if options.acquisition_only || journal.completed.is_empty() {
        json!({"skipped":true})
    } else {
        writeln!(output, "Physical work finished. Remove the floppy; processing saved files, conversions, audit and workbook...")
            .map_err(|e| e.to_string())?;
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
    telemetry.finish(results.len(), project.current_disk_number())?;
    let measured = benchmark::report(project)?;
    let (benchmark_json, benchmark_csv) = benchmark::export(project, &measured)?;
    Ok(CliResponse {
        output: if options.json_output {
            json!({"project":root,"scanned":results.len(),"partial_this_session":partial,
                "next_disk":project.current_disk_number(),"total_scanned":journal.completed.len(),
                "resumed_advances":resumed_advances,"pending_disk":journal.pending.as_ref().map(|p| p.disk),
                "disks":results,"processing":processing,
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
                "Greaseweazle scan stopped: {} disk(s) saved, {partial} partial this session. Next: {:03}. Total: {}.\nDownstream: {downstream}\nPilot benchmark: {}\nPer-disk CSV: {}",
                results.len(),
                project.current_disk_number(),
                journal.completed.len(),
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
            profile_map: BTreeMap::new(),
            drive: 'B',
            protected: true,
            policy: RecoveryPolicy::default(),
            count: None,
            last_disk: None,
            acquisition_only: true,
            json_output: true,
        }
    }
    fn result(project: &ProjectState, disk: u32, partial: bool) -> RecoveryResult {
        RecoveryResult {
            disk,
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
