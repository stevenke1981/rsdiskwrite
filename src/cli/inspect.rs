//! `rsdiskwrite inspect IMAGE`.

use crate::cli::InspectArgs;
use crate::error::Result;
use crate::image::{self, ImageInfo};
use crate::output::{Output, Style, format_size};

/// Inspects an image. JSON `data` is [`ImageInfo`].
pub fn run(args: InspectArgs, out: &Output) -> Result<()> {
    let info = image::inspect(&args.image, args.sha256)?;
    out.emit(&info, render)
}

/// Human rendering of [`ImageInfo`]: aligned key/value lines.
pub fn render(info: &ImageInfo, out: &Output) -> String {
    let mut rows: Vec<(&str, String)> = vec![
        ("Image", info.path.display().to_string()),
        ("Format", format!("{:?}", info.format).to_lowercase()),
        (
            "File size",
            format!("{} ({} bytes)", format_size(info.file_size), info.file_size),
        ),
        (
            "Writes",
            format!(
                "{} ({} bytes)",
                format_size(info.decompressed_size),
                info.decompressed_size
            ),
        ),
    ];
    if info.gzip_members > 0 {
        rows.push(("Gzip members", info.gzip_members.to_string()));
    }
    if info.trailing_bytes > 0 {
        rows.push((
            "Trailing bytes",
            format!("{} (ignored, not written)", info.trailing_bytes),
        ));
    }
    if let Some(sha) = info.sha256 {
        rows.push(("SHA-256 file", sha.to_string()));
    }
    if let Some(sha) = info.content_sha256 {
        rows.push(("SHA-256 data", sha.to_string()));
    }
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0) + 1;
    let mut text = rows
        .iter()
        .map(|(k, v)| format!("{:<width$} {v}", format!("{k}:")))
        .map(|line| match line.split_once(' ') {
            Some((k, rest)) => format!("{} {rest}", out.paint(k, Style::Bold)),
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if info.trailing_bytes > 0 {
        text.push_str(&format!(
            "\n{} data after the compressed stream is ignored; check the image source.",
            out.paint("note:", Style::Yellow)
        ));
    }
    if info.sha256.is_none() && !out.is_quiet() {
        text.push_str(&format!(
            "\n{}",
            out.paint("Add --sha256 to compute checksums.", Style::Dim)
        ));
    }
    text
}
