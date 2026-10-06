//! Semantic, ASCII-first terminal cues. Never decorate machine-readable stdout.

use std::io::Write;

#[derive(Clone, Copy, Default)]
pub(super) enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err("--color must be auto, always, or never".to_owned()),
        }
    }

    pub(super) fn enabled(self, terminal: bool, no_color: bool, dumb_terminal: bool) -> bool {
        match self {
            Self::Auto => terminal && !no_color && !dumb_terminal,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Cue {
    Action,
    Success,
    Attention,
    Error,
}

pub(super) fn banner(
    output: &mut impl Write,
    color: bool,
    cue: Cue,
    headline: &str,
    detail: &str,
) -> Result<(), String> {
    let escape = if color {
        match cue {
            Cue::Action => "\x1b[1;36m",
            Cue::Success => "\x1b[1;32m",
            Cue::Attention => "\x1b[1;33m",
            Cue::Error => "\x1b[1;31m",
        }
    } else {
        ""
    };
    let reset = if color { "\x1b[0m" } else { "" };
    writeln!(
        output,
        "\n{escape}============================================================\n{headline}\n{detail}\n============================================================{reset}"
    )
    .and_then(|_| output.flush())
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_color_respects_redirection_no_color_and_dumb_terminals() {
        assert!(ColorMode::Auto.enabled(true, false, false));
        assert!(!ColorMode::Auto.enabled(false, false, false));
        assert!(!ColorMode::Auto.enabled(true, true, false));
        assert!(!ColorMode::Auto.enabled(true, false, true));
        assert!(ColorMode::Always.enabled(false, true, true));
        assert!(!ColorMode::Never.enabled(true, false, false));
        assert!(ColorMode::parse("sometimes").is_err());
    }

    #[test]
    fn cues_are_ascii_and_only_explicit_color_emits_escapes() {
        for cue in [Cue::Action, Cue::Success, Cue::Attention, Cue::Error] {
            let mut plain = Vec::new();
            banner(
                &mut plain,
                false,
                cue,
                "DONE 004",
                "REMOVE 004 / INSERT 005",
            )
            .unwrap();
            assert!(plain.is_ascii());
            assert!(!plain.contains(&0x1b));
            let mut colored = Vec::new();
            banner(
                &mut colored,
                true,
                cue,
                "DONE 004",
                "REMOVE 004 / INSERT 005",
            )
            .unwrap();
            assert!(colored.contains(&0x1b));
            assert!(String::from_utf8(colored).unwrap().ends_with("\x1b[0m\n"));
        }
    }
}
