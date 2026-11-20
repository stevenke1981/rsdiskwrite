//! Physical disk enumeration.

use crate::disk::ioctl::{self, Access};
use crate::disk::model::{Disk, Enumeration, EnumerationMethod};
use crate::disk::volume;
use crate::error::{Error, ErrorKind, Result};
use std::fs::File;
use std::io;

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
    // TODO(phase2-A): SetupAPI enumeration (GUID_DEVINTERFACE_DISK +
    // IOCTL_STORAGE_GET_DEVICE_NUMBER), falling back to probing.
    let system = volume::system_disks().unwrap_or_default();
    let disks = (0..=MAX_PROBED_DISK)
        .filter_map(|n| query(n).ok())
        .map(|mut disk| {
            disk.is_system = system.contains(&disk.number);
            disk
        })
        .collect();
    Ok(Enumeration {
        disks,
        method: EnumerationMethod::Probe,
    })
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
