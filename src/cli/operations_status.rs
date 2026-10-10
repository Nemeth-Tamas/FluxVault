//! Additive, read-only project dashboard. Recorded stations never authorize a
//! physical action over the original console's current custody prompt.
use super::{dual_scan, flux_scan, production_flow};
use crate::{
    benchmark, dual_benchmark,
    production::{self, Station},
    project::ProjectState,
};
use serde_json::{Value, json};

pub(super) fn inspect(project: &ProjectState, processing: &Value) -> Result<Value, String> {
    let control = crate::run_control::status(project)?;
    let flow = production_flow::status(project)?;
    let dual = production::status(project)?;
    let single = flux_scan::saved_status(project)?;
    let single_timing = benchmark::report(project)?;
    let dual_timing = dual_benchmark::report(project)?;
    let mode = mode(&control, &flow, &dual, &single);
    let stations = if mode == "dual" {
        [Station::Usb, Station::Greaseweazle]
            .into_iter()
            .map(|station| {
                let held = dual_scan::held_in_state(&dual, station);
                json!({"station":station,"disk":held.map(|(n,_)|n),
                "recorded_phase":held.map_or("ready",|(_,p)|p),
                "next_action":dual_scan::next_action(&dual, station, held),
                "reader_active":null})
            })
            .collect::<Vec<_>>()
    } else if mode == "single_gw" {
        let disk = single["pending_disk"].as_u64();
        let next = u64::from(project.current_disk_number());
        let action = if let Some(n) = disk {
            format!(
                "CHECK original console / DO NOT REMOVE potentially active {n:03}; if stopped, resume and reconfirm the same label"
            )
        } else if single["last_disk"].as_u64().is_some_and(|last| next > last) {
            "Numbered endpoint recorded; check original console before removal/finishing".into()
        } else {
            format!(
                "Follow original console's INSERT {next:03} / physical confirmation prompt; resume saved scan if stopped"
            )
        };
        vec![
            json!({"station":"greaseweazle","drive":single["drive"],"disk":disk,
            "recorded_phase":if disk.is_some(){"pending"}else{"between_disks"},
            "next_action":action,"reader_active":null}),
        ]
    } else {
        Vec::new()
    };
    let timing = if mode == "dual" {
        let rate = dual_timing["finished_invocations_saved_labels_per_hour"].as_f64();
        let remaining = dual["remaining_unclaimed_fresh_labels"]
            .as_u64()
            .zip(dual["pending_initial_reads"].as_u64())
            .map(|(a, b)| a + b);
        let samples = dual_timing["finished_invocations_timed_saved_unique_labels"]
            .as_u64()
            .unwrap_or(0);
        pace(rate, remaining, samples, dual["paused"] == true)
    } else if mode == "single_gw" {
        let rate = single_timing
            .projected_136_feed_hours
            .filter(|hours| *hours > 0.0)
            .map(|hours| 136.0 / hours);
        let remaining = single["last_disk"].as_u64().map(|last| {
            last.saturating_add(1)
                .saturating_sub(u64::from(project.current_disk_number()))
        });
        pace(
            rate,
            remaining,
            single_timing.timed_physical_jobs as u64,
            false,
        )
    } else {
        pace(None, None, 0, false)
    };
    Ok(json!({"schema_version":1,"physical_media_access":false,
        "snapshot_scope":"separate saved records, not an atomic live-reader snapshot",
        "mode":mode,"run":control,"production":flow,"stations":stations,
        "saved_single_gw":single,"saved_dual":dual,
        "usb_transfer_pending":dual["usb_transfer_pending"].as_array().cloned().unwrap_or_default(),
        "removal_confirmed_gw_queue":dual["usb_recovery_queue"].as_array().cloned().unwrap_or_default(),
        "recommended_gw_disk":dual["recommended_gw_disk"],
        "paused":dual["paused"]==true,"timing":timing,
        "processing":{"pending":processing["pending"],"attention":processing["attention"],
            "recorded_stage":processing["worker"]["stage"],"owner_active":processing["owner_active"]},
        "operator_confirmations":{"single_numbered_insertions":single_timing.confirmed_insertions,
            "dual_numbered_reads":dual_timing["numbered_read_confirmations"],
            "dual_explicit_removals":dual_timing["explicit_removal_confirmations"],
            "scope":"recorded confirmation commands, not all physical touches"},
        "warnings": ["Use the original feeding console's current ACTION/swap cue. A process owner is not proof of an active physical reader; recorded reading can survive a stopped process.",
            "ETA estimates only fresh feeding from historical samples; recovery transfers and file-processing/packaging tail are excluded. No whole-job deadline guarantee."]}))
}

fn mode(control: &Value, flow: &Value, dual: &Value, single: &Value) -> &'static str {
    if control["active"] == true {
        match control["record"]["operation"].as_str() {
            Some("dual_scan") => return "dual",
            Some("scan") => return "single_gw",
            Some("production_start") => {
                return if flow["record"]["dual"] == true {
                    "dual"
                } else {
                    "single_gw"
                };
            }
            _ => {}
        }
    }
    match (dual["initialized"] == true, single["initialized"] == true) {
        (true, false) => "dual",
        (false, true) => "single_gw",
        (true, true) => "ambiguous_saved_modes",
        _ => "none",
    }
}

pub(super) fn next_actions(value: &Value) -> Vec<String> {
    if value["run"]["active"] == true {
        return vec!["A controllable operation is active. Follow its original console's current ACTION/swap prompt; `fv stop` requests a safe stop.".into()];
    }
    if value["mode"] == "ambiguous_saved_modes" {
        return vec!["Both single-GW and dual journals exist. Inspect `fv production status` and your original scan command before resuming; no mode or physical custody is inferred.".into()];
    }
    let mut actions = value["stations"]
        .as_array()
        .map(|stations| {
            stations
                .iter()
                .filter_map(|s| s["next_action"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if value["production"]["record"].is_object() {
        actions.push("Inspect `fv production status`; use `fv production resume` for unfinished production work. Completed package receipts are historical, not fresh integrity certificates.".into());
    } else if value["processing"]["pending"]
        .as_u64()
        .is_some_and(|n| n > 0)
    {
        actions.push("Saved-file jobs remain; `fv processing resume` continues them without a physical read.".into());
    }
    actions
}

fn pace(rate: Option<f64>, remaining: Option<u64>, samples: u64, paused: bool) -> Value {
    let rate = rate.filter(|n| n.is_finite() && *n > 0.0);
    let minutes = if samples >= 3 && !paused {
        rate.zip(remaining).map(|(r, n)| n as f64 / r * 60.0)
    } else {
        None
    };
    json!({"measurement_scope":"historical_fresh_feeding_only","saved_labels_per_hour":rate,
        "samples":samples,"remaining_fresh_labels":remaining,"rough_feed_eta_minutes":minutes,
        "paused":paused,"whole_job_eta_minutes":null})
}

pub(super) fn human(value: &Value) -> String {
    let mut lines = vec![format!(
        "Operation: {} | controller active: {} | production phase: {} (recorded)",
        value["run"]["record"]["operation"]
            .as_str()
            .unwrap_or("none"),
        value["run"]["active"],
        value["production"]["record"]["phase"]
            .as_str()
            .unwrap_or("none")
    )];
    if value["mode"] == "ambiguous_saved_modes" {
        lines.push("Both single-GW and dual journals exist: no live mode or new read is inferred. Inspect production status/original command.".into());
    }
    if let Some(stations) = value["stations"].as_array() {
        for s in stations {
            let disk = s["disk"].as_u64().map_or("-".into(), |n| format!("{n:03}"));
            lines.push(format!(
                "{}: {} / {disk} (recorded; not a reader probe)\n  ACTION: {}",
                s["station"].as_str().unwrap_or("unknown"),
                s["recorded_phase"].as_str().unwrap_or("unknown"),
                s["next_action"]
                    .as_str()
                    .unwrap_or("Check original console")
            ));
        }
    }
    lines.push(format!(
        "USB -> GW pending: {} | recommended: {}",
        value["usb_transfer_pending"], value["recommended_gw_disk"]
    ));
    let timing = &value["timing"];
    let rate = timing["saved_labels_per_hour"]
        .as_f64()
        .map_or("unavailable".into(), |r| {
            format!("{r:.1} labels/hour (historical)")
        });
    let eta = timing["rough_feed_eta_minutes"]
        .as_f64()
        .map_or("unavailable".into(), |n| {
            format!("{n:.1} min (rough fresh-feed only)")
        });
    lines.push(format!(
        "Pace: {rate} | ETA: {eta}; excludes transfer recovery/processing tail."
    ));
    lines.push("Original feeding console's current swap cue remains authoritative; inspection starts no reads.".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eta_requires_samples_and_excludes_paused_and_unknown_endpoints() {
        assert!(pace(Some(60.0), Some(20), 2, false)["rough_feed_eta_minutes"].is_null());
        assert!(pace(Some(60.0), Some(20), 3, true)["rough_feed_eta_minutes"].is_null());
        assert!(pace(Some(60.0), None, 3, false)["rough_feed_eta_minutes"].is_null());
        assert!(pace(Some(f64::NAN), Some(20), 3, false)["rough_feed_eta_minutes"].is_null());
        assert_eq!(
            pace(Some(60.0), Some(20), 3, false)["rough_feed_eta_minutes"],
            20.0
        );
        assert!(pace(Some(60.0), Some(20), 3, false)["whole_job_eta_minutes"].is_null());
    }
    #[test]
    fn active_mode_wins_but_two_inactive_histories_do_not_authorize_a_guess() {
        let both = json!({"initialized":true});
        assert_eq!(
            mode(&json!({"active":false}), &json!({}), &both, &both),
            "ambiguous_saved_modes"
        );
        assert_eq!(
            mode(
                &json!({"active":true,"record":{"operation":"scan"}}),
                &json!({}),
                &both,
                &both
            ),
            "single_gw"
        );
        assert_eq!(
            mode(
                &json!({"active":true,"record":{"operation":"dual_scan"}}),
                &json!({}),
                &both,
                &both
            ),
            "dual"
        );
    }
}
