//! High-level operations: plan (dry run, all safety refusals), execute (write) and verify.
//!
//! Data types are platform-independent and serializable (they are the `data` of the
//! `write`/`verify` JSON output and MCP results); the operations are Windows-only.

use crate::disk::{Disk, DiskIdentity};
use crate::error::{Error, Result};
use crate::hash::Digest;
use crate::image::ImageFormat;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(windows)]
use crate::error::ErrorKind;
#[cfg(windows)]
use std::path::Path;

/// What the caller wants written. Build with [`WriteRequest::new`] and adjust fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteRequest {
    /// Image file (raw or gzip).
    pub image: PathBuf,
    /// Target `PhysicalDriveN` number.
    pub disk: u32,
    /// When set, planning fails with [`crate::ErrorKind::IdentityChanged`] unless the disk
    /// currently at `disk` has exactly this identity (what the user saw/confirmed).
    pub expected: Option<DiskIdentity>,
    /// Permit disks larger than [`crate::disk::LARGE_DISK_THRESHOLD`] (`-u`).
    pub allow_large: bool,
    /// Permit non-removable disks on non-USB/SD/MMC buses (`--allow-fixed`).
    pub allow_fixed: bool,
    /// Read the disk back after writing and compare (default `true`; `--no-verify`).
    pub verify: bool,
    /// Expected SHA-256 of the image file *as stored* (compressed bytes for `.gz`);
    /// mismatch is [`crate::ErrorKind::HashMismatch`] before anything is written.
    pub expect_sha256: Option<Digest>,
    /// Compute [`PlannedImage::sha256`] during planning even without `expect_sha256`
    /// (used by MCP `plan_write`).
    pub hash_image: bool,
}

impl WriteRequest {
    /// Request with safe defaults: no large/fixed disks, verify on, no hash expectations.
    pub fn new(image: impl Into<PathBuf>, disk: u32) -> Self {
        Self {
            image: image.into(),
            disk,
            expected: None,
            allow_large: false,
            allow_fixed: false,
            verify: true,
            expect_sha256: None,
            hash_image: false,
        }
    }
}

/// Machine-readable reason a write is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    /// Target holds `%SystemRoot%`.
    SystemDisk,
    /// Target holds the image file.
    SourceDisk,
    /// Target holds the temp directory needed to decompress a gzip image.
    TempDisk,
    /// Target is not removable and not on a USB/SD/MMC bus, without `allow_fixed`.
    FixedDisk,
    /// Target is larger than 2 GiB, without `allow_large`.
    LargeDisk,
    /// Image (raw size, or decompressed size when known) exceeds the disk.
    ImageTooLarge,
    /// Image size is not a multiple of the disk's logical sector size.
    SectorMisaligned,
}

/// One safety refusal found while planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
    pub hint: Option<String>,
}

impl Refusal {
    /// The refusal as an [`crate::ErrorKind::SafetyRefusal`] error.
    pub fn to_error(&self) -> Error {
        let error = Error::refusal(self.message.clone());
        match &self.hint {
            Some(hint) => error.with_hint(hint.clone()),
            None => error,
        }
    }
}

/// The image as seen while planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedImage {
    /// Canonical image path.
    pub path: PathBuf,
    pub format: ImageFormat,
    /// File size as stored.
    pub file_size: u64,
    /// SHA-256 of the file as stored, when `hash_image` or `expect_sha256` was requested.
    pub sha256: Option<Digest>,
}

/// Result of [`plan`]: everything [`execute`] needs, plus all refusals. Producing a plan
/// never writes anything; it is the `--dry-run` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritePlan {
    pub request: WriteRequest,
    /// Target as described at planning time; [`execute`] re-checks this identity after
    /// locking the volumes.
    pub disk: Disk,
    pub image: PlannedImage,
    /// Empty when the write is allowed.
    pub refusals: Vec<Refusal>,
    /// Non-fatal notes for the user (e.g. "gzip image will be decompressed to %TEMP%").
    pub warnings: Vec<String>,
}

impl WritePlan {
    /// True when there are no refusals.
    pub fn is_allowed(&self) -> bool {
        self.refusals.is_empty()
    }

    /// `Ok(())` when allowed, otherwise the first refusal as a
    /// [`crate::ErrorKind::SafetyRefusal`] error.
    pub fn check(&self) -> Result<()> {
        match self.refusals.first() {
            Some(refusal) => Err(refusal.to_error()),
            None => Ok(()),
        }
    }
}

/// Stage of a long-running operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Opening / decompressing the image (`total` = file size).
    Preparing,
    /// Hashing an image or disk.
    Hashing,
    /// Writing to the disk.
    Writing,
    /// Flushing device caches.
    Flushing,
    /// Reading back and comparing.
    Verifying,
}

/// A progress update: `done` of `total` bytes in `phase` (`total` may be 0 if unknown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

/// Result of [`verify`] (and of the verify step of [`execute`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Disk that was read.
    pub disk: DiskIdentity,
    pub image: PathBuf,
    /// Number of bytes compared (the image's decompressed size).
    pub bytes: u64,
    /// True when every compared byte matched.
    pub matched: bool,
    /// Byte offset of the first difference when `matched` is false.
    pub first_mismatch: Option<u64>,
    /// SHA-256 of the compared image content, when computed.
    pub content_sha256: Option<Digest>,
}

/// Result of a successful [`execute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteReport {
    pub disk: Disk,
    pub image: PathBuf,
    pub format: ImageFormat,
    pub bytes_written: u64,
    /// SHA-256 of the bytes written, when computed.
    pub content_sha256: Option<Digest>,
    /// Read-back verification result; `None` when verification was not requested
    /// (or is unavailable in this build).
    pub verify: Option<VerifyReport>,
    /// Wall-clock duration of the whole operation.
    pub elapsed_ms: u64,
}

/// Facts about the image needed to evaluate refusals.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImageFacts {
    pub format: ImageFormat,
    pub file_size: u64,
}

/// Pure refusal evaluation (no I/O). `source`/`temp` are the disks holding the image and
/// the temp directory; `temp` is only consulted for gzip images.
pub(crate) fn evaluate(
    disk: &Disk,
    request: &WriteRequest,
    image: ImageFacts,
    source: &[u32],
    temp: &[u32],
) -> Vec<Refusal> {
    let mut refusals = Vec::new();
    let mut refuse = |code, message: String, hint: Option<&str>| {
        refusals.push(Refusal {
            code,
            message,
            hint: hint.map(str::to_owned),
        })
    };
    if disk.is_system {
        refuse(
            RefusalCode::SystemDisk,
            "Refusing to overwrite the Windows system disk".into(),
            None,
        );
    }
    if source.contains(&disk.number) {
        refuse(
            RefusalCode::SourceDisk,
            "Refusing to overwrite the disk containing the source image".into(),
            Some("Copy the image to another disk first"),
        );
    }
    if image.format == ImageFormat::Gzip && temp.contains(&disk.number) {
        refuse(
            RefusalCode::TempDisk,
            "Refusing to overwrite the disk containing the temporary image directory".into(),
            Some("Point TEMP/TMP at a directory on another disk"),
        );
    }
    if disk.is_large() && !request.allow_large {
        refuse(
            RefusalCode::LargeDisk,
            "Disk is larger than 2 GiB. Explicitly enable large disks to proceed.".into(),
            Some("Pass -u/--allow-large if this really is the right disk"),
        );
    }
    if !disk.is_removable_target() && !request.allow_fixed {
        refuse(
            RefusalCode::FixedDisk,
            format!("Refusing to overwrite a fixed {} disk", disk.bus.label()),
            Some("Pass --allow-fixed if this really is the right disk"),
        );
    }
    if image.format == ImageFormat::Raw {
        if image.file_size > disk.size {
            refuse(
                RefusalCode::ImageTooLarge,
                format!(
                    "Image ({} bytes) exceeds disk capacity ({} bytes)",
                    image.file_size, disk.size
                ),
                None,
            );
        }
        if disk.sector != 0 && !image.file_size.is_multiple_of(u64::from(disk.sector)) {
            refuse(
                RefusalCode::SectorMisaligned,
                format!(
                    "Image size is not a multiple of the disk's {}-byte logical sector size",
                    disk.sector
                ),
                None,
            );
        }
    }
    refusals
}

/// Builds a [`WritePlan`] for `request` without writing anything (`--dry-run`).
///
/// Errors (not refusals): [`ErrorKind::DiskNotFound`], [`ErrorKind::IdentityChanged`]
/// (when `request.expected` differs), [`ErrorKind::HashMismatch`] (when
/// `request.expect_sha256` differs from the file's SHA-256), [`ErrorKind::Io`].
/// Safety problems are collected in [`WritePlan::refusals`].
#[cfg(windows)]
pub fn plan(request: &WriteRequest) -> Result<WritePlan> {
    use crate::disk;
    let disk = disk::get(request.disk)?;
    if let Some(expected) = &request.expected
        && *expected != disk.identity()
    {
        return Err(Error::new(
            ErrorKind::IdentityChanged,
            format!("{} is no longer the disk that was selected", disk.name()),
        )
        .with_hint("Refresh the disk list and select the target again"));
    }
    let path = std::fs::canonicalize(&request.image)
        .map_err(|e| Error::io(format!("Cannot open image {}", request.image.display()), e))?;
    let file_size = std::fs::metadata(&path)
        .map_err(|e| Error::io("Cannot read image", e))?
        .len();
    let format = crate::image::detect_format(&path)?;
    let sha256 = if request.hash_image || request.expect_sha256.is_some() {
        Some(crate::hash::sha256_file(&path, &|_| {})?)
    } else {
        None
    };
    if let (Some(expected), Some(actual)) = (request.expect_sha256, sha256)
        && expected != actual
    {
        return Err(Error::new(
            ErrorKind::HashMismatch,
            format!("Image SHA-256 is {actual}, expected {expected}"),
        ));
    }
    let source = disk::volume_disks(&path)?;
    let temp = if format == ImageFormat::Gzip {
        disk::volume_disks(&std::env::temp_dir())?
    } else {
        Vec::new()
    };
    let refusals = evaluate(
        &disk,
        request,
        ImageFacts { format, file_size },
        &source,
        &temp,
    );
    let mut warnings = Vec::new();
    if format == ImageFormat::Gzip {
        warnings.push(format!(
            "gzip image will be decompressed to {} before writing",
            std::env::temp_dir().display()
        ));
    }
    Ok(WritePlan {
        request: request.clone(),
        disk,
        image: PlannedImage {
            path,
            format,
            file_size,
            sha256,
        },
        refusals,
        warnings,
    })
}

/// Executes an allowed plan: prepares (decompresses) the image, locks and dismounts the
/// target's volumes, opens the disk for writing, re-checks its identity, writes, flushes,
/// and (when `plan.request.verify`) reads back and compares.
///
/// Errors: [`ErrorKind::SafetyRefusal`] if the plan has refusals (nothing is touched),
/// [`ErrorKind::ElevationRequired`], [`ErrorKind::IdentityChanged`],
/// [`ErrorKind::VerifyMismatch`], [`ErrorKind::Io`]. `progress` is called from the calling
/// thread.
#[cfg(windows)]
pub fn execute(plan: &WritePlan, progress: &dyn Fn(Progress)) -> Result<WriteReport> {
    use crate::disk::{self, enumerate, ioctl};
    plan.check()?;
    let started = std::time::Instant::now();
    let disk = &plan.disk;
    progress(Progress {
        phase: Phase::Preparing,
        done: 0,
        total: plan.image.file_size,
    });
    let mut prepared = crate::image::prepare(&plan.image.path, disk.size, || {
        let temp_dir = std::env::temp_dir();
        if disk::volume_disks(&temp_dir)?.contains(&disk.number) {
            return Err(Error::refusal(
                "Refusing to overwrite the disk containing the temporary image directory",
            ));
        }
        tempfile::tempfile_in(temp_dir).map_err(|e| Error::io("Cannot create temporary image", e))
    })?;
    let size = prepared.size;
    let _locks = disk::lock_and_dismount(disk.number)?;
    let mut target = open_disk(disk.number, ioctl::Access::ReadWrite)?;
    let current = enumerate::describe(disk.number, &target)
        .map_err(|e| Error::io("Cannot recheck target", e))?;
    if !disk::same_identity(&current, disk) {
        return Err(Error::new(
            ErrorKind::IdentityChanged,
            "Disk identity changed; refresh and select it again",
        ));
    }
    let sector = current.sector;
    if sector == 0 || sector > 1024 * 1024 || !size.is_multiple_of(u64::from(sector)) {
        return Err(Error::refusal(format!(
            "Image size is not a multiple of the disk's {sector}-byte logical sector size"
        )));
    }
    let mut hasher = crate::hash::Hasher::new();
    crate::copy::copy(
        &mut prepared.file,
        &mut target,
        size,
        sector as usize,
        &|done| {
            progress(Progress {
                phase: Phase::Writing,
                done,
                total: size,
            })
        },
        Some(&mut hasher),
    )
    .map_err(|e| Error::io("Write failed; the disk may be partially overwritten", e))?;
    let content_sha256 = hasher.finish();
    progress(Progress {
        phase: Phase::Flushing,
        done: size,
        total: size,
    });
    target.sync_all().map_err(|e| {
        Error::io(
            "Disk flush failed; the disk may be partially overwritten",
            e,
        )
    })?;
    // Ask Windows to re-read the new partition table. Advisory: the data is already on the
    // disk, and replugging the device has the same effect, so a failure is not an error.
    let _ = ioctl::ioctl(&target, ioctl::IOCTL_DISK_UPDATE_PROPERTIES, &[], &mut []);
    let verify = if plan.request.verify {
        // Same handle, still holding the volume locks: what we read is what the device
        // stores, not something a remounted file system wrote in between.
        let first_mismatch = read_back(&mut prepared.file, &mut target, size, sector, progress)?;
        if let Some(offset) = first_mismatch {
            return Err(Error::new(
                ErrorKind::VerifyMismatch,
                format!(
                    "Verification failed: {} differs from the image at byte {offset}",
                    current.name()
                ),
            )
            .with_hint("The device may be faulty or counterfeit; try another one"));
        }
        Some(VerifyReport {
            disk: current.identity(),
            image: plan.image.path.clone(),
            bytes: size,
            matched: true,
            first_mismatch: None,
            content_sha256: Some(content_sha256),
        })
    } else {
        None
    };
    Ok(WriteReport {
        disk: current,
        image: plan.image.path.clone(),
        format: prepared.format,
        bytes_written: size,
        content_sha256: Some(content_sha256),
        verify,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Opens a physical disk for reading or writing, mapping "access denied" to
/// [`ErrorKind::ElevationRequired`] and a missing device to [`ErrorKind::DiskNotFound`].
#[cfg(windows)]
fn open_disk(number: u32, access: crate::disk::ioctl::Access) -> Result<std::fs::File> {
    use crate::disk::ioctl::{self, Access};
    ioctl::open_disk(number, access).map_err(|e| {
        let purpose = if access == Access::ReadWrite {
            "writing"
        } else {
            "reading"
        };
        let context = format!("Cannot open PhysicalDrive{number} for {purpose}");
        match e.kind() {
            std::io::ErrorKind::NotFound => Error::new(
                ErrorKind::DiskNotFound,
                format!("PhysicalDrive{number} does not exist"),
            )
            .with_hint("Run 'rsdiskwrite list' to see available disks"),
            std::io::ErrorKind::PermissionDenied => Error::io_elevation(context, e),
            _ => Error::io(context, e).with_hint("Close applications using the disk"),
        }
    })
}

/// Rewinds both handles and compares `size` bytes of `image` with `device` using
/// sector-aligned reads, reporting [`Phase::Verifying`] progress.
#[cfg(windows)]
fn read_back(
    image: &mut std::fs::File,
    device: &mut std::fs::File,
    size: u64,
    sector: u32,
    progress: &dyn Fn(Progress),
) -> Result<Option<u64>> {
    use std::io::{Seek, SeekFrom};
    progress(Progress {
        phase: Phase::Verifying,
        done: 0,
        total: size,
    });
    image
        .seek(SeekFrom::Start(0))
        .map_err(|e| Error::io("Cannot rewind image for verification", e))?;
    device
        .seek(SeekFrom::Start(0))
        .map_err(|e| Error::io("Cannot rewind disk for verification", e))?;
    crate::copy::compare(
        std::io::BufReader::with_capacity(crate::copy::CHUNK_SIZE, image),
        device,
        size,
        sector as usize,
        &|done| {
            progress(Progress {
                phase: Phase::Verifying,
                done,
                total: size,
            })
        },
    )
    .map_err(|e| Error::io("Read-back verification failed", e))
}

/// A reader that feeds everything it reads to a [`crate::hash::Hasher`].
#[cfg(windows)]
struct HashingReader<'a, R> {
    inner: R,
    hasher: &'a mut crate::hash::Hasher,
}

#[cfg(windows)]
impl<R: std::io::Read> std::io::Read for HashingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// Logical sector size to use for reads (512 when the device does not report one).
#[cfg(windows)]
fn read_sector(disk: &Disk) -> Result<u32> {
    match disk.sector {
        0 => Ok(512),
        s if s <= crate::copy::CHUNK_SIZE as u32 => Ok(s),
        s => Err(Error::new(
            ErrorKind::Unsupported,
            format!("Unsupported logical sector size {s}"),
        )),
    }
}

/// Reads disk `disk` back and compares it with `image` (decompressing gzip), without
/// writing. Requires elevation. Returns a report with `matched == false` on mismatch
/// (callers turn that into [`ErrorKind::VerifyMismatch`]).
#[cfg(windows)]
pub fn verify(image: &Path, disk: u32, progress: &dyn Fn(Progress)) -> Result<VerifyReport> {
    use crate::disk::{enumerate, ioctl::Access};
    let path = std::fs::canonicalize(image)
        .map_err(|e| Error::io(format!("Cannot open image {}", image.display()), e))?;
    let mut device = open_disk(disk, Access::Read)?;
    let current =
        enumerate::describe(disk, &device).map_err(|e| Error::io("Cannot query disk", e))?;
    let sector = read_sector(&current)?;
    progress(Progress {
        phase: Phase::Preparing,
        done: 0,
        total: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
    });
    let mut prepared = crate::image::prepare(&path, current.size, || {
        tempfile::tempfile_in(std::env::temp_dir())
            .map_err(|e| Error::io("Cannot create temporary image", e))
    })?;
    let size = prepared.size;
    let mut hasher = crate::hash::Hasher::new();
    let expected = HashingReader {
        inner: std::io::BufReader::with_capacity(crate::copy::CHUNK_SIZE, &mut prepared.file),
        hasher: &mut hasher,
    };
    progress(Progress {
        phase: Phase::Verifying,
        done: 0,
        total: size,
    });
    let first_mismatch =
        crate::copy::compare(expected, &mut device, size, sector as usize, &|done| {
            progress(Progress {
                phase: Phase::Verifying,
                done,
                total: size,
            })
        })
        .map_err(|e| Error::io("Read-back verification failed", e))?;
    Ok(VerifyReport {
        disk: current.identity(),
        image: path,
        bytes: size,
        matched: first_mismatch.is_none(),
        first_mismatch,
        // Only meaningful when every byte was read.
        content_sha256: first_mismatch.is_none().then(|| hasher.finish()),
    })
}

/// SHA-256 of the first `len` bytes of disk `disk` (whole disk when `None`). Requires
/// elevation. Reads are sector-aligned; `len` need not be.
#[cfg(windows)]
pub fn hash_disk(disk: u32, len: Option<u64>, progress: &dyn Fn(Progress)) -> Result<Digest> {
    use crate::disk::{enumerate, ioctl::Access};
    let device = open_disk(disk, Access::Read)?;
    let current =
        enumerate::describe(disk, &device).map_err(|e| Error::io("Cannot query disk", e))?;
    let sector = read_sector(&current)?;
    let total = match len {
        Some(len) if len > current.size => {
            return Err(Error::usage(format!(
                "Requested {len} bytes but {} has only {} bytes",
                current.name(),
                current.size
            )));
        }
        Some(len) => len,
        None => current.size,
    };
    crate::copy::hash_aligned(&device, total, sector as usize, &|done| {
        progress(Progress {
            phase: Phase::Hashing,
            done,
            total,
        })
    })
    .map_err(|e| Error::io(format!("Cannot read {}", current.name()), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{BusType, model::test_disk};

    fn codes(refusals: &[Refusal]) -> Vec<RefusalCode> {
        refusals.iter().map(|r| r.code).collect()
    }

    #[test]
    fn refusals_cover_every_rule() {
        let raw = ImageFacts {
            format: ImageFormat::Raw,
            file_size: 4096,
        };
        let mut disk = test_disk(1);
        let request = WriteRequest::new("x.img", 1);
        assert!(evaluate(&disk, &request, raw, &[0], &[1]).is_empty());

        disk.is_system = true;
        disk.size = 8 * 1024 * 1024 * 1024;
        disk.bus = BusType::Nvme;
        disk.removable = false;
        let gzip = ImageFacts {
            format: ImageFormat::Gzip,
            file_size: 4096,
        };
        assert_eq!(
            codes(&evaluate(&disk, &request, gzip, &[1], &[1])),
            [
                RefusalCode::SystemDisk,
                RefusalCode::SourceDisk,
                RefusalCode::TempDisk,
                RefusalCode::LargeDisk,
                RefusalCode::FixedDisk
            ]
        );
        let mut permissive = request.clone();
        permissive.allow_large = true;
        permissive.allow_fixed = true;
        disk.is_system = false;
        assert!(evaluate(&disk, &permissive, gzip, &[0], &[0]).is_empty());

        let small = test_disk(1);
        let odd = ImageFacts {
            format: ImageFormat::Raw,
            file_size: small.size + 1,
        };
        assert_eq!(
            codes(&evaluate(&small, &request, odd, &[], &[])),
            [RefusalCode::ImageTooLarge, RefusalCode::SectorMisaligned]
        );
    }

    #[test]
    fn plan_check_reports_first_refusal() {
        let plan = WritePlan {
            request: WriteRequest::new("x.img", 1),
            disk: test_disk(1),
            image: PlannedImage {
                path: "x.img".into(),
                format: ImageFormat::Raw,
                file_size: 512,
                sha256: None,
            },
            refusals: vec![Refusal {
                code: RefusalCode::LargeDisk,
                message: "large".into(),
                hint: None,
            }],
            warnings: vec![],
        };
        assert!(!plan.is_allowed());
        assert_eq!(plan.check().unwrap_err().exit_code(), 3);
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["refusals"][0]["code"], "large_disk");
    }
}
