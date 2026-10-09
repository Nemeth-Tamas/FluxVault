//! Generate a static trusted shell script; never inspect tools/projects/media.
use super::CliResponse;
pub(super) const POWERSHELL: &str = include_str!("../../scripts/FluxVault.Completion.ps1");

pub(super) fn run(args: &[String]) -> Result<CliResponse, String> {
    if args != ["completions", "powershell"] {
        return Err("Use completions powershell (PowerShell 7+); script generation does not accept other options".into());
    }
    Ok(CliResponse {
        output: POWERSHELL.to_owned(),
        exit_code: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_completion_needs_no_project_or_tools() {
        let result = crate::cli::run(
            &["completions".into(), "powershell".into()],
            std::path::Path::new("A:\\nonexistent"),
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.output, POWERSHELL);
        assert!(result.output.contains("Register-ArgumentCompleter -Native"));
    }
    #[test]
    fn generator_refuses_unsupported_shells_and_flags() {
        for args in [
            vec!["completions"],
            vec!["completions", "bash"],
            vec!["completions", "powershell", "--json"],
            vec!["completions", "powershell", "--project", "A:"],
        ] {
            assert!(run(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn suggested_option_names_exist_in_current_cli_help() {
        let help = crate::cli::run(&["--help".into()], std::path::Path::new("."))
            .unwrap()
            .output;
        let flags = |text: &str| {
            text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .filter(|word| word.starts_with("--") && word.len() > 2)
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>()
        };
        let known = flags(&help);
        for flag in flags(POWERSHELL) {
            assert!(
                known.contains(&flag),
                "Completion suggests undocumented/unsupported option {flag}"
            );
        }
    }
}
