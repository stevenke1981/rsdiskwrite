//! SHA-256 helpers used for image/disk integrity checks (`hash`, `--expect-sha256`,
//! MCP plan binding).

use crate::error::{Error, ErrorKind, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::Digest as _;
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

/// A SHA-256 digest. Displays, parses and (de)serializes as 64 lowercase hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    /// Lowercase hex representation (64 characters).
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for Digest {
    type Err = Error;
    /// Parses 64 hex digits (either case, surrounding whitespace ignored).
    /// Fails with [`ErrorKind::Usage`] otherwise.
    fn from_str(s: &str) -> Result<Self> {
        parse_hex(s)
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_hex(&text).map_err(serde::de::Error::custom)
    }
}

/// Incremental SHA-256 state (thin wrapper so callers need not depend on `sha2`).
#[derive(Clone, Default)]
pub struct Hasher(sha2::Sha256);

impl Hasher {
    /// A fresh hasher.
    pub fn new() -> Self {
        Self::default()
    }
    /// Feeds `data` into the digest.
    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }
    /// Consumes the hasher and returns the digest.
    pub fn finish(self) -> Digest {
        Digest(self.0.finalize().into())
    }
}

/// Lowercase hex encoding of arbitrary bytes.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 15)] as char);
    }
    out
}

/// Parses a 64-digit hex SHA-256 string (case-insensitive, trimmed).
/// Returns [`ErrorKind::Usage`] for any other input.
pub fn parse_hex(text: &str) -> Result<Digest> {
    let text = text.trim();
    let invalid = || {
        Error::new(
            ErrorKind::Usage,
            format!("invalid SHA-256 digest '{text}': expected 64 hex digits"),
        )
    };
    if text.len() != 64 || !text.is_ascii() {
        return Err(invalid());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let pair = text.get(2 * i..2 * i + 2).ok_or_else(invalid)?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| invalid())?;
    }
    Ok(Digest(out))
}

/// Hashes `reader`. With `len = Some(n)` exactly `n` bytes are read (a short read is an
/// [`ErrorKind::Io`] error); with `None` the reader is consumed to EOF. `progress` is
/// called with the cumulative number of bytes hashed after each chunk.
pub fn sha256_reader(
    mut reader: impl Read,
    len: Option<u64>,
    progress: &dyn Fn(u64),
) -> Result<Digest> {
    let mut hasher = Hasher::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut done = 0u64;
    loop {
        let want = match len {
            Some(total) if done >= total => break,
            Some(total) => (total - done).min(buffer.len() as u64) as usize,
            None => buffer.len(),
        };
        let read = match reader.read(&mut buffer[..want]) {
            Ok(0) if len.is_none() => break,
            Ok(0) => {
                return Err(Error::new(
                    ErrorKind::Io,
                    format!(
                        "unexpected end of data after {done} of {} bytes",
                        len.unwrap_or(0)
                    ),
                ));
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::io("read failed while hashing", e)),
        };
        hasher.update(&buffer[..read]);
        done += read as u64;
        progress(done);
    }
    Ok(hasher.finish())
}

/// Hashes a whole file as stored on disk (no gzip decoding).
pub fn sha256_file(path: &Path, progress: &dyn Fn(u64)) -> Result<Digest> {
    let file = std::fs::File::open(path)
        .map_err(|e| Error::io(format!("cannot open {}", path.display()), e))?;
    sha256_reader(std::io::BufReader::new(file), None, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn known_vector_and_hex_round_trip() {
        let digest = sha256_reader(&b"abc"[..], None, &|_| {}).unwrap();
        assert_eq!(digest.to_hex(), ABC);
        assert_eq!(ABC.to_uppercase().parse::<Digest>().unwrap(), digest);
        assert_eq!(serde_json::to_value(digest).unwrap(), ABC);
        assert!(parse_hex("abc").is_err());
        assert!(parse_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn length_is_enforced() {
        let digest = sha256_reader(&b"abcdef"[..], Some(3), &|_| {}).unwrap();
        assert_eq!(digest.to_hex(), ABC);
        assert!(sha256_reader(&b"ab"[..], Some(3), &|_| {}).is_err());
    }
}
