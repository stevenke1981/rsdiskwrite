//! `rsdiskwrite completions SHELL`.

use crate::cli::{Cli, CompletionsArgs};
use crate::error::Result;
use crate::output::Output;
use clap::CommandFactory;
use serde::Serialize;

/// JSON `data` of the `completions` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompletionsData {
    /// Shell the script is for.
    pub shell: String,
    /// The completion script.
    pub script: String,
}

/// Writes the completion script to stdout (raw script; with `--json` wrapped in the envelope).
pub fn run(args: CompletionsArgs, out: &Output) -> Result<()> {
    let mut buf = Vec::new();
    clap_complete::generate(args.shell, &mut Cli::command(), "rsdiskwrite", &mut buf);
    let data = CompletionsData {
        shell: args.shell.to_string(),
        script: String::from_utf8_lossy(&buf).into_owned(),
    };
    out.emit(&data, |d, _| d.script.clone())
}
