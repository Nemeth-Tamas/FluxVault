//! Per-sector USB pass attribution from a separately hash/map-bound acquisition
//! log. Implicit initial success is explicitly an inference from completed
//! coverage, not a sensor event, CRC check or proof of physical label identity.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split('|')
        .map(str::trim)
        .find_map(|s| s.strip_prefix(&format!("{key}=")))
}
pub(crate) fn index(
    log: &str,
    total: usize,
    bad: &BTreeSet<u64>,
    recovered: usize,
) -> Result<Option<Vec<Value>>, String> {
    if !log.contains("RETRY_POLICY |") {
        return Ok(None);
    }
    if total == 0 || total > 8192 {
        return Err("USB pass attribution exceeds sector bound".into());
    }
    let mut rows = BTreeMap::<usize, (usize, bool, bool)>::new();
    let unread = if log.contains("USB_HANDOFF_POLICY |") || log.contains("UNREAD_RANGE |") {
        crate::legacy_logs::parse_archiver_log(log)?
            .unread_sectors
            .into_iter()
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    for full in log.lines() {
        let line = full.split_once("] ").map_or(full, |(_, s)| s);
        let kind = line.split('|').next().unwrap_or("").trim();
        if !matches!(
            kind,
            "SECTOR_READ_FAILED" | "SECTOR_RETRY_FAILED" | "SECTOR_RECOVERED_AFTER_RETRY"
        ) {
            continue;
        }
        let line = line.split(" | error=").next().unwrap();
        let lba = field(line, "LBA")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n < total)
            .ok_or("Invalid USB pass LBA")?;
        let row = rows.entry(lba).or_insert((0, false, false));
        if kind == "SECTOR_READ_FAILED" {
            if row.1 || row.0 != 0 {
                return Err("Contradictory USB initial failure record".into());
            }
            row.1 = true;
        } else {
            let pass = field(line, "retry_pass")
                .and_then(|s| s.split('/').next()?.parse::<usize>().ok())
                .filter(|n| *n > 0 && *n <= 100)
                .ok_or("Invalid USB retry pass")?;
            let direction = field(line, "direction").ok_or("Missing USB retry direction")?;
            if !row.1
                || row.2
                || pass <= row.0
                || direction != if pass % 2 == 1 { "backward" } else { "forward" }
            {
                return Err("Contradictory USB retry sequence/direction".into());
            }
            row.0 = pass;
            row.2 = kind == "SECTOR_RECOVERED_AFTER_RETRY";
        }
    }
    if rows.values().filter(|r| r.2).count() != recovered
        || rows.iter().any(|(l, r)| bad.contains(&(*l as u64)) == r.2)
        || bad
            .iter()
            .any(|l| !unread.contains(l) && !rows.contains_key(&(*l as usize)))
        || unread
            .iter()
            .any(|l| !bad.contains(l) || rows.contains_key(&(*l as usize)))
    {
        return Err("USB pass observations contradict the completed acquisition map".into());
    }
    Ok(Some((0..total).map(|lba| {
        if unread.contains(&(lba as u64)) {
            return json!({"lba":lba,"outcome":"not_attempted_placeholder","successful_pass":null,"last_attempted_pass":null,"direction":null,"evidence":"explicit_usb_fast_handoff_unattempted_range","initial_track_vs_sector_fallback":null,"independent_crc_verified":false,"physical_label_identity_verified":false});
        }
        let row=rows.get(&lba);let failed=bad.contains(&(lba as u64));let pass=row.map_or(0,|r|r.0);
        json!({"lba":lba,"outcome":if failed{"unreadable_placeholder"}else{"readable_saved_sector"},
            "successful_pass":if failed{None}else{Some(pass)},"last_attempted_pass":pass,
            "direction":if pass.is_multiple_of(2){"forward"}else{"backward"},
            "evidence":if row.is_some_and(|r|r.2){"explicit_retry_recovery_log_record"}else if failed{"explicit_failed_read_log_and_final_bad_map"}else{"implicit_initial_success_from_complete_log_and_no_sector_failure"},
            "initial_track_vs_sector_fallback":null,"independent_crc_verified":false,"physical_label_identity_verified":false})
    }).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forward_backward_retry_and_placeholders_have_exact_distinct_origins() {
        let log = "RETRY_POLICY | retries=2\nSECTOR_READ_FAILED | LBA=1 | pass=initial_forward\nSECTOR_READ_FAILED | LBA=2 | pass=initial_forward\nSECTOR_RETRY_FAILED | LBA=1 | retry_pass=1/2 | direction=backward\nSECTOR_RECOVERED_AFTER_RETRY | LBA=1 | retry_pass=2 | direction=forward\nSECTOR_RETRY_FAILED | LBA=2 | retry_pass=1/2 | direction=backward\nSECTOR_RETRY_FAILED | LBA=2 | retry_pass=2/2 | direction=forward";
        let v = index(log, 3, &[2].into(), 1).unwrap().unwrap();
        assert_eq!(v[0]["successful_pass"], 0);
        assert_eq!(v[1]["successful_pass"], 2);
        assert!(v[2]["successful_pass"].is_null());
        assert_eq!(
            v[0]["evidence"],
            "implicit_initial_success_from_complete_log_and_no_sector_failure"
        );
        assert_eq!(v[1]["evidence"], "explicit_retry_recovery_log_record");
        assert!(
            index(
                &log.replace("direction=backward", "direction=forward"),
                3,
                &[2].into(),
                1
            )
            .is_err()
        );
        assert!(index(log, 3, &[].into(), 1).is_err());
        assert!(index(log, 3, &[2].into(), 2).is_err());
        assert!(
            index("older log without policy", 3, &[2].into(), 0)
                .unwrap()
                .is_none()
        );
    }
}
