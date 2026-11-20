//! Physical disks: data model (all platforms) and Windows enumeration, volume mapping
//! and locking.

pub mod model;

#[cfg(windows)]
pub mod enumerate;
#[cfg(windows)]
pub(crate) mod ioctl;
#[cfg(windows)]
pub mod volume;

pub use model::{
    BusType, Disk, DiskIdentity, Enumeration, EnumerationMethod, LARGE_DISK_THRESHOLD,
    same_identity,
};

#[cfg(windows)]
pub use enumerate::{enumerate, get, list};
#[cfg(windows)]
pub use volume::{VolumeInfo, VolumeLocks, lock_and_dismount, system_disks, volume_disks, volumes};
