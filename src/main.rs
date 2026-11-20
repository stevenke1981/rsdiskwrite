#[cfg(not(windows))]
compile_error!("rsdiskwrite currently supports Windows only");

use clap::Parser;
use rsdiskwrite::cli::{self, Cli};
use rsdiskwrite::output::{ColorChoice, Output};

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            use clap::error::ErrorKind;
            let printable = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            if !printable && std::env::args_os().any(|a| a == "--json") {
                Output::new(true, false, ColorChoice::Never).error(&cli::usage_error(&error));
                std::process::exit(2);
            }
            error.exit();
        }
    };
    let serve = matches!(
        &cli.command,
        Some(cli::Command::Mcp(cli::McpArgs {
            command: cli::McpCommand::Serve(_)
        }))
    );
    let out = Output::new(cli.global.json, cli.global.quiet, cli.global.color);
    if let Err(error) = cli::run(cli, &out) {
        if serve {
            // MCP stdout carries only JSON-RPC; never emit an envelope there.
            if !error.reported {
                eprintln!("error: {error}");
            }
        } else if !error.reported {
            out.error(&error);
        }
        std::process::exit(i32::from(error.exit_code()));
    }
}
