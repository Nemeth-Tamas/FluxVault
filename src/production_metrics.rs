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
    let mut categorized = BTreeMap::<String, Vec<f64>>::new();
    for row in report["timings"].as_array().into_iter().flatten() {
        if let (Some(station), Some(ms)) = (row["station"].as_str(), row["read_decode_ms"].as_u64())
        {
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
    let workers = processing["worker"]["effective_conversion_workers"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(
            crate::project_settings::get(project)?
                .conversion_workers
                .unwrap_or(conversion_run::DEFAULT_CONVERSION_WORKERS),
        );
    let history = crate::performance_history::samples(project)?;
    let fresh_count = fresh["remaining"].as_u64();
    let current_labels = crate::imaging::load_project_statistics(&project.images_dir())?
        .disks
        .len() as u64;
    let estimated_bytes = fresh_count.and_then(|n| {
        crate::performance_history::projected_package_bytes(
            &history,
            current_labels.saturating_add(n),
        )
    });
    let processing_count =
        fresh_count.map(|n| n.saturating_add(transfers).saturating_add(pending_files));
    let file_work = processing_count.map_or_else(
        || json!({"seconds_range":null,"reason":"unknown_remaining_labels"}),
        |n| {
            quantiles(
                crate::performance_history::rates(&history, "processing", workers),
                n,
            )
        },
    );
    let package_work = estimated_bytes.map_or_else(
        || json!({"seconds_range":null,"reason":"no_historical_packaged_label_volume"}),
        |n| {
            quantiles(
                crate::performance_history::rates(&history, "packaging", workers),
                n,
            )
        },
    );
    let reader_work = reader_envelope(&fresh, &gw);
    let retry_work = reader_envelope(
        &quantiles(
            sample("USB:retry")
                .into_iter()
                .chain(sample("GW:retry"))
                .collect(),
            fresh_retries,
        ),
        &quantiles(sample("GW:retry"), gw_retries),
    );
    let reader_work = reader_envelope(&reader_work, &retry_work);
    let paused = state["paused"] == true;
    let whole = if paused {
        Value::Null
    } else {
        completion_envelope(&reader_work, &file_work, &package_work)
    };
    Ok(
        json!({"schema_version":3,"physical_media_access":false,"forecast_kind":"conditional_empirical_completion_scenario_not_deadline",
        "paused":state["paused"],"fresh_read_decode":fresh,"queued_gw_read_decode":gw,
        "interrupted_fresh_retry_serial":quantiles(sample("USB:retry").into_iter().chain(sample("GW:retry")).collect(),fresh_retries),
        "interrupted_gw_retry_serial":quantiles(sample("GW:retry"),gw_retries),"observed_reader_groups":categorized,
        "pending_saved_file_disks":pending_files,"conversion":conversion,
        "reader_scenario":reader_work,"saved_file_processing_scenario":file_work,"packaging_scenario":package_work,
        "whole_batch_seconds":whole,"estimated_future_package_source_bytes":estimated_bytes,"packing_audit_package_seconds":package_work["seconds_range"],
        "processing_timing_workers":workers,"processing_history_samples":crate::performance_history::rates(&history,"processing",workers).len(),
        "note":"Conditional scenario: remaining disks resemble observed read/recovery and incremental processing cohorts, historically observed stations stay available, and packaged bytes per label resemble the last measured package. CPU work includes extraction/recovery/conversion/audit; it overlaps readers. In-flight work is counted at full cost. Empirical quartiles are NOT hard bounds. Exceptional damage, new file yield, contention, operator swaps, storage packing delays and root handoff copying can extend or invalidate the scenario. Missing observations remain unknown; paused state gives no finish deadline."}),
    )
}

fn interval(v: &Value) -> Option<[f64; 2]> {
    let a = v["seconds_range"][0].as_f64()?;
    let b = v["seconds_range"][1].as_f64()?;
    (a.is_finite() && b.is_finite() && a >= 0.0 && b >= a).then_some([a, b])
}
fn reader_envelope(fresh: &Value, recovery: &Value) -> Value {
    match (interval(fresh), interval(recovery)) {
        (Some(a), Some(b)) => {
            json!({"seconds_range":[a[0].max(b[0]),a[1]+b[1]],"scope":"observed_reader_overlap_to_serial_scenario_not_hard_bounds"})
        }
        _ => json!({"seconds_range":null,"reason":"insufficient_reader_phase_observations"}),
    }
}
fn completion_envelope(readers: &Value, files: &Value, package: &Value) -> Value {
    match (interval(readers), interval(files), interval(package)) {
        (Some(r), Some(f), Some(p)) => json!([r[0].max(f[0]) + p[0], r[1] + f[1] + p[1]]),
        _ => Value::Null,
    }
}

pub fn estimate_single(
    project: &ProjectState,
    single: &crate::benchmark::BenchmarkReport,
    saved: &Value,
) -> Result<Value, String> {
    let next = u64::from(project.current_disk_number());
    let remaining = saved["last_disk"]
        .as_u64()
        .map(|last| last.saturating_add(1).saturating_sub(next));
    let rows = single
        .disks
        .iter()
        .filter(|d| {
            d["outcome"]["physical_reads_this_run"]
                .as_u64()
                .unwrap_or(0)
                > 0
        })
        .filter_map(|d| {
            d["timing"]["recovery_ms"]
                .as_u64()
                .map(|ms| json!({"station":"GW","read_decode_ms":ms,"work":"fresh","retry":false}))
        })
        .collect::<Vec<_>>();
    let mut input = json!({"custody":{"remaining_unclaimed_fresh_labels":remaining,"pending_initial_reads":0,"usb_transfer_pending":[],"disks":{}},"timings":rows});
    if saved["pending_disk"].is_number() {
        input["custody"]["disks"] =
            json!({"active":{"phase":"interrupted","ticket":{"work":"fresh"}}});
    }
    let mut v = estimate(project, &input)?;
    v["mode"] = json!("single_gw");
    v["single_sample_scope"] = json!(
        "committed_per_label_whole_recovery_durations_include_internal_retry_work; exact retry-separated history unavailable"
    );
    Ok(v)
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
    let whole = match (
        value["whole_batch_seconds"][0].as_f64(),
        value["whole_batch_seconds"][1].as_f64(),
    ) {
        (Some(a), Some(b)) => format!(
            "{:.1}–{:.1} min / conditional observed scenario, NOT a deadline",
            a / 60.0,
            b / 60.0
        ),
        _ => "unknown / timing history incomplete or paused".into(),
    };
    format!(
        "Observed production forecast{}\nFresh read/decode: {}\nQueued GW recovery read/decode: {}\nSaved-file disks pending: {}\nProcessing/conversion/audit: {}\nPackaging: {}\nWhole-batch scenario: {}\n{}",
        if value["paused"] == true {
            " / PAUSED"
        } else {
            ""
        },
        phase("fresh_read_decode"),
        phase("queued_gw_read_decode"),
        value["pending_saved_file_disks"],
        phase("saved_file_processing_scenario"),
        phase("packaging_scenario"),
        whole,
        value["note"].as_str().unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_processing_packaging_rates_make_conditional_estimate_and_pause_clears_it() {
        let p = ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-forecast-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )))
        .unwrap();
        let mut report = json!({"custody":{"remaining_unclaimed_fresh_labels":2,"pending_initial_reads":0,"usb_transfer_pending":[],"disks":{}},
            "timings":[{"station":"GW","work":"fresh","read_decode_ms":10000},{"station":"GW","work":"fresh","read_decode_ms":20000},{"station":"GW","work":"fresh","read_decode_ms":30000}]});
        assert!(estimate(&p, &report).unwrap()["whole_batch_seconds"].is_null());
        let mut rows = String::new();
        for elapsed in [1000, 2000, 3000] {
            for (kind, units, labels) in [("processing", 1, 0), ("packaging", 1000, 1)] {
                rows.push_str(&json!({"schema":1,"kind":kind,"units":units,"elapsed_ms":elapsed,"workers":4,"unix_ms":1,"labels":labels}).to_string());
                rows.push('\n');
            }
        }
        let history = p.logs_dir().join("PerformanceHistory.jsonl");
        std::fs::write(&history, &rows).unwrap();
        let v = estimate(&p, &report).unwrap();
        assert_eq!(v["estimated_future_package_source_bytes"], 2000);
        assert_eq!(
            v["saved_file_processing_scenario"]["seconds_range"],
            json!([2.0, 6.0])
        );
        assert_eq!(v["whole_batch_seconds"], json!([22.0, 72.0]));
        report["custody"]["paused"] = json!(true);
        assert!(estimate(&p, &report).unwrap()["whole_batch_seconds"].is_null());
        report["custody"]["paused"] = json!(false);
        std::fs::write(history, rows.replace("\"workers\":4", "\"workers\":8")).unwrap();
        let v = estimate(&p, &report).unwrap();
        assert_eq!(v["processing_history_samples"], 0);
        assert!(v["whole_batch_seconds"].is_null());
        std::fs::remove_dir_all(p.root()).unwrap();
    }
    #[test]
    fn completion_scenario_respects_overlap_tail_missing_samples_and_invalid_intervals() {
        let r = reader_envelope(
            &json!({"seconds_range":[10.0,40.0]}),
            &json!({"seconds_range":[20.0,60.0]}),
        );
        assert_eq!(r["seconds_range"], json!([20.0, 100.0]));
        let p = json!({"seconds_range":[30.0,50.0]});
        let zip = json!({"seconds_range":[5.0,9.0]});
        assert_eq!(completion_envelope(&r, &p, &zip), json!([35.0, 159.0]));
        assert!(completion_envelope(&r, &p, &json!({"seconds_range":null})).is_null());
        assert!(interval(&json!({"seconds_range":[-1.0,2.0]})).is_none());
        assert!(interval(&json!({"seconds_range":[3.0,2.0]})).is_none());
    }
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
