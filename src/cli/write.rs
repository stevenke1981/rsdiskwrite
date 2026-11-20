//! `rsdiskwrite write IMAGE -d N` (and the legacy top-level form).

use crate::cli::WriteArgs;
use crate::confirm;
use crate::disk;
use crate::error::{Error, ErrorKind, Result};
use crate::ops::{self, WritePlan, WriteReport, WriteRequest};
use crate::output::{Output, Style, format_duration, format_size};
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
        let error = refusal_error(&plan);
        if out.is_json() {
            out.error_with_details(&error, &plan);
        } else {
            out.emit(&plan, render_plan)?;
            out.error(&error);
        }
        return Err(error.into_reported());
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
            )
            .with_hint(format!(
                "Pass --confirm \"{}\" exactly (case-sensitive), or omit --confirm to be asked at a terminal",
                confirm::phrase(&plan.disk)
            )));
        }
        None if interactive => {
            if !out.is_quiet() {
                eprintln!("{}", plan_text(&plan, out, false));
            }
            let prompt = out.paint_err(
                &confirm::warning(&plan.disk, &plan.image.path),
                Style::Yellow,
            );
            let answer = out.prompt(&prompt)?;
            if !confirm::matches(&answer, &plan.disk) {
                return Err(
                    Error::new(ErrorKind::NotConfirmed, "cancelled: nothing was written")
                        .with_hint(format!(
                            "Type exactly {} to confirm",
                            confirm::phrase(&plan.disk)
                        )),
                );
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
    if let Some(verify) = report.verify.as_ref().filter(|v| !v.matched) {
        let error = crate::cli::verify::mismatch_error(verify);
        out.error_with_details(&error, &report);
        return Err(error.into_reported());
    }
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
    plan_text(plan, out, true)
}

fn plan_text(plan: &WritePlan, out: &Output, final_line: bool) -> String {
    let label = |text: &str| out.paint(&format!("{text:<8}"), Style::Bold);
    let disk = &plan.disk;
    let mut text = format!(
        "{} {} ({:?}, {})",
        label("Image:"),
        plan.image.path.display(),
        plan.image.format,
        format_size(plan.image.file_size)
    );
    if let Some(sha) = plan.image.sha256 {
        text.push_str(&format!("\n{} {sha}", label("SHA-256:")));
    }
    text.push_str(&format!(
        "\n{} {} - {} ({} bytes), {}{}{}",
        label("Target:"),
        disk.name(),
        format_size(disk.size),
        disk.size,
        disk.bus.label(),
        if disk.removable { ", removable" } else { "" },
        if disk.is_system { ", SYSTEM" } else { "" },
    ));
    text.push_str(&format!(
        "\n{} {}{}",
        label("Device:"),
        disk.model,
        disk.serial
            .as_deref()
            .map(|s| format!(", serial {s}"))
            .unwrap_or_default()
    ));
    text.push_str(&format!(
        "\n{} {}",
        label("Verify:"),
        if plan.request.verify {
            "read back and compare after writing"
        } else {
            "skipped (--no-verify)"
        }
    ));
    for warning in &plan.warnings {
        text.push_str(&format!(
            "\n{} {warning}",
            out.paint("warning:", Style::Yellow)
        ));
    }
    for refusal in &plan.refusals {
        text.push_str(&format!(
            "\n{} {}",
            out.paint("REFUSED:", Style::Red),
            refusal.message
        ));
        if let Some(hint) = &refusal.hint {
            text.push_str(&format!("\n  {} {hint}", out.paint("hint:", Style::Cyan)));
        }
    }
    if plan.refusals.is_empty() && final_line {
        text.push_str(&format!(
            "\n{} all safety checks passed; nothing was written (dry run)",
            out.paint("OK:", Style::Green)
        ));
    }
    text
}

/// Human rendering of a successful write: bytes, time, speed and verify result.
pub fn render_report(report: &WriteReport, out: &Output) -> String {
    let secs = report.elapsed_ms as f64 / 1000.0;
    let speed = if secs > 0.0 {
        format!(
            ", {:.1} MiB/s",
            report.bytes_written as f64 / secs / (1024.0 * 1024.0)
        )
    } else {
        String::new()
    };
    let mut text = format!(
        "{} wrote {} ({} bytes) to {} in {}{speed}.",
        out.paint("Done:", Style::Green),
        format_size(report.bytes_written),
        report.bytes_written,
        report.disk.name(),
        format_duration(report.elapsed_ms / 1000)
    );
    match &report.verify {
        Some(verify) if verify.matched => text.push_str(&format!(
            "\n{} read back {} bytes; disk matches the image.",
            out.paint("Verified:", Style::Green),
            verify.bytes
        )),
        Some(_) => text.push_str(&format!(
            "\n{} disk differs from the image.",
            out.paint("Verification FAILED:", Style::Red)
        )),
        None => text.push_str("\nVerification: skipped."),
    }
    if let Some(sha) = report.content_sha256 {
        text.push_str(&format!("\nSHA-256 of data: {sha}"));
    }
    text
}
