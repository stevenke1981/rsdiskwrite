use flate2::bufread::GzDecoder;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::path::Path;
use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_READ, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDriveTypeW,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

const LENGTH: u32 = 0x0007_405c;
const QUERY: u32 = 0x002d_1400;
const EXTENTS: u32 = 0x0056_0000;
const GEOMETRY: u32 = 0x0007_0000;
const LOCK_VOLUME: u32 = 0x0009_0018;
const LIMIT: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Disk {
    pub number: u32,
    pub size: u64,
    pub model: String,
}

fn ioctl(file: &File, code: u32, input: &[u8], output: &mut [u8]) -> io::Result<usize> {
    let mut returned = 0u32;
    // Handle and buffers remain valid for this synchronous call.
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
        Ok(returned as usize)
    }
}

fn open_disk(number: u32, write: bool) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(write)
        .share_mode(3)
        .open(format!(r"\\.\PhysicalDrive{number}"))
}
fn describe(number: u32, file: &File) -> io::Result<Disk> {
    let mut length = [0u8; 8];
    if ioctl(file, LENGTH, &[], &mut length)? < 8 {
        return Err(io::Error::other("short disk length response"));
    }
    let size = u64::from_le_bytes(length);
    let mut descriptor = [0u8; 1024];
    let model = match ioctl(file, QUERY, &[0u8; 12], &mut descriptor) {
        Ok(n) if n >= 28 => {
            let offset = u32::from_le_bytes(descriptor[16..20].try_into().unwrap()) as usize;
            if offset >= n || offset == 0 {
                String::from("Unknown model")
            } else {
                let bytes = &descriptor[offset..n];
                let mut model = String::from_utf8_lossy(
                    &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())],
                )
                .trim()
                .to_string();
                let serial = u32::from_le_bytes(descriptor[24..28].try_into().unwrap()) as usize;
                if serial > 0 && serial < n {
                    let bytes = &descriptor[serial..n];
                    let value = String::from_utf8_lossy(
                        &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())],
                    );
                    if !value.trim().is_empty() {
                        model.push_str(&format!(" / serial {}", value.trim()));
                    }
                }
                model
            }
        }
        _ => String::from("Unknown model"),
    };
    Ok(Disk {
        number,
        size,
        model,
    })
}
pub fn list_disks() -> Result<Vec<Disk>, String> {
    Ok((0..32)
        .filter_map(|n| {
            open_disk(n, false)
                .ok()
                .and_then(|file| describe(n, &file).ok())
        })
        .collect())
}

fn volume_disks(path: &Path) -> Result<Vec<u32>, String> {
    let full = std::fs::canonicalize(path)
        .map_err(|e| format!("Cannot resolve {}: {e}", path.display()))?;
    let wide: Vec<u16> = full.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = [0u16; 1024];
    // Windows fills these buffers synchronously and reports errors via the return value.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(format!(
            "Cannot determine volume for {}: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }
    let mut volume = [0u16; 1024];
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
    } == 0
    {
        return Err(format!(
            "Cannot identify volume for {}: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }
    let end = volume
        .iter()
        .position(|c| *c == 0)
        .ok_or("invalid volume name")?;
    let name = String::from_utf16_lossy(&volume[..end]);
    let handle = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(name.trim_end_matches('\\'))
        .map_err(|e| format!("Cannot inspect volume for {}: {e}", path.display()))?;
    disk_extents(&handle)
}
fn disk_extents(handle: &File) -> Result<Vec<u32>, String> {
    let mut extents = [0u8; 4096];
    let n = ioctl(handle, EXTENTS, &[], &mut extents)
        .map_err(|e| format!("Cannot identify volume disks: {e}"))?;
    if n < 8 {
        return Err("Invalid volume extents response".into());
    }
    let count = u32::from_le_bytes(extents[..4].try_into().unwrap()) as usize;
    if count == 0 || 8 + count * 24 > n {
        return Err("Invalid or incomplete volume extents response".into());
    }
    Ok((0..count)
        .map(|i| u32::from_le_bytes(extents[8 + i * 24..12 + i * 24].try_into().unwrap()))
        .collect())
}
struct VolumeSearch(windows_sys::Win32::Foundation::HANDLE);
impl Drop for VolumeSearch {
    fn drop(&mut self) {
        unsafe {
            FindVolumeClose(self.0);
        }
    }
}
fn lock_target_volumes(number: u32) -> Result<Vec<File>, String> {
    let mut name = [0u16; 1024];
    // FindFirstVolumeW supplies a search handle closed by VolumeSearch.
    let find = VolumeSearch(unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) });
    if find.0 == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Cannot enumerate volumes: {}",
            io::Error::last_os_error()
        ));
    }
    let mut locked = Vec::new();
    loop {
        let end = name
            .iter()
            .position(|c| *c == 0)
            .ok_or("invalid volume name")?;
        let path = String::from_utf16_lossy(&name[..end]);
        // Optical and remote volumes do not correspond to a physical disk target.
        let kind = unsafe { GetDriveTypeW(name.as_ptr()) };
        if kind != 4 && kind != 5 {
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(path.trim_end_matches('\\'))
                .map_err(|e| format!("Cannot inspect volume {path}: {e}"))?;
            if disk_extents(&file)
                .map_err(|e| format!("Cannot inspect volume {path}: {e}"))?
                .contains(&number)
            {
                drop(file);
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .share_mode(3)
                    .open(path.trim_end_matches('\\'))
                    .map_err(|e| format!("Cannot open target volume {path}: {e}"))?;
                if !disk_extents(&file)?.contains(&number) {
                    return Err("Target volume changed during locking".into());
                }
                ioctl(&file, LOCK_VOLUME, &[], &mut []).map_err(|e| {
                    format!("Cannot lock target volume {path}; close programs using it: {e}")
                })?;
                locked.push(file);
            }
        }
        name.fill(0);
        if unsafe { FindNextVolumeW(find.0, name.as_mut_ptr(), name.len() as u32) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
                return Err(format!("Volume enumeration failed: {error}"));
            }
            break;
        }
    }
    Ok(locked)
}
fn prepare_image(
    path: &Path,
    capacity: u64,
    create_temp: impl FnOnce() -> Result<File, String>,
) -> Result<(File, u64), String> {
    // Keep the source stable while reading it. Sharing only reads also blocks replacement.
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|e| {
            format!("Cannot open image for reading (close any program modifying it): {e}")
        })?;
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 2];
    let read = reader.read(&mut magic).map_err(|e| e.to_string())?;
    reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut prepared = if read == 2 && magic == [0x1f, 0x8b] {
        let mut temp = create_temp()?;
        let mut remaining = capacity;
        loop {
            // Allow one extra byte to detect oversized images while still checking the gzip
            // trailer when the decompressed size is exactly the remaining capacity.
            let mut member = GzDecoder::new(&mut reader).take(remaining.saturating_add(1));
            let decoded = io::copy(&mut member, &mut temp)
                .map_err(|e| format!("Cannot decompress image: {e}"))?;
            remaining = remaining.checked_sub(decoded).ok_or_else(|| {
                format!("Decompressed image exceeds disk capacity ({capacity} bytes)")
            })?;
            let mut next = [0u8; 2];
            let first = reader.read(&mut next[..1]).map_err(|e| e.to_string())?;
            let second = if first == 1 {
                reader.read(&mut next[1..]).map_err(|e| e.to_string())?
            } else {
                0
            };
            reader
                .seek(SeekFrom::Current(-((first + second) as i64)))
                .map_err(|e| e.to_string())?;
            if first + second != 2 || next != [0x1f, 0x8b] {
                break;
            }
        }
        temp
    } else {
        reader.into_inner()
    };
    let size = prepared.metadata().map_err(|e| e.to_string())?.len();
    if size > capacity {
        return Err(format!(
            "Image ({size} bytes) exceeds disk capacity ({capacity} bytes)"
        ));
    }
    prepared
        .seek(SeekFrom::Start(0))
        .map_err(|e| e.to_string())?;
    if size == 0 {
        return Err("Image is empty".into());
    }
    Ok((prepared, size))
}
fn copy_image(
    mut source: impl Read,
    mut target: impl Write,
    total: u64,
    sector: usize,
    progress: impl Fn(u64),
) -> io::Result<u64> {
    let mut buffer = vec![0u8; (1024 * 1024 / sector) * sector];
    let mut count = 0;
    while count < total {
        let take = (total - count).min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..take])?;
        target.write_all(&buffer[..take])?;
        count += take as u64;
        progress(count);
    }
    target.flush()?;
    Ok(count)
}
pub fn write_image(
    image: &Path,
    disk: &Disk,
    allow_large: bool,
    progress: impl Fn(u64) + Send + 'static,
) -> Result<u64, String> {
    if disk.size > LIMIT && !allow_large {
        return Err("Disk is larger than 2 GiB. Explicitly enable large disks to proceed.".into());
    }
    let windows = std::env::var_os("SystemRoot").ok_or("Cannot determine Windows system volume")?;
    if volume_disks(Path::new(&windows))?.contains(&disk.number) {
        return Err("Refusing to overwrite the Windows system disk".into());
    }
    if volume_disks(image)?.contains(&disk.number) {
        return Err("Refusing to overwrite the disk containing the source image".into());
    }
    let (mut source, size) = prepare_image(image, disk.size, || {
        let temp_dir = std::env::temp_dir();
        if volume_disks(&temp_dir)?.contains(&disk.number) {
            return Err(
                "Refusing to overwrite the disk containing the temporary image directory".into(),
            );
        }
        tempfile::tempfile_in(temp_dir).map_err(|e| format!("Cannot create temporary image: {e}"))
    })?;
    let _locked_volumes = lock_target_volumes(disk.number)?;
    let mut target = open_disk(disk.number, true).map_err(|e| format!("Cannot open disk for writing; run as Administrator and close applications using it: {e}"))?;
    let current =
        describe(disk.number, &target).map_err(|e| format!("Cannot recheck target: {e}"))?;
    if current.size != disk.size || current.model != disk.model {
        return Err("Disk identity changed; refresh and select it again".into());
    }
    let mut geometry = [0u8; 24];
    if ioctl(&target, GEOMETRY, &[], &mut geometry)
        .map_err(|e| format!("Cannot determine sector size: {e}"))?
        < 24
    {
        return Err("Invalid disk geometry".into());
    }
    let sector = u32::from_le_bytes(geometry[20..24].try_into().unwrap());
    if sector == 0 || sector > 1024 * 1024 || size % u64::from(sector) != 0 {
        return Err(format!(
            "Image size is not a multiple of the disk's {sector}-byte logical sector size"
        ));
    }
    copy_image(&mut source, &mut target, size, sector as usize, progress)
        .map_err(|e| format!("Write failed; the disk may be partially overwritten: {e}"))?;
    target
        .sync_all()
        .map_err(|e| format!("Disk flush failed; the disk may be partially overwritten: {e}"))?;
    Ok(size)
}
#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    fn temp_file() -> Result<File, String> {
        tempfile::tempfile().map_err(|e| e.to_string())
    }

    #[test]
    fn gzip_magic_and_trailing_signature_are_handled() {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&vec![42u8; 1024]).unwrap();
        temp.write_all(&gzip.finish().unwrap()).unwrap();
        temp.write_all(b"signature").unwrap();
        let temp = temp.into_temp_path();
        let (mut image, size) = prepare_image(&temp, 1024, temp_file).unwrap();
        let mut bytes = Vec::new();
        image.read_to_end(&mut bytes).unwrap();
        assert_eq!(size, 1024);
        assert_eq!(bytes, vec![42u8; 1024]);
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
        let (mut image, size) = prepare_image(&file, u64::MAX, temp_file).unwrap();
        let mut bytes = Vec::new();
        image.read_to_end(&mut bytes).unwrap();
        assert_eq!(size, 11);
        assert_eq!(bytes, b"firstsecond");
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
        let (mut image, size) = prepare_image(&file, u64::MAX, temp_file).unwrap();
        let mut bytes = Vec::new();
        image.read_to_end(&mut bytes).unwrap();
        assert_eq!(size, 8172);
        assert_eq!(&bytes[8168..], b"next");
    }
    #[test]
    fn raw_image_excludes_other_writers_while_prepared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.img");
        std::fs::write(&path, [42u8; 512]).unwrap();
        let (source, _) =
            prepare_image(&path, 512, || panic!("raw image used a temp file")).unwrap();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(source);
        assert!(OpenOptions::new().write(true).open(&path).is_ok());
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
        let error = prepare_image(&path, 1536, || {
            File::create(&output).map_err(|e| e.to_string())
        })
        .unwrap_err();
        assert!(error.contains("exceeds disk capacity"), "{error}");
        assert_eq!(std::fs::metadata(output).unwrap().len(), 1537);
        assert_eq!(prepare_image(&path, 2048, temp_file).unwrap().1, 2048);

        let mut corrupt = std::fs::read(&path).unwrap();
        let crc = corrupt.len() - 8;
        corrupt[crc] ^= 1;
        std::fs::write(&path, corrupt).unwrap();
        assert!(
            prepare_image(&path, 2048, temp_file)
                .unwrap_err()
                .contains("Cannot decompress")
        );
    }

    #[test]
    fn copy_detects_short_input() {
        let mut dest = Vec::new();
        assert_eq!(copy_image(&b"abc"[..], &mut dest, 3, 1, |_| {}).unwrap(), 3);
        assert_eq!(dest, b"abc");
        assert_eq!(
            copy_image(&b"ab"[..], Vec::new(), 3, 1, |_| {})
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
