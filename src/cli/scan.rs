//! Guided, single-drive acquisition loop. Every media change requires an operator confirmation.

use std::io::{self, BufRead, Write};

use serde_json::json;

use crate::{
    project::ProjectState,
    recovery_plan::{self, RecoveryAction},
};

use super::{CliResponse, acquire};

struct ScanConfig<'a> {
    json_output: bool,
    drive: Option<&'a str>,
    count: Option<usize>,
    write_blocker_verified: bool,
}

pub(super) fn run(
    project: &mut ProjectState,
    json_output: bool,
    drive: Option<&str>,
    retries: usize,
    count: Option<usize>,
    write_blocker_verified: bool,
) -> Result<CliResponse, String> {
    let stdin = io::stdin();
    let mut stderr = io::stderr().lock();
    run_with_io(
        project,
        ScanConfig {
            json_output,
            drive,
            count,
            write_blocker_verified,
        },
        stdin.lock(),
        &mut stderr,
        |project, disk| acquire::run(project, false, drive, Some(disk), retries, true),
    )
}

fn run_with_io<
    R: BufRead,
    W: Write,
    F: FnMut(&ProjectState, u32) -> Result<CliResponse, String>,
>(
    project: &mut ProjectState,
    config: ScanConfig<'_>,
    mut input: R,
    output: &mut W,
    mut acquire_disk: F,
) -> Result<CliResponse, String> {
    if !config.write_blocker_verified {
        return Err("Scan is blocked until the drive/write blocker has been independently verified with a known-good disposable disk. Do not validate using customer media.".to_owned());
    }
    let drive = config.drive.ok_or("scan requires --drive LETTER:")?;
    if config.count == Some(0) {
        return Err("--count must be positive".to_owned());
    }
    let mut scanned = 0usize;
    let mut partial = 0usize;
    loop {
        if config.count.is_some_and(|limit| scanned >= limit) {
            break;
        }
        let disk = project.current_disk_number();
        let next = disk.checked_add(1).ok_or("Disk number overflow")?;
        writeln!(
            output,
            "Insert floppy {disk:03} in {drive} with its write-protect tab set. Type READ to image it, or QUIT to stop:"
        )
        .map_err(|error| format!("Cannot display scan prompt: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("Cannot flush scan prompt: {error}"))?;
        let mut answer = String::new();
        if input
            .read_line(&mut answer)
            .map_err(|error| format!("Cannot read scan confirmation: {error}"))?
            == 0
        {
            break;
        }
        match answer.trim().to_ascii_uppercase().as_str() {
            "QUIT" | "Q" => break,
            "READ" => {}
            _ => {
                writeln!(output, "No read started. Type READ or QUIT.")
                    .map_err(|error| format!("Cannot display scan prompt: {error}"))?;
                continue;
            }
        }
        let response = acquire_disk(project, disk)?;
        if response.exit_code != 0 && response.exit_code != 3 {
            return Err(format!("Acquisition for floppy {disk:03} did not complete"));
        }
        project.set_current_disk_number_without_session(next)?;
        partial += usize::from(response.exit_code == 3);
        scanned += 1;
        writeln!(output, "{}", response.output)
            .map_err(|error| format!("Cannot display scan result: {error}"))?;
    }
    let queue = recovery_plan::plan_project(&project.images_dir())?
        .into_iter()
        .filter(|plan| plan.action != RecoveryAction::Complete)
        .map(|plan| plan.disk_number)
        .collect::<Vec<_>>();
    Ok(CliResponse {
        output: if config.json_output {
            json!({
                "project": project.root(),
                "scanned": scanned,
                "partial_this_session": partial,
                "next_disk": project.current_disk_number(),
                "recovery_queue": queue,
                "source_media_access": "read_only"
            })
            .to_string()
        } else {
            format!(
                "Scan stopped: {scanned} disk(s) imaged, {partial} partial in this session. Next disk: {:03}. Recovery queue: {}.",
                project.current_disk_number(),
                queue.len()
            )
        },
        exit_code: if partial > 0 || !queue.is_empty() {
            3
        } else {
            0
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Cursor,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn guided_scan_requires_confirmation_and_persists_next_disk() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-cli-scan-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut project = ProjectState::create_without_session(root.clone()).unwrap();
        let mut output = Vec::new();
        let mut attempted = Vec::new();
        let response = run_with_io(
            &mut project,
            ScanConfig {
                json_output: true,
                drive: Some("A:"),
                count: Some(2),
                write_blocker_verified: true,
            },
            Cursor::new(b"wrong\nREAD\nREAD\n"),
            &mut output,
            |_, disk| {
                attempted.push(disk);
                Ok(CliResponse {
                    output: format!("synthetic disk {disk}"),
                    exit_code: 0,
                })
            },
        )
        .unwrap();
        assert_eq!(attempted, vec![1, 2]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response.output).unwrap()["next_disk"],
            3
        );
        assert_eq!(
            ProjectState::open_without_session(root.clone())
                .unwrap()
                .current_disk_number(),
            3
        );
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("No read started")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn guided_scan_rejects_unverified_hardware_before_reading_input_or_media() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-cli-scan-gate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut project = ProjectState::create_without_session(root.clone()).unwrap();
        let result = run_with_io(
            &mut project,
            ScanConfig {
                json_output: false,
                drive: Some("A:"),
                count: None,
                write_blocker_verified: false,
            },
            Cursor::new(b"READ\n"),
            &mut Vec::new(),
            |_, _| panic!("unverified hardware must never be acquired"),
        );
        assert!(result.unwrap_err().contains("independently verified"));
        assert_eq!(project.current_disk_number(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn guided_scan_reports_partial_and_never_advances_after_failed_acquisition() {
        let root = std::env::temp_dir().join(format!(
            "fluxvault-cli-scan-partial-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut project = ProjectState::create_without_session(root.clone()).unwrap();
        let partial = run_with_io(
            &mut project,
            ScanConfig {
                json_output: true,
                drive: Some("A:"),
                count: Some(1),
                write_blocker_verified: true,
            },
            Cursor::new(b"READ\n"),
            &mut Vec::new(),
            |_, _| {
                Ok(CliResponse {
                    output: "synthetic partial".to_owned(),
                    exit_code: 3,
                })
            },
        )
        .unwrap();
        assert_eq!(partial.exit_code, 3);
        assert_eq!(project.current_disk_number(), 2);
        let failed = run_with_io(
            &mut project,
            ScanConfig {
                json_output: true,
                drive: Some("A:"),
                count: Some(1),
                write_blocker_verified: true,
            },
            Cursor::new(b"READ\n"),
            &mut Vec::new(),
            |_, _| Err("synthetic read failure".to_owned()),
        );
        assert_eq!(failed.unwrap_err(), "synthetic read failure");
        assert_eq!(
            ProjectState::open_without_session(root.clone())
                .unwrap()
                .current_disk_number(),
            2
        );
        fs::remove_dir_all(root).unwrap();
    }
}
