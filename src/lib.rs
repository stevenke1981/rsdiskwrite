//! rsdiskwrite: write disk images to USB sticks and SD cards on Windows.
//!
//! The crate is a library plus a thin binary. Platform-independent modules (data types,
//! image handling, hashing, output, MCP protocol) compile everywhere; modules that talk to
//! Windows devices are `#[cfg(windows)]`.
//!
//! Layering: [`cli`], [`gui`] and [`mcp`] are front ends over [`ops`] (plan / execute /
//! verify), which uses [`disk`], [`image`], [`copy`] and [`hash`]. Every fallible function
//! returns [`Result`] with an [`Error`] whose [`ErrorKind`] defines the JSON error code and
//! the process exit code.

pub mod confirm;
pub mod copy;
pub mod disk;
pub mod doctor;
pub mod error;
pub mod hash;
pub mod image;
pub mod mcp;
pub mod ops;
pub mod output;

#[cfg(windows)]
pub mod cli;
#[cfg(windows)]
pub mod gui;

pub use error::{Error, ErrorKind, Result};

/// The crate version (`CARGO_PKG_VERSION`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
