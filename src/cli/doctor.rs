//! `rsdiskwrite doctor`.

use crate::cli::DoctorArgs;
use crate::doctor::{self, CheckStatus, DoctorReport};
use crate::error::{Error, ErrorKind, Result};
use crate::output::{Output, Style};

/// Runs every check and prints the report (JSON `data` is [`DoctorReport`]). Exits 1 when
/// any check fails (the report is still printed, as a success envelope).
pub fn run(args: DoctorArgs, out: &Output) -> Result<()> {
    let DoctorArgs {} = args;
    let report = DoctorReport::new(doctor::run());
    out.emit(&report, render)?;
    if report.has_failures() {
        return Err(Error::new(ErrorKind::Internal, "some checks failed").into_reported());
    }
    Ok(())
}

/// Human rendering of a [`DoctorReport`].
pub fn render(report: &DoctorReport, out: &Output) -> String {
    // TODO(phase2-D): polish the human layout.
    let mut text = format!("rsdiskwrite {}", report.version);
    for check in &report.checks {
        let status = match check.status {
            CheckStatus::Ok => out.paint("ok  ", Style::Green),
            CheckStatus::Warn => out.paint("warn", Style::Yellow),
            CheckStatus::Fail => out.paint("FAIL", Style::Red),
        };
        text.push_str(&format!("\n[{status}] {}: {}", check.id, check.detail));
        if let Some(hint) = &check.hint {
            text.push_str(&format!("\n       {hint}"));
        }
    }
    text
}
