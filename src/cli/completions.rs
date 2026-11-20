//! `rsdiskwrite completions SHELL`.

use crate::cli::{Cli, CompletionsArgs};
use crate::error::Result;
use crate::output::Output;
use clap::CommandFactory;

/// Writes the completion script to stdout (raw script even with `--json`).
pub fn run(args: CompletionsArgs, out: &Output) -> Result<()> {
    let _ = out;
    clap_complete::generate(
        args.shell,
        &mut Cli::command(),
        "rsdiskwrite",
        &mut std::io::stdout(),
    );
    Ok(())
}
