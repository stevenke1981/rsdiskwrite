//! Thin, crate-internal wrappers around `DeviceIoControl` and raw device handles.
//!
//! Buffer parsing is done with bounds-checked helpers; never index ioctl output without
//! checking the returned length first.

use crate::disk::model::BusType;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use windows_sys::Win32::System::IO::DeviceIoControl;

pub(crate) const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x0007_00a0;
pub(crate) const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002d_1400;
pub(crate) const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
pub(crate) const FSCTL_LOCK_VOLUME: u32 = 0x0009_0018;
#[allow(dead_code)] // TODO(phase2-A): used by dismount.
pub(crate) const FSCTL_DISMOUNT_VOLUME: u32 = 0x0009_0020;
#[allow(dead_code)] // TODO(phase2-A): issued after a successful write.
pub(crate) const IOCTL_DISK_UPDATE_PROPERTIES: u32 = 0x0007_0140;
#[allow(dead_code)] // TODO(phase2-A): SetupAPI enumeration.
pub(crate) const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002d_1080;

/// `FILE_SHARE_READ | FILE_SHARE_WRITE`.
const SHARE_READ_WRITE: u32 = 3;

/// Requested access for a device handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Desired access 0: enough for `FILE_ANY_ACCESS` ioctls; works unelevated.
    Query,
    /// `GENERIC_READ` (needs Administrator for physical disks).
    #[allow(dead_code)] // TODO(phase2-A): used by verify and disk hashing.
    Read,
    /// `GENERIC_READ | GENERIC_WRITE` (needs Administrator).
    ReadWrite,
}

/// Issues a synchronous `DeviceIoControl`, returning the number of output bytes.
pub(crate) fn ioctl(file: &File, code: u32, input: &[u8], output: &mut [u8]) -> io::Result<usize> {
    let mut returned = 0u32;
    // SAFETY: the handle and both buffers remain valid for this synchronous call and the
    // lengths passed match the buffers.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            code,
            input.as_ptr() as _,
            input.len() as u32,
            output.as_mut_ptr() as _,
            output.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((returned as usize).min(output.len()))
    }
}

/// Opens `path` (a device or volume path) with `access`, sharing reads and writes.
pub(crate) fn open_device(path: &str, access: Access) -> io::Result<File> {
    let mut options = OpenOptions::new();
    match access {
        Access::Query => options.access_mode(0),
        Access::Read => options.read(true),
        Access::ReadWrite => options.read(true).write(true),
    };
    options.share_mode(SHARE_READ_WRITE).open(path)
}

/// Opens `\\.\PhysicalDrive{number}`.
pub(crate) fn open_disk(number: u32, access: Access) -> io::Result<File> {
    open_device(&format!(r"\\.\PhysicalDrive{number}"), access)
}

pub(crate) fn read_u32(buf: &[u8], offset: usize) -> Option<u32> {
    buf.get(offset..offset.checked_add(4)?)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn read_u64(buf: &[u8], offset: usize) -> Option<u64> {
    let b = buf.get(offset..offset.checked_add(8)?)?;
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(b);
    Some(u64::from_le_bytes(bytes))
}

/// Size and logical sector size of a disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Geometry {
    pub size: u64,
    pub sector: u32,
}

/// `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX` (`FILE_ANY_ACCESS`, so a [`Access::Query`]
/// handle suffices).
pub(crate) fn geometry(file: &File) -> io::Result<Geometry> {
    let mut buffer = [0u8; 256];
    let n = ioctl(file, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut buffer)?;
    parse_geometry(&buffer[..n]).ok_or_else(|| io::Error::other("short disk geometry response"))
}

/// Parses `DISK_GEOMETRY_EX`: `BytesPerSector` at offset 20, `DiskSize` at 24.
pub(crate) fn parse_geometry(buf: &[u8]) -> Option<Geometry> {
    Some(Geometry {
        sector: read_u32(buf, 20)?,
        size: read_u64(buf, 24)?,
    })
}

/// Fields of `STORAGE_DEVICE_DESCRIPTOR` that rsdiskwrite uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceDescriptor {
    pub model: String,
    pub serial: Option<String>,
    pub bus: BusType,
    pub removable: bool,
}

/// `IOCTL_STORAGE_QUERY_PROPERTY` / `StorageDeviceProperty` (`FILE_ANY_ACCESS`).
pub(crate) fn device_descriptor(file: &File) -> io::Result<DeviceDescriptor> {
    let mut buffer = [0u8; 1024];
    // STORAGE_PROPERTY_QUERY { PropertyId = StorageDeviceProperty (0), QueryType = 0 }.
    let n = ioctl(file, IOCTL_STORAGE_QUERY_PROPERTY, &[0u8; 12], &mut buffer)?;
    Ok(parse_device_descriptor(&buffer[..n]))
}

fn descriptor_string(buf: &[u8], offset_field: usize) -> Option<String> {
    let offset = read_u32(buf, offset_field)? as usize;
    if offset == 0 || offset >= buf.len() {
        return None;
    }
    let bytes = &buf[offset..];
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let value = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// Parses a `STORAGE_DEVICE_DESCRIPTOR`: `RemovableMedia` at 10, vendor/product/serial
/// string offsets at 12/16/24, `BusType` at 28. Missing fields get defaults.
pub(crate) fn parse_device_descriptor(buf: &[u8]) -> DeviceDescriptor {
    let vendor = descriptor_string(buf, 12);
    let product = descriptor_string(buf, 16);
    let model = match (vendor, product) {
        (Some(v), Some(p)) if p.starts_with(&v) => p,
        (Some(v), Some(p)) => format!("{v} {p}"),
        (Some(v), None) => v,
        (None, Some(p)) => p,
        (None, None) => "Unknown model".into(),
    };
    DeviceDescriptor {
        model,
        serial: descriptor_string(buf, 24),
        bus: read_u32(buf, 28).map_or(BusType::Unknown, BusType::from_raw),
        removable: buf.get(10).is_some_and(|b| *b != 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_parsing_is_bounds_checked() {
        let mut buf = vec![0u8; 80];
        buf[10] = 1;
        buf[12..16].copy_from_slice(&40u32.to_le_bytes());
        buf[16..20].copy_from_slice(&48u32.to_le_bytes());
        buf[24..28].copy_from_slice(&64u32.to_le_bytes());
        buf[28..32].copy_from_slice(&7u32.to_le_bytes());
        buf[40..47].copy_from_slice(b"SanDisk");
        buf[48..54].copy_from_slice(b"Cruzer");
        buf[64..68].copy_from_slice(b" 42 ");
        let d = parse_device_descriptor(&buf);
        assert_eq!(d.model, "SanDisk Cruzer");
        assert_eq!(d.serial.as_deref(), Some("42"));
        assert_eq!(d.bus, BusType::Usb);
        assert!(d.removable);

        // Truncated and out-of-range offsets never panic.
        for len in 0..buf.len() {
            parse_device_descriptor(&buf[..len]);
        }
        buf[16..20].copy_from_slice(&5000u32.to_le_bytes());
        assert_eq!(parse_device_descriptor(&buf).model, "SanDisk");
        assert_eq!(parse_device_descriptor(&[]).model, "Unknown model");
    }

    #[test]
    fn geometry_parsing() {
        let mut buf = [0u8; 32];
        buf[20..24].copy_from_slice(&512u32.to_le_bytes());
        buf[24..32].copy_from_slice(&4096u64.to_le_bytes());
        assert_eq!(
            parse_geometry(&buf),
            Some(Geometry {
                size: 4096,
                sector: 512
            })
        );
        assert_eq!(parse_geometry(&buf[..31]), None);
    }
}
