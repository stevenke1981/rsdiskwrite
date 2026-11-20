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
    let exe = exe.to_string_lossy();
    let mut args = vec!["mcp", "serve"];
    if allow_write {
        args.push("--allow-write");
    }
    let (format, location, text, mut notes) = match client {
        Client::ClaudeDesktop => {
            let value = serde_json::json!({
                "mcpServers": {"rsdiskwrite": {"command": exe, "args": args}}
            });
            let text = serde_json::to_string_pretty(&value)
                .map_err(|e| Error::internal(format!("cannot render JSON: {e}")))?;
            (
                SnippetFormat::Json,
                Some("%APPDATA%\\Claude\\claude_desktop_config.json".to_owned()),
                text,
                vec![
                    "Merge the \"rsdiskwrite\" entry into an existing \"mcpServers\" object if the file has one.".to_owned(),
                    "Restart Claude Desktop after saving.".to_owned(),
                ],
            )
        }
        Client::ClaudeCode => (
            SnippetFormat::Shell,
            None,
            format!(
                "claude mcp add --scope user rsdiskwrite -- \"{exe}\" {}",
                args.join(" ")
            ),
            vec![
                "Run this once in any terminal; it registers the server for your user.".to_owned(),
            ],
        ),
        Client::Codex => {
            let list = args
                .iter()
                .map(|a| toml_string(a))
                .collect::<Vec<_>>()
                .join(", ");
            (
                SnippetFormat::Toml,
                Some("~/.codex/config.toml".to_owned()),
                format!(
                    "[mcp_servers.rsdiskwrite]\ncommand = {}\nargs = [{list}]",
                    toml_string(&exe)
                ),
                vec!["Restart Codex after saving.".to_owned()],
            )
        }
        Client::OpenaiAgents => {
            let list = args
                .iter()
                .map(|a| py_string(a))
                .collect::<Vec<_>>()
                .join(", ");
            (
                SnippetFormat::Python,
                None,
                format!(
                    "from agents.mcp import MCPServerStdio\n\nrsdiskwrite = MCPServerStdio(\n    params={{\"command\": {}, \"args\": [{list}]}},\n)",
                    py_string(&exe)
                ),
                vec![
                    "Use it as an async context manager and pass it in Agent(mcp_servers=[rsdiskwrite]).".to_owned(),
                ],
            )
        }
    };
    if allow_write {
        notes.push(
            "WARNING: --allow-write exposes the destructive write_image tool to the AI client. \
             Every write still needs a plan_id and a human clicking Yes in a dialog on this \
             computer, but only enable it if you accept that risk."
                .to_owned(),
        );
    } else {
        notes.push(
            "Read-only: write_image is not offered (add --allow-write to enable it).".to_owned(),
        );
    }
    Ok(Snippet {
        client,
        format,
        location,
        text,
        notes,
    })
}

/// A TOML basic string (backslashes and quotes escaped).
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Python string literal; the TOML basic-string escapes are valid Python too.
fn py_string(s: &str) -> String {
    toml_string(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn exe() -> PathBuf {
        PathBuf::from(r"C:\Program Files\rsdiskwrite\rsdiskwrite.exe")
    }

    #[test]
    fn claude_desktop_is_valid_json_with_escaped_backslashes() {
        let s = snippet(Client::ClaudeDesktop, &exe(), false).unwrap();
        assert!(
            s.text
                .contains(r"C:\\Program Files\\rsdiskwrite\\rsdiskwrite.exe")
        );
        let v: serde_json::Value = serde_json::from_str(&s.text).unwrap();
        let server = &v["mcpServers"]["rsdiskwrite"];
        assert_eq!(server["command"], exe().to_str().unwrap());
        assert_eq!(server["args"], serde_json::json!(["mcp", "serve"]));
        assert_eq!(
            s.location.as_deref(),
            Some(r"%APPDATA%\Claude\claude_desktop_config.json")
        );
    }

    #[test]
    fn allow_write_adds_flag_and_warning() {
        for client in [
            Client::ClaudeDesktop,
            Client::ClaudeCode,
            Client::Codex,
            Client::OpenaiAgents,
        ] {
            let on = snippet(client, &exe(), true).unwrap();
            let off = snippet(client, &exe(), false).unwrap();
            assert!(on.text.contains("--allow-write"), "{client:?}");
            assert!(!off.text.contains("--allow-write"), "{client:?}");
            assert!(on.notes.iter().any(|n| n.contains("WARNING")));
            assert!(!off.notes.iter().any(|n| n.contains("WARNING")));
        }
    }

    #[test]
    fn claude_code_command() {
        let s = snippet(Client::ClaudeCode, &exe(), false).unwrap();
        assert_eq!(
            s.text,
            format!(
                "claude mcp add --scope user rsdiskwrite -- \"{}\" mcp serve",
                exe().display()
            )
        );
    }

    #[test]
    fn codex_toml_escapes_backslashes() {
        let s = snippet(Client::Codex, &exe(), true).unwrap();
        assert!(s.text.starts_with("[mcp_servers.rsdiskwrite]"));
        assert!(
            s.text
                .contains(r#"command = "C:\\Program Files\\rsdiskwrite\\rsdiskwrite.exe""#)
        );
        assert!(
            s.text
                .contains(r#"args = ["mcp", "serve", "--allow-write"]"#)
        );
    }

    #[test]
    fn openai_agents_python() {
        let s = snippet(Client::OpenaiAgents, &exe(), false).unwrap();
        assert!(s.text.contains("MCPServerStdio"));
        assert!(s.text.contains(r#""args": ["mcp", "serve"]"#));
        assert!(s.text.contains(r"C:\\Program Files"));
    }
}
