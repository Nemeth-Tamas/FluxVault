use std::path::{Path, PathBuf};

use crate::{external_tools, safety::MediaSafetyPolicy};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreaseweazleCommand {
    arguments: Vec<String>,
}

impl GreaseweazleCommand {
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
            forbidden
                .iter()
                .any(|forbidden| argument.eq_ignore_ascii_case(forbidden))
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

#[derive(Debug, Clone)]
pub struct GreaseweazleExecution {
    pub mode: BackendMode,
    pub command: GreaseweazleCommand,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub trait GreaseweazleBackend {
    fn mode(&self) -> BackendMode;
    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String>;
}

pub struct ProcessGreaseweazleBackend {
    executable: PathBuf,
    audit_path: PathBuf,
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
        })
    }
}

impl GreaseweazleBackend for ProcessGreaseweazleBackend {
    fn mode(&self) -> BackendMode {
        BackendMode::Process
    }

    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String> {
        MediaSafetyPolicy::assert_invariants();
        command.validate_safe()?;
        let result = external_tools::run_audited_command(
            "Greaseweazle",
            &self.executable,
            command.arguments(),
            &self.audit_path,
        );
        if let Some(error) = result.audit_error {
            return Err(format!("A Greaseweazle parancsnapló nem írható: {error}"));
        }
        Ok(GreaseweazleExecution {
            mode: BackendMode::Process,
            command: command.clone(),
            success: result.audit.success,
            exit_code: result.audit.exit_code,
            stdout: result.audit.stdout,
            stderr: result.audit.stderr,
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
}
