//! Sector-sized block copy and comparison between a source and a target stream.

use crate::hash::{Digest, Hasher};
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
    let mut buffer = vec![0u8; chunk_len(sector)?];
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
    mut expected: impl Read,
    mut actual: impl Read,
    total: u64,
    sector: usize,
    progress: &dyn Fn(u64),
) -> io::Result<Option<u64>> {
    let chunk = chunk_len(sector)?;
    let mut want = vec![0u8; chunk];
    let mut got = vec![0u8; chunk];
    let mut count = 0u64;
    while count < total {
        let take = (total - count).min(chunk as u64) as usize;
        expected.read_exact(&mut want[..take])?;
        read_aligned(&mut actual, &mut got, take, sector)?;
        if let Some(index) = want[..take]
            .iter()
            .zip(&got[..take])
            .position(|(a, b)| a != b)
        {
            return Ok(Some(count + index as u64));
        }
        count += take as u64;
        progress(count);
    }
    Ok(None)
}

/// SHA-256 of the first `total` bytes of `reader` (e.g. a raw disk handle), reading in
/// chunks that are a multiple of `sector` bytes. `total` need not be sector-aligned: the
/// last read is rounded up to a whole sector and only the first `total` bytes are hashed.
/// A short read is `UnexpectedEof`. `progress` receives the cumulative bytes hashed.
pub fn hash_aligned(
    mut reader: impl Read,
    total: u64,
    sector: usize,
    progress: &dyn Fn(u64),
) -> io::Result<Digest> {
    let chunk = chunk_len(sector)?;
    let mut buffer = vec![0u8; chunk];
    let mut hasher = Hasher::new();
    let mut count = 0u64;
    while count < total {
        let take = (total - count).min(chunk as u64) as usize;
        read_aligned(&mut reader, &mut buffer, take, sector)?;
        hasher.update(&buffer[..take]);
        count += take as u64;
        progress(count);
    }
    Ok(hasher.finish())
}

/// Validates `sector` and returns the chunk length (largest multiple of `sector` that
/// fits in [`CHUNK_SIZE`]).
fn chunk_len(sector: usize) -> io::Result<usize> {
    if sector == 0 || sector > CHUNK_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid sector size {sector}"),
        ));
    }
    Ok((CHUNK_SIZE / sector) * sector)
}

/// Reads at least `want` bytes into `buffer`, requesting `want` rounded up to a whole
/// number of sectors (raw device handles reject unaligned lengths). Bytes past `want` are
/// padding: hitting EOF inside the padding is fine (ordinary files and slices), hitting it
/// before `want` bytes is `UnexpectedEof`.
fn read_aligned(
    reader: &mut impl Read,
    buffer: &mut [u8],
    want: usize,
    sector: usize,
) -> io::Result<()> {
    let aligned = want
        .div_ceil(sector)
        .saturating_mul(sector)
        .min(buffer.len());
    let mut filled = 0;
    while filled < aligned {
        match reader.read(&mut buffer[filled..aligned]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if filled < want {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("short read: got {filled} of {want} bytes"),
        ));
    }
    Ok(())
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

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn compare_matching_input() {
        let data = pattern(3 * 512);
        let last = std::cell::Cell::new(0);
        let result = compare(&data[..], &data[..], 1536, 512, &|n| last.set(n)).unwrap();
        assert_eq!(result, None);
        assert_eq!(last.get(), 1536);
        // Comparing a prefix only reads what is needed.
        assert_eq!(
            compare(&data[..], &data[..], 1024, 512, &|_| {}).unwrap(),
            None
        );
    }

    #[test]
    fn compare_reports_first_mismatch_offset() {
        let expected = pattern(4096);
        let mut actual = expected.clone();
        actual[1500] ^= 0xff;
        actual[3000] ^= 0xff;
        assert_eq!(
            compare(&expected[..], &actual[..], 4096, 512, &|_| {}).unwrap(),
            Some(1500)
        );
        // Offsets are absolute across chunk boundaries.
        let total = CHUNK_SIZE + 4096;
        let expected = pattern(total);
        let mut actual = expected.clone();
        actual[CHUNK_SIZE + 7] ^= 1;
        assert_eq!(
            compare(&expected[..], &actual[..], total as u64, 4096, &|_| {}).unwrap(),
            Some(CHUNK_SIZE as u64 + 7)
        );
        assert_eq!(
            compare(&expected[..], &actual[..], CHUNK_SIZE as u64, 4096, &|_| {}).unwrap(),
            None
        );
    }

    #[test]
    fn compare_detects_short_input() {
        let data = pattern(1024);
        let eof = |e: io::Error| e.kind();
        assert_eq!(
            compare(&data[..512], &data[..], 1024, 512, &|_| {})
                .map_err(eof)
                .unwrap_err(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(
            compare(&data[..], &data[..1000], 1024, 512, &|_| {})
                .map_err(eof)
                .unwrap_err(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(
            compare(&data[..], &data[..], 1024, 0, &|_| {})
                .map_err(eof)
                .unwrap_err(),
            io::ErrorKind::InvalidInput
        );
    }

    /// Reads from a slice, recording each requested length (like a raw device would see).
    struct Device<'a> {
        data: &'a [u8],
        lengths: Vec<usize>,
    }

    impl Read for Device<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.lengths.push(buf.len());
            self.data.read(buf)
        }
    }

    #[test]
    fn device_reads_are_sector_aligned_for_unaligned_totals() {
        let data = pattern(2048);
        let mut device = Device {
            data: &data,
            lengths: Vec::new(),
        };
        assert_eq!(
            compare(&data[..1000], &mut device, 1000, 512, &|_| {}).unwrap(),
            None
        );
        assert!(
            device.lengths.iter().all(|n| n % 512 == 0),
            "{:?}",
            device.lengths
        );
        assert_eq!(device.lengths[0], 1024);

        // A source that ends inside the padding of the last sector is still fine.
        assert_eq!(
            compare(&data[..1000], &data[..1000], 1000, 512, &|_| {}).unwrap(),
            None
        );

        let mut device = Device {
            data: &data,
            lengths: Vec::new(),
        };
        let digest = hash_aligned(&mut device, 1000, 512, &|_| {}).unwrap();
        assert!(device.lengths.iter().all(|n| n % 512 == 0));
        let mut hasher = Hasher::new();
        hasher.update(&data[..1000]);
        assert_eq!(digest, hasher.finish());
        assert_eq!(
            hash_aligned(&data[..999], 1000, 512, &|_| {})
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
