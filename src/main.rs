#[cfg(not(windows))]
compile_error!("rsdiskwrite currently supports Windows only");

mod disk;
mod gui;

use std::io::{self, Write};
use std::path::Path;

fn main() {
    if let Err(error) = run() {
        eprintln!("rsdiskwrite: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return gui::run();
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Usage: rsdiskwrite [-u] [-d DRIVE_NUMBER] IMAGE\nNo arguments opens the GUI. Administrator privileges are required.\n-u permits disks larger than 2 GiB; all writes require confirmation."
        );
        return Ok(());
    }
    let mut allow_large = false;
    let mut number = None;
    let mut image = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-u" => allow_large = true,
            "-d" => {
                number = Some(
                    iter.next()
                        .ok_or("-d requires a drive number")?
                        .parse::<u32>()
                        .map_err(|_| "invalid drive number")?,
                )
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ if image.is_none() => image = Some(arg),
            _ => return Err("only one image may be specified".into()),
        }
    }
    let image = image.ok_or("image path required")?;
    let disks = disk::list_disks()?;
    if disks.is_empty() {
        return Err("No accessible disks found. Run as Administrator.".into());
    }
    for disk in &disks {
        println!(
            "{}: {} ({:.2} GiB)",
            disk.number,
            disk.model,
            disk.size as f64 / 1_073_741_824.0
        );
    }
    let number = match number {
        Some(n) => n,
        None => {
            print!("Disk number: ");
            io::stdout().flush().map_err(|e| e.to_string())?;
            let mut s = String::new();
            io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
            s.trim().parse().map_err(|_| "invalid disk number")?
        }
    };
    let disk = disks
        .iter()
        .find(|d| d.number == number)
        .ok_or("disk not found")?;
    println!(
        "ALL DATA on disk {} ({}, {} bytes) will be destroyed. Type ERASE {} to continue:",
        disk.number, disk.model, disk.size, disk.number
    );
    let mut confirmation = String::new();
    io::stdin()
        .read_line(&mut confirmation)
        .map_err(|e| e.to_string())?;
    if confirmation.trim() != format!("ERASE {}", disk.number) {
        return Err("cancelled".into());
    }
    let count = disk::write_image(Path::new(image), disk, allow_large, |n| {
        eprint!("\r{n} bytes written");
    })?;
    eprintln!("\nDone: {count} bytes written.");
    Ok(())
}
