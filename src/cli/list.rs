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

/// Human table for [`run`].
pub fn render(enumeration: &Enumeration, out: &Output) -> String {
    if enumeration.disks.is_empty() {
        return "No disks found. Check device connections (or run 'rsdiskwrite doctor').".into();
    }
    let mut text = out.paint(
        &format!(
            "{:<5} {:>11}  {:<7} {:<9} {}",
            "DISK", "SIZE", "BUS", "REMOVABLE", "MODEL / SERIAL"
        ),
        Style::Bold,
    );
    for disk in &enumeration.disks {
        let mut model = disk.model.clone();
        if let Some(serial) = &disk.serial {
            model.push_str(" / ");
            model.push_str(serial);
        }
        if disk.is_system {
            model.push_str(&out.paint("  [system]", Style::Red));
        }
        text.push_str(&format!(
            "\n{:<5} {:>11}  {:<7} {:<9} {}",
            disk.number,
            format_size(disk.size),
            disk.bus.label(),
            if disk.removable { "yes" } else { "no" },
            model
        ));
    }
    text
}
