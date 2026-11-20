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
    let mut text = out.paint(
        &format!("rsdiskwrite {} doctor", report.version),
        Style::Bold,
    );
    for check in &report.checks {
        let marker = match check.status {
            CheckStatus::Ok => out.paint(" OK ", Style::Green),
            CheckStatus::Warn => out.paint("WARN", Style::Yellow),
            CheckStatus::Fail => out.paint("FAIL", Style::Red),
        };
        text.push_str(&format!("\n[{marker}] {}: {}", check.id, check.detail));
        if check.status != CheckStatus::Ok
            && let Some(hint) = &check.hint
        {
            text.push_str(&format!("\n       -> {hint}"));
        }
    }
    let count = |s: CheckStatus| report.checks.iter().filter(|c| c.status == s).count();
    let (ok, warn, fail) = (
        count(CheckStatus::Ok),
        count(CheckStatus::Warn),
        count(CheckStatus::Fail),
    );
    text.push_str(&format!(
        "\n\n{ok} ok, {warn} warning(s), {fail} failure(s)"
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Check;
    use crate::output::ProgressMode;

    #[test]
    fn report_shape() {
        let out = Output::with_settings(false, false, false, false, ProgressMode::Off);
        let report = DoctorReport::new(vec![
            Check::ok("a", "fine"),
            Check::warn("b", "meh", "do x"),
            Check::fail("c", "bad", "do y"),
        ]);
        let text = render(&report, &out);
        assert!(text.contains("[ OK ] a: fine"));
        assert!(text.contains("[WARN] b: meh"));
        assert!(text.contains("-> do x"));
        assert!(text.contains("[FAIL] c: bad"));
        assert!(text.ends_with("1 ok, 1 warning(s), 1 failure(s)"));
    }
}
