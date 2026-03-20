//! Terminal styling and color resolution (NO_COLOR, --color, TTY).

use std::io::{self, IsTerminal};

use clap::ValueEnum;

/// When to emit ANSI colors for grip-controlled output (not clap's own help colors).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorWhen {
    Auto,
    /// Emit ANSI colors whenever NO_COLOR is unset (default).
    #[default]
    Always,
    Never,
}

impl std::fmt::Display for ColorWhen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColorWhen::Auto => write!(f, "auto"),
            ColorWhen::Always => write!(f, "always"),
            ColorWhen::Never => write!(f, "never"),
        }
    }
}

/// Global output preferences from the CLI.
#[derive(Clone, Copy, Debug)]
pub struct OutputCfg {
    pub quiet: bool,
    pub verbose: bool,
    pub color_when: ColorWhen,
}

impl OutputCfg {
    pub fn use_color_stderr(&self) -> bool {
        resolve_color(self.color_when, std::env::var_os("NO_COLOR").is_some(), io::stderr().is_terminal())
    }

    pub fn use_color_stdout(&self) -> bool {
        resolve_color(self.color_when, std::env::var_os("NO_COLOR").is_some(), io::stdout().is_terminal())
    }
}

fn resolve_color(when: ColorWhen, no_color: bool, is_tty: bool) -> bool {
    if no_color {
        return false;
    }
    match when {
        ColorWhen::Never => false,
        ColorWhen::Always => true,
        ColorWhen::Auto => is_tty,
    }
}

pub fn green(colored: bool, s: &str) -> String {
    if colored {
        format!("\x1b[32m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn red(colored: bool, s: &str) -> String {
    if colored {
        format!("\x1b[31m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn yellow(colored: bool, s: &str) -> String {
    if colored {
        format!("\x1b[33m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn dim(colored: bool, s: &str) -> String {
    if colored {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn install_spinner_template(colored: bool) -> &'static str {
    if colored {
        "  {prefix:.bold.dim} {spinner:.cyan} {msg}"
    } else {
        "  {prefix} {spinner} {msg}"
    }
}

pub fn success_checkmark(colored: bool) -> String {
    green(colored, "✓")
}

pub fn warn_glyph(colored: bool) -> String {
    yellow(colored, "⚠")
}

pub fn fail_glyph(colored: bool) -> String {
    red(colored, "✗")
}
