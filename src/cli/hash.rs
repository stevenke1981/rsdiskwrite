//! `rsdiskwrite hash IMAGE` / `rsdiskwrite hash --disk N`.

use crate::cli::HashArgs;
use crate::error::{Error, Result};
use crate::hash::{self, Digest};
use crate::ops::{self, Phase, Progress};
use crate::output::Output;
use serde::Serialize;
use std::path::PathBuf;

/// JSON `data` of the `hash` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HashData {
    /// Hashed image file, if any.
    pub image: Option<PathBuf>,
    /// Hashed disk number, if any.
    pub disk: Option<u32>,
    /// Number of bytes hashed.
    pub bytes: u64,
    pub sha256: Digest,
}

/// Hashes an image file (as stored) or a disk.
pub fn run(args: HashArgs, out: &Output) -> Result<()> {
    let sink = out.progress();
    let data = match (&args.image, args.disk) {
        (Some(image), None) => {
            let total = std::fs::metadata(image)
                .map_err(|e| Error::io(format!("cannot open {}", image.display()), e))?
                .len();
            let digest = hash::sha256_file(image, &|done| {
                sink.update(Progress {
                    phase: Phase::Hashing,
                    done,
                    total,
                })
            });
            sink.finish();
            HashData {
                image: Some(image.clone()),
                disk: None,
                bytes: total,
                sha256: digest?,
            }
        }
        (None, Some(disk)) => {
            // TODO(phase2-C): report the actual byte count hashed (whole disk size).
            let digest = ops::hash_disk(disk, args.length, &|p| sink.update(p));
            sink.finish();
            HashData {
                image: None,
                disk: Some(disk),
                bytes: args.length.unwrap_or(0),
                sha256: digest?,
            }
        }
        _ => return Err(Error::usage("give either IMAGE or --disk N")),
    };
    out.emit(&data, |data, _| {
        let target = data
            .image
            .as_ref()
            .map(|p| p.display().to_string())
            .or(data.disk.map(|d| format!("PhysicalDrive{d}")))
            .unwrap_or_default();
        format!("{}  {target}", data.sha256)
    })
}
