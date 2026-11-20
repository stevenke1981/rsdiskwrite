//! Output rendering shared by every CLI command: the JSON envelope, human text, colour
//! decision and progress reporting.
//!
//! Contract (`--json`): stdout carries exactly one JSON object,
//! `{"schema":1,"ok":true,"data":{...}}` or
//! `{"schema":1,"ok":false,"error":{"code","message","hint"}}`; progress goes to stderr as
//! NDJSON (`{"type":"progress","phase":..,"done":..,"total":..}`). Human mode prints data
//! to stdout and diagnostics/progress to stderr.

use crate::error::{Error, Result};
use crate::ops::{Phase, Progress};
use serde::Serialize;
use serde_json::{Value, json};
use std::cell::Cell;
use std::io::{self, BufRead, IsTerminal, Write};

/// Version of the JSON envelope (`"schema"`).
pub const SCHEMA_VERSION: u32 = 1;

/// `--color` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ColorChoice {
    /// Colour when the stream is a terminal and `NO_COLOR` is not set.
    #[default]
    Auto,
    /// Always colour (overrides `NO_COLOR`).
    Always,
    /// Never colour.
    Never,
}

/// How progress is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    /// No progress output.
    Off,
    /// A single-line bar on stderr (human mode, stderr is a terminal).
    Bar,
    /// NDJSON lines on stderr (`--json` mode).
    Ndjson,
}

/// Text styles for [`Output::paint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Bold,
    Dim,
    Red,
    Green,
    Yellow,
    Cyan,
}

impl Style {
    fn ansi(self) -> &'static str {
        match self {
            Style::Bold => "1",
            Style::Dim => "2",
            Style::Red => "31",
            Style::Green => "32",
            Style::Yellow => "33",
            Style::Cyan => "36",
        }
    }
}

/// Colour decision: `Always`/`Never` win; `Auto` colours only a terminal without
/// `NO_COLOR` (any non-empty value).
pub fn color_enabled(choice: ColorChoice, is_terminal: bool, no_color: bool) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => is_terminal && !no_color,
    }
}

/// True when the `NO_COLOR` environment variable is set to a non-empty value.
pub fn no_color_env() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// `{"schema":1,"ok":true,"data":data}`.
pub fn envelope_ok<T: Serialize + ?Sized>(data: &T) -> Result<Value> {
    let data = serde_json::to_value(data)
        .map_err(|e| Error::internal(format!("cannot serialize output: {e}")))?;
    Ok(json!({"schema": SCHEMA_VERSION, "ok": true, "data": data}))
}

/// `{"schema":1,"ok":false,"error":{"code","message","hint"}}`.
pub fn envelope_err(error: &Error) -> Value {
    json!({"schema": SCHEMA_VERSION, "ok": false, "error": error})
}

/// Human-readable binary size: `"512 B"`, `"1.50 KiB"`, `"7.45 GiB"`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

/// Wraps `text` in ANSI codes for `style` when `enabled`.
pub fn paint(text: &str, style: Style, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{}m{text}\x1b[0m", style.ansi())
    } else {
        text.to_owned()
    }
}

/// Output settings for one CLI invocation. Created once in `main` and passed to every
/// command's `run`.
#[derive(Debug, Clone)]
pub struct Output {
    json: bool,
    quiet: bool,
    color_stdout: bool,
    color_stderr: bool,
    progress: ProgressMode,
}

impl Output {
    /// Detects terminals (`std::io::IsTerminal`) and `NO_COLOR`. Progress is NDJSON in
    /// JSON mode, a bar when stderr is a terminal, otherwise off; `quiet` turns it off.
    pub fn new(json: bool, quiet: bool, color: ColorChoice) -> Self {
        let no_color = no_color_env();
        let stderr_tty = io::stderr().is_terminal();
        let progress = if quiet {
            ProgressMode::Off
        } else if json {
            ProgressMode::Ndjson
        } else if stderr_tty {
            ProgressMode::Bar
        } else {
            ProgressMode::Off
        };
        Self {
            json,
            quiet,
            color_stdout: !json && color_enabled(color, io::stdout().is_terminal(), no_color),
            color_stderr: color_enabled(color, stderr_tty, no_color),
            progress,
        }
    }

    /// Explicit settings (tests, MCP self-test).
    pub fn with_settings(
        json: bool,
        quiet: bool,
        color_stdout: bool,
        color_stderr: bool,
        progress: ProgressMode,
    ) -> Self {
        Self {
            json,
            quiet,
            color_stdout,
            color_stderr,
            progress,
        }
    }

    /// `--json` mode.
    pub fn is_json(&self) -> bool {
        self.json
    }

    /// `--quiet` mode.
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Progress mode in effect.
    pub fn progress_mode(&self) -> ProgressMode {
        self.progress
    }

    /// Styles text destined for stdout.
    pub fn paint(&self, text: &str, style: Style) -> String {
        paint(text, style, self.color_stdout)
    }

    /// Styles text destined for stderr.
    pub fn paint_err(&self, text: &str, style: Style) -> String {
        paint(text, style, self.color_stderr)
    }

    /// Emits a command's result: in JSON mode the success envelope around `data`; in
    /// human mode the string returned by `human(data, self)` (a trailing newline is added if missing;
    /// nothing is printed for an empty string). Data is printed even with `--quiet`.
    pub fn emit<T: Serialize + ?Sized>(
        &self,
        data: &T,
        human: impl FnOnce(&T, &Output) -> String,
    ) -> Result<()> {
        let mut stdout = io::stdout().lock();
        let text = if self.json {
            let mut line = envelope_ok(data)?.to_string();
            line.push('\n');
            line
        } else {
            let mut text = human(data, self);
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text
        };
        stdout
            .write_all(text.as_bytes())
            .and_then(|_| stdout.flush())
            .map_err(|e| Error::io("cannot write to stdout", e))
    }

    /// Renders an error: JSON envelope on stdout in JSON mode, otherwise
    /// `rsdiskwrite: error: ...` (+ `hint: ...`) on stderr. Never fails.
    pub fn error(&self, error: &Error) {
        if self.json {
            let _ = writeln!(io::stdout().lock(), "{}", envelope_err(error));
        } else {
            let mut stderr = io::stderr().lock();
            let _ = writeln!(
                stderr,
                "rsdiskwrite: {} {}",
                self.paint_err("error:", Style::Red),
                error.message
            );
            if let Some(hint) = &error.hint {
                let _ = writeln!(stderr, "{} {hint}", self.paint_err("hint:", Style::Cyan));
            }
        }
    }

    /// Informational line on stderr (suppressed by `--quiet`).
    pub fn info(&self, message: &str) {
        if !self.quiet {
            let _ = writeln!(io::stderr().lock(), "{message}");
        }
    }

    /// Warning line on stderr (suppressed by `--quiet`).
    pub fn warn(&self, message: &str) {
        if !self.quiet {
            let _ = writeln!(
                io::stderr().lock(),
                "{} {message}",
                self.paint_err("warning:", Style::Yellow)
            );
        }
    }

    /// Writes `prompt` to stderr and reads one line from stdin (without the newline).
    /// Callers must check that stdin is a terminal first; this never decides that.
    pub fn prompt(&self, prompt: &str) -> Result<String> {
        let mut stderr = io::stderr().lock();
        write!(stderr, "{prompt} ")
            .and_then(|_| stderr.flush())
            .map_err(|e| Error::io("cannot write prompt", e))?;
        let mut line = String::new();
        io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| Error::io("cannot read from stdin", e))?;
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }

    /// A progress sink honouring this output's [`ProgressMode`]. Use as
    /// `let sink = out.progress(); op(&|p| sink.update(p)); sink.finish();`.
    pub fn progress(&self) -> ProgressSink {
        ProgressSink {
            mode: self.progress,
            last: Cell::new(None),
            drawn: Cell::new(false),
        }
    }
}

/// Throttled progress renderer created by [`Output::progress`].
#[derive(Debug)]
pub struct ProgressSink {
    mode: ProgressMode,
    /// Last (phase, permille) rendered.
    last: Cell<Option<(Phase, u64)>>,
    drawn: Cell<bool>,
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Preparing => "preparing",
        Phase::Hashing => "hashing",
        Phase::Writing => "writing",
        Phase::Flushing => "flushing",
        Phase::Verifying => "verifying",
    }
}

impl ProgressSink {
    /// Renders `progress` if the phase changed or at least 0.1% (bar) / 1% (NDJSON)
    /// elapsed since the last render.
    pub fn update(&self, progress: Progress) {
        if self.mode == ProgressMode::Off {
            return;
        }
        let permille = (progress.done.saturating_mul(1000))
            .checked_div(progress.total)
            .unwrap_or(0)
            .min(1000);
        let step = if self.mode == ProgressMode::Ndjson {
            10
        } else {
            1
        };
        let bucket = permille / step;
        if self.last.get() == Some((progress.phase, bucket)) {
            return;
        }
        self.last.set(Some((progress.phase, bucket)));
        let mut stderr = io::stderr().lock();
        let _ = match self.mode {
            ProgressMode::Ndjson => writeln!(
                stderr,
                "{}",
                json!({"type": "progress", "phase": progress.phase, "done": progress.done, "total": progress.total})
            ),
            _ => {
                self.drawn.set(true);
                write!(
                    stderr,
                    "\r{:<9} {:>5.1}%  {} / {}   ",
                    phase_name(progress.phase),
                    permille as f64 / 10.0,
                    format_size(progress.done),
                    format_size(progress.total)
                )
                .and_then(|_| stderr.flush())
            }
        };
    }

    /// Ends the bar line (if one was drawn).
    pub fn finish(&self) {
        if self.drawn.replace(false) {
            let _ = writeln!(io::stderr().lock());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    #[test]
    fn colour_decision() {
        assert!(color_enabled(ColorChoice::Auto, true, false));
        assert!(!color_enabled(ColorChoice::Auto, true, true));
        assert!(!color_enabled(ColorChoice::Auto, false, false));
        assert!(color_enabled(ColorChoice::Always, false, true));
        assert!(!color_enabled(ColorChoice::Never, true, false));
        assert_eq!(paint("x", Style::Red, false), "x");
        assert_eq!(paint("x", Style::Red, true), "\x1b[31mx\x1b[0m");
    }

    #[test]
    fn envelopes() {
        assert_eq!(
            envelope_ok(&json!({"a": 1})).unwrap(),
            json!({"schema": 1, "ok": true, "data": {"a": 1}})
        );
        let error = Error::new(ErrorKind::DiskNotFound, "gone");
        assert_eq!(
            envelope_err(&error),
            json!({"schema": 1, "ok": false, "error": {"code": "disk_not_found", "message": "gone", "hint": null}})
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.50 KiB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.00 GiB");
    }
}
