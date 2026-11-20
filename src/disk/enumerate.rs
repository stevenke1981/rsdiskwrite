//! Physical disk enumeration.

use crate::disk::ioctl::{self, Access};
use crate::disk::model::{Disk, Enumeration, EnumerationMethod};
use crate::disk::volume;
use crate::error::{Error, ErrorKind, Result};
use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Foundation::ERROR_NO_MORE_ITEMS;
use windows_sys::Win32::System::Ioctl::GUID_DEVINTERFACE_DISK;

/// Highest `PhysicalDriveN` probed by the fallback enumeration (inclusive).
pub const MAX_PROBED_DISK: u32 = 127;

/// Reads number/size/sector/model/serial/bus/removable from an open disk handle.
/// `is_system` is left `false`; callers fill it in.
pub(crate) fn describe(number: u32, file: &File) -> io::Result<Disk> {
    let geometry = ioctl::geometry(file)?;
    let descriptor = ioctl::device_descriptor(file).unwrap_or(ioctl::DeviceDescriptor {
        model: "Unknown model".into(),
        serial: None,
        bus: crate::disk::BusType::Unknown,
        removable: false,
    });
    Ok(Disk {
        number,
        size: geometry.size,
        model: descriptor.model,
        serial: descriptor.serial,
        bus: descriptor.bus,
        removable: descriptor.removable,
        sector: geometry.sector,
        is_system: false,
    })
}

/// Opens disk `number` with query-only access and describes it (works unelevated).
pub(crate) fn query(number: u32) -> io::Result<Disk> {
    let file = ioctl::open_disk(number, Access::Query)?;
    describe(number, &file)
}

/// Enumerates physical disks without requiring elevation.
///
/// Uses SetupAPI (`GUID_DEVINTERFACE_DISK`) when available and falls back to probing
/// `PhysicalDrive0..=`[`MAX_PROBED_DISK`]. Disks without media (empty card readers)
/// are omitted. `is_system` is filled in from `%SystemRoot%`'s volume.
pub fn enumerate() -> Result<Enumeration> {
    let system = volume::system_disks().unwrap_or_default();
    let (found, method) = match setupapi_disks() {
        Ok(found) if !found.is_empty() => (found, EnumerationMethod::SetupApi),
        _ => (probe_disks(), EnumerationMethod::Probe),
    };
    let disks = found
        .into_values()
        .map(|mut disk| {
            disk.is_system = system.contains(&disk.number);
            disk
        })
        .collect();
    Ok(Enumeration { disks, method })
}

/// Probes `PhysicalDrive0..=MAX_PROBED_DISK`.
fn probe_disks() -> BTreeMap<u32, Disk> {
    (0..=MAX_PROBED_DISK)
        .filter_map(|n| query(n).ok().map(|disk| (n, disk)))
        .collect()
}

/// Owns a SetupAPI device information set.
struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: the set came from a successful SetupDiGetClassDevsW.
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// Present disk interfaces via SetupAPI; each interface path is opened with query access
/// and mapped to its `PhysicalDriveN` number with `IOCTL_STORAGE_GET_DEVICE_NUMBER`.
/// Interfaces that cannot be opened or described (e.g. no media) are skipped.
fn setupapi_disks() -> io::Result<BTreeMap<u32, Disk>> {
    let mut disks = BTreeMap::new();
    for path in disk_interface_paths()? {
        let Ok(file) = ioctl::open_device(&path, Access::Query) else {
            continue;
        };
        let Ok(number) = ioctl::device_number(&file) else {
            continue;
        };
        if let Ok(disk) = describe(number, &file) {
            disks.insert(number, disk);
        }
    }
    Ok(disks)
}

/// Device interface paths of all present `GUID_DEVINTERFACE_DISK` devices.
fn disk_interface_paths() -> io::Result<Vec<String>> {
    // SAFETY: valid GUID pointer, no enumerator, no parent window.
    let set = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_DISK,
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == -1 {
        return Err(io::Error::last_os_error());
    }
    let set = DeviceInfoSet(set);
    let mut paths = Vec::new();
    for index in 0u32.. {
        // SAFETY: SP_DEVICE_INTERFACE_DATA is plain data; all-zero is a valid value.
        let mut data: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        // SAFETY: valid set, GUID and output structure with cbSize set.
        let ok = unsafe {
            SetupDiEnumDeviceInterfaces(
                set.0,
                std::ptr::null(),
                &GUID_DEVINTERFACE_DISK,
                index,
                &mut data,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_ITEMS as i32) {
                break;
            }
            return Err(error);
        }
        if let Some(path) = interface_path(&set, &data) {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// `SetupDiGetDeviceInterfaceDetailW` for one interface; `None` on failure.
fn interface_path(set: &DeviceInfoSet, data: &SP_DEVICE_INTERFACE_DATA) -> Option<String> {
    let mut required = 0u32;
    // SAFETY: size query with no output buffer; expected to fail with
    // ERROR_INSUFFICIENT_BUFFER and report the required size.
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set.0,
            data,
            std::ptr::null_mut(),
            0,
            &mut required,
            std::ptr::null_mut(),
        );
    }
    let header = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    let required = required as usize;
    if required <= 4 || required > 64 * 1024 {
        return None;
    }
    // u32 storage keeps the structure 4-byte aligned.
    let mut buffer = vec![0u32; required.max(header).div_ceil(4)];
    let bytes = buffer.len() * 4;
    let detail = buffer
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    // SAFETY: `buffer` is at least `header` bytes and suitably aligned; cbSize must be the
    // size of the fixed structure (not of the buffer).
    unsafe {
        (*detail).cbSize = header as u32;
    }
    // SAFETY: `detail` points to `bytes` writable bytes.
    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set.0,
            data,
            detail,
            bytes as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    // DevicePath (UTF-16, NUL-terminated) starts right after the u32 cbSize.
    let wide: Vec<u16> = buffer
        .iter()
        .flat_map(|word| {
            let b = word.to_le_bytes();
            [
                u16::from_le_bytes([b[0], b[1]]),
                u16::from_le_bytes([b[2], b[3]]),
            ]
        })
        .collect();
    parse_wide_path(wide.get(2..)?)
}

/// UTF-16 up to the first NUL; `None` when empty.
fn parse_wide_path(wide: &[u16]) -> Option<String> {
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    let path = String::from_utf16_lossy(wide.get(..end)?);
    (!path.is_empty()).then_some(path)
}

/// [`enumerate`] without the method.
pub fn list() -> Result<Vec<Disk>> {
    enumerate().map(|e| e.disks)
}

/// Describes one disk freshly. Missing disks are [`ErrorKind::DiskNotFound`].
pub fn get(number: u32) -> Result<Disk> {
    let mut disk = query(number).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            Error::new(
                ErrorKind::DiskNotFound,
                format!("PhysicalDrive{number} does not exist"),
            )
            .with_hint("Run 'rsdiskwrite list' to see available disks")
        } else {
            Error::io(format!("Cannot query PhysicalDrive{number}"), e)
        }
    })?;
    disk.is_system = volume::system_disks()?.contains(&number);
    Ok(disk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_paths_stop_at_nul() {
        let wide: Vec<u16> = "\\\\?\\usbstor#disk\0junk".encode_utf16().collect();
        assert_eq!(parse_wide_path(&wide).as_deref(), Some(r"\\?\usbstor#disk"));
        assert_eq!(parse_wide_path(&[0, 65]), None);
        assert_eq!(parse_wide_path(&[]), None);
    }

    #[test]
    fn enumeration_reports_a_method_and_sorted_disks() {
        // Read-only, unelevated query; never opens a disk for writing.
        let enumeration = enumerate().unwrap();
        let numbers: Vec<u32> = enumeration.disks.iter().map(|d| d.number).collect();
        let mut sorted = numbers.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(numbers, sorted);
    }
}
