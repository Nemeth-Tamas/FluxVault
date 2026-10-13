//! Bounded timing observations, not integrity certificates. Unknown history
//! yields no forecast; previous rates are never a promised recovery deadline.
use crate::{project::ProjectState, sector_inspection};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
};
const FILE: &str = "PerformanceHistory.jsonl";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sample {
    schema: u32,
    kind: String,
    units: u64,
    elapsed_ms: u64,
    workers: usize,
    unix_ms: u64,
    #[serde(default)]
    labels: usize,
}
fn valid(v: &Sample) -> bool {
    v.schema == 1
        && matches!(v.kind.as_str(), "processing" | "packaging")
        && v.units > 0
        && v.elapsed_ms > 0
        && v.elapsed_ms <= 86_400_000
        && (1..=16).contains(&v.workers)
        && v.labels <= 4096
        && v.unix_ms > 0
}
pub(crate) fn samples(project: &ProjectState) -> Result<Vec<Sample>, String> {
    let logs = project.logs_dir();
    sector_inspection::regular(&logs, true)?;
    if logs.canonicalize().map_err(|e| e.to_string())?.parent()
        != Some(
            project
                .root()
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_path(),
        )
    {
        return Err("Performance history directory escapes project".into());
    }
    let path = project.logs_dir().join(FILE);
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(Vec::new());
    }
    sector_inspection::regular(&path, false)?;
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Performance history exceeds bound".into());
    }
    // A truncated last row is ignored for a read-only forecast. An append
    // refuses it below, preserving diagnostic bytes rather than repairing them.
    let last = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
    let mut values = Vec::new();
    for line in bytes[..last]
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
    {
        let v: Sample = serde_json::from_slice(line)
            .map_err(|e| format!("Invalid performance observation: {e}"))?;
        if !valid(&v) || values.len() >= 4096 {
            return Err("Invalid/bounded performance observation".into());
        }
        values.push(v);
    }
    Ok(values)
}
pub(crate) fn record(
    project: &ProjectState,
    kind: &str,
    units: u64,
    elapsed_ms: u64,
    workers: usize,
) -> Result<(), String> {
    if units == 0 || elapsed_ms == 0 {
        return Ok(());
    }
    let value = Sample {
        schema: 1,
        kind: kind.into(),
        units,
        elapsed_ms,
        workers,
        unix_ms: crate::external_tools::current_unix_ms(),
        labels: if kind == "packaging" {
            crate::imaging::load_project_statistics(&project.images_dir())?
                .disks
                .len()
        } else {
            0
        },
    };
    if !valid(&value) {
        return Err("Invalid performance observation".into());
    }
    let path = project.logs_dir().join(FILE);
    let old = samples(project)?;
    if old.len() >= 4096 {
        return Err("Performance history is full".into());
    }
    // Reject a torn append rather than concatenate a new row onto it.
    if path.try_exists().map_err(|e| e.to_string())? {
        let mut tail = Vec::new();
        fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut tail)
            .map_err(|e| e.to_string())?;
        if tail.len() > 1024 * 1024 || tail.last() != Some(&b'\n') {
            return Err("Torn/oversized performance tail retained; no append".into());
        }
    }
    let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
pub(crate) fn rates(samples: &[Sample], kind: &str, workers: usize) -> Vec<f64> {
    samples
        .iter()
        .filter(|s| s.kind == kind && (kind != "processing" || s.workers == workers))
        .map(|s| s.elapsed_ms as f64 / 1000.0 / s.units as f64)
        .collect()
}

pub(crate) fn projected_package_bytes(samples: &[Sample], labels: u64) -> Option<u64> {
    let sample = samples
        .iter()
        .rev()
        .find(|s| s.kind == "packaging" && s.labels > 0)?;
    let estimate = (sample.units as f64 / sample.labels as f64 * labels as f64).ceil();
    (estimate.is_finite() && estimate >= 0.0 && estimate <= u64::MAX as f64)
        .then_some(estimate as u64)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timings_are_bounded_separate_worker_rates_and_reject_partial_append() {
        let p = ProjectState::create_without_session(std::env::temp_dir().join(format!(
                "fv-timing-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
        .unwrap();
        record(&p, "processing", 2, 10000, 4).unwrap();
        record(&p, "processing", 2, 20000, 8).unwrap();
        record(&p, "packaging", 1000, 2000, 4).unwrap();
        let all = samples(&p).unwrap();
        assert_eq!(rates(&all, "processing", 4), vec![5.0]);
        assert_eq!(rates(&all, "processing", 8), vec![10.0]);
        assert_eq!(rates(&all, "packaging", 12), vec![0.002]);
        let file = p.logs_dir().join(FILE);
        let mut f = OpenOptions::new().append(true).open(&file).unwrap();
        f.write_all(b"{partial").unwrap();
        drop(f);
        assert_eq!(samples(&p).unwrap().len(), 3);
        assert!(record(&p, "processing", 1, 5000, 4).is_err());
        fs::remove_dir_all(p.root()).unwrap();
    }
}
