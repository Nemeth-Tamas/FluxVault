//! Host-process CPU and sampled resident-memory observations, not hard quotas.
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Debug, Serialize)]
pub struct Measurement {
    pub elapsed_seconds: f64,
    pub process_cpu_seconds: Option<f64>,
    pub sampled_peak_resident_bytes: Option<u64>,
    pub sample_interval_ms: u64,
    pub scope: &'static str,
}
pub(crate) struct Meter {
    started: Instant,
    cpu: Option<f64>,
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}
#[cfg(windows)]
fn sample() -> Option<(f64, u64)> {
    use windows::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .ok()?;
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut memory,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .ok()?;
    }
    let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
    Some((
        (ticks(kernel) + ticks(user)) as f64 / 10_000_000.0,
        memory.WorkingSetSize as u64,
    ))
}
#[cfg(not(windows))]
fn sample() -> Option<(f64, u64)> {
    None
}
impl Meter {
    pub(crate) fn start() -> Self {
        let initial = sample();
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(initial.map_or(0, |v| v.1)));
        let s = stop.clone();
        let p = peak.clone();
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Acquire) {
                if let Some((_, rss)) = sample() {
                    p.fetch_max(rss, Ordering::AcqRel);
                }
                std::thread::park_timeout(Duration::from_millis(20));
            }
        });
        Self {
            started: Instant::now(),
            cpu: initial.map(|v| v.0),
            stop,
            peak,
            thread: Some(thread),
        }
    }
    pub(crate) fn finish(mut self) -> Measurement {
        self.end();
        let last = sample();
        if let Some((_, rss)) = last {
            self.peak.fetch_max(rss, Ordering::AcqRel);
        }
        let peak = self.peak.load(Ordering::Acquire);
        Measurement {
            elapsed_seconds: self.started.elapsed().as_secs_f64(),
            process_cpu_seconds: self.cpu.zip(last).map(|(a, b)| (b.0 - a).max(0.0)),
            sampled_peak_resident_bytes: (peak > 0).then_some(peak),
            sample_interval_ms: 20,
            scope: "whole_current_process_during_stage_includes_other_threads_not_child_tools_or_hard_peak",
        }
    }
    fn end(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}
impl Drop for Meter {
    fn drop(&mut self) {
        self.end();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meters_join_on_error_and_expose_explicit_scope() {
        drop(Meter::start());
        let result = Meter::start().finish();
        assert!(result.elapsed_seconds.is_finite());
        assert_eq!(result.sample_interval_ms, 20);
        #[cfg(windows)]
        {
            assert!(result.process_cpu_seconds.is_some());
            assert!(result.sampled_peak_resident_bytes.unwrap() > 0);
        }
    }
}
