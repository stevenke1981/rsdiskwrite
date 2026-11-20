//! `rsdiskwrite write IMAGE -d N` (and the legacy top-level form).

use crate::cli::WriteArgs;
use crate::confirm;
use crate::disk;
use crate::error::{Error, ErrorKind, Result};
use crate::ops::{self, WritePlan, WriteReport, WriteRequest};
use crate::output::{Output, Style, format_size};
use std::io::IsTerminal;

/// Plans, confirms and executes a write.
///
/// Confirmation: `--confirm PHRASE` must equal `ERASE <N>`; without it the phrase is read
/// from stdin only when stdin is a terminal, otherwise the write is refused with
/// [`ErrorKind::NotConfirmed`]. A missing `-d` is prompted for only at a terminal.
/// `--dry-run` prints the plan and writes nothing. Refusals exit 3 before any prompt.
pub fn run(args: WriteArgs, out: &Output) -> Result<()> {
    let interactive = std::io::stdin().is_terminal();
    let number = match args.disk {
        Some(number) => number,
        None if !interactive => {
            return Err(
                Error::usage("-d/--disk is required when stdin is not a terminal").with_hint(
                    format!(
                        "rsdiskwrite write {} -d N --confirm \"ERASE N\"",
                        args.image.display()
                    ),
                ),
            );
        }
        None => prompt_disk(out)?,
    };
    let request = WriteRequest {
        allow_large: args.allow_large,
        allow_fixed: args.allow_fixed,
        verify: !args.no_verify,
        expect_sha256: args.expect_sha256,
        ..WriteRequest::new(&args.image, number)
    };
    let plan = ops::plan(&request)?;
    if !plan.is_allowed() {
        if !out.is_json() {
            out.emit(&plan, render_plan)?;
        }
        return Err(refusal_error(&plan));
    }
    if args.dry_run {
        return out.emit(&plan, render_plan);
    }
    for warning in &plan.warnings {
        out.warn(warning);
    }
    match &args.confirm {
        Some(phrase) if confirm::matches(phrase, &plan.disk) => {}
        Some(_) => {
            return Err(Error::new(
                ErrorKind::NotConfirmed,
                format!(
                    "confirmation phrase does not match; expected '{}'",
                    confirm::phrase(&plan.disk)
                ),
            ));
        }
        None if interactive => {
            let prompt = out.paint_err(
                &confirm::warning(&plan.disk, &plan.image.path),
                Style::Yellow,
            );
            let answer = out.prompt(&prompt)?;
            if !confirm::matches(&answer, &plan.disk) {
                return Err(Error::new(ErrorKind::NotConfirmed, "cancelled"));
            }
        }
        None => {
            return Err(Error::new(
                ErrorKind::NotConfirmed,
                "refusing to write without confirmation: stdin is not a terminal",
            )
            .with_hint(format!(
                "Check the target with --dry-run, then pass --confirm \"{}\"",
                confirm::phrase(&plan.disk)
            )));
        }
    }
    let sink = out.progress();
    let report = ops::execute(&plan, &|p| sink.update(p));
    sink.finish();
    let report = report?;
    out.emit(&report, render_report)
}

fn prompt_disk(out: &Output) -> Result<u32> {
    let enumeration = disk::enumerate()?;
    if enumeration.disks.is_empty() {
        return Err(Error::new(ErrorKind::DiskNotFound, "No disks found")
            .with_hint("Check device connections, or run 'rsdiskwrite doctor'"));
    }
    eprintln!("{}", crate::cli::list::render(&enumeration, out));
    out.prompt("Disk number:")?
        .trim()
        .parse()
        .map_err(|_| Error::usage("invalid disk number"))
}

fn refusal_error(plan: &WritePlan) -> Error {
    let first = &plan.refusals[0];
    let mut error = first.to_error();
    if plan.refusals.len() > 1 {
        let rest: Vec<&str> = plan.refusals[1..]
            .iter()
            .map(|r| r.message.as_str())
            .collect();
        error.message = format!("{}; also: {}", error.message, rest.join("; "));
    }
    error
}

/// Human rendering of a plan (dry run or refusal).
pub fn render_plan(plan: &WritePlan, out: &Output) -> String {
    let mut text = format!(
        "{} {} ({:?}, {})\n{} {}",
        out.paint("Image:", Style::Bold),
        plan.image.path.display(),
        plan.image.format,
        format_size(plan.image.file_size),
        out.paint("Target:", Style::Bold),
        plan.disk.summary()
    );
    if let Some(sha) = plan.image.sha256 {
        text.push_str(&format!("\n{} {sha}", out.paint("SHA-256:", Style::Bold)));
    }
    for warning in &plan.warnings {
        text.push_str(&format!("\n{} {warning}", out.paint("note:", Style::Cyan)));
    }
    if plan.refusals.is_empty() {
        text.push_str(&format!(
            "\n{} all safety checks passed; nothing was written",
            out.paint("OK:", Style::Green)
        ));
    }
    for refusal in &plan.refusals {
        text.push_str(&format!(
            "\n{} {}",
            out.paint("REFUSED:", Style::Red),
            refusal.message
        ));
    }
    text
}

/// Human rendering of a successful write.
pub fn render_report(report: &WriteReport, out: &Output) -> String {
    let mut text = format!(
        "{} {} bytes written to {}.",
        out.paint("Done:", Style::Green),
        report.bytes_written,
        report.disk.name()
    );
    match &report.verify {
        Some(verify) if verify.matched => text.push_str(" Verified."),
        Some(_) => text.push_str(" Verification FAILED."),
        None => {}
    }
    text
}
