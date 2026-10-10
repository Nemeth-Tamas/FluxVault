//! Sequential, bounded aggregation of the same per-disk saved-evidence checks.
use super::{WARNING, inspect};
use crate::{project::ProjectState, sector_inspection};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    time::SystemTime,
};

const MAX_DISKS: usize = 4096;
const MAX_ENTRIES: usize = 100_000;
const MAX_REPORT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Inventory {
    disks: BTreeSet<u32>,
    entries: BTreeMap<PathBuf, (u64, SystemTime)>,
}

// Filenames are discovery hints, not trusted metadata. Malformed/legacy/raw-only
// labels still receive an explicit refused row from the strict image inspector.
fn label(name: &str, flux: bool) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let stem = lower.split_once("_attempt_").map(|(n, _)| n).or_else(|| {
        if flux {
            None
        } else {
            [".img", ".ima", ".bin"]
                .iter()
                .find_map(|s| lower.strip_suffix(s))
        }
    })?;
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse::<u32>().ok().filter(|n| *n > 0)
}

fn inventory(project: &ProjectState) -> Result<Inventory, String> {
    sector_inspection::regular(project.root(), true)?;
    let mut result = Inventory {
        disks: BTreeSet::new(),
        entries: BTreeMap::new(),
    };
    let mut total = 0;
    for (dir, flux) in [
        (project.images_dir(), false),
        (project.root().join("Flux"), true),
    ] {
        if flux && !dir.try_exists().map_err(|e| e.to_string())? {
            continue;
        }
        sector_inspection::regular(&dir, true)?;
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            total += 1;
            if total > MAX_ENTRIES {
                return Err("Batch impact inventory exceeds 100,000 entries".into());
            }
            crate::cancellation::check()?;
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let Some(disk) = name.to_str().and_then(|n| label(n, flux)) else {
                continue;
            };
            // No symlink/reparse/device query is followed, even for raw-only rows.
            sector_inspection::regular(&entry.path(), false)?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            result.entries.insert(
                entry.path(),
                (meta.len(), meta.modified().map_err(|e| e.to_string())?),
            );
            result.disks.insert(disk);
            if result.disks.len() > MAX_DISKS {
                return Err("Batch impact exceeds 4,096 disk labels".into());
            }
        }
    }
    let preferred = project.root().join(".fluxvault-preferred-images");
    if preferred.try_exists().map_err(|e| e.to_string())? {
        sector_inspection::regular(&preferred, true)?;
        for entry in fs::read_dir(&preferred).map_err(|e| e.to_string())? {
            total += 1;
            if total > MAX_ENTRIES {
                return Err("Batch impact inventory exceeds 100,000 entries".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            sector_inspection::regular(&entry.path(), false)?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            result.entries.insert(
                entry.path(),
                (meta.len(), meta.modified().map_err(|e| e.to_string())?),
            );
        }
    }
    Ok(result)
}

/// Each disk uses its preferred/default native image, never a global attempt.
/// Per-disk refusals remain visible and do not hide other inspected results.
/// A batch is not an atomic snapshot: inventory churn is explicitly reported.
pub fn inspect_all(
    project: &ProjectState,
    mut progress: impl FnMut(usize, usize, u32),
) -> Result<Value, String> {
    let before = inventory(project)?;
    if before.disks.is_empty() {
        return Err("No saved disk labels found in Images/Flux for batch impact".into());
    }
    let mut reports = Vec::new();
    let mut bytes = 0usize;
    let (mut inspected, mut refused, mut attention, mut unknown) = (0usize, 0usize, 0usize, 0usize);
    let mut totals = BTreeMap::<&str, u64>::new();
    for (i, disk) in before.disks.iter().enumerate() {
        crate::cancellation::check()?;
        progress(i + 1, before.disks.len(), *disk);
        let report = match inspect(project, *disk, None, None) {
            Ok(mut v) => {
                inspected += 1;
                attention += usize::from(v["attention_required"] == true);
                unknown += usize::from(!v["filesystem_error"].is_null());
                for key in [
                    "problem_sectors",
                    "known_files_with_problem_dependencies",
                    "mapped_complete_payloads",
                    "incomplete_or_ambiguous_files",
                    "recorded_dependencies",
                ] {
                    *totals.entry(key).or_default() += v["counts"][key].as_u64().unwrap_or(0);
                }
                v["inspection_state"] = json!("inspected");
                v
            }
            Err(e) => {
                if crate::cancellation::stopped(&e) {
                    return Err(e);
                }
                refused += 1;
                attention += 1;
                json!({"disk":disk,"inspection_state":"refused","attention_required":true,"error":e})
            }
        };
        bytes = bytes
            .checked_add(
                serde_json::to_vec(&report)
                    .map_err(|e| e.to_string())?
                    .len(),
            )
            .ok_or("Batch impact size overflow")?;
        if bytes > MAX_REPORT_BYTES {
            return Err("Batch impact exceeds 64 MiB; inspect disks individually instead".into());
        }
        reports.push(report);
    }
    let changed = inventory(project)? != before;
    Ok(
        json!({"schema_version":1,"mode":"batch_sector_impact","project":project.root(),
        "physical_media_access":false,"files_written":0,"customer_delivery_certified":false,
        "independent_flux_crc_verified":false,"attention_required":attention>0 || changed,
        "warning":WARNING,"selection":"one_preferred_or_default_native_image_per_discovered_label",
        "snapshot_scope":"per_disk_verified_snapshots_not_an_atomic_project_snapshot",
        "declared_batch_completeness_verified":false,"inventory_changed":changed,
        "counts":{"saved_labels":before.disks.len(),"inspected_disks":inspected,"refused_disks":refused,
            "attention_disks":attention,"no_attention_in_inspected_scope":inspected-(attention-refused),
            "unknown_filesystem_disks":unknown,"inspected_image_totals":totals},
        "disks":reports}),
    )
}

fn n(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

pub fn render_all(value: &Value) -> String {
    let c = &value["counts"];
    let t = &c["inspected_image_totals"];
    let mut out = format!(
        "BATCH DAMAGE IMPACT / SAVED IMAGES ONLY\n{} saved labels | {} inspected | {} refused | {} need attention\n{} problem sectors | {} known files depend on them\n{} mapped complete payloads | {} incomplete/ambiguous files | {} unknown filesystems\nCounts describe one selected image per saved label, not customer completeness.\n\n",
        n(c, "saved_labels"),
        n(c, "inspected_disks"),
        n(c, "refused_disks"),
        n(c, "attention_disks"),
        n(t, "problem_sectors"),
        n(t, "known_files_with_problem_dependencies"),
        n(t, "mapped_complete_payloads"),
        n(t, "incomplete_or_ambiguous_files"),
        n(c, "unknown_filesystem_disks")
    );
    out.push_str("DISK  ATTEMPT  SCOPE        SECTORS  AFFECTED  INCOMPLETE\n");
    for r in value["disks"].as_array().into_iter().flatten() {
        if r["inspection_state"] == "refused" {
            out.push_str(&format!(
                "{:03}   --       REFUSED      --       --        --\n  {}\n",
                n(r, "disk"),
                r["error"].as_str().unwrap_or("Unknown refusal")
            ));
            continue;
        }
        out.push_str(&format!(
            "{:03}   {:03}      {:<12} {:<8} {:<9} {}\n",
            n(r, "disk"),
            n(r, "attempt"),
            if r["attention_required"] == true {
                "ATTENTION"
            } else {
                "NO ATTENTION"
            },
            n(&r["counts"], "problem_sectors"),
            n(&r["counts"], "known_files_with_problem_dependencies"),
            n(&r["counts"], "incomplete_or_ambiguous_files")
        ));
    }
    out.push_str("\nATTENTION DETAILS\n");
    for r in value["disks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["attention_required"] == true && r["inspection_state"] == "inspected")
    {
        out.push_str(&format!(
            "\nDisk {:03} / attempt {:03}\n",
            n(r, "disk"),
            n(r, "attempt")
        ));
        if let Some(e) = r["filesystem_error"].as_str() {
            out.push_str(&format!("  UNKNOWN: {e}\n"));
        }
        if let Some(w) = r["layout_evidence"]["warning"].as_str() {
            out.push_str(&format!("  LAYOUT HYPOTHESIS: {w}\n"));
        }
        let mut paths = BTreeSet::new();
        let mut unattributed = 0;
        for s in r["sectors"].as_array().into_iter().flatten() {
            let deps = s["file_dependencies"].as_array();
            unattributed += usize::from(deps.is_none_or(Vec::is_empty));
            for d in deps.into_iter().flatten() {
                if let Some(path) = d["path"].as_str() {
                    paths.insert(path);
                }
            }
        }
        for path in paths {
            out.push_str(&format!("  Known dependency: {path}\n"));
        }
        for tail in r["unmapped_tails"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  Unmapped tail: {} / {} bytes / {}\n",
                tail["path"].as_str().unwrap_or("?"),
                tail["bytes"],
                tail["reason"].as_str().unwrap_or("?")
            ));
        }
        if unattributed > 0 {
            out.push_str(&format!("  {unattributed} problem sectors lack known file attribution; NOT proof of harmless space.\n"));
        }
        let gaps = r["directory_gaps"].as_array().map_or(0, Vec::len);
        let skipped = r["skipped_entries"].as_array().map_or(0, Vec::len);
        out.push_str(&format!(
            "  Directory gaps: {gaps}; skipped entries: {skipped}.\n"
        ));
        if r["dependencies_truncated"] == true {
            out.push_str("  Dependency ceiling reached: file listing is not exhaustive.\n");
        }
    }
    if value["inventory_changed"] == true {
        out.push_str("\nWARNING: saved inventory changed during this batch; rerun after processing is idle.\n");
    }
    out.push_str(&format!("\n{WARNING}\nUse recovery impact N for offsets/LBAs; --json includes full per-disk details.\n"));
    out
}
