//! The stdio MCP server loop.

use crate::error::{Error, Result};
use crate::mcp::gate::{Confirmer, DialogConfirmer, PLAN_TTL};
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

/// Server settings.
#[derive(Clone)]
pub struct ServerConfig {
    /// Expose and accept `write_image` (`mcp serve --allow-write`).
    pub allow_write: bool,
    /// Human confirmation for writes. Always [`DialogConfirmer`] in production.
    pub confirmer: Arc<dyn Confirmer>,
    /// `plan_id` lifetime.
    pub plan_ttl: Duration,
}

impl ServerConfig {
    /// Production configuration: dialog confirmer and [`PLAN_TTL`].
    pub fn new(allow_write: bool) -> Self {
        Self {
            allow_write,
            confirmer: Arc::new(DialogConfirmer),
            plan_ttl: PLAN_TTL,
        }
    }
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("allow_write", &self.allow_write)
            .field("plan_ttl", &self.plan_ttl)
            .finish_non_exhaustive()
    }
}

/// Serves MCP over newline-delimited JSON-RPC until `reader` reaches EOF.
///
/// Handles `initialize`, `notifications/initialized`, `ping`, `tools/list` and
/// `tools/call`; unknown methods get `-32601`; notifications are never answered. Every
/// line written to `writer` is one JSON-RPC message (responses and `notifications/progress`);
/// writes run on a worker thread so `ping` stays responsive, which is why `writer` must be
/// `Send + 'static`. Returns `Ok(())` on EOF; I/O errors on `writer` end the loop with
/// [`crate::ErrorKind::Io`].
pub fn serve<R: BufRead, W: Write + Send + 'static>(
    reader: R,
    writer: W,
    config: ServerConfig,
) -> Result<()> {
    // TODO(phase2-B): implement the server loop.
    let _ = (reader, writer, config);
    Err(Error::unsupported("the MCP server"))
}
