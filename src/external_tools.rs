use std::{
    collections::HashSet,
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const SETTINGS_FILE_NAME: &str = "settings.json";
const TOOL_AUDIT_FILE_NAME: &str = "external-tools.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    SevenZip,
    LibreOffice,
    Greaseweazle,
}

impl ToolKind {
    pub const ALL: [Self; 3] = [Self::SevenZip, Self::LibreOffice, Self::Greaseweazle];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::SevenZip => "7-Zip",
            Self::LibreOffice => "LibreOffice",
            Self::Greaseweazle => "Greaseweazle",
        }
    }

    fn executable_names(self) -> &'static [&'static str] {
        match self {
            Self::SevenZip => &["7z.exe", "7zz.exe", "7za.exe"],
            Self::LibreOffice => &["soffice.com", "soffice.exe"],
            Self::Greaseweazle => &["gw.exe"],
        }
    }

    fn version_arguments(self) -> &'static [&'static str] {
        match self {
            Self::SevenZip => &["i"],
            Self::LibreOffice | Self::Greaseweazle => &["--version"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolHealth {
    NotChecked,
    Checking,
    Ready,
    Missing,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandAudit {
    pub tool: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub started_unix_ms: u64,
    pub duration_ms: u128,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub struct ToolStatus {
    pub kind: ToolKind,
    pub health: ToolHealth,
    pub executable: Option<PathBuf>,
    pub version: Option<String>,
    pub detail: String,
    pub audit: Option<CommandAudit>,
    pub audit_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuditedCommandResult {
    pub audit: CommandAudit,
    pub audit_error: Option<String>,
}

impl ToolStatus {
    fn new(kind: ToolKind, health: ToolHealth, detail: impl Into<String>) -> Self {
        Self {
            kind,
            health,
            executable: None,
            version: None,
            detail: detail.into(),
            audit: None,
            audit_error: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolSettings {
    pub seven_zip_path: Option<PathBuf>,
    pub libreoffice_path: Option<PathBuf>,
    pub greaseweazle_path: Option<PathBuf>,
}

impl ToolSettings {
    pub fn path(&self, kind: ToolKind) -> Option<&Path> {
        match kind {
            ToolKind::SevenZip => self.seven_zip_path.as_deref(),
            ToolKind::LibreOffice => self.libreoffice_path.as_deref(),
            ToolKind::Greaseweazle => self.greaseweazle_path.as_deref(),
        }
    }

    pub fn set_path(&mut self, kind: ToolKind, path: Option<PathBuf>) {
        match kind {
            ToolKind::SevenZip => self.seven_zip_path = path,
            ToolKind::LibreOffice => self.libreoffice_path = path,
            ToolKind::Greaseweazle => self.greaseweazle_path = path,
        }
    }
}

#[derive(Debug)]
pub enum ToolCheckEvent {
    Status(ToolStatus),
    Finished,
}

pub fn initial_statuses() -> Vec<ToolStatus> {
    ToolKind::ALL
        .into_iter()
        .map(|kind| ToolStatus::new(kind, ToolHealth::NotChecked, "Még nincs ellenőrizve."))
        .collect()
}

pub fn checking_statuses() -> Vec<ToolStatus> {
    ToolKind::ALL
        .into_iter()
        .map(|kind| ToolStatus::new(kind, ToolHealth::Checking, "Ellenőrzés folyamatban..."))
        .collect()
}

pub fn load_settings() -> Result<ToolSettings, String> {
    let path = settings_file_path();

    if !path.exists() {
        return Ok(ToolSettings::default());
    }

    let json = fs::read_to_string(&path).map_err(|error| {
        format!(
            "Nem sikerült beolvasni a beállításokat {}: {error}",
            path.display()
        )
    })?;

    serde_json::from_str(&json)
        .map_err(|error| format!("Hibás eszközbeállítás-fájl {}: {error}", path.display()))
}

pub fn save_settings(settings: &ToolSettings) -> Result<(), String> {
    let path = settings_file_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Nem sikerült létrehozni a beállítási mappát {}: {error}",
                parent.display()
            )
        })?;
    }

    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("Eszközbeállítás JSON hiba: {error}"))?;

    fs::write(&path, json).map_err(|error| {
        format!(
            "Nem sikerült menteni a beállításokat {}: {error}",
            path.display()
        )
    })
}

pub fn default_audit_path() -> PathBuf {
    app_data_directory().join(TOOL_AUDIT_FILE_NAME)
}

pub fn spawn_checks(settings: ToolSettings, audit_path: PathBuf) -> Receiver<ToolCheckEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        for kind in ToolKind::ALL {
            let status = check_tool(kind, settings.path(kind), &audit_path);

            if sender.send(ToolCheckEvent::Status(status)).is_err() {
                return;
            }
        }

        let _ = sender.send(ToolCheckEvent::Finished);
    });

    receiver
}

pub(crate) fn find_ready_tool(
    kind: ToolKind,
    configured: Option<&Path>,
    audit_path: &Path,
) -> Result<PathBuf, String> {
    let status = check_tool(kind, configured, audit_path);
    if let Some(error) = &status.audit_error {
        return Err(format!(
            "{} health-check audit could not be saved: {error}",
            kind.display_name()
        ));
    }
    if status.health != ToolHealth::Ready {
        return Err(format!(
            "{} is unavailable: {}",
            kind.display_name(),
            status.detail
        ));
    }
    status
        .executable
        .ok_or_else(|| format!("{} has no executable path", kind.display_name()))
}

pub fn run_audited_command(
    tool_name: &str,
    executable: &Path,
    arguments: &[String],
    audit_path: &Path,
) -> AuditedCommandResult {
    let started_unix_ms = current_unix_ms();
    let started = Instant::now();
    let output = Command::new(executable).args(arguments).output();
    let duration_ms = started.elapsed().as_millis();

    let audit = match output {
        Ok(output) => CommandAudit {
            tool: tool_name.to_owned(),
            executable: executable.to_path_buf(),
            arguments: arguments.to_vec(),
            started_unix_ms,
            duration_ms,
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        },
        Err(error) => CommandAudit {
            tool: tool_name.to_owned(),
            executable: executable.to_path_buf(),
            arguments: arguments.to_vec(),
            started_unix_ms,
            duration_ms,
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    };
    let audit_error = append_audit(audit_path, &audit).err();

    AuditedCommandResult { audit, audit_error }
}

fn check_tool(kind: ToolKind, configured: Option<&Path>, audit_path: &Path) -> ToolStatus {
    let candidates = candidate_paths(kind, configured);
    let Some(executable) = candidates.into_iter().find(|path| path.is_file()) else {
        let detail = if let Some(path) = configured {
            format!("A beállított fájl nem található: {}", path.display())
        } else {
            "Nem található ismert telepítési helyen vagy a PATH változóban.".to_owned()
        };

        return ToolStatus::new(kind, ToolHealth::Missing, detail);
    };

    let arguments = kind
        .version_arguments()
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect::<Vec<_>>();
    let result = run_audited_command(kind.display_name(), &executable, &arguments, audit_path);
    let version = first_non_empty_line(&result.audit.stdout)
        .or_else(|| first_non_empty_line(&result.audit.stderr))
        .map(str::to_owned);

    ToolStatus {
        kind,
        health: if result.audit.success {
            ToolHealth::Ready
        } else {
            ToolHealth::Failed
        },
        executable: Some(executable),
        version,
        detail: if result.audit.success {
            format!(
                "Egészségügyi ellenőrzés sikeres ({} ms).",
                result.audit.duration_ms
            )
        } else if result.audit.exit_code.is_some() {
            format!(
                "A verzióellenőrzés hibakóddal tért vissza: {:?}.",
                result.audit.exit_code
            )
        } else {
            format!("Nem sikerült elindítani: {}", result.audit.stderr)
        },
        audit: Some(result.audit),
        audit_error: result.audit_error,
    }
}

fn candidate_paths(kind: ToolKind, configured: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = configured {
        candidates.push(path.to_path_buf());
    }

    if let Some(path_value) = env::var_os("PATH") {
        for directory in env::split_paths(&path_value) {
            for name in kind.executable_names() {
                candidates.push(directory.join(name));
            }
        }
    }

    match kind {
        ToolKind::SevenZip => {
            add_program_files_candidates(&mut candidates, "7-Zip", kind.executable_names());
        }
        ToolKind::LibreOffice => {
            add_program_files_candidates(
                &mut candidates,
                "LibreOffice\\program",
                kind.executable_names(),
            );
        }
        ToolKind::Greaseweazle => add_python_script_candidates(&mut candidates),
    }

    let mut seen = HashSet::new();
    candidates.retain(|path| seen.insert(path.to_string_lossy().to_ascii_lowercase()));
    candidates
}

fn add_program_files_candidates(
    candidates: &mut Vec<PathBuf>,
    relative_directory: &str,
    executable_names: &[&str],
) {
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        let Some(root) = env::var_os(variable) else {
            continue;
        };

        for name in executable_names {
            candidates.push(PathBuf::from(&root).join(relative_directory).join(name));
        }
    }
}

fn add_python_script_candidates(candidates: &mut Vec<PathBuf>) {
    let Some(local_app_data) = env::var_os("LOCALAPPDATA") else {
        return;
    };

    let python_root = PathBuf::from(local_app_data)
        .join("Programs")
        .join("Python");
    let Ok(entries) = fs::read_dir(python_root) else {
        return;
    };

    for entry in entries.flatten() {
        candidates.push(entry.path().join("Scripts").join("gw.exe"));
    }
}

pub(crate) fn append_audit(path: &Path, audit: &CommandAudit) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Nem sikerült létrehozni az eszköznapló mappáját {}: {error}",
                parent.display()
            )
        })?;
    }

    let line =
        serde_json::to_string(audit).map_err(|error| format!("Eszköznapló JSON hiba: {error}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Nem sikerült megnyitni az eszköznaplót {}: {error}",
                path.display()
            )
        })?;

    writeln!(file, "{line}").map_err(|error| {
        format!(
            "Nem sikerült írni az eszköznaplót {}: {error}",
            path.display()
        )
    })
}

fn settings_file_path() -> PathBuf {
    app_data_directory().join(SETTINGS_FILE_NAME)
}

fn app_data_directory() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("FluxVault")
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn first_non_empty_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_path_is_the_first_candidate() {
        let configured = Path::new(r"C:\Tools\7-Zip\7z.exe");
        let candidates = candidate_paths(ToolKind::SevenZip, Some(configured));

        assert_eq!(candidates.first().map(PathBuf::as_path), Some(configured));
    }

    #[test]
    fn version_parser_skips_blank_lines() {
        assert_eq!(
            first_non_empty_line("\r\n  \r\n7-Zip 24.09\r\nCopyright"),
            Some("7-Zip 24.09")
        );
    }

    #[test]
    fn greaseweazle_health_check_is_version_only() {
        assert_eq!(ToolKind::Greaseweazle.version_arguments(), &["--version"]);
    }
}
