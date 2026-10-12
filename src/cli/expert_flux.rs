//! Optional arbitrary GW definitions/selected tracks. Never automatic donors.
use super::{CliResponse, media_reservation::GreaseweazleReservation, resolve_project_root};
use crate::{
    external_tools::{self, ToolKind},
    greaseweazle::{
        GreaseweazleBackend, GreaseweazleCommand, GreaseweazleDeviceStatus,
        ProcessGreaseweazleBackend, classify_info_output,
    },
    project::ProjectState,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LIMIT: u64 = 128 * 1024 * 1024;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    crate::sector_inspection::regular(path, false)?;
    let resolved = path.canonicalize().map_err(|e| e.to_string())?;
    crate::sector_inspection::regular(&resolved, false)?;
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > limit {
        return Err("Expert source exceeds size bound".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Expert source grew beyond size bound".into());
    }
    Ok(bytes)
}
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
fn resolve(cwd: &Path, p: &str) -> PathBuf {
    let p = PathBuf::from(p);
    if p.is_absolute() { p } else { cwd.join(p) }
}

#[derive(Debug, Clone)]
struct Options {
    mode: String,
    disk: u32,
    format: String,
    definitions: Option<PathBuf>,
    input: Option<PathBuf>,
    expected: Option<u64>,
    cylinders: Vec<u32>,
    heads: String,
    step: u32,
    revs: u32,
    drive: String,
    protected: bool,
    json: bool,
    plan: bool,
    project: Option<PathBuf>,
}
fn cylinders(text: &str) -> Result<Vec<u32>, String> {
    if text.len() > 512 {
        return Err("Cylinder selection too long".into());
    }
    let mut out = BTreeSet::new();
    for part in text.split(',') {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let a = a
            .parse::<u32>()
            .map_err(|_| "Use cylinders 0..81 as numbers/ranges")?;
        let b = b
            .parse::<u32>()
            .map_err(|_| "Use cylinders 0..81 as numbers/ranges")?;
        if a > b || b > 81 {
            return Err("Cylinders must be within 0..81".into());
        }
        for n in a..=b {
            if !out.insert(n) {
                return Err("Duplicate cylinder selection".into());
            }
        }
    }
    if out.is_empty() {
        return Err("Empty cylinder selection".into());
    }
    Ok(out.into_iter().collect())
}
fn parse(args: &[String], cwd: &Path) -> Result<Options, String> {
    let mode = args
        .get(2)
        .filter(|s| matches!(s.as_str(), "capture" | "decode"))
        .ok_or("Use greaseweazle expert capture|decode N")?
        .clone();
    let disk = args
        .get(3)
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n > 0 && *n < u32::MAX)
        .ok_or("Expert command requires a positive disk label")?;
    let mut o = Options {
        mode,
        disk,
        format: String::new(),
        definitions: None,
        input: None,
        expected: None,
        cylinders: (0..80).collect(),
        heads: "0-1".into(),
        step: 1,
        revs: 3,
        drive: "B".into(),
        protected: false,
        json: false,
        plan: false,
        project: None,
    };
    let mut seen = BTreeSet::new();
    let mut i = 4;
    while i < args.len() {
        let key = args[i].as_str();
        if !seen.insert(key) {
            return Err("Duplicate expert option".into());
        }
        match key {
            "--json" => o.json = true,
            "--plan" => o.plan = true,
            "--source-write-protected" => o.protected = true,
            _ => {
                i += 1;
                let value = args.get(i).ok_or("Missing expert option value")?;
                match key {
                    "--format" => o.format = value.clone(),
                    "--diskdefs" => o.definitions = Some(resolve(cwd, value)),
                    "--input" => o.input = Some(resolve(cwd, value)),
                    "--project" => o.project = Some(resolve(cwd, value)),
                    "--expected-bytes" => {
                        o.expected = Some(
                            value
                                .parse::<u64>()
                                .ok()
                                .filter(|n| *n > 0 && *n <= 16 * 1024 * 1024)
                                .ok_or("Expected image size must be 1..16777216 bytes")?,
                        )
                    }
                    "--cylinders" => o.cylinders = cylinders(value)?,
                    "--heads" => {
                        o.heads = match value.as_str() {
                            "0" => "0",
                            "1" => "1",
                            "both" => "0-1",
                            _ => return Err("Heads must be 0, 1 or both".into()),
                        }
                        .into()
                    }
                    "--step" => {
                        o.step = value
                            .parse::<u32>()
                            .ok()
                            .filter(|n| matches!(n, 1 | 2))
                            .ok_or("Step must be 1 or 2")?
                    }
                    "--revs" => {
                        o.revs = value
                            .parse::<u32>()
                            .ok()
                            .filter(|n| (1..=10).contains(n))
                            .ok_or("Revolutions must be 1..10")?
                    }
                    "--gw-drive" => {
                        o.drive = match value.to_ascii_uppercase().as_str() {
                            "A" => "A",
                            "B" => "B",
                            _ => return Err("GW selector must be A or B".into()),
                        }
                        .into()
                    }
                    _ => return Err(format!("Unsupported expert option {key}")),
                }
            }
        }
        i += 1;
    }
    if o.format.is_empty()
        || o.format.len() > 80
        || !o
            .format
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        || !o.format.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(
            "Use a safe GW definition name (letters, digits, dot, underscore or hyphen)".into(),
        );
    }
    if o.mode == "capture" && (o.input.is_some() || o.expected.is_some()) {
        return Err("--input/--expected-bytes are offline-decode options".into());
    }
    if o.mode == "decode"
        && (o.input.is_none()
            || o.expected.is_none()
            || o.protected
            || seen.iter().any(|k| matches!(*k, "--gw-drive" | "--revs")))
    {
        return Err("Expert decode requires --input saved.scp and --expected-bytes N, not physical-drive options".into());
    }
    if o.cylinders.iter().any(|n| n * o.step > 81) {
        return Err(
            "Selection/step exceeds physical cylinder 81; choose 0-39 for double stepping".into(),
        );
    }
    if let Some(p) = &o.input
        && p.extension().and_then(|e| e.to_str()) != Some("scp")
    {
        return Err("Expert decode input must be a saved .scp file".into());
    }
    Ok(o)
}
fn command(
    o: &Options,
    output: &Path,
    definitions: Option<&Path>,
) -> Result<GreaseweazleCommand, String> {
    let mut a = vec![
        if o.mode == "capture" {
            "read"
        } else {
            "convert"
        }
        .into(),
        format!("--format={}", o.format),
        "--no-clobber".into(),
        format!(
            "--tracks=c={}:h={}:step={}",
            o.cylinders
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
            o.heads,
            o.step
        ),
    ];
    if let Some(p) = definitions {
        a.push(format!("--diskdefs={}", p.display()));
    }
    if o.mode == "capture" {
        a.extend([
            "--raw".into(),
            format!("--drive={}", o.drive),
            format!("--revs={}", o.revs),
            "--retries=0".into(),
            "--seek-retries=0".into(),
        ]);
    } else {
        a.push(o.input.as_ref().unwrap().display().to_string());
    }
    a.push(output.display().to_string());
    GreaseweazleCommand::from_raw_arguments(a)
}
pub(super) fn run(args: &[String], cwd: &Path) -> Result<CliResponse, String> {
    let o = parse(args, cwd)?;
    let defs = o
        .definitions
        .as_ref()
        .map(|p| read(p, 256 * 1024))
        .transpose()?;
    if let Some(b) = &defs {
        let s = std::str::from_utf8(b).map_err(|_| "Definitions require UTF-8")?;
        if s.lines()
            .any(|l| matches!(l.split_whitespace().next(), Some("import" | "include")))
        {
            return Err(
                "Use a self-contained diskdefs file; external includes/imports are not hash-bound"
                    .into(),
            );
        }
    }
    let source = o.input.as_ref().map(|p| read(p, LIMIT)).transpose()?;
    let mut record = json!({"schema_version":1,"disk":o.disk,"mode":o.mode,"format":o.format,
        "source_sha256":source.as_ref().map(|b|hash(b)),"diskdefs_sha256":defs.as_ref().map(|b|hash(b)),
        "source_media_access":if o.mode=="capture"{"read_only"}else{"none"},"physical_media_access":false, "automatic_donor_eligible":false,
        "quality":"unverified_expert_result","expected_image_bytes_asserted_by_operator":o.expected,
        "note":"Expert format/track interpretation. No complete acquisition map, independent CRC proof, native filesystem promise, custody advance or automatic donor promotion."});
    if o.plan {
        record["command"] = json!(
            command(
                &o,
                Path::new(if o.mode == "capture" {
                    "NEW.partial.scp"
                } else {
                    "NEW.partial.img"
                }),
                o.definitions.as_deref()
            )?
            .arguments()
        );
        record["plan"] = json!(true);
        return Ok(CliResponse {
            exit_code: 0,
            output: if o.json {
                record.to_string()
            } else {
                format!(
                    "Expert plan only; no tool, media or project writes.\n{}",
                    serde_json::to_string_pretty(&record).unwrap()
                )
            },
        });
    }
    if o.mode == "capture" && !o.protected {
        return Err("Expert physical capture requires --source-write-protected".into());
    }
    let root = resolve_project_root(cwd, o.project.as_deref())?;
    let project = ProjectState::open_without_session(root)?;
    crate::processing::validate_workspace(&project)?;
    let _budget =
        crate::resource_budget::foreground(&[(project.root(), 512 * crate::resource_budget::MIB)])?;
    let _owner = crate::project_work::reserve(project.root())?;
    let _device = if o.mode == "capture" {
        Some(GreaseweazleReservation::acquire()?)
    } else {
        None
    };
    let settings = project.tool_settings()?;
    let audit = project.logs_dir().join("external-tools.jsonl");
    let executable = if o.mode == "capture" {
        external_tools::find_ready_tool(
            ToolKind::Greaseweazle,
            settings.path(ToolKind::Greaseweazle),
            &audit,
        )?
    } else {
        external_tools::find_offline_greaseweazle(settings.path(ToolKind::Greaseweazle))?
    };
    let mut backend = if o.mode == "capture" {
        ProcessGreaseweazleBackend::new(executable, audit)?
    } else {
        ProcessGreaseweazleBackend::new_offline(executable, audit)?
    };
    backend.set_operation_timeout(Duration::from_secs(600));
    if o.mode == "capture" {
        let info = backend.execute(&GreaseweazleCommand::info())?;
        if !info.success
            || classify_info_output(&info.output_text()) != GreaseweazleDeviceStatus::Connected
        {
            return Err("Connected board not verified; no expert capture started".into());
        }
    }
    let flux = crate::flux_capture::project_flux_dir(&project)?;
    let parent = flux.join("Expert");
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    crate::sector_inspection::regular(&parent, true)?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    if parent.parent() != Some(flux.as_path()) {
        return Err("Expert directory escapes Flux".into());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = parent.join(format!(
        "{:03}_{}_{}_{}",
        o.disk,
        o.mode,
        nonce,
        std::process::id()
    ));
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let definitions = defs
        .as_ref()
        .map(|b| {
            let path = directory.join("diskdefs.cfg");
            write_new(&path, b)?;
            Ok::<_, String>(path)
        })
        .transpose()?;
    // Bind the host's actual input to the bytes we inspected, even if an
    // arbitrary external source is edited/restored during the host run (ABA).
    let input_snapshot = source
        .as_ref()
        .map(|b| {
            let path = directory.join("source.scp");
            write_new(&path, b)?;
            Ok::<_, String>(path)
        })
        .transpose()?;
    let mut invocation = o.clone();
    if let Some(path) = &input_snapshot {
        invocation.input = Some(path.clone());
    }
    let output = directory.join(if o.mode == "capture" {
        "evidence.partial.scp"
    } else {
        "evidence.partial.img"
    });
    let cmd = command(&invocation, &output, definitions.as_deref())?;
    record["original_input"] = json!(o.input);
    record["source_snapshot"] = json!(input_snapshot);
    record["command"] = json!(cmd.arguments());
    record["status"] = json!("started");
    write_new(
        &directory.join("intent.json"),
        &serde_json::to_vec_pretty(&record).unwrap(),
    )?;
    eprintln!(
        "{}: expert {} disk {:03}; bounded host run; no automatic catalog promotion",
        if o.mode == "capture" {
            "READ ONLY"
        } else {
            "OFFLINE"
        },
        o.mode,
        o.disk
    );
    let result = (|| {
        record["physical_media_access"] = json!(o.mode == "capture");
        let execution = backend.execute(&cmd)?;
        record["host_version"] = json!(execution.host_version);
        record["host_duration_ms"] = json!(execution.duration_ms);
        if !execution.success {
            return Err(format!("Expert host failed: {}", execution.output_text()));
        }
        let bytes = read(&output, LIMIT)?;
        if bytes.is_empty() || o.expected.is_some_and(|n| n != bytes.len() as u64) {
            return Err(
                "Expert output is empty or differs from asserted size; partial evidence kept"
                    .into(),
            );
        }
        if let Some(p) = &o.input
            && read(p, LIMIT)? != *source.as_ref().unwrap()
        {
            return Err("Expert raw source changed; output not committed".into());
        }
        if let Some(p) = &input_snapshot
            && read(p, LIMIT)? != *source.as_ref().unwrap()
        {
            return Err("Bound expert source snapshot changed; output not committed".into());
        }
        if let Some(p) = &definitions
            && read(p, 256 * 1024)? != *defs.as_ref().unwrap()
        {
            return Err("Bound definition changed; output not committed".into());
        }
        let final_path = directory.join(if o.mode == "capture" {
            "evidence.scp"
        } else {
            "evidence.img"
        });
        fs::hard_link(&output, &final_path).map_err(|e| e.to_string())?;
        record["output"] = json!(final_path);
        record["output_bytes"] = json!(bytes.len());
        record["output_sha256"] = json!(hash(&bytes));
        record["status"] = json!("complete_unverified");
        Ok::<_, String>(())
    })();
    if let Err(e) = &result {
        record["status"] = json!("failed_partial_retained");
        record["error"] = json!(e);
    }
    write_new(
        &directory.join("result.json"),
        &serde_json::to_vec_pretty(&record).unwrap(),
    )?;
    result?;
    // This exact, newly created partial is ours; committed bytes/result remain.
    fs::remove_file(&output)
        .map_err(|e| format!("Expert result committed, partial cleanup failed: {e}"))?;
    Ok(CliResponse {
        exit_code: 3,
        output: if o.json {
            record.to_string()
        } else {
            format!(
                "Expert result preserved with attention: {}\nDefinition/source/output hashes and command: {}\nNot automatically extracted or treated as clean donor sectors.",
                record["output"].as_str().unwrap_or(""),
                directory.join("result.json").display()
            )
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn args(rest: &[&str]) -> Vec<String> {
        ["greaseweazle", "expert", "capture", "1"]
            .into_iter()
            .chain(rest.iter().copied())
            .map(str::to_owned)
            .collect()
    }
    #[test]
    fn dynamic_formats_and_expert_track_settings_use_guarded_read_arrays() {
        let o = parse(
            &args(&[
                "--format",
                "acorn.adfs.640",
                "--cylinders",
                "0-39",
                "--heads",
                "0",
                "--step",
                "2",
            ]),
            Path::new("."),
        )
        .unwrap();
        let c = command(&o, Path::new("new.scp"), Some(Path::new("my defs.cfg"))).unwrap();
        assert_eq!(c.subcommand(), "read");
        assert!(c.arguments().iter().any(|a| a == "--raw"));
        assert!(c.arguments().iter().any(|a| a.ends_with(":h=0:step=2")));
        assert!(c.arguments().iter().any(|a| a == "--diskdefs=my defs.cfg"));
    }
    #[test]
    fn unsafe_formats_tracks_steps_flags_and_missing_decode_contract_refuse_before_tools() {
        for extra in [
            vec!["--format", "../evil"],
            vec!["--format", "--write"],
            vec!["--format", "ibm.1440", "--step", "2"],
            vec!["--format", "ibm.1440", "--cylinders", "0-82"],
            vec!["--format", "ibm.1440", "--cylinders", "0,0"],
            vec!["--format", "ibm.1440", "--heads", "2"],
            vec!["--format", "ibm.1440", "--write", "true"],
        ] {
            assert!(parse(&args(&extra), Path::new(".")).is_err());
        }
    }
    #[test]
    fn expert_plan_starts_no_project_board_or_writer() {
        let r = run(
            &args(&["--format", "amiga.amigados", "--plan", "--json"]),
            Path::new("."),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(r.exit_code, 0);
        assert_eq!(v["plan"], true);
        assert_eq!(v["automatic_donor_eligible"], false);
    }
}
