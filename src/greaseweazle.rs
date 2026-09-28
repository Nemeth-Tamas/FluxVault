use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::{
    external_tools::{self, CommandAudit},
    safety::MediaSafetyPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GreaseweazleProfile {
    Ibm1440,
    Ibm720,
}

impl GreaseweazleProfile {
    pub fn argument(self) -> &'static str {
        match self {
            Self::Ibm1440 => "ibm.1440",
            Self::Ibm720 => "ibm.720",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "ibm.1440" | "1440" | "1.44" => Ok(Self::Ibm1440),
            "ibm.720" | "720" => Ok(Self::Ibm720),
            _ => Err(format!(
                "Unsupported Greaseweazle profile {value}; choose ibm.1440 or ibm.720"
            )),
        }
    }

    pub fn expected_sector_image_bytes(self) -> u64 {
        match self {
            Self::Ibm1440 => 1_474_560,
            Self::Ibm720 => 737_280,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreaseweazleCommand {
    arguments: Vec<String>,
}

impl GreaseweazleCommand {
    pub fn subcommand(&self) -> &str {
        self.arguments
            .first()
            .map(|value| value.as_str())
            .unwrap_or("")
    }

    pub fn info() -> Self {
        Self {
            arguments: vec!["info".to_owned()],
        }
    }

    pub fn raw_flux_read(
        profile: GreaseweazleProfile,
        drive: char,
        revolutions: u32,
        output_path: &Path,
    ) -> Result<Self, String> {
        MediaSafetyPolicy::assert_invariants();
        let drive = validate_drive(drive)?;
        if revolutions == 0 {
            return Err("A flux capture revolution count must be at least 1.".to_owned());
        }
        if !output_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("scp"))
        {
            return Err(
                "FluxVault preservation capture currently requires an .scp output file.".to_owned(),
            );
        }

        Self::new_checked(vec![
            "read".to_owned(),
            format!("--format={}", profile.argument()),
            "--raw".to_owned(),
            "--no-clobber".to_owned(),
            format!("--drive={drive}"),
            format!("--revs={revolutions}"),
            output_path.display().to_string(),
        ])
    }

    pub fn convert_flux_to_sector_image(
        profile: GreaseweazleProfile,
        input_path: &Path,
        output_path: &Path,
    ) -> Result<Self, String> {
        MediaSafetyPolicy::assert_invariants();
        if input_path == output_path {
            return Err("A derived image cannot replace its raw-flux source.".to_owned());
        }
        if !input_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("scp"))
        {
            return Err("Greaseweazle conversion currently requires an .scp source.".to_owned());
        }
        if !output_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("img") || extension.eq_ignore_ascii_case("ima")
            })
        {
            return Err("The derived sector image must use .img or .ima.".to_owned());
        }

        Self::new_checked(vec![
            "convert".to_owned(),
            format!("--format={}", profile.argument()),
            "--no-clobber".to_owned(),
            input_path.display().to_string(),
            output_path.display().to_string(),
        ])
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub fn from_raw_arguments(arguments: Vec<String>) -> Result<Self, String> {
        Self::new_checked(arguments)
    }

    fn new_checked(arguments: Vec<String>) -> Result<Self, String> {
        let command = Self { arguments };
        command.validate_safe()?;
        Ok(command)
    }

    fn validate_safe(&self) -> Result<(), String> {
        let Some(subcommand) = self.arguments.first().map(|value| value.as_str()) else {
            return Err("Empty Greaseweazle command.".to_owned());
        };
        if !matches!(subcommand, "info" | "read" | "convert") {
            return Err(format!(
                "Greaseweazle operation is not allowed by the read-only policy: {subcommand}"
            ));
        }
        let forbidden = ["write", "erase", "clean", "update"];
        if let Some(argument) = self.arguments.iter().find(|argument| {
            let stripped = argument.trim_start_matches('-');
            forbidden
                .iter()
                .any(|forbidden| stripped.eq_ignore_ascii_case(forbidden))
        }) {
            return Err(format!(
                "Forbidden Greaseweazle argument in read-only mode: {argument}"
            ));
        }
        if subcommand == "read"
            && self
                .arguments
                .iter()
                .any(|argument| argument.starts_with("--format="))
            && !self.arguments.iter().any(|argument| argument == "--raw")
        {
            return Err(
                "A formatted raw-flux read must include --raw to prevent regenerated flux."
                    .to_owned(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendMode {
    Process,
    MockNoHardware,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GreaseweazleDeviceStatus {
    Connected,
    NotFound,
    Bootloader,
    Unknown,
}

impl GreaseweazleDeviceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::NotFound => "not_found",
            Self::Bootloader => "bootloader",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GreaseweazleDeviceInfo {
    pub status: GreaseweazleDeviceStatus,
    pub host_tools_version: Option<String>,
    pub port: Option<String>,
    pub model: Option<String>,
    pub firmware: Option<String>,
}

pub fn parse_info_output(stdout: &str) -> GreaseweazleDeviceInfo {
    let mut host_tools_version = None;
    let mut port = None;
    let mut model = None;
    let mut firmware = None;
    let mut in_device = false;
    let mut device_not_found = false;
    let mut is_bootloader = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(ver) = trimmed.strip_prefix("Host Tools:") {
            host_tools_version = Some(ver.trim().to_owned());
            continue;
        }
        if trimmed == "Device:" {
            in_device = true;
            continue;
        }
        if in_device {
            if trimmed.eq_ignore_ascii_case("Not found") {
                device_not_found = true;
                continue;
            }
            if let Some(val) = trimmed.strip_prefix("Port:") {
                port = Some(val.trim().to_owned());
            } else if let Some(val) = trimmed.strip_prefix("Model:") {
                model = Some(val.trim().to_owned());
            } else if let Some(val) = trimmed.strip_prefix("Firmware:") {
                let fw = val.trim();
                if fw.contains("Bootloader") {
                    is_bootloader = true;
                }
                firmware = Some(fw.to_owned());
            }
        }
    }

    let status = if device_not_found {
        GreaseweazleDeviceStatus::NotFound
    } else if is_bootloader {
        GreaseweazleDeviceStatus::Bootloader
    } else if model.is_some() && firmware.is_some() {
        GreaseweazleDeviceStatus::Connected
    } else {
        GreaseweazleDeviceStatus::Unknown
    };

    GreaseweazleDeviceInfo {
        status,
        host_tools_version,
        port,
        model,
        firmware,
    }
}

pub fn parse_info_host_version(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("Host Tools:")
            .map(|v| v.trim().to_owned())
    })
}

/// `gw info` has historically returned exit code zero with `Device: Not found`.
/// Require actual device fields before treating an info result as connected.
pub fn classify_info_output(stdout: &str) -> GreaseweazleDeviceStatus {
    parse_info_output(stdout).status
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GreaseweazleProgressEvent {
    Track {
        cylinder: u32,
        head: u32,
        mode: String,
        detail: String,
    },
    ReadingRange {
        cylinders: String,
        heads: String,
        revolutions: Option<u32>,
    },
    Converting {
        source: String,
        destination: String,
    },
    Summary {
        found: usize,
        total: usize,
    },
    Warning(String),
    Error(String),
    Other(String),
}

impl GreaseweazleProgressEvent {
    pub fn display_progress(&self) -> Option<String> {
        match self {
            Self::Track {
                cylinder,
                head,
                detail,
                ..
            } => Some(format!("gw: T{cylinder}.{head}: {detail}")),
            Self::ReadingRange {
                cylinders,
                heads,
                revolutions,
            } => Some(format!(
                "gw: Reading c={cylinders}:h={heads}{}",
                revolutions
                    .map(|r| format!(" revs={r}"))
                    .unwrap_or_default()
            )),
            Self::Converting {
                source,
                destination,
            } => Some(format!("gw: Converting {source} -> {destination}")),
            Self::Summary { found, total } => Some(format!("gw: Found {found} sectors of {total}")),
            Self::Warning(msg) => Some(format!("gw warning: {msg}")),
            Self::Error(msg) => Some(format!("gw error: {msg}")),
            Self::Other(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(format!("gw: {trimmed}"))
                }
            }
        }
    }
}

pub fn parse_progress_line(line: &str) -> GreaseweazleProgressEvent {
    let trimmed = line.trim();
    if trimmed.starts_with("** WARNING:") || trimmed.starts_with("WARNING:") {
        return GreaseweazleProgressEvent::Warning(trimmed.to_owned());
    }
    if trimmed.starts_with("** ERROR:") || trimmed.starts_with("ERROR:") {
        return GreaseweazleProgressEvent::Error(trimmed.to_owned());
    }
    if let Some(rest) = trimmed.strip_prefix('T') {
        if let Some((chs, detail)) = rest.split_once(':') {
            if let Some((cyl_str, head_str)) = chs.split_once('.') {
                if let (Ok(cyl), Ok(head)) = (
                    cyl_str.trim().parse::<u32>(),
                    head_str.trim().parse::<u32>(),
                ) {
                    let detail = detail.trim().to_owned();
                    let mode = detail
                        .split_once('(')
                        .map(|(m, _)| m.trim().to_owned())
                        .unwrap_or_else(|| detail.clone());
                    return GreaseweazleProgressEvent::Track {
                        cylinder: cyl,
                        head,
                        mode,
                        detail,
                    };
                }
            }
        }
    }
    if let Some(rest) = trimmed.strip_prefix("Reading ") {
        let mut parts = rest.split_whitespace();
        if let Some(ch) = parts.next() {
            if let Some((c, h)) = ch.split_once(':') {
                let cylinders = c.strip_prefix("c=").unwrap_or(c).to_owned();
                let heads = h.strip_prefix("h=").unwrap_or(h).to_owned();
                let revolutions = parts
                    .find_map(|p| p.strip_prefix("revs="))
                    .and_then(|r| r.parse::<u32>().ok());
                return GreaseweazleProgressEvent::ReadingRange {
                    cylinders,
                    heads,
                    revolutions,
                };
            }
        }
    }
    if let Some(rest) = trimmed.strip_prefix("Converting ") {
        if let Some((src, dst)) = rest.split_once(" -> ") {
            return GreaseweazleProgressEvent::Converting {
                source: src.trim().to_owned(),
                destination: dst.trim().to_owned(),
            };
        }
    }
    if let Some(rest) = trimmed.strip_prefix("Found ") {
        if let Some((found_str, total_rest)) = rest.split_once(" sectors of ") {
            let total_str = total_rest.split_whitespace().next().unwrap_or(total_rest);
            if let (Ok(found), Ok(total)) = (
                found_str.trim().parse::<usize>(),
                total_str.trim().parse::<usize>(),
            ) {
                return GreaseweazleProgressEvent::Summary { found, total };
            }
        }
    }
    GreaseweazleProgressEvent::Other(trimmed.to_owned())
}

#[derive(Debug, Clone)]
pub struct GreaseweazleExecution {
    pub mode: BackendMode,
    pub command: GreaseweazleCommand,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub host_version: Option<String>,
    pub started_unix_ms: u64,
    pub duration_ms: u128,
}

pub trait GreaseweazleBackend {
    fn mode(&self) -> BackendMode;
    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String>;
}

pub type GreaseweazleProgressCallback = Box<dyn FnMut(&GreaseweazleProgressEvent) + Send>;

pub struct ProcessGreaseweazleBackend {
    executable: PathBuf,
    audit_path: PathBuf,
    timeout: Option<Duration>,
    stream_to_stderr: bool,
    host_version: Option<String>,
    progress_callback: Option<GreaseweazleProgressCallback>,
    extra_envs: HashMap<String, String>,
}

enum StreamMessage {
    StdoutLine { event: GreaseweazleProgressEvent },
    StderrLine(String),
}

impl ProcessGreaseweazleBackend {
    pub fn new(executable: PathBuf, audit_path: PathBuf) -> Result<Self, String> {
        if !executable.is_file() {
            return Err(format!(
                "A Greaseweazle executable nem található: {}",
                executable.display()
            ));
        }
        Ok(Self {
            executable,
            audit_path,
            timeout: None,
            stream_to_stderr: true,
            host_version: None,
            progress_callback: None,
            extra_envs: HashMap::new(),
        })
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_envs.insert(key.into(), value.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }

    pub fn with_stream_to_stderr(mut self, stream: bool) -> Self {
        self.stream_to_stderr = stream;
        self
    }

    pub fn set_stream_to_stderr(&mut self, stream: bool) {
        self.stream_to_stderr = stream;
    }

    pub fn with_host_version(mut self, version: impl Into<String>) -> Self {
        self.host_version = Some(version.into());
        self
    }

    pub fn set_host_version(&mut self, version: Option<String>) {
        self.host_version = version;
    }

    pub fn host_version(&self) -> Option<&str> {
        self.host_version.as_deref()
    }

    pub fn set_progress_callback(&mut self, callback: Option<GreaseweazleProgressCallback>) {
        self.progress_callback = callback;
    }

    fn handle_stream_message(&mut self, msg: StreamMessage) {
        match msg {
            StreamMessage::StdoutLine { event } => {
                if let Some(cb) = &mut self.progress_callback {
                    cb(&event);
                }
                if self.stream_to_stderr {
                    if let Some(formatted) = event.display_progress() {
                        eprintln!("{formatted}");
                    }
                }
            }
            StreamMessage::StderrLine(line) => {
                if let Some(cb) = &mut self.progress_callback {
                    cb(&GreaseweazleProgressEvent::Error(line.clone()));
                }
                if self.stream_to_stderr && !line.is_empty() {
                    eprintln!("gw [stderr]: {line}");
                }
            }
        }
    }
}

fn query_host_version(executable: &Path) -> Option<String> {
    let output = Command::new(executable)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    external_tools::first_non_empty_line(&stdout_text)
        .or_else(|| external_tools::first_non_empty_line(&stderr_text))
        .map(str::to_owned)
}

impl GreaseweazleBackend for ProcessGreaseweazleBackend {
    fn mode(&self) -> BackendMode {
        BackendMode::Process
    }

    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String> {
        MediaSafetyPolicy::assert_invariants();
        command.validate_safe()?;

        if self.host_version.is_none() && command.subcommand() != "info" {
            self.host_version = query_host_version(&self.executable);
        }

        let timeout = self.timeout.unwrap_or_else(|| match command.subcommand() {
            "info" => Duration::from_secs(15),
            "read" => Duration::from_secs(300),
            "convert" => Duration::from_secs(60),
            _ => Duration::from_secs(60),
        });

        let started_unix_ms = external_tools::current_unix_ms();
        let started = Instant::now();

        let mut cmd = Command::new(&self.executable);
        cmd.args(command.arguments())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, val) in &self.extra_envs {
            cmd.env(key, val);
        }
        let mut child = cmd
            .spawn()
            .map_err(|error| format!("A Greaseweazle indítása sikertelen: {error}"))?;

        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture Greaseweazle stdout".to_owned())?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Failed to capture Greaseweazle stderr".to_owned())?;

        let (event_tx, event_rx) = mpsc::channel();
        let event_tx_stdout = event_tx.clone();
        let event_tx_stderr = event_tx;

        let stdout_handle = thread::spawn(move || {
            let mut collected = String::new();
            let mut reader = BufReader::new(child_stdout);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line) {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_owned();
                let event = parse_progress_line(&trimmed);
                let _ = event_tx_stdout.send(StreamMessage::StdoutLine { event });
                collected.push_str(&line);
                line.clear();
            }
            collected
        });

        let stderr_handle = thread::spawn(move || {
            let mut collected = String::new();
            let mut reader = BufReader::new(child_stderr);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line) {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_owned();
                let _ = event_tx_stderr.send(StreamMessage::StderrLine(trimmed));
                collected.push_str(&line);
                line.clear();
            }
            collected
        });

        let mut timed_out = false;
        let mut termination_issue = None;
        let exit_status = loop {
            while let Ok(msg) = event_rx.try_recv() {
                self.handle_stream_message(msg);
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    break Some(status);
                }
                Ok(None) if started.elapsed() >= timeout => {
                    timed_out = true;
                    termination_issue = external_tools::terminate_process_tree(&mut child).err();
                    break child.wait().ok();
                }
                Ok(None) => thread::sleep(Duration::from_millis(20)),
                Err(error) => {
                    let issue = external_tools::terminate_process_tree(&mut child).err();
                    let _ = child.wait();
                    return Err(format!(
                        "Greaseweazle folyamat várakozási hiba: {error}{}",
                        issue
                            .map(|i| format!(" (termination error: {i})"))
                            .unwrap_or_default()
                    ));
                }
            }
        };

        while let Ok(msg) = event_rx.recv_timeout(Duration::from_millis(50)) {
            self.handle_stream_message(msg);
        }

        let stdout_text = stdout_handle.join().unwrap_or_default();
        let mut stderr_text = stderr_handle.join().unwrap_or_default();

        if timed_out {
            let msg = format!(
                "\nProcess timed out after {}s; process tree terminated{}",
                timeout.as_secs(),
                termination_issue
                    .as_ref()
                    .map(|issue| format!(" (issue: {issue})"))
                    .unwrap_or_default()
            );
            stderr_text.push_str(&msg);
            if self.stream_to_stderr {
                eprintln!("gw: {msg}");
            }
        }

        let duration_ms = started.elapsed().as_millis();
        let success = exit_status.as_ref().is_some_and(|s| s.success()) && !timed_out;
        let exit_code = exit_status.and_then(|s| s.code());

        let stdout_trimmed = stdout_text.trim().to_owned();
        let stderr_trimmed = stderr_text.trim().to_owned();

        if self.host_version.is_none() && command.subcommand() == "info" {
            self.host_version = parse_info_host_version(&stdout_trimmed);
        }

        let audit = CommandAudit {
            tool: "Greaseweazle".to_owned(),
            executable: self.executable.clone(),
            arguments: command.arguments().to_vec(),
            started_unix_ms,
            duration_ms,
            success,
            exit_code,
            stdout: stdout_trimmed.clone(),
            stderr: stderr_trimmed.clone(),
            version: self.host_version.clone(),
        };

        let audit_error = external_tools::append_audit(&self.audit_path, &audit).err();
        if let Some(error) = audit_error {
            return Err(format!("A Greaseweazle parancsnapló nem írható: {error}"));
        }

        Ok(GreaseweazleExecution {
            mode: BackendMode::Process,
            command: command.clone(),
            success,
            exit_code,
            stdout: stdout_trimmed,
            stderr: stderr_trimmed,
            timed_out,
            host_version: self.host_version.clone(),
            started_unix_ms,
            duration_ms,
        })
    }
}

#[derive(Debug, Default)]
pub struct MockGreaseweazleBackend {
    commands: Vec<GreaseweazleCommand>,
}

impl MockGreaseweazleBackend {
    pub fn commands(&self) -> &[GreaseweazleCommand] {
        &self.commands
    }
}

impl GreaseweazleBackend for MockGreaseweazleBackend {
    fn mode(&self) -> BackendMode {
        BackendMode::MockNoHardware
    }

    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String> {
        command.validate_safe()?;
        self.commands.push(command.clone());
        Ok(GreaseweazleExecution {
            mode: BackendMode::MockNoHardware,
            command: command.clone(),
            success: true,
            exit_code: Some(0),
            stdout: "Mock/no-hardware command accepted; nothing was executed.".to_owned(),
            stderr: String::new(),
            timed_out: false,
            host_version: Some("1.23-mock".to_owned()),
            started_unix_ms: external_tools::current_unix_ms(),
            duration_ms: 0,
        })
    }
}

fn validate_drive(drive: char) -> Result<char, String> {
    let drive = drive.to_ascii_uppercase();
    if matches!(drive, 'A' | 'B') {
        Ok(drive)
    } else {
        Err("Greaseweazle drive must be A or B.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_parser_does_not_trust_zero_exit_or_host_version_alone() {
        assert_eq!(
            classify_info_output("Host Tools: 1.23\nDevice:\n  Not found\n"),
            GreaseweazleDeviceStatus::NotFound
        );
        assert_eq!(
            classify_info_output(
                "Host Tools: 1.23\nDevice:\n  Port: COM3\n  Model: Greaseweazle V4\n  Firmware: 1.23\n"
            ),
            GreaseweazleDeviceStatus::Connected
        );
        assert_eq!(
            classify_info_output("Host Tools: 1.23\nDevice:\n"),
            GreaseweazleDeviceStatus::Unknown
        );
        assert_eq!(
            classify_info_output(
                "Device:\n  Model: Greaseweazle V4\n  Firmware: 1.23 (Bootloader)\n"
            ),
            GreaseweazleDeviceStatus::Bootloader
        );
        assert_eq!(
            classify_info_output("Mock/no-hardware command accepted"),
            GreaseweazleDeviceStatus::Unknown
        );
    }

    #[test]
    fn raw_flux_read_always_combines_format_with_raw_and_no_clobber() {
        let command = GreaseweazleCommand::raw_flux_read(
            GreaseweazleProfile::Ibm1440,
            'a',
            3,
            Path::new("disk001.scp"),
        )
        .unwrap();

        assert_eq!(command.arguments()[0], "read");
        assert!(command.arguments().iter().any(|arg| arg == "--raw"));
        assert!(
            command
                .arguments()
                .iter()
                .any(|arg| arg == "--format=ibm.1440")
        );
        assert!(command.arguments().iter().any(|arg| arg == "--no-clobber"));
        assert!(!command.arguments().iter().any(|arg| arg == "write"));
    }

    #[test]
    fn supports_both_required_ibm_profiles() {
        let hd = GreaseweazleCommand::raw_flux_read(
            GreaseweazleProfile::Ibm1440,
            'A',
            2,
            Path::new("hd.scp"),
        )
        .unwrap();
        let dd = GreaseweazleCommand::raw_flux_read(
            GreaseweazleProfile::Ibm720,
            'B',
            2,
            Path::new("dd.scp"),
        )
        .unwrap();
        assert!(hd.arguments().contains(&"--format=ibm.1440".to_owned()));
        assert!(dd.arguments().contains(&"--format=ibm.720".to_owned()));
    }

    #[test]
    fn conversion_cannot_replace_raw_source() {
        let source = Path::new("disk.scp");
        let result = GreaseweazleCommand::convert_flux_to_sector_image(
            GreaseweazleProfile::Ibm1440,
            source,
            source,
        );
        assert!(result.is_err());
    }

    #[test]
    fn mock_backend_records_without_executing() {
        let command = GreaseweazleCommand::info();
        let mut backend = MockGreaseweazleBackend::default();
        let execution = backend.execute(&command).unwrap();
        assert_eq!(backend.mode(), BackendMode::MockNoHardware);
        assert_eq!(backend.commands(), &[command]);
        assert!(execution.success);
        assert_eq!(execution.mode, BackendMode::MockNoHardware);
    }

    #[test]
    fn unsafe_subcommands_are_rejected_even_inside_module() {
        for forbidden in ["write", "erase", "clean", "update"] {
            assert!(
                GreaseweazleCommand::new_checked(vec![forbidden.to_owned()]).is_err(),
                "{forbidden} must be rejected"
            );
        }
    }

    #[test]
    fn formatted_read_without_raw_is_rejected() {
        let result = GreaseweazleCommand::new_checked(vec![
            "read".to_owned(),
            "--format=ibm.1440".to_owned(),
            "unsafe.scp".to_owned(),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_info_output_extracts_all_fields() {
        let stdout =
            "Host Tools: 1.23\nDevice:\n  Port: COM4\n  Model: Greaseweazle V4\n  Firmware: 1.24\n";
        let info = parse_info_output(stdout);
        assert_eq!(info.status, GreaseweazleDeviceStatus::Connected);
        assert_eq!(info.host_tools_version.as_deref(), Some("1.23"));
        assert_eq!(info.port.as_deref(), Some("COM4"));
        assert_eq!(info.model.as_deref(), Some("Greaseweazle V4"));
        assert_eq!(info.firmware.as_deref(), Some("1.24"));
    }

    #[test]
    fn parse_progress_line_recognizes_gw_output_patterns() {
        let track = parse_progress_line("T0.0: Raw Flux (28123 flux in 200.1ms)");
        assert_eq!(
            track,
            GreaseweazleProgressEvent::Track {
                cylinder: 0,
                head: 0,
                mode: "Raw Flux".to_owned(),
                detail: "Raw Flux (28123 flux in 200.1ms)".to_owned(),
            }
        );
        assert_eq!(
            track.display_progress(),
            Some("gw: T0.0: Raw Flux (28123 flux in 200.1ms)".to_owned())
        );

        let reading = parse_progress_line("Reading c=0-79:h=0-1 revs=3");
        assert_eq!(
            reading,
            GreaseweazleProgressEvent::ReadingRange {
                cylinders: "0-79".to_owned(),
                heads: "0-1".to_owned(),
                revolutions: Some(3),
            }
        );
        assert_eq!(
            reading.display_progress(),
            Some("gw: Reading c=0-79:h=0-1 revs=3".to_owned())
        );

        let converting = parse_progress_line("Converting in.scp -> out.img");
        assert_eq!(
            converting,
            GreaseweazleProgressEvent::Converting {
                source: "in.scp".to_owned(),
                destination: "out.img".to_owned(),
            }
        );
        assert_eq!(
            converting.display_progress(),
            Some("gw: Converting in.scp -> out.img".to_owned())
        );

        let summary = parse_progress_line("Found 2880 sectors of 2880 (100%)");
        assert_eq!(
            summary,
            GreaseweazleProgressEvent::Summary {
                found: 2880,
                total: 2880,
            }
        );
        assert_eq!(
            summary.display_progress(),
            Some("gw: Found 2880 sectors of 2880".to_owned())
        );

        let warning = parse_progress_line("** WARNING: weak flux on track 1");
        assert_eq!(
            warning,
            GreaseweazleProgressEvent::Warning("** WARNING: weak flux on track 1".to_owned())
        );

        let error = parse_progress_line("** ERROR: motor speed unstable");
        assert_eq!(
            error,
            GreaseweazleProgressEvent::Error("** ERROR: motor speed unstable".to_owned())
        );

        let empty = parse_progress_line("   ");
        assert_eq!(empty.display_progress(), None);
    }
}
