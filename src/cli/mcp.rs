//! `rsdiskwrite mcp serve` and `rsdiskwrite mcp config`.

use crate::cli::{McpArgs, McpCommand};
use crate::error::{Error, Result};
use crate::mcp::{self, ServerConfig, client_config};
use crate::output::Output;

/// Dispatches the MCP subcommands. `serve` owns stdout for JSON-RPC (global `--json` is
/// irrelevant there and nothing else is printed to stdout); `config` prints a snippet
/// (JSON `data` is [`client_config::Snippet`]).
pub fn run(args: McpArgs, out: &Output) -> Result<()> {
    match args.command {
        McpCommand::Serve(serve) => {
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
            // TODO(phase2-D): human layout (location + notes around the snippet text).
            out.emit(&snippet, |snippet, _| snippet.text.clone())
        }
    }
}
