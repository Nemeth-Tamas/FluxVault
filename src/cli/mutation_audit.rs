//! Automatic integrity/report refresh for direct expert mutations. Normal
//! scanning/processing/finalization already reconcile their own background tail.
use super::CliResponse;
use crate::project::ProjectState;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const PENDING: &str = ".fluxvault-audit-pending.json";
thread_local! {static ACTIVE:std::cell::Cell<bool>=const {std::cell::Cell::new(false)};}
pub(super) fn active() -> bool {
    ACTIVE.get()
}
pub(crate) fn clear_after_audit(project: &ProjectState) -> Result<(), String> {
    let pending = project.root().join(PENDING);
    if !pending.try_exists().map_err(|e| e.to_string())? {
        return Ok(());
    }
    crate::sector_inspection::regular(&pending, false)?;
    if fs::metadata(&pending).map_err(|e| e.to_string())?.len() > 16384 {
        return Err("Invalid audit pending marker; retained".into());
    }
    let v: Value = serde_json::from_slice(&fs::read(&pending).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if v["engine"] != "fluxvault-mutation-audit-v1"
        || v["schema_version"] != 1
        || v["project"] != project.root().display().to_string()
    {
        return Err("Audit pending marker binding mismatch; retained".into());
    }
    fs::remove_file(pending).map_err(|e| e.to_string())
}
pub(super) fn applies(args: &[String]) -> bool {
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "--help" | "-h" | "--plan"))
    {
        return false;
    }
    let a = args.first().map(String::as_str).unwrap_or("");
    let b = args.get(1).map(String::as_str).unwrap_or("");
    matches!(a, "acquire" | "extract")
        || (a == "conversion" && matches!(b, "run" | "retry"))
        || (a == "files" && b == "manifest")
        || (a == "disk" && b == "prefer")
        || (a == "recovery"
            && matches!(
                b,
                "composite"
                    | "crc-composite"
                    | "fat"
                    | "import"
                    | "extract"
                    | "documents"
                    | "backup"
            ))
        || (a == "storage" && matches!(b, "pack" | "prune"))
        || (a == "greaseweazle"
            && (matches!(b, "capture" | "decode" | "identify" | "recover")
                || b == "expert"
                    && args
                        .get(2)
                        .is_some_and(|c| matches!(c.as_str(), "capture" | "decode"))))
}
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
fn inventory(project: &ProjectState) -> Result<Value, String> {
    use sha2::{Digest, Sha256};
    let mut rows = Vec::new();
    for name in [
        "Images",
        "Extracted",
        "Recovery",
        "Converted",
        "Flux",
        ".fluxvault-preferred-images",
    ] {
        let root = project.root().join(name);
        if !root.try_exists().map_err(|e| e.to_string())? {
            continue;
        }
        let mut dirs = vec![root];
        while let Some(dir) = dirs.pop() {
            crate::sector_inspection::regular(&dir, true)?;
            for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if m.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if name.starts_with('.') && !name.ends_with(".json") || name.ends_with(".lock") {
                    continue;
                }
                crate::sector_inspection::regular(&path, false)?;
                let modified = m
                    .modified()
                    .map_err(|e| e.to_string())?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos()
                    .to_string();
                let control_hash = if name.ends_with(".json") && m.len() <= 8 * 1024 * 1024 {
                    Some(crate::conversion::sha256_file(&path)?)
                } else {
                    None
                };
                rows.push(json!({"path":path.strip_prefix(project.root()).map_err(|e|e.to_string())?,"bytes":m.len(),"modified":modified,"control_sha256":control_hash}));
                if rows.len() + dirs.len() > 100_000 {
                    return Err("Mutation inventory exceeds bound; audit remains pending".into());
                }
            }
        }
    }
    for name in [
        "Reports/ConversionState.json",
        "Reports/MasterFileList.csv",
        "project.json",
    ] {
        let path = project.root().join(name);
        if path.try_exists().map_err(|e| e.to_string())? {
            crate::sector_inspection::regular(&path, false)?;
            rows.push(json!({"path":name,"sha256":crate::conversion::sha256_file(&path)?}));
        }
    }
    rows.sort_by_key(|r| r["path"].as_str().unwrap_or("").to_owned());
    Ok(
        json!({"entries":rows.len(),"change_detection_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&rows).map_err(|e|e.to_string())?)),
        "scope":"change_detector_metadata_hashes_size_mtime_not_an_integrity_certificate"}),
    )
}

#[cfg(test)]
#[path = "mutation_audit_tests.rs"]
mod tests;
pub(super) fn run(
    args: &[String],
    cwd: &Path,
    work: impl FnOnce() -> Result<CliResponse, String>,
) -> Result<CliResponse, String> {
    let mut overrides = args.windows(2).filter(|p| p[0] == "--project");
    let project_override = overrides.next().map(|p| PathBuf::from(&p[1]));
    if overrides.next().is_some() {
        return Err("Duplicate --project option".into());
    }
    let root = super::resolve_project_root(cwd, project_override.as_deref())?;
    let project = ProjectState::open_without_session(root)?;
    crate::processing::validate_workspace(&project)?;
    let owner = crate::project_work::reserve(project.root())?;
    crate::project_work::with_owner(project.root(), &owner, || {
        let pending = project.root().join(PENDING);
        let prior = pending.try_exists().map_err(|e| e.to_string())?;
        if prior {
            crate::sector_inspection::regular(&pending, false)?;
            if fs::metadata(&pending).map_err(|e| e.to_string())?.len() > 16384 {
                return Err("Invalid audit pending marker; retained".into());
            }
            let value: Value =
                serde_json::from_slice(&fs::read(&pending).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("Invalid audit pending marker; retained: {e}"))?;
            if value["schema_version"] != 1
                || value["engine"] != "fluxvault-mutation-audit-v1"
                || value["project"] != project.root().display().to_string()
            {
                return Err("Audit pending marker binding mismatch; retained".into());
            }
        }
        let before = inventory(&project)?;
        if !prior {
            write(&pending,&serde_json::to_vec(&json!({"schema_version":1,"engine":"fluxvault-mutation-audit-v1","project":project.root(),"command":args,"before":before,"customer_delivery_certified":false})).map_err(|e|e.to_string())?)?;
        }
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                ACTIVE.set(self.0);
            }
        }
        let restore = Restore(ACTIVE.replace(true));
        let outcome = work();
        drop(restore);
        let after = inventory(&project)?;
        if !prior && before == after {
            fs::remove_file(&pending).map_err(|e| e.to_string())?;
            return outcome;
        }
        if crate::cancellation::requested() {
            return outcome;
        }
        let refresh = (|| {
            let _gate = crate::project_work::snapshot(project.root())?;
            let mut audit = crate::audit::run_audit(&project, &|s| eprintln!("[AUDIT] {s}"))?;
            let report = crate::final_report::export(
                &project,
                &audit,
                crate::project_settings::language(&project)?,
            )?;
            crate::final_report::reconcile_audit(&mut audit, &report);
            Ok::<_, String>(
                json!({"audit_json":audit.json_path,"audit_csv":audit.csv_path,"workbook":report.workbook,
                "attention_disks":audit.attention_disks,"customer_delivery_certified":false,
                "pending_marker_cleared":true,"old_customer_archives_unchanged":true}),
            )
        })();
        let refresh = match refresh {
            Ok(v) => v,
            Err(e) => {
                return Err(format!(
                    "{}Automatic audit refresh failed; evidence kept and pending marker retained: {e}",
                    outcome
                        .as_ref()
                        .err()
                        .map(|s| format!("{s}\n"))
                        .unwrap_or_default()
                ));
            }
        };
        fs::remove_file(&pending).map_err(|e| e.to_string())?;
        match outcome {
            Ok(mut response) => {
                if args.iter().any(|a| a == "--json") {
                    let mut value: Value =
                        serde_json::from_str(&response.output).map_err(|e| e.to_string())?;
                    if let Some(object) = value.as_object_mut() {
                        object.insert("automatic_audit_refresh".into(), refresh);
                        response.output = value.to_string();
                    }
                } else {
                    response.output.push_str("\nIntegrity audit and customer reports refreshed automatically; earlier delivery archives retained.");
                }
                Ok(response)
            }
            Err(e) => Err(format!(
                "{e}\nMaterial changes were audited automatically; see current reports."
            )),
        }
    })?
}
