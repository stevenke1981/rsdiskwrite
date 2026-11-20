//! `rsdiskwrite mcp serve` and `rsdiskwrite mcp config`.

use crate::cli::{McpArgs, McpCommand};
use crate::error::{Error, Result};
use crate::mcp::{self, ServerConfig, client_config};
use crate::output::{Output, Style};

/// Dispatches the MCP subcommands. `serve` owns stdout for JSON-RPC (global `--json` is
/// irrelevant there and nothing else is printed to stdout); `config` prints a snippet
/// (JSON `data` is [`client_config::Snippet`]).
pub fn run(args: McpArgs, out: &Output) -> Result<()> {
    match args.command {
        McpCommand::Serve(serve) => {
            use std::io::IsTerminal;
            if std::io::stderr().is_terminal() {
                eprintln!(
                    "rsdiskwrite MCP server ({}) listening on stdin/stdout{}; press Ctrl+Z then Enter to stop",
                    crate::VERSION,
                    if serve.allow_write {
                        ", writes enabled"
                    } else {
                        ""
                    }
                );
            }
            let stdin = std::io::stdin().lock();
            mcp::serve(
                stdin,
                std::io::stdout(),
                ServerConfig::new(serve.allow_write),
            )
        }
        McpCommand::Config(config) => {
            let exe = match config.exe {
                Some(exe) => exe,
                None => std::env::current_exe()
                    .map_err(|e| Error::io("cannot determine executable path", e))?,
            };
            let snippet = client_config::snippet(config.client, &exe, config.allow_write)?;
            out.emit(&snippet, render_snippet)
        }
    }
}

/// Human rendering of a [`client_config::Snippet`].
pub fn render_snippet(snippet: &client_config::Snippet, out: &Output) -> String {
    let mut text = String::new();
    if let Some(location) = &snippet.location {
        text.push_str(&format!(
            "{} {location}\n\n",
            out.paint("Add to:", Style::Bold)
        ));
    } else {
        text.push_str(&format!("{}\n\n", out.paint("Run / use:", Style::Bold)));
    }
    text.push_str(&snippet.text);
    text.push('\n');
    for note in &snippet.notes {
        let styled = if note.starts_with("WARNING") {
            out.paint(note, Style::Red)
        } else {
            note.clone()
        };
        text.push_str(&format!("\n- {styled}"));
    }
    text
}
