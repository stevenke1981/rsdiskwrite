//! `rsdiskwrite verify IMAGE -d N`.

use crate::cli::VerifyArgs;
use crate::error::{Error, ErrorKind, Result};
use crate::ops::{self, VerifyReport};
use crate::output::{Output, Style};

/// Reads disk N back and compares it with IMAGE. A mismatch is
/// [`ErrorKind::VerifyMismatch`] (exit 6).
pub fn run(args: VerifyArgs, out: &Output) -> Result<()> {
    let sink = out.progress();
    let report = ops::verify(&args.image, args.disk, &|p| sink.update(p));
    sink.finish();
    let report = report?;
    if !report.matched {
        // TODO(phase2-C): decide whether JSON mode should also carry the report.
        return Err(Error::new(
            ErrorKind::VerifyMismatch,
            format!(
                "PhysicalDrive{} differs from the image at byte {}",
                args.disk,
                report.first_mismatch.unwrap_or(0)
            ),
        ));
    }
    out.emit(&report, render)
}

/// Human rendering of a successful [`VerifyReport`].
pub fn render(report: &VerifyReport, out: &Output) -> String {
    format!(
        "{} PhysicalDrive{} matches {} ({} bytes compared).",
        out.paint("OK:", Style::Green),
        report.disk.number,
        report.image.display(),
        report.bytes
    )
}
