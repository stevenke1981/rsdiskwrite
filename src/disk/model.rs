//! Platform-independent disk data types (serializable for `--json` and MCP).

use crate::output::format_size;
use serde::{Deserialize, Serialize};

/// Disks larger than this need `--allow-large` / `allow_large` (2 GiB).
pub const LARGE_DISK_THRESHOLD: u64 = 2 * 1024 * 1024 * 1024;

/// Storage bus reported by `IOCTL_STORAGE_QUERY_PROPERTY` (`STORAGE_BUS_TYPE`).
/// Serialized in snake_case (`"usb"`, `"sd"`, `"nvme"`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusType {
    /// Not reported, or a value this build does not know.
    Unknown,
    Scsi,
    Atapi,
    Ata,
    Ieee1394,
    Ssa,
    Fibre,
    Usb,
    Raid,
    Iscsi,
    Sas,
    Sata,
    Sd,
    Mmc,
    Virtual,
    FileBackedVirtual,
    Spaces,
    Nvme,
    Scm,
    Ufs,
}

impl BusType {
    /// Maps a raw `STORAGE_BUS_TYPE` value; unknown values become [`BusType::Unknown`].
    pub fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::Scsi,
            2 => Self::Atapi,
            3 => Self::Ata,
            4 => Self::Ieee1394,
            5 => Self::Ssa,
            6 => Self::Fibre,
            7 => Self::Usb,
            8 => Self::Raid,
            9 => Self::Iscsi,
            10 => Self::Sas,
            11 => Self::Sata,
            12 => Self::Sd,
            13 => Self::Mmc,
            14 => Self::Virtual,
            15 => Self::FileBackedVirtual,
            16 => Self::Spaces,
            17 => Self::Nvme,
            18 => Self::Scm,
            19 => Self::Ufs,
            _ => Self::Unknown,
        }
    }

    /// True for buses that normally carry removable flash media (USB, SD, MMC).
    pub fn is_removable_class(self) -> bool {
        matches!(self, Self::Usb | Self::Sd | Self::Mmc)
    }

    /// Short human label, e.g. `"USB"`, `"NVMe"`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Scsi => "SCSI",
            Self::Atapi => "ATAPI",
            Self::Ata => "ATA",
            Self::Ieee1394 => "1394",
            Self::Ssa => "SSA",
            Self::Fibre => "Fibre",
            Self::Usb => "USB",
            Self::Raid => "RAID",
            Self::Iscsi => "iSCSI",
            Self::Sas => "SAS",
            Self::Sata => "SATA",
            Self::Sd => "SD",
            Self::Mmc => "MMC",
            Self::Virtual => "virtual",
            Self::FileBackedVirtual => "VHD",
            Self::Spaces => "Spaces",
            Self::Nvme => "NVMe",
            Self::Scm => "SCM",
            Self::Ufs => "UFS",
        }
    }
}

/// A physical disk (`\\.\PhysicalDriveN`) as seen at enumeration time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    /// The N in `\\.\PhysicalDriveN`.
    pub number: u32,
    /// Capacity in bytes.
    pub size: u64,
    /// Vendor and product id, trimmed (`"Unknown model"` when not reported).
    pub model: String,
    /// Device serial number, when reported and non-empty.
    pub serial: Option<String>,
    /// Storage bus.
    pub bus: BusType,
    /// The device reports removable media (`STORAGE_DEVICE_DESCRIPTOR.RemovableMedia`).
    pub removable: bool,
    /// Logical sector size in bytes (0 when unknown).
    pub sector: u32,
    /// The disk holds the running Windows installation (`%SystemRoot%`).
    pub is_system: bool,
}

impl Disk {
    /// Device path, e.g. `\\.\PhysicalDrive1`.
    pub fn path(&self) -> String {
        format!(r"\\.\PhysicalDrive{}", self.number)
    }

    /// Display name, e.g. `PhysicalDrive1`.
    pub fn name(&self) -> String {
        format!("PhysicalDrive{}", self.number)
    }

    /// The fields that must stay identical between planning/confirmation and writing.
    pub fn identity(&self) -> DiskIdentity {
        DiskIdentity {
            number: self.number,
            size: self.size,
            model: self.model.clone(),
            serial: self.serial.clone(),
        }
    }

    /// Larger than [`LARGE_DISK_THRESHOLD`].
    pub fn is_large(&self) -> bool {
        self.size > LARGE_DISK_THRESHOLD
    }

    /// Acceptable as a target without `--allow-fixed`: removable media or a
    /// USB/SD/MMC bus.
    pub fn is_removable_target(&self) -> bool {
        self.removable || self.bus.is_removable_class()
    }

    /// One-line human description, e.g.
    /// `PhysicalDrive1: SanDisk Cruzer (7.45 GiB, USB, removable, serial 1234)`.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "{}: {} ({}, {}",
            self.name(),
            self.model,
            format_size(self.size),
            self.bus.label()
        );
        if self.removable {
            out.push_str(", removable");
        }
        if self.is_system {
            out.push_str(", SYSTEM");
        }
        if let Some(serial) = &self.serial {
            out.push_str(", serial ");
            out.push_str(serial);
        }
        out.push(')');
        out
    }
}

/// Identity of a disk: number + size + model + serial. Used for re-checks after locking
/// and for binding confirmations / MCP plans to one specific device.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiskIdentity {
    pub number: u32,
    pub size: u64,
    pub model: String,
    pub serial: Option<String>,
}

/// True when `a` and `b` have the same number, size, model and serial.
pub fn same_identity(a: &Disk, b: &Disk) -> bool {
    a.identity() == b.identity()
}

/// How the disk list was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnumerationMethod {
    /// SetupAPI `GUID_DEVINTERFACE_DISK` + `IOCTL_STORAGE_GET_DEVICE_NUMBER`.
    SetupApi,
    /// Opening `\\.\PhysicalDrive0..=127` one by one.
    Probe,
}

/// Result of [`crate::disk::enumerate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enumeration {
    /// Disks sorted by number.
    pub disks: Vec<Disk>,
    pub method: EnumerationMethod,
}

#[cfg(test)]
pub(crate) fn test_disk(number: u32) -> Disk {
    Disk {
        number,
        size: 1024 * 1024 * 1024,
        model: "Test disk".into(),
        serial: Some("SN1".into()),
        bus: BusType::Usb,
        removable: true,
        sector: 512,
        is_system: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_includes_serial_and_size() {
        let a = test_disk(1);
        let mut b = a.clone();
        assert!(same_identity(&a, &b));
        b.is_system = true;
        assert!(same_identity(&a, &b), "flags are not identity");
        b.serial = Some("SN2".into());
        assert!(!same_identity(&a, &b));
        let mut c = a.clone();
        c.size += 512;
        assert!(!same_identity(&a, &c));
    }

    #[test]
    fn bus_mapping_and_removable_target() {
        assert_eq!(BusType::from_raw(7), BusType::Usb);
        assert_eq!(BusType::from_raw(17), BusType::Nvme);
        assert_eq!(BusType::from_raw(999), BusType::Unknown);
        let mut disk = test_disk(2);
        disk.removable = false;
        assert!(disk.is_removable_target());
        disk.bus = BusType::Nvme;
        assert!(!disk.is_removable_target());
        assert_eq!(disk.path(), r"\\.\PhysicalDrive2");
        assert_eq!(
            serde_json::to_value(BusType::FileBackedVirtual).unwrap(),
            "file_backed_virtual"
        );
    }
}
