//! `rsdiskwrite verify IMAGE -d N`.

use crate::cli::VerifyArgs;
use crate::error::{Error, ErrorKind, Result};
use crate::ops::{self, VerifyReport};
use crate::output::{Output, Style, format_size};

/// Reads disk N back and compares it with IMAGE.
///
/// A mismatch is [`ErrorKind::VerifyMismatch`] (exit 6). `Error` has no details field, so
/// the mismatch is rendered here: in `--json` mode the single stdout object is the error
/// envelope with the full [`VerifyReport`] under `error.details`; the error is then
/// returned marked as reported so `main` prints nothing more and exits 6.
pub fn run(args: VerifyArgs, out: &Output) -> Result<()> {
    let sink = out.progress();
    let report = ops::verify(&args.image, args.disk, &|p| sink.update(p));
    sink.finish();
    let report = report?;
    if !report.matched {
        let error = mismatch_error(&report);
        out.error_with_details(&error, &report);
        return Err(error.into_reported());
    }
    out.emit(&report, render)
}

/// The [`ErrorKind::VerifyMismatch`] error for a failed [`VerifyReport`].
pub fn mismatch_error(report: &VerifyReport) -> Error {
    Error::new(
        ErrorKind::VerifyMismatch,
        format!(
            "PhysicalDrive{} differs from {} at byte {} (of {} compared)",
            report.disk.number,
            report.image.display(),
            report.first_mismatch.unwrap_or(0),
            report.bytes
        ),
    )
    .with_hint(
        "The write may have failed or the media is faulty; write again, try another port or card, \
         and do not trust this disk until it verifies",
    )
}

/// Human rendering of a successful [`VerifyReport`].
pub fn render(report: &VerifyReport, out: &Output) -> String {
    let mut text = format!(
        "{} PhysicalDrive{} matches {} ({}, {} bytes compared).",
        out.paint("OK:", Style::Green),
        report.disk.number,
        report.image.display(),
        format_size(report.bytes),
        report.bytes
    );
    if let Some(sha) = report.content_sha256 {
        text.push_str(&format!("\nSHA-256 of data: {sha}"));
    }
    text
}
