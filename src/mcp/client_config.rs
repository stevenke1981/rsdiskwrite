//! Configuration snippets for MCP clients (`rsdiskwrite mcp config`). Only prints; never
//! edits any file.

use crate::error::{Error, Result};
use serde::Serialize;
use std::path::Path;

/// Supported MCP clients (`--client` values are kebab-case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Client {
    /// Claude Desktop (`claude_desktop_config.json`, `mcpServers`).
    ClaudeDesktop,
    /// Claude Code (`claude mcp add` command).
    ClaudeCode,
    /// OpenAI Codex CLI (`[mcp_servers.rsdiskwrite]` in `~/.codex/config.toml`).
    Codex,
    /// OpenAI Agents SDK (`MCPServerStdio(params=...)` Python).
    OpenaiAgents,
}

/// Snippet language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnippetFormat {
    Json,
    Toml,
    Shell,
    Python,
}

/// A configuration snippet (the `mcp config` command's JSON `data`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snippet {
    pub client: Client,
    pub format: SnippetFormat,
    /// Where to paste it (e.g. `%APPDATA%\\Claude\\claude_desktop_config.json`), if a file.
    pub location: Option<String>,
    /// The snippet itself.
    pub text: String,
    /// Extra advice (e.g. "restart the client", "--allow-write enables a destructive tool").
    pub notes: Vec<String>,
}

/// Builds the snippet that runs `exe mcp serve` (`--allow-write` appended when
/// `allow_write`). `exe` should be an absolute path.
pub fn snippet(client: Client, exe: &Path, allow_write: bool) -> Result<Snippet> {
    // TODO(phase2-D): generate snippets for every client.
    let _ = (client, exe, allow_write);
    Err(Error::unsupported("MCP client configuration snippets"))
}
