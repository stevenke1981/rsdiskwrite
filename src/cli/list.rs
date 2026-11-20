//! `rsdiskwrite list`.

use crate::cli::ListArgs;
use crate::disk::{self, Enumeration};
use crate::error::Result;
use crate::output::{Output, Style, format_size};

/// Lists physical disks. JSON `data` is [`Enumeration`] (`{"disks":[...],"method":...}`).
pub fn run(args: ListArgs, out: &Output) -> Result<()> {
    let ListArgs {} = args;
    let enumeration = disk::enumerate()?;
    out.emit(&enumeration, render)
}

/// Human table for [`run`]: one row per disk with size (human and bytes), bus,
/// removable flag, model, serial and flags (`SYSTEM`).
pub fn render(enumeration: &Enumeration, out: &Output) -> String {
    if enumeration.disks.is_empty() {
        return "No disks found. Check device connections, or run 'rsdiskwrite doctor' to diagnose."
            .into();
    }
    let rows: Vec<[String; 7]> = enumeration
        .disks
        .iter()
        .map(|d| {
            [
                d.number.to_string(),
                d.bus.label().to_owned(),
                if d.removable { "yes" } else { "no" }.to_owned(),
                format_size(d.size),
                d.size.to_string(),
                d.model.clone(),
                d.serial.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect();
    let headers = [
        "DISK",
        "BUS",
        "REMOVABLE",
        "SIZE",
        "BYTES",
        "MODEL",
        "SERIAL",
    ];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    // Right-align the numeric columns (0, 3, 4); left-align the rest.
    let line = |cells: &[String]| {
        let mut text = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                text.push_str("  ");
            }
            let pad = widths[i].saturating_sub(cell.chars().count());
            if matches!(i, 0 | 3 | 4) {
                text.push_str(&" ".repeat(pad));
                text.push_str(cell);
            } else {
                text.push_str(cell);
                text.push_str(&" ".repeat(pad));
            }
        }
        text
    };
    let header: Vec<String> = headers.iter().map(|h| (*h).to_owned()).collect();
    let mut text = out.paint(&format!("{}  FLAGS", line(&header)), Style::Bold);
    for (disk, row) in enumeration.disks.iter().zip(&rows) {
        let mut flags = Vec::new();
        if disk.is_system {
            flags.push("SYSTEM");
        }
        if !disk.is_removable_target() {
            flags.push("FIXED");
        }
        let mut row_text = format!("{}  {}", line(row), flags.join(","));
        if disk.is_system {
            row_text = out.paint(&row_text, Style::Red);
        }
        text.push('\n');
        text.push_str(row_text.trim_end());
    }
    text.push_str(&format!(
        "\n{}",
        out.paint(
            "SYSTEM disks and FIXED (non-removable, non-USB/SD) disks are refused by 'write' (FIXED needs --allow-fixed).",
            Style::Dim
        )
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::EnumerationMethod;
    use crate::disk::model::test_disk;
    use crate::output::ProgressMode;

    #[test]
    fn table_is_aligned_and_flags_system() {
        let mut system = test_disk(0);
        system.is_system = true;
        let enumeration = Enumeration {
            disks: vec![system, test_disk(12)],
            method: EnumerationMethod::SetupApi,
        };
        let out = Output::with_settings(false, false, false, false, ProgressMode::Off);
        let text = render(&enumeration, &out);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("DISK"), "{text}");
        assert!(lines[1].contains("SYSTEM"), "{text}");
        assert!(!lines[2].contains("SYSTEM"), "{text}");
        // Columns line up: the BUS column starts at the same offset in every row.
        let bus_col = lines[0].find("BUS").unwrap();
        assert_eq!(lines[1].find(test_disk(0).bus.label()), Some(bus_col));
    }
}
