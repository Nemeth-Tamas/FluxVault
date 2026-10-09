//! Optional workstation-only cues. Never block custody, touch media, or emit BEL/ANSI bytes.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum SoundMode {
    #[default]
    Off,
    On,
}

impl SoundMode {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "off" => Ok(Self::Off),
            "on" => Ok(Self::On),
            _ => Err("--sound requires on or off".into()),
        }
    }

    pub(super) fn enabled(self, terminal: bool, dumb: bool) -> bool {
        self == Self::On && terminal && !dumb && cfg!(windows)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Station {
    Usb,
    Greaseweazle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Saved,
    PartialSaved,
    Failed,
}

#[derive(Clone, Copy, Debug)]
struct Notice {
    station: Station,
    outcome: Outcome,
    queued: Instant,
}

impl Notice {
    fn tones(self) -> Vec<(u32, u32)> {
        let pitch = match self.station {
            Station::Usb => 660,
            Station::Greaseweazle => 990,
        };
        match self.outcome {
            Outcome::Saved => vec![(pitch, 100)],
            Outcome::PartialSaved => vec![(pitch, 100), (330, 120)],
            Outcome::Failed => vec![(pitch, 100), (220, 100), (220, 100)],
        }
    }
}

/// One bounded, disposable worker per scan. Notices are best-effort and may be
/// dropped when busy; banners are always the authoritative removal instructions.
pub(super) struct Cues {
    sender: Option<SyncSender<Notice>>,
    stopping: Arc<AtomicBool>,
}

impl Cues {
    pub(super) fn start(enabled: bool) -> Self {
        Self::with_player(enabled, play)
    }

    fn with_player(enabled: bool, player: impl Fn(u32, u32) + Send + 'static) -> Self {
        let stopping = Arc::new(AtomicBool::new(false));
        let sender = if enabled {
            let (sender, receiver) = mpsc::sync_channel(2);
            let stop = stopping.clone();
            thread::Builder::new()
                .name("fluxvault-sound-cues".into())
                .spawn(move || worker(receiver, &stop, player))
                .ok()
                .map(|_| sender)
        } else {
            None
        };
        Self { sender, stopping }
    }

    pub(super) fn notify(&self, station: Station, outcome: Outcome) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(Notice {
                station,
                outcome,
                queued: Instant::now(),
            });
        }
    }
}

impl Drop for Cues {
    fn drop(&mut self) {
        // Do not join or wait for the audio device during shutdown/cancellation.
        // The sender closes on drop; discard queued/stale notices immediately.
        self.stopping.store(true, Ordering::Release);
    }
}

fn worker(receiver: Receiver<Notice>, stop: &AtomicBool, player: impl Fn(u32, u32)) {
    while let Ok(notice) = receiver.recv() {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if notice.queued.elapsed() > Duration::from_secs(2) {
            continue;
        }
        for (index, (frequency, duration)) in notice.tones().into_iter().enumerate() {
            if index > 0 {
                thread::sleep(Duration::from_millis(30));
            }
            if stop.load(Ordering::Acquire) {
                break;
            }
            player(frequency, duration);
        }
    }
}

fn play(frequency: u32, duration: u32) {
    #[cfg(windows)]
    // SAFETY: Beep takes bounded numeric values, no handles or memory pointers.
    // Audio failure is intentionally non-fatal; it cannot affect acquisition.
    unsafe {
        let _ = windows::Win32::System::Diagnostics::Debug::Beep(frequency, duration);
    }
    #[cfg(not(windows))]
    let _ = (frequency, duration);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn sound_is_explicit_terminal_only_and_independent_of_color() {
        assert_eq!(SoundMode::default(), SoundMode::Off);
        assert_eq!(SoundMode::parse("on").unwrap(), SoundMode::On);
        assert_eq!(SoundMode::parse("off").unwrap(), SoundMode::Off);
        assert!(SoundMode::parse("always").is_err());
        assert!(!SoundMode::Off.enabled(true, false));
        assert!(!SoundMode::On.enabled(false, false));
        assert!(!SoundMode::On.enabled(true, true));
        assert_eq!(SoundMode::On.enabled(true, false), cfg!(windows));
    }

    #[test]
    fn all_station_outcome_patterns_are_distinct_and_bounded() {
        let mut patterns = Vec::new();
        for station in [Station::Usb, Station::Greaseweazle] {
            for outcome in [Outcome::Saved, Outcome::PartialSaved, Outcome::Failed] {
                let tones = Notice {
                    station,
                    outcome,
                    queued: Instant::now(),
                }
                .tones();
                assert!(
                    tones
                        .iter()
                        .all(|(hz, ms)| (37..=32767).contains(hz) && *ms <= 120)
                );
                assert!(tones.iter().map(|(_, ms)| ms).sum::<u32>() <= 300);
                assert!(!patterns.contains(&tones));
                patterns.push(tones);
            }
        }
    }

    #[test]
    fn stale_and_shutdown_notices_do_not_play() {
        let (tx, rx) = mpsc::sync_channel(2);
        tx.send(Notice {
            station: Station::Usb,
            outcome: Outcome::Saved,
            queued: Instant::now() - Duration::from_secs(3),
        })
        .unwrap();
        drop(tx);
        worker(rx, &AtomicBool::new(false), |_, _| {
            panic!("Stale notice played")
        });
        let (tx, rx) = mpsc::sync_channel(2);
        tx.send(Notice {
            station: Station::Usb,
            outcome: Outcome::Saved,
            queued: Instant::now(),
        })
        .unwrap();
        drop(tx);
        worker(rx, &AtomicBool::new(true), |_, _| {
            panic!("Shutdown notice played")
        });
    }

    #[test]
    fn disabled_cues_never_start_player() {
        let cues = Cues::with_player(false, |_, _| panic!("Disabled player ran"));
        assert!(cues.sender.is_none());
        cues.notify(Station::Usb, Outcome::Saved);
    }

    #[test]
    fn busy_audio_never_blocks_feeding_and_drop_discards_remaining_tones() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (ended_tx, ended_rx) = mpsc::channel();
        let tones = Arc::new(Mutex::new(Vec::new()));
        let captured = tones.clone();
        let cues = Cues::with_player(true, move |hz, ms| {
            captured.lock().unwrap().push((hz, ms));
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            ended_tx.send(()).unwrap();
        });
        cues.notify(Station::Greaseweazle, Outcome::Failed);
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..100 {
            cues.notify(Station::Usb, Outcome::Saved);
        }
        assert_eq!(tones.lock().unwrap().len(), 1);
        drop(cues); // Must not wait for the blocked player.
        release_tx.send(()).unwrap();
        ended_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // No additional tone can start after Drop set the cancellation flag.
        assert_eq!(*tones.lock().unwrap(), vec![(990, 100)]);
    }
}
