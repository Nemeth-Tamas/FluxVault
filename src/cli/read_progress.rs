//! A temporary ASCII status line for interactive recovery. Track visitation is
//! not sector recovery/yield. Redirected output keeps ordinary stage messages.
use crate::greaseweazle::{GreaseweazleProgressCallback, GreaseweazleProgressEvent};
use std::{
    collections::BTreeSet,
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

pub(super) type ReadProgress = Progress<io::Stderr>;

pub(super) fn interactive() -> bool {
    io::stderr().is_terminal() && !std::env::var("TERM").is_ok_and(|value| value == "dumb")
}

struct State<W> {
    output: W,
    disk: u32,
    phase: &'static str,
    expected: Option<BTreeSet<(u32, u32)>>,
    visited: BTreeSet<(u32, u32)>,
    width: usize,
}

// Bounded host-reported ranges, including discontiguous targeted rereads.
// Unsupported syntax uses an indeterminate bar, never an invented percentage.
fn range(value: &str, maximum: u32) -> Option<BTreeSet<u32>> {
    if value.len() > 2048 {
        return None;
    }
    let mut result = BTreeSet::new();
    for part in value.split(',') {
        let (first, last) = if let Some((first, last)) = part.split_once('-') {
            (first.parse::<u32>().ok()?, last.parse::<u32>().ok()?)
        } else {
            let n = part.parse::<u32>().ok()?;
            (n, n)
        };
        if first > last || last > maximum {
            return None;
        }
        result.extend(first..=last);
    }
    (!result.is_empty()).then_some(result)
}

impl<W: Write> State<W> {
    fn clear(&mut self) -> io::Result<()> {
        if self.width > 0 {
            write!(self.output, "\r{}\r", " ".repeat(self.width))?;
            self.width = 0;
            self.output.flush()?;
        }
        Ok(())
    }

    fn line(&self, tick: usize, seconds: u64) -> String {
        let spinner = ["|", "/", "-", "\\"][tick % 4];
        let (bar, detail) = if let Some(expected) = &self.expected {
            let done = self.visited.len();
            let filled = done * 12 / expected.len();
            (
                format!("{}{}", "#".repeat(filled), "-".repeat(12 - filled)),
                format!("{done}/{} tracks", expected.len()),
            )
        } else {
            let mut bar = ['-'; 12];
            bar[tick % 12] = '#';
            (bar.into_iter().collect(), "working".into())
        };
        format!(
            "{spinner} {:03} {} [{bar}] {detail} {}:{:02}",
            self.disk,
            self.phase,
            seconds / 60,
            seconds % 60
        )
    }

    fn draw(&mut self, tick: usize, seconds: u64) -> io::Result<()> {
        let line = self.line(tick, seconds);
        let padding = self.width.saturating_sub(line.len());
        write!(self.output, "\r{line}{}", " ".repeat(padding))?;
        self.width = line.len();
        self.output.flush()
    }

    fn message(&mut self, message: &str) {
        let _ = self.clear();
        let _ = writeln!(self.output, "{message}");
        let _ = self.output.flush();
        self.phase = if message.contains("pass: reading") || message.contains("pass: rereading") {
            "Reading"
        } else if message.starts_with("Identifying") {
            "Decoding"
        } else {
            "Verifying"
        };
        self.expected = None;
        self.visited.clear();
    }

    fn event(&mut self, event: &GreaseweazleProgressEvent) {
        match event {
            GreaseweazleProgressEvent::ReadingRange {
                cylinders, heads, ..
            } => {
                self.expected = range(cylinders, 255).zip(range(heads, 1)).map(|(c, h)| {
                    c.into_iter()
                        .flat_map(|c| h.iter().map(move |h| (c, *h)))
                        .collect()
                });
                self.visited.clear();
                if self.phase != "Decoding" {
                    self.phase = "Reading";
                }
            }
            GreaseweazleProgressEvent::Track { cylinder, head, .. } => {
                let track = (*cylinder, *head);
                if self
                    .expected
                    .as_ref()
                    .is_some_and(|set| set.contains(&track))
                {
                    self.visited.insert(track);
                }
            }
            GreaseweazleProgressEvent::Converting { .. } => {
                self.phase = "Decoding";
                self.expected = None;
                self.visited.clear();
            }
            GreaseweazleProgressEvent::Warning(_) | GreaseweazleProgressEvent::Error(_) => {
                // Do not overwrite diagnostics with the next animation frame.
                let _ = self.clear();
                if let Some(message) = event.display_progress() {
                    let _ = writeln!(self.output, "{message}");
                    let _ = self.output.flush();
                }
            }
            _ => {}
        }
    }
}

pub(super) struct Progress<W: Write + Send + 'static> {
    shared: Arc<Mutex<State<W>>>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl<W: Write + Send + 'static> Progress<W> {
    pub(super) fn new(disk: u32, output: W, animate: bool) -> Self {
        let shared = Arc::new(Mutex::new(State {
            output,
            disk,
            phase: "Checking",
            expected: None,
            visited: BTreeSet::new(),
            width: 0,
        }));
        let mut progress = Self {
            shared: shared.clone(),
            stop: None,
            worker: None,
        };
        if animate {
            let (stop, receive) = mpsc::channel();
            if let Ok(worker) = thread::Builder::new()
                .name("fv-read-progress".into())
                .spawn(move || {
                    let started = Instant::now();
                    let mut tick = 0;
                    loop {
                        let Ok(mut state) = shared.lock() else { break };
                        if state.draw(tick, started.elapsed().as_secs()).is_err() {
                            break;
                        }
                        drop(state);
                        tick += 1;
                        if receive.recv_timeout(Duration::from_millis(200))
                            != Err(mpsc::RecvTimeoutError::Timeout)
                        {
                            break;
                        }
                    }
                })
            {
                progress.stop = Some(stop);
                progress.worker = Some(worker);
            }
        }
        progress
    }

    pub(super) fn message(&self, message: &str) {
        if let Ok(mut state) = self.shared.lock() {
            state.message(message);
        }
    }

    pub(super) fn callback(&self) -> GreaseweazleProgressCallback {
        let shared = self.shared.clone();
        Box::new(move |event| {
            if let Ok(mut state) = shared.lock() {
                state.event(event);
            }
        })
    }
}

impl<W: Write + Send + 'static> Drop for Progress<W> {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut state) = self.shared.lock() {
            let _ = state.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::greaseweazle::parse_progress_line;

    fn state() -> State<Vec<u8>> {
        State {
            output: vec![],
            disk: 59,
            phase: "Checking",
            expected: None,
            visited: BTreeSet::new(),
            width: 0,
        }
    }

    #[test]
    fn track_progress_is_unique_bounded_and_resets_for_each_targeted_pass() {
        let mut state = state();
        state.event(&parse_progress_line("Reading c=0-79:h=0-1 revs=2"));
        for line in [
            "T0.0: Raw Flux",
            "T0.0: Raw Flux",
            "T0.1: Raw Flux",
            "T999.1: Raw Flux",
        ] {
            state.event(&parse_progress_line(line));
        }
        assert!(state.line(0, 105).contains("2/160 tracks 1:45"));
        state.event(&parse_progress_line("Reading c=0,1,2,78:h=0-1 revs=5"));
        assert!(state.line(1, 106).contains("0/8 tracks"));
        state.event(&parse_progress_line("T78.1: Raw Flux"));
        assert!(state.line(1, 106).contains("1/8 tracks"));
        state.event(&parse_progress_line("Converting capture.scp -> disk.img"));
        assert!(state.line(2, 107).contains("Decoding"));
        assert!(state.line(2, 107).contains("working"));
        for value in ["80-0", "0-4294967295", "-1", "0-79/2", "garbage", ""] {
            assert!(range(value, 255).is_none());
        }
    }

    #[test]
    fn heartbeat_is_ascii_has_no_fake_percentage_and_clears_before_messages() {
        let mut state = state();
        let first = state.line(0, 0);
        assert_ne!(first, state.line(1, 1));
        assert!(first.contains("working"));
        assert!(!first.contains('%'));
        assert!(first.is_ascii());
        state.draw(0, 0).unwrap();
        let width = state.width;
        state.message("Fast pass: reading the whole floppy");
        let output = String::from_utf8(state.output.clone()).unwrap();
        assert!(output.ends_with(&format!(
            "\r{}\rFast pass: reading the whole floppy\n",
            " ".repeat(width)
        )));
        assert_eq!(state.width, 0);
        state.event(&parse_progress_line("WARNING: weak flux"));
        assert!(
            String::from_utf8(state.output)
                .unwrap()
                .contains("gw warning: WARNING: weak flux")
        );
    }

    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn live_worker_stops_and_erases_on_success_and_error_unwind() {
        for failed in [false, true] {
            let output = Output::default();
            let result: Result<(), &str> = (|| {
                let progress = Progress::new(59, output.clone(), true);
                let deadline = Instant::now() + Duration::from_secs(2);
                while output.0.lock().unwrap().is_empty() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                assert!(!output.0.lock().unwrap().is_empty());
                progress.callback()(&parse_progress_line("Reading c=0-79:h=0-1"));
                if failed {
                    return Err("read failed");
                }
                Ok(())
            })();
            assert_eq!(result.is_err(), failed);
            let bytes = output.0.lock().unwrap().clone();
            assert!(bytes.ends_with(b"\r"));
            assert!(!bytes.ends_with(b"\n"));
            thread::sleep(Duration::from_millis(220));
            assert_eq!(*output.0.lock().unwrap(), bytes);
        }
    }

    #[test]
    fn redirected_output_never_animates_or_adds_control_characters() {
        let output = Output::default();
        {
            let progress = Progress::new(59, output.clone(), false);
            progress.message("Fast pass: reading the whole floppy");
            progress.callback()(&parse_progress_line("Reading c=0-79:h=0-1"));
        }
        assert_eq!(
            *output.0.lock().unwrap(),
            b"Fast pass: reading the whole floppy\n"
        );
    }
}
