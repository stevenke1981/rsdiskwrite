//! Sector-sized block copy and comparison between a source and a target stream.

use crate::hash::Hasher;
use std::io::{self, Read, Write};

/// Size of each I/O chunk, rounded down to a multiple of the sector size.
pub const CHUNK_SIZE: usize = 1024 * 1024;

/// Copies exactly `total` bytes from `source` to `target` in chunks that are a multiple of
/// `sector` bytes (`sector` must be non-zero and at most [`CHUNK_SIZE`]), then flushes.
///
/// `progress` receives the cumulative byte count after every chunk. When `hasher` is
/// given, every byte written is also fed to it (so the caller gets the SHA-256 of what was
/// written without a second pass). A short source fails with
/// `io::ErrorKind::UnexpectedEof`. Returns the number of bytes written (= `total`).
pub fn copy(
    mut source: impl Read,
    mut target: impl Write,
    total: u64,
    sector: usize,
    progress: &dyn Fn(u64),
    mut hasher: Option<&mut Hasher>,
) -> io::Result<u64> {
    if sector == 0 || sector > CHUNK_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid sector size {sector}"),
        ));
    }
    let mut buffer = vec![0u8; (CHUNK_SIZE / sector) * sector];
    let mut count = 0;
    while count < total {
        let take = (total - count).min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..take])?;
        target.write_all(&buffer[..take])?;
        if let Some(hasher) = hasher.as_deref_mut() {
            hasher.update(&buffer[..take]);
        }
        count += take as u64;
        progress(count);
    }
    target.flush()?;
    Ok(count)
}

/// Compares the first `total` bytes of `expected` (the image) and `actual` (the disk),
/// reading in chunks that are a multiple of `sector` bytes (required for raw device
/// handles). Returns `Ok(None)` when identical, or `Ok(Some(offset))` with the byte offset
/// of the first difference. A short read on either side is `UnexpectedEof`.
/// `progress` receives the cumulative number of bytes compared.
pub fn compare(
    expected: impl Read,
    actual: impl Read,
    total: u64,
    sector: usize,
    progress: &dyn Fn(u64),
) -> io::Result<Option<u64>> {
    // TODO(phase2-A): implement read-back comparison (sector-aligned reads of `actual`).
    let _ = (expected, actual, total, sector, progress);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "read-back comparison is not implemented in this build",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_detects_short_input() {
        let mut dest = Vec::new();
        assert_eq!(
            copy(&b"abc"[..], &mut dest, 3, 1, &|_| {}, None).unwrap(),
            3
        );
        assert_eq!(dest, b"abc");
        assert_eq!(
            copy(&b"ab"[..], Vec::new(), 3, 1, &|_| {}, None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn copy_feeds_the_hasher() {
        let mut hasher = Hasher::new();
        copy(&b"abc"[..], Vec::new(), 3, 1, &|_| {}, Some(&mut hasher)).unwrap();
        assert_eq!(
            hasher.finish().to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
