//! Volume to disk mapping and volume locking.

use crate::disk::ioctl::{self, Access};
use crate::error::{Error, Result};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDriveTypeW,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
};

/// `GetDriveTypeW` values for volumes that never live on a local physical disk target.
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;

/// A volume and the physical disks it spans.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VolumeInfo {
    /// `\\?\Volume{GUID}\` name.
    pub name: String,
    /// `GetDriveTypeW` result.
    pub drive_type: u32,
    /// Physical disk numbers (`None` when the extents could not be read).
    pub disks: Option<Vec<u32>>,
}

/// Locked and dismounted volume handles. Dropping releases the locks.
#[derive(Debug)]
pub struct VolumeLocks {
    handles: Vec<File>,
}

impl VolumeLocks {
    /// Number of volumes locked.
    pub fn count(&self) -> usize {
        self.handles.len()
    }
}

fn wide_until_nul(buffer: &[u16]) -> Result<String> {
    let end = buffer
        .iter()
        .position(|c| *c == 0)
        .ok_or_else(|| Error::internal("invalid volume name"))?;
    Ok(String::from_utf16_lossy(&buffer[..end]))
}

/// Physical disks holding the volume that contains `path` (which must exist).
pub fn volume_disks(path: &Path) -> Result<Vec<u32>> {
    let full = std::fs::canonicalize(path)
        .map_err(|e| Error::io(format!("Cannot resolve {}", path.display()), e))?;
    let wide: Vec<u16> = full.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = [0u16; 1024];
    // SAFETY: Windows fills these buffers synchronously and reports errors via the return value.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(Error::io(
            format!("Cannot determine volume for {}", path.display()),
            io::Error::last_os_error(),
        ));
    }
    let mut volume = [0u16; 1024];
    // SAFETY: as above; `root` is NUL-terminated by GetVolumePathNameW.
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
    } == 0
    {
        return Err(Error::io(
            format!("Cannot identify volume for {}", path.display()),
            io::Error::last_os_error(),
        ));
    }
    let name = wide_until_nul(&volume)?;
    let handle = ioctl::open_device(name.trim_end_matches('\\'), Access::Query)
        .map_err(|e| Error::io(format!("Cannot inspect volume for {}", path.display()), e))?;
    disk_extents(&handle)
}

/// Physical disks holding `%SystemRoot%`.
pub fn system_disks() -> Result<Vec<u32>> {
    let windows = std::env::var_os("SystemRoot")
        .ok_or_else(|| Error::internal("Cannot determine Windows system volume"))?;
    volume_disks(Path::new(&windows))
}

fn disk_extents(handle: &File) -> Result<Vec<u32>> {
    let mut extents = [0u8; 4096];
    let n = ioctl::ioctl(
        handle,
        ioctl::IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        &[],
        &mut extents,
    )
    .map_err(|e| Error::io("Cannot identify volume disks", e))?;
    let extents = &extents[..n];
    // VOLUME_DISK_EXTENTS: NumberOfDiskExtents u32, padding, then 24-byte DISK_EXTENTs
    // whose first field is the DiskNumber.
    let count = ioctl::read_u32(extents, 0)
        .ok_or_else(|| Error::internal("Invalid volume extents response"))?
        as usize;
    if count == 0 || 8 + count * 24 > extents.len() {
        return Err(Error::internal(
            "Invalid or incomplete volume extents response",
        ));
    }
    (0..count)
        .map(|i| {
            ioctl::read_u32(extents, 8 + i * 24)
                .ok_or_else(|| Error::internal("Invalid volume extents response"))
        })
        .collect()
}

struct VolumeSearch(HANDLE);
impl Drop for VolumeSearch {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful FindFirstVolumeW.
        unsafe {
            FindVolumeClose(self.0);
        }
    }
}

/// All volume GUID names with their drive type.
fn volume_names() -> Result<Vec<(String, u32)>> {
    let mut name = [0u16; 1024];
    // SAFETY: FindFirstVolumeW supplies a search handle closed by VolumeSearch.
    let find = VolumeSearch(unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) });
    if find.0 == INVALID_HANDLE_VALUE {
        return Err(Error::io(
            "Cannot enumerate volumes",
            io::Error::last_os_error(),
        ));
    }
    let mut names = Vec::new();
    loop {
        let path = wide_until_nul(&name)?;
        // SAFETY: `name` is a NUL-terminated volume GUID path.
        let kind = unsafe { GetDriveTypeW(name.as_ptr()) };
        names.push((path, kind));
        name.fill(0);
        // SAFETY: valid search handle and buffer.
        if unsafe { FindNextVolumeW(find.0, name.as_mut_ptr(), name.len() as u32) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
                return Err(Error::io("Volume enumeration failed", error));
            }
            break;
        }
    }
    Ok(names)
}

/// Lists all volumes with the disks they span (unelevated; per-volume failures are
/// reported as `disks: None`).
pub fn volumes() -> Result<Vec<VolumeInfo>> {
    Ok(volume_names()?
        .into_iter()
        .map(|(name, drive_type)| {
            let disks = ioctl::open_device(name.trim_end_matches('\\'), Access::Query)
                .ok()
                .and_then(|f| disk_extents(&f).ok());
            VolumeInfo {
                name,
                drive_type,
                disks,
            }
        })
        .collect())
}

/// `FSCTL_LOCK_VOLUME`, retried briefly because Explorer, indexers and antivirus often
/// hold a volume open for a moment after it is mounted.
fn lock_volume(file: &File) -> io::Result<()> {
    const ATTEMPTS: u32 = 4;
    let mut attempt = 1;
    loop {
        match ioctl::ioctl(file, ioctl::FSCTL_LOCK_VOLUME, &[], &mut []) {
            Ok(_) => return Ok(()),
            Err(e) if attempt >= ATTEMPTS => return Err(e),
            Err(_) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}

/// Locks (`FSCTL_LOCK_VOLUME`) and dismounts (`FSCTL_DISMOUNT_VOLUME`) every volume on
/// disk `number`. Fails if any volume cannot be inspected or locked (e.g. files are open);
/// requires elevation ([`crate::ErrorKind::ElevationRequired`] otherwise). Keep the result
/// alive for the whole write.
pub fn lock_and_dismount(number: u32) -> Result<VolumeLocks> {
    let mut locked = Vec::new();
    for (path, kind) in volume_names()? {
        // Optical and remote volumes do not correspond to a physical disk target.
        if kind == DRIVE_REMOTE || kind == DRIVE_CDROM {
            continue;
        }
        let device = path.trim_end_matches('\\');
        let file = ioctl::open_device(device, Access::Query)
            .map_err(|e| Error::io(format!("Cannot inspect volume {path}"), e))?;
        let on_target = disk_extents(&file)
            .map_err(|e| Error::new(e.kind, format!("Cannot inspect volume {path}: {e}")))?
            .contains(&number);
        drop(file);
        if !on_target {
            continue;
        }
        let file = ioctl::open_device(device, Access::ReadWrite)
            .map_err(|e| Error::io_elevation(format!("Cannot open target volume {path}"), e))?;
        if !disk_extents(&file)?.contains(&number) {
            return Err(Error::new(
                crate::ErrorKind::IdentityChanged,
                "Target volume changed during locking",
            ));
        }
        lock_volume(&file).map_err(|e| {
            Error::io(format!("Cannot lock target volume {path}"), e)
                .with_hint("Close programs and Explorer windows using the disk")
        })?;
        // Dismounting a locked volume invalidates its cached file-system state so nothing
        // is flushed over the new image; the volume stays unavailable while we hold the lock.
        ioctl::ioctl(&file, ioctl::FSCTL_DISMOUNT_VOLUME, &[], &mut [])
            .map_err(|e| Error::io(format!("Cannot dismount target volume {path}"), e))?;
        locked.push(file);
    }
    Ok(VolumeLocks { handles: locked })
}
