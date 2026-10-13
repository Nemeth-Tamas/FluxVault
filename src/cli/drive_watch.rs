use super::CliResponse;
use crate::{floppy, media_change};
use serde_json::json;
pub(super) fn run(args: &[String]) -> Result<CliResponse, String> {
    let watch = args.get(1).is_some_and(|a| a == "watch");
    let mut drive = None;
    let mut samples = if watch { 20 } else { 1 };
    let mut interval = 1000u64;
    let mut json_output = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut i = 2;
    while i < args.len() {
        let key = args[i].as_str();
        if !seen.insert(key) {
            return Err(format!("Duplicate media observation option: {key}"));
        }
        match key {
            "--json" => json_output = true,
            "--drive" => {
                i += 1;
                drive = Some(args.get(i).ok_or("--drive requires A:")?.clone());
            }
            "--samples" | "--interval-ms" if watch => {
                i += 1;
                let n = args.get(i).ok_or("Media watch option needs a number")?
                    .parse::<u64>().map_err(|_| "Invalid media watch number")?;
                if key == "--samples" {
                    if !(1..=120).contains(&n) {
                        return Err("--samples requires 1..120".into());
                    }
                    samples = n;
                } else {
                    if !(100..=5000).contains(&n) {
                        return Err("--interval-ms requires 100..5000".into());
                    }
                    interval = n;
                }
            }
            _=>return Err("Use drive media --drive A: [--json], or drive watch --drive A: [--samples 20] [--interval-ms 1000] [--json]".into()),
        }
        i += 1;
    }
    let drive = drive.ok_or("Media observation requires --drive A:")?;
    let reservation = super::media_reservation::UsbReservation::acquire(&drive)?;
    let devices = floppy::enumerate_removable_drives()?;
    let device = super::select_removable_drive(&devices, &drive)?;
    let mut detector = media_change::Detector::default();
    let mut readings = Vec::new();
    let mut events = Vec::new();
    for n in 0..samples {
        crate::cancellation::check()?;
        let observation = media_change::probe(&device);
        let event = detector.update(&observation);
        if let Some(kind) = event {
            events.push(json!({"sample":n+1,"kind":kind}));
            eprintln!(
                "[MEDIA {}] {kind}; metadata observation only, NOT disk identity or permission to move active media",
                device.root
            );
        }
        readings.push(json!({"sample":n+1,"observation":observation}));
        if n + 1 < samples {
            let until = std::time::Instant::now() + std::time::Duration::from_millis(interval);
            while std::time::Instant::now() < until {
                crate::cancellation::check()?;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    drop(reservation);
    let attention = readings
        .iter()
        .any(|r| r["observation"]["state"] == "unknown");
    let value = json!({"drive":device.root,"readings":readings,"stable_events":events,"device_access":"FILE_READ_ATTRIBUTES","device_write_access_requested":false,"data_stream_reads":false,"host_tools_invoked":false,"project_modified":false,"custody_changed":false,"attention_required":attention,"limits":"Driver metadata can be absent/unsupported/reset; unknown or NOT_READY is not proof of removal. Three consistent observations debounce changes. A counter change is not a label fingerprint. This never starts acquisition, acknowledges removal or changes numbering. Stop this watcher before scanning the same USB drive. Kernel-driver call latency is outside the poll interval; physical confirmations remain the reliable fallback."});
    Ok(CliResponse {
        exit_code: if attention { 3 } else { 0 },
        output: if json_output {
            value.to_string()
        } else {
            format!(
                "Metadata-only media observation: {}\nLast observation: {}\nStable events: {}\n{}",
                device.root,
                readings
                    .last()
                    .map(|r| r["observation"].to_string())
                    .unwrap_or_default(),
                events.len(),
                value["limits"].as_str().unwrap()
            )
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_options_refuse_before_device_reservation_or_probe() {
        for a in [
            vec!["drive", "media"],
            vec!["drive", "media", "--samples", "1"],
            vec!["drive", "watch", "--samples", "0"],
            vec!["drive", "watch", "--interval-ms", "99"],
            vec!["drive", "watch", "--json", "--json"],
        ] {
            assert!(run(&a.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }
}
