//! Stable disk identity and operator annotations over immutable acquisition
//! records. Inspection never runs recovery, refreshes reports, or touches media.
use crate::{
    extraction::{self, ExtractionPresence},
    imaging,
    project::ProjectState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    schema_version: u32,
    disk: u32,
    text: String,
    updated_unix_ms: u64,
}

fn valid_disk(disk: u32) -> Result<(), String> {
    if disk == 0 || disk == u32::MAX {
        return Err("Disk number must be positive and leave room for the next label".into());
    }
    Ok(())
}

fn regular(path: &Path, limit: u64) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > limit {
        return Err("Disk inspection requires bounded regular records".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("Disk record is a reparse point".into());
        }
    }
    Ok(())
}

fn read_json(path: &Path, limit: u64) -> Result<Value, String> {
    regular(path, limit)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Disk inspection record grew beyond its bound".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn notes_dir(project: &ProjectState) -> Result<PathBuf, String> {
    let directory = project.reports_dir().join("DiskNotes");
    if directory.try_exists().map_err(|e| e.to_string())?
        && directory
            .canonicalize()
            .map_err(|e| e.to_string())?
            .parent()
            != Some(
                project
                    .reports_dir()
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .as_path(),
            )
    {
        return Err("Disk notes directory escapes Reports".into());
    }
    Ok(directory)
}

pub fn note(project: &ProjectState, disk: u32) -> Result<Value, String> {
    valid_disk(disk)?;
    crate::processing::validate_workspace(project)?;
    let path = notes_dir(project)?.join(format!("{disk:03}.json"));
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(json!({"text":"","updated_unix_ms":null}));
    }
    let value = read_json(&path, 8192)?;
    let record: Note = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if record.schema_version != 1
        || record.disk != disk
        || record.text.len() > 4096
        || record.text.chars().any(|c| c.is_control())
    {
        return Err("Invalid disk note identity/text".into());
    }
    Ok(value)
}

/// Explicit optional annotation only. It never renumbers acquisitions or alters
/// custody. Earlier notes are retained before atomic replacement of the cursor.
pub fn set_note(project: &ProjectState, disk: u32, text: &str) -> Result<Value, String> {
    valid_disk(disk)?;
    if text.len() > 4096 || text.chars().any(|c| c.is_control()) {
        return Err(
            "Disk note must be at most 4096 UTF-8 bytes, without control characters".into(),
        );
    }
    crate::processing::validate_workspace(project)?;
    let _owner = crate::project_work::reserve(project.root())?;
    let _snapshot = crate::project_work::snapshot(project.root())?;
    let previous = note(project, disk)?;
    let directory = notes_dir(project)?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = notes_dir(project)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let partial = directory.join(format!(
        ".{disk:03}-{}-{stamp}.partial.json",
        std::process::id()
    ));
    let record = Note {
        schema_version: 1,
        disk,
        text: text.into(),
        updated_unix_ms: crate::external_tools::current_unix_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
    write_new(&partial, &bytes)?;
    let path = directory.join(format!("{disk:03}.json"));
    if path.try_exists().map_err(|e| e.to_string())? {
        // Validate again so a changed/malformed old note is never overwritten.
        if note(project, disk)? != previous {
            return Err("Disk note changed during publication".into());
        }
        write_new(
            &directory.join(format!(
                "{disk:03}-{}-{stamp}.history.json",
                std::process::id()
            )),
            &serde_json::to_vec_pretty(&previous).map_err(|e| e.to_string())?,
        )?;
    }
    fs::rename(&partial, &path)
        .map_err(|e| format!("Cannot commit disk note (partial retained): {e}"))?;
    note(project, disk)
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

pub fn inspect(
    project: &ProjectState,
    disk: u32,
    attempts: &[imaging::AttemptSummary],
) -> Result<Value, String> {
    valid_disk(disk)?;
    crate::processing::validate_workspace(project)?;
    let preferred =
        imaging::best_attempt(attempts).ok_or("No saved image attempts for this disk")?;
    let label = format!("{disk:03}");
    let extraction = match extraction::inspect_extraction_presence(
        &project.extracted_dir(),
        disk,
        preferred.attempt_number,
    )? {
        ExtractionPresence::Missing { expected_directory } => {
            json!({"state":"missing","directory":expected_directory})
        }
        ExtractionPresence::ManualRecovery {
            output_directory,
            file_count,
            total_bytes,
        } => {
            json!({"state":"manual_or_legacy","directory":output_directory,"files":file_count,"bytes":total_bytes,"provenance_verified":false})
        }
        ExtractionPresence::InvalidAutomatic {
            output_directory,
            detail,
        } => json!({"state":"invalid","directory":output_directory,"detail":detail}),
        ExtractionPresence::Automatic {
            output_directory,
            file_count,
            total_bytes,
            source_sha256,
            recovery_attention,
        } => {
            let binding = source_sha256 == preferred.sha256;
            json!({"state":if binding {"automatic"}else{"stale_source"},"directory":output_directory,
                "files":file_count,"bytes":total_bytes,"source_sha256":source_sha256,"matches_preferred_image":binding,
                "recovery_attention":recovery_attention,"fresh_integrity_audit_performed":false})
        }
    };
    let processing = crate::processing::status(project)?;
    let jobs = processing["jobs"]
        .as_array()
        .map(|jobs| {
            jobs.iter()
                .filter(|j| j["disk"] == disk)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let audit_path = project.reports_dir().join("EvidenceAudit.json");
    let audit = if audit_path.try_exists().map_err(|e| e.to_string())? {
        let report = read_json(&audit_path, 32 * 1024 * 1024)?;
        if report["schema_version"] != 1 || report["project"] != project.name() {
            return Err("Saved audit schema/project identity differs".into());
        }
        let matching = report["disks"]
            .as_array()
            .ok_or("Missing audit disk records")?
            .iter()
            .filter(|r| r["disk"] == label)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err("Duplicate audit disk identities".into());
        }
        let row = matching.first().copied().cloned().unwrap_or(Value::Null);
        json!({"record":row,"matches_preferred_image":row["attempt"]==preferred.attempt_number && row["image_sha256"]==preferred.sha256,
            "historical_report_only":true,"fresh_integrity_audit_performed":false,"path":audit_path})
    } else {
        json!({"record":null,"historical_report_only":true})
    };
    let conversion_path = crate::conversion_run::snapshot_path(&project.reports_dir());
    let conversion = if conversion_path.try_exists().map_err(|e| e.to_string())? {
        let snapshot =
            crate::conversion_run::load_snapshot(&project.reports_dir(), project.root())?;
        let rows = snapshot.rows.iter().filter(|r|r.job.floppy==label).map(|r|json!({"source":r.job.source_path,
            "source_sha256":r.job.source_sha256,"status":r.status(),"modern":r.job.modern_path,"pdf":r.job.pdf_path})).collect::<Vec<_>>();
        json!({"recorded_jobs":rows,"historical_snapshot_only":true,"outputs_freshly_verified":false,"path":conversion_path})
    } else {
        json!({"recorded_jobs":[],"historical_snapshot_only":true})
    };
    let recovery = crate::recovery_plan::plan_project(&project.images_dir())?
        .into_iter()
        .find(|p| p.disk_number == disk);
    let decisions_path = project.reports_dir().join("OfflineRecoveryDecisions.json");
    let decisions = if decisions_path.try_exists().map_err(|e| e.to_string())? {
        let report = read_json(&decisions_path, 32 * 1024 * 1024)?;
        if report["schema_version"] != 1 {
            return Err("Unsupported offline recovery decision schema".into());
        }
        report["decisions"]
            .as_array()
            .ok_or("Missing offline recovery decisions")?
            .iter()
            .filter(|r| r["disk_number"] == disk)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(
        json!({"schema_version":1,"disk":disk,"label":label,"note":note(project,disk)?,
        "preferred_image":{"selection":if preferred.preferred { "operator hash-bound preference" } else { "automatic evidence ranking" },"attempt":preferred.attempt_number,"image":preferred.image_file,
            "sha256":preferred.sha256,"bad_sectors":preferred.bad_sectors,"attention_required":preferred.attention_required},
        "extraction":extraction,"recovery":recovery,"conversion":conversion,"audit":audit,"processing_jobs":jobs,
        "automated_decisions":{"records":decisions,"path":decisions_path,"historical_only":true,"fresh_source_bindings_verified":false},
        "physical_media_access":false,"customer_delivery_certified":false,
        "scope":"Read-only lifecycle projection. Historical audit/conversion states do not certify present integrity."}),
    )
}

pub fn human(value: &Value) -> String {
    format!(
        "Note: {}\nPreferred ({}): #{} / {}\nExtraction: {} | recovery: {}\nConversion: {} recorded jobs (historical) | audit matches preferred image: {} (historical)\nProcessing jobs: {}\nNo physical media accessed; recorded reports are not fresh integrity certificates.",
        value["note"]["text"].as_str().unwrap_or(""),
        value["preferred_image"]["selection"]
            .as_str()
            .unwrap_or("unknown"),
        value["preferred_image"]["attempt"],
        value["preferred_image"]["image"],
        value["extraction"]["state"].as_str().unwrap_or("unknown"),
        value["recovery"]["action"].as_str().unwrap_or("unknown"),
        value["conversion"]["recorded_jobs"]
            .as_array()
            .map_or(0, Vec::len),
        value["audit"]["matches_preferred_image"],
        value["processing_jobs"].as_array().map_or(0, Vec::len)
    )
}
