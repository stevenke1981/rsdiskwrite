//! The typed confirmation phrase for destructive writes (`ERASE <n>`).

use crate::disk::Disk;
use crate::output::format_size;
use std::path::Path;

/// The exact phrase the user must type (or pass via `--confirm`) to write `disk`:
/// `"ERASE <number>"`.
pub fn phrase(disk: &Disk) -> String {
    format!("ERASE {}", disk.number)
}

/// True when `input`, after trimming surrounding whitespace (including the newline from
/// `read_line`), is exactly [`phrase`]. Case-sensitive; no other normalisation.
pub fn matches(input: &str, disk: &Disk) -> bool {
    input.trim() == phrase(disk)
}

/// Multi-line warning shown before asking for the phrase (no trailing newline), e.g.
/// `ALL DATA on PhysicalDrive1 (SanDisk, 7.45 GiB, 8004304896 bytes) will be destroyed.`
pub fn warning(disk: &Disk, image: &Path) -> String {
    let serial = disk
        .serial
        .as_deref()
        .map(|s| format!(", serial {s}"))
        .unwrap_or_default();
    format!(
        "ALL DATA on {} ({}{serial}, {}, {} bytes) will be destroyed.\nImage: {}\nType {} to continue:",
        disk.name(),
        disk.model,
        format_size(disk.size),
        disk.size,
        image.display(),
        phrase(disk)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::model::test_disk;

    #[test]
    fn phrase_must_match_exactly() {
        let disk = test_disk(3);
        assert_eq!(phrase(&disk), "ERASE 3");
        assert!(matches("ERASE 3\r\n", &disk));
        assert!(matches("  ERASE 3 ", &disk));
        assert!(!matches("erase 3", &disk));
        assert!(!matches("ERASE 30", &disk));
        assert!(!matches("ERASE  3", &disk));
        assert!(!matches("", &disk));
        assert!(warning(&disk, Path::new("x.img")).contains("Type ERASE 3"));
    }
}
