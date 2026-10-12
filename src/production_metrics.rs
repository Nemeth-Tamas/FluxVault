//! Evidence-bound estimates and per-label operator-action accounting.
//! Historical commands are observations, never proof of physical identity/touches.
use crate::{conversion_run, project::ProjectState};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) fn actions(ledger: &BTreeMap<u32, Value>, custody: &Value) -> Value {
    let mut rows = Vec::new();
    if let Some(disks) = custody["disks"].as_object() {
        for (label, state) in disks {
            let Ok(disk) = label.parse::<u32>() else {
                continue;
            };
            let observed = ledger.get(&disk).cloned().unwrap_or_else(|| json!({}));
            let transfer = state["usb"].is_object() && state["gw"].is_object();
            let reads = observed["read_confirmations"].as_u64().unwrap_or(0);
            let removals = observed["removal_confirmations"].as_u64().unwrap_or(0);
            let exceptional = observed["identity_releases"].as_u64().unwrap_or(0)
                + observed["same_disk_confirmations"].as_u64().unwrap_or(0);
            rows.push(json!({"disk":disk,"observed":observed,
                "confirmed_operator_actions":reads+removals+exceptional,
                "nominal_insert_remove_target":if transfer {4} else {2},
                "usb_to_gw_transfer":transfer,"observation_available":ledger.contains_key(&disk),
                "physical_touch_count":null,"unrecorded_actions_unknown":true}));
        }
    }
    let total: u64 = rows
        .iter()
        .filter_map(|r| r["confirmed_operator_actions"].as_u64())
        .sum();
    json!({"schema_version":1,"scope":"durable_confirmation_commands_not_hand_motion_measurement",
        "confirmed_operator_actions":total,"disks":rows,"physical_touch_count":null,
        "technical_decisions_required_by_default":0,
        "note":"Target is two insert/remove actions for fresh media, four with a USB-to-GW transfer. Retries, reseats and identity exceptions are separately visible; implicit removals and unlogged physical actions remain unknown. Zero routine technical choices is a workflow design target, not a measured zero-intervention certificate."})
}

pub(crate) fn count(ledger: &mut BTreeMap<u32, Value>, disk: u32, key: &str) {
    let row = ledger.entry(disk).or_insert_with(|| json!({}));
    row[key] = json!(row[key].as_u64().unwrap_or(0) + 1);
}

fn quantiles(mut samples: Vec<f64>, jobs: u64) -> Value {
    samples.retain(|v| v.is_finite() && *v > 0.0 && *v <= 86_400.0);
    samples.sort_by(f64::total_cmp);
    if jobs == 0 {
        return json!({"remaining":0,"seconds_range":[0.0,0.0],"samples":samples.len()});
    }
    if samples.len() < 3 {
        return json!({"remaining":jobs,"seconds_range":null,"samples":samples.len(),"reason":"fewer_than_three_usable_observations"});
    }
    let lower = samples[(samples.len() - 1) / 4];
    let upper = samples[((samples.len() - 1) * 3).div_ceil(4)];
    json!({"remaining":jobs,"seconds_range":[lower*jobs as f64,upper*jobs as f64],"samples":samples.len(),"median_seconds":samples[samples.len()/2]})
}

pub fn estimate(project: &ProjectState, report: &Value) -> Result<Value, String> {
    let state = &report["custody"];
    let remaining = state["remaining_unclaimed_fresh_labels"].as_u64();
    let initial = state["pending_initial_reads"].as_u64().unwrap_or(0);
    let mut transfers = state["usb_transfer_pending"]
        .as_array()
        .map_or(0, |a| a.len()) as u64;
    let mut station_samples = BTreeMap::<String, Vec<f64>>::new();
    let mut categorized = BTreeMap::<String, Vec<f64>>::new();
    for row in report["timings"].as_array().into_iter().flatten() {
        if let (Some(station), Some(ms)) = (row["station"].as_str(), row["read_decode_ms"].as_u64())
        {
            station_samples
                .entry(station.into())
                .or_default()
                .push(ms as f64 / 1000.0);
            let work = if row["retry"] == true {
                "retry"
            } else {
                row["work"].as_str().unwrap_or("unknown")
            };
            categorized
                .entry(format!("{station}:{work}"))
                .or_default()
                .push(ms as f64 / 1000.0);
        }
    }
    let sample = |key: &str| categorized.get(key).cloned().unwrap_or_default();
    let mut fresh_samples = sample("USB:fresh");
    fresh_samples.extend(sample("GW:fresh"));
    let mut fresh_retries = 0;
    let mut gw_retries = 0;
    for disk in state["disks"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        if matches!(
            disk["phase"].as_str(),
            Some("reserved" | "reading" | "interrupted")
        ) && disk["ticket"]["work"] == "usb_recovery"
        {
            transfers += 1;
        }
        if disk["phase"] == "interrupted" {
            if disk["ticket"]["work"] == "usb_recovery" {
                gw_retries += 1;
            } else {
                fresh_retries += 1;
            }
        }
    }
    let fresh = remaining.map_or_else(
        || json!({"seconds_range":null,"reason":"no_saved_range_endpoint"}),
        |n| quantiles(fresh_samples, n + initial),
    );
    let gw = quantiles(sample("GW:usb_recovery"), transfers);
    let processing = crate::processing::status(project)?;
    let pending_files = processing["pending"].as_u64().unwrap_or(0);
    // Reused outputs have nearly-zero durations and must not predict fresh work.
    let conversion = if conversion_run::snapshot_path(&project.reports_dir())
        .try_exists()
        .map_err(|e| e.to_string())?
    {
        let saved = conversion_run::load_snapshot(&project.reports_dir(), project.root())?;
        let samples = saved
            .rows
            .iter()
            .filter(|r| {
                r.status() == "OK"
                    && r.modern.state != conversion_run::OutputState::Reused
                    && r.pdf.state != conversion_run::OutputState::Reused
            })
            .map(|r| r.duration_seconds)
            .collect();
        let retries = saved
            .rows
            .iter()
            .filter(|r| r.retried() && r.duration_seconds > 0.0)
            .map(|r| r.duration_seconds)
            .collect();
        let missing_rows = saved
            .planning
            .jobs
            .iter()
            .filter(|job| {
                !saved.rows.iter().any(|r| {
                    r.job.source_path == job.source_path && r.job.source_sha256 == job.source_sha256
                })
            })
            .count() as u64;
        let retry_candidates = saved.rows.iter().filter(|r| r.status() != "OK").count() as u64;
        json!({"per_fresh_job":quantiles(samples,1),"known_unrecorded_jobs":missing_rows,
            "observed_retry_candidate_jobs":retry_candidates,"observed_retry_serial_seconds_range":quantiles(retries,retry_candidates),
            "future_jobs":null,"reason":"saved_planning_scope_only_future_extraction_and_recovery_yield_unknown"})
    } else {
        json!({"seconds_range":null,"reason":"no_saved_conversion_observations"})
    };
    Ok(
        json!({"schema_version":2,"physical_media_access":false,"forecast_kind":"rough_observed_phase_ranges_not_deadline",
        "paused":state["paused"],"fresh_read_decode":fresh,"queued_gw_read_decode":gw,
        "interrupted_fresh_retry_serial":quantiles(sample("USB:retry").into_iter().chain(sample("GW:retry")).collect(),fresh_retries),
        "interrupted_gw_retry_serial":quantiles(sample("GW:retry"),gw_retries),"observed_reader_groups":categorized,
        "pending_saved_file_disks":pending_files,"conversion":conversion,
        "whole_batch_seconds":null,"packing_audit_package_seconds":null,
        "note":"Reader ranges use observed 25th/75th percentiles and count in-flight work at full cost. Stations/workers overlap; do not sum these ranges. Swap delays, exceptional damage, future file count, conversion contention, packing and final packaging remain unknown. Paused state does not predict a restart time."}),
    )
}

pub fn render(value: &Value) -> String {
    let phase = |name: &str| {
        let v = &value[name];
        match (
            v["seconds_range"][0].as_f64(),
            v["seconds_range"][1].as_f64(),
        ) {
            (Some(a), Some(b)) => format!(
                "{:.1}–{:.1} min ({} remaining, {} samples)",
                a / 60.0,
                b / 60.0,
                v["remaining"],
                v["samples"]
            ),
            _ => format!(
                "unknown ({})",
                v["reason"].as_str().unwrap_or("insufficient evidence")
            ),
        }
    };
    format!(
        "Observed production forecast{}\nFresh read/decode: {}\nQueued GW recovery read/decode: {}\nSaved-file disks pending: {}\nWhole-batch finish: unknown; file yield and final tail are not predicted.\n{}",
        if value["paused"] == true {
            " / PAUSED"
        } else {
            ""
        },
        phase("fresh_read_decode"),
        phase("queued_gw_read_decode"),
        value["pending_saved_file_disks"],
        value["note"].as_str().unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_zero_reused_invalid_and_paused_observations_do_not_invent_eta() {
        assert!(quantiles(vec![1.0, 2.0], 2)["seconds_range"].is_null());
        assert!(quantiles(vec![f64::NAN, 0.0, 100_000.0], 1)["seconds_range"].is_null());
        assert_eq!(quantiles(vec![], 0)["seconds_range"], json!([0.0, 0.0]));
        let q = quantiles(vec![90.0, 60.0, 120.0, 30.0], 2);
        assert_eq!(q["seconds_range"], json!([60.0, 240.0]));
    }
    #[test]
    fn per_label_actions_keep_unknown_touches_transfer_target_and_exceptions_separate() {
        let mut ledger = BTreeMap::new();
        count(&mut ledger, 7, "read_confirmations");
        count(&mut ledger, 7, "read_confirmations");
        count(&mut ledger, 7, "identity_releases");
        let custody = json!({"disks":{"7":{"usb":{},"gw":{}},"8":{"usb":{}},"9":{}}});
        let r = actions(&ledger, &custody);
        assert_eq!(r["confirmed_operator_actions"], 3);
        assert_eq!(r["disks"][0]["nominal_insert_remove_target"], 4);
        assert_eq!(r["disks"][1]["observation_available"], false);
        assert!(r["disks"][0]["physical_touch_count"].is_null());
    }
}
