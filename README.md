# rsdiskwrite

A Windows physical-disk image writer in Rust, inspired by [physdiskwrite](https://m0n0.ch/wall/physdiskwrite.php), with an [AcmeUI Native](https://github.com/stevenke1981/AcmeUI-Native) desktop interface. **Experimental: physical-disk writes have not yet been validated on a disposable device.** Use only after independently verifying the target and keeping backups.

## Usage

Run `rsdiskwrite.exe` as Administrator to open the GUI. Choose an image, select a physical disk, inspect its number, model, serial (when reported), and capacity, then confirm the irreversible overwrite. The 2 GiB disk-size limit is enabled by default and can be explicitly disabled in the GUI.

CLI usage:

```text
rsdiskwrite [-u] [-d DRIVE_NUMBER] IMAGE
```

`-u` permits disks larger than 2 GiB. `-d` selects a disk; unlike original physdiskwrite, an explicit `ERASE <number>` confirmation is always required. With no `-d`, the CLI prompts for a disk number. `--help` prints usage.

Raw images and gzip images are detected by content. Concatenated gzip members are decoded and non-gzip trailing data is ignored, accommodating the documented m0n0wall signed-image layout; **signatures are not authenticated**. Compressed images are decompressed into the system temporary directory before writing. Decompression stops as soon as the image exceeds the selected disk capacity. Close any programs modifying the source image before starting. The image must fit the target and be a multiple of its logical sector size.

The program refuses the Windows-volume disk, the source-image disk, and, for gzip images, the temporary-directory disk. It checks device identity again before writing, and attempts to lock all target volumes before writing. Device identifiers can be missing or duplicated; do not disconnect, reconnect, or swap drives during the operation. If a volume cannot be inspected or locked, the operation fails before writing. Some card readers, encrypted or virtual volumes may therefore prevent writing to an otherwise usable device. An I/O or power failure during the write can leave the target partially overwritten. The application does not automatically delete partitions or bypass Windows access controls.

## Build and checks

Windows x86-64 and Rust 1.88 or later are required. AcmeUI Native is pinned to a Git commit in `Cargo.toml`; dependencies are locked in `Cargo.lock`.

```powershell
cargo test --locked
cargo build --release --locked
```

On GNU Windows toolchains, a non-ASCII workspace path may break `dlltool`; set `CARGO_TARGET_DIR` to an ASCII-only path before building. Actual device writes require elevation; the unit tests use temporary files and do not write physical disks.

## Scope and limitations

This is an independent implementation, not the original physdiskwrite. It probes physical drives 0–31. Digitally signed m0n0wall sample images, administrator-mode enumeration, volume locking, real-disk writing and hardware-specific behavior have not yet been validated on a disposable target. Review source and test with a disposable virtual or physical disk before relying on it.

Licensed under the MIT license. AcmeUI Native and other dependencies retain their respective licenses.
