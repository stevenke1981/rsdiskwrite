//! `rsdiskwrite inspect IMAGE`.

use crate::cli::InspectArgs;
use crate::error::Result;
use crate::image::{self, ImageInfo};
use crate::output::{Output, format_size};

/// Inspects an image. JSON `data` is [`ImageInfo`].
pub fn run(args: InspectArgs, out: &Output) -> Result<()> {
    let info = image::inspect(&args.image, args.sha256)?;
    out.emit(&info, render)
}

/// Human rendering of [`ImageInfo`].
pub fn render(info: &ImageInfo, _out: &Output) -> String {
    // TODO(phase2-C): polish the human layout.
    let mut text = format!(
        "Image:   {}\nFormat:  {:?}\nFile:    {} ({} bytes)\nWrites:  {} ({} bytes)",
        info.path.display(),
        info.format,
        format_size(info.file_size),
        info.file_size,
        format_size(info.decompressed_size),
        info.decompressed_size
    );
    if info.gzip_members > 0 {
        text.push_str(&format!("\nMembers: {}", info.gzip_members));
    }
    if info.trailing_bytes > 0 {
        text.push_str(&format!(
            "\nTrailing bytes (not written): {}",
            info.trailing_bytes
        ));
    }
    if let Some(sha) = info.sha256 {
        text.push_str(&format!("\nSHA-256 (file):    {sha}"));
    }
    if let Some(sha) = info.content_sha256 {
        text.push_str(&format!("\nSHA-256 (content): {sha}"));
    }
    text
}
