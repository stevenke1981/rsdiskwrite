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
    let out = Output::new(cli.global.json, cli.global.quiet, cli.global.color);
    if let Err(error) = cli::run(cli, &out) {
        if !error.reported {
            out.error(&error);
        }
        std::process::exit(i32::from(error.exit_code()));
    }
}
