//! Model Context Protocol server (newline-delimited JSON-RPC 2.0 over stdio) and client
//! configuration snippets.
//!
//! While serving, nothing but JSON-RPC messages may be written to stdout; logs go to stderr.

pub mod client_config;
pub mod gate;
pub mod protocol;
#[cfg(windows)]
pub mod server;
#[cfg(windows)]
pub mod tools;

#[cfg(windows)]
pub use server::{ServerConfig, serve};

/// `serverInfo.name` reported by `initialize`.
pub const SERVER_NAME: &str = "rsdiskwrite";

/// Protocol revisions accepted in `initialize`, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Revision answered when the client requests one we do not support.
pub const LATEST_PROTOCOL_VERSION: &str = SUPPORTED_PROTOCOL_VERSIONS[0];

/// The protocol version to answer for a client's requested version: the same string when
/// supported, otherwise [`LATEST_PROTOCOL_VERSION`].
pub fn negotiate_protocol_version(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|r| SUPPORTED_PROTOCOL_VERSIONS.into_iter().find(|v| *v == r))
        .unwrap_or(LATEST_PROTOCOL_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_negotiation() {
        assert_eq!(negotiate_protocol_version(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate_protocol_version(Some("1999-01-01")), "2025-06-18");
        assert_eq!(negotiate_protocol_version(None), "2025-06-18");
    }
}
