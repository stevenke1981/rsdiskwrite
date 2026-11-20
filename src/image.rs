//! Image files: format detection, inspection and preparation for writing.
//!
//! Supported formats are raw images and gzip (including concatenated gzip members).
//! Bytes after the last gzip member (e.g. an m0n0wall signature block) are ignored.

use crate::error::{Error, ErrorKind, Result};
use crate::hash::Digest;
use flate2::bufread::GzDecoder;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Image container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    /// Written byte-for-byte.
    Raw,
    /// gzip-compressed (magic `1f 8b`); decompressed to a temporary file before writing.
    Gzip,
}

/// Result of [`inspect`] (the `inspect` command's JSON `data`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// Canonical path of the image.
    pub path: PathBuf,
    pub format: ImageFormat,
    /// Size of the file as stored, in bytes.
    pub file_size: u64,
    /// Number of bytes that would be written to the disk (= `file_size` for raw).
    pub decompressed_size: u64,
    /// Number of gzip members (0 for raw).
    pub gzip_members: u32,
    /// Bytes after the last gzip member that are not written (0 for raw).
    pub trailing_bytes: u64,
    /// SHA-256 of the file as stored (what download pages publish and what
    /// `--expect-sha256` is compared against); `None` unless requested.
    pub sha256: Option<Digest>,
    /// SHA-256 of the decompressed bytes that would be written; `None` unless requested.
    pub content_sha256: Option<Digest>,
}

/// An image ready to be copied: positioned at offset 0, exactly `size` bytes long.
#[derive(Debug)]
pub struct Prepared {
    /// For raw images the original file (opened sharing only reads so it cannot be
    /// modified meanwhile); for gzip the temporary file holding the decompressed bytes.
    pub file: File,
    /// Number of bytes to write.
    pub size: u64,
    /// Format of the source file.
    pub format: ImageFormat,
}

fn open_source(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Keep the source stable while reading it. Sharing only reads also blocks replacement.
        options.share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ);
    }
    options.open(path).map_err(|e| {
        Error::io(
            "Cannot open image for reading (close any program modifying it)",
            e,
        )
    })
}

/// Detects the format from the first two bytes (gzip magic `1f 8b`, otherwise raw).
pub fn detect_format(path: &Path) -> Result<ImageFormat> {
    let mut file = open_source(path)?;
    let mut magic = [0u8; 2];
    let mut read = 0;
    while read < 2 {
        match file.read(&mut magic[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Error::io("Cannot read image", e)),
        }
    }
    Ok(if read == 2 && magic == [0x1f, 0x8b] {
        ImageFormat::Gzip
    } else {
        ImageFormat::Raw
    })
}

/// Inspects `path` without writing anything: format, sizes, gzip member count and
/// trailing bytes (streams through the whole file for gzip). When `sha256` is true also
/// computes [`ImageInfo::sha256`] and [`ImageInfo::content_sha256`].
/// Corrupt gzip data is [`ErrorKind::InvalidImage`].
pub fn inspect(path: &Path, sha256: bool) -> Result<ImageInfo> {
    // TODO(phase2-A): implement streaming inspection (members, trailing bytes, hashes).
    let _ = (path, sha256);
    Err(Error::unsupported("image inspection"))
}

/// Prepares `path` for writing to a disk of `capacity` bytes.
///
/// Raw images are returned as-is (file opened sharing reads only). Gzip images are
/// decompressed member by member into the file returned by `make_temp` (called at most
/// once, only for gzip), checking each member's CRC trailer; trailing non-gzip bytes are
/// ignored. Fails with [`ErrorKind::SafetyRefusal`] when the result exceeds `capacity`,
/// [`ErrorKind::InvalidImage`] when it is empty or the gzip data is corrupt, and
/// [`ErrorKind::Io`] for file errors.
pub fn prepare(
    path: &Path,
    capacity: u64,
    make_temp: impl FnOnce() -> Result<File>,
) -> Result<Prepared> {
    let file = open_source(path)?;
    let io_err = |e: io::Error| Error::io("Cannot read image", e);
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 2];
    let read = reader.read(&mut magic).map_err(io_err)?;
    reader.seek(SeekFrom::Start(0)).map_err(io_err)?;
    let format = if read == 2 && magic == [0x1f, 0x8b] {
        ImageFormat::Gzip
    } else {
        ImageFormat::Raw
    };
    let mut prepared = if format == ImageFormat::Gzip {
        let mut temp = make_temp()?;
        let mut remaining = capacity;
        loop {
            // Allow one extra byte to detect oversized images while still checking the gzip
            // trailer when the decompressed size is exactly the remaining capacity.
            let mut member = GzDecoder::new(&mut reader).take(remaining.saturating_add(1));
            let decoded = io::copy(&mut member, &mut temp).map_err(|e| {
                Error::new(
                    ErrorKind::InvalidImage,
                    format!("Cannot decompress image: {e}"),
                )
            })?;
            remaining = remaining.checked_sub(decoded).ok_or_else(|| {
                Error::refusal(format!(
                    "Decompressed image exceeds disk capacity ({capacity} bytes)"
                ))
            })?;
            let mut next = [0u8; 2];
            let first = reader.read(&mut next[..1]).map_err(io_err)?;
            let second = if first == 1 {
                reader.read(&mut next[1..]).map_err(io_err)?
            } else {
                0
            };
            reader
                .seek(SeekFrom::Current(-((first + second) as i64)))
                .map_err(io_err)?;
            if first + second != 2 || next != [0x1f, 0x8b] {
                break;
            }
        }
        temp
    } else {
        reader.into_inner()
    };
    let size = prepared.metadata().map_err(io_err)?.len();
    if size > capacity {
        return Err(Error::refusal(format!(
            "Image ({size} bytes) exceeds disk capacity ({capacity} bytes)"
        )));
    }
    prepared.seek(SeekFrom::Start(0)).map_err(io_err)?;
    if size == 0 {
        return Err(Error::new(ErrorKind::InvalidImage, "Image is empty"));
    }
    Ok(Prepared {
        file: prepared,
        size,
        format,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;

    fn temp_file() -> Result<File> {
        tempfile::tempfile().map_err(|e| Error::io("temp", e))
    }

    fn read_all(mut prepared: Prepared) -> Vec<u8> {
        let mut bytes = Vec::new();
        prepared.file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn gzip_magic_and_trailing_signature_are_handled() {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&vec![42u8; 1024]).unwrap();
        temp.write_all(&gzip.finish().unwrap()).unwrap();
        temp.write_all(b"signature").unwrap();
        let temp = temp.into_temp_path();
        assert_eq!(detect_format(&temp).unwrap(), ImageFormat::Gzip);
        let image = prepare(&temp, 1024, temp_file).unwrap();
        assert_eq!(image.size, 1024);
        assert_eq!(image.format, ImageFormat::Gzip);
        assert_eq!(read_all(image), vec![42u8; 1024]);
    }

    #[test]
    fn concatenated_gzip_members_are_all_decoded() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for part in [b"first".as_slice(), b"second".as_slice()] {
            let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
            gzip.write_all(part).unwrap();
            file.write_all(&gzip.finish().unwrap()).unwrap();
        }
        let file = file.into_temp_path();
        let image = prepare(&file, u64::MAX, temp_file).unwrap();
        assert_eq!(image.size, 11);
        assert_eq!(read_all(image), b"firstsecond");
    }

    #[test]
    fn gzip_member_magic_crosses_buffer_boundary() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut gzip = GzEncoder::new(Vec::new(), Compression::none());
        gzip.write_all(&vec![7u8; 8168]).unwrap();
        let member = gzip.finish().unwrap();
        assert_eq!(member.len(), 8191);
        file.write_all(&member).unwrap();
        let mut next = GzEncoder::new(Vec::new(), Compression::default());
        next.write_all(b"next").unwrap();
        file.write_all(&next.finish().unwrap()).unwrap();
        let file = file.into_temp_path();
        let image = prepare(&file, u64::MAX, temp_file).unwrap();
        assert_eq!(image.size, 8172);
        assert_eq!(&read_all(image)[8168..], b"next");
    }

    #[cfg(windows)]
    #[test]
    fn raw_image_excludes_other_writers_while_prepared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.img");
        std::fs::write(&path, [42u8; 512]).unwrap();
        assert_eq!(detect_format(&path).unwrap(), ImageFormat::Raw);
        let source = prepare(&path, 512, || panic!("raw image used a temp file")).unwrap();
        assert_eq!(source.format, ImageFormat::Raw);
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(source);
        assert!(OpenOptions::new().write(true).open(&path).is_ok());
    }

    #[test]
    fn empty_and_oversized_raw_images_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.img");
        std::fs::write(&path, []).unwrap();
        assert_eq!(
            prepare(&path, 512, temp_file).unwrap_err().kind,
            ErrorKind::InvalidImage
        );
        std::fs::write(&path, [0u8; 1024]).unwrap();
        assert_eq!(
            prepare(&path, 512, temp_file).unwrap_err().kind,
            ErrorKind::SafetyRefusal
        );
    }

    #[test]
    fn gzip_capacity_is_enforced_across_members() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.gz");
        let output = dir.path().join("output.img");
        let mut source = File::create(&path).unwrap();
        for _ in 0..2 {
            let mut gzip = GzEncoder::new(&mut source, Compression::default());
            gzip.write_all(&[42u8; 1024]).unwrap();
            gzip.finish().unwrap();
        }
        drop(source);
        let error = prepare(&path, 1536, || {
            File::create(&output).map_err(|e| Error::io("create", e))
        })
        .unwrap_err();
        assert!(error.message.contains("exceeds disk capacity"), "{error}");
        assert_eq!(error.kind, ErrorKind::SafetyRefusal);
        assert_eq!(std::fs::metadata(output).unwrap().len(), 1537);
        assert_eq!(prepare(&path, 2048, temp_file).unwrap().size, 2048);

        let mut corrupt = std::fs::read(&path).unwrap();
        let crc = corrupt.len() - 8;
        corrupt[crc] ^= 1;
        std::fs::write(&path, corrupt).unwrap();
        let error = prepare(&path, 2048, temp_file).unwrap_err();
        assert!(error.message.contains("Cannot decompress"), "{error}");
        assert_eq!(error.kind, ErrorKind::InvalidImage);
    }
}
