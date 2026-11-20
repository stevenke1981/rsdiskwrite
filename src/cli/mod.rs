//! Command-line interface: clap definitions, legacy-form mapping and dispatch.
//!
//! Each subcommand lives in its own file with `pub fn run(args, &Output) -> Result<()>`.

pub mod completions;
pub mod doctor;
pub mod gui;
pub mod hash;
pub mod inspect;
pub mod list;
pub mod mcp;
pub mod verify;
pub mod write;

use crate::error::{Error, Result};
use crate::hash::Digest;
use crate::mcp::client_config::Client;
use crate::output::{ColorChoice, Output};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

const ABOUT: &str = "Write disk images to USB sticks and SD cards on Windows";

const LONG_ABOUT: &str = "\
Write raw or gzip-compressed disk images to USB sticks and SD cards on Windows.

Writing destroys everything on the target disk. rsdiskwrite refuses the Windows system \
disk, the disk holding the image, fixed (non-USB/SD) disks unless --allow-fixed, and \
disks over 2 GiB unless -u/--allow-large. Every write must be confirmed by typing \
ERASE <N> at a terminal, or by passing --confirm \"ERASE <N>\" when stdin is not a \
terminal. Writing needs an elevated (Administrator) terminal; listing does not.

Run without arguments to open the graphical interface.";

const EXAMPLES: &str = "\
Examples:
  rsdiskwrite list
      Show physical disks (number, size, bus, model, serial).
  rsdiskwrite inspect firmware.img.gz
      Show the image format and how many bytes would be written.
  rsdiskwrite write firmware.img -d 2 --dry-run
      Check every safety rule for disk 2 without writing.
  rsdiskwrite write firmware.img -d 2
      Write to disk 2; you will be asked to type ERASE 2.
  rsdiskwrite write firmware.img -d 2 --confirm \"ERASE 2\" --json
      Non-interactive write with machine-readable output.
  rsdiskwrite verify firmware.img -d 2
      Compare disk 2 with the image without writing.
  rsdiskwrite doctor
      Diagnose permissions, disks and the MCP server.
  rsdiskwrite mcp config --client claude-code
      Print the command that registers rsdiskwrite with Claude Code.
  rsdiskwrite
      Open the graphical interface.

Legacy form (0.1.x):
  rsdiskwrite [-u] [-d N] IMAGE   is the same as   rsdiskwrite write IMAGE [-u] [-d N]

Exit codes:
  0 success                 4 not confirmed
  1 I/O or internal error   5 elevation (Administrator) required
  2 usage error             6 verification or SHA-256 mismatch
  3 safety refusal          7 disk not found or disk identity changed";

/// The whole command line.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "rsdiskwrite",
    bin_name = "rsdiskwrite",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_long_help = EXAMPLES,
    max_term_width = 100
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    /// 0.1.x form: `rsdiskwrite [-u] [-d N] IMAGE`.
    #[command(flatten)]
    pub legacy: LegacyArgs,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Flags accepted by every subcommand.
#[derive(Debug, Clone, Default, Args)]
pub struct GlobalArgs {
    /// Print exactly one JSON object on stdout: {"schema":1,"ok":true,"data":...} or
    /// {"schema":1,"ok":false,"error":{"code","message","hint"}}; progress goes to stderr as
    /// NDJSON
    #[arg(long, global = true)]
    pub json: bool,
    /// Suppress progress and informational messages
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,
    /// When to use colours (NO_COLOR is honoured in auto mode)
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = ColorChoice::Auto, global = true)]
    pub color: ColorChoice,
}

/// The 0.1.x top-level form, mapped to `write`.
#[derive(Debug, Clone, Default, Args)]
#[command(next_help_heading = "Legacy usage (same as 'write')")]
pub struct LegacyArgs {
    /// Allow disks larger than 2 GiB
    #[arg(short = 'u')]
    pub allow_large: bool,
    /// Target disk number (prompted for when omitted at a terminal)
    #[arg(short = 'd', value_name = "N")]
    pub disk: Option<u32>,
    /// Image to write
    #[arg(value_name = "IMAGE")]
    pub image: Option<PathBuf>,
}

/// Subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// List physical disks
    List(ListArgs),
    /// Show an image's format, sizes and (optionally) SHA-256
    Inspect(InspectArgs),
    /// Compute the SHA-256 of an image file or of a disk
    Hash(HashArgs),
    /// Write an image to a disk (DESTROYS ALL DATA on the disk)
    Write(WriteArgs),
    /// Compare a disk with an image without writing
    Verify(VerifyArgs),
    /// Diagnose permissions, disks, volumes and the MCP server
    Doctor(DoctorArgs),
    /// Run the MCP server or print client configuration
    Mcp(McpArgs),
    /// Print a shell completion script
    Completions(CompletionsArgs),
    /// Open the graphical interface
    Gui(GuiArgs),
}

/// `list` (no options).
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
#[command(after_long_help = "Examples:\n  rsdiskwrite list\n  rsdiskwrite list --json")]
pub struct ListArgs {}

/// `inspect IMAGE`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(
    after_long_help = "Examples:\n  rsdiskwrite inspect firmware.img.gz\n  rsdiskwrite inspect firmware.img --sha256 --json"
)]
pub struct InspectArgs {
    /// Image file (raw or gzip)
    #[arg(value_name = "IMAGE")]
    pub image: PathBuf,
    /// Also compute the SHA-256 of the file and of its decompressed content
    #[arg(long)]
    pub sha256: bool,
}

/// `hash IMAGE` or `hash --disk N`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(
    after_long_help = "Examples:\n  rsdiskwrite hash firmware.img.gz\n  rsdiskwrite hash --disk 2 --length 268435456"
)]
pub struct HashArgs {
    /// Image file to hash as stored (compressed bytes for .gz)
    #[arg(
        value_name = "IMAGE",
        required_unless_present = "disk",
        conflicts_with = "disk"
    )]
    pub image: Option<PathBuf>,
    /// Hash a physical disk instead (needs Administrator)
    #[arg(short = 'd', long, value_name = "N")]
    pub disk: Option<u32>,
    /// With --disk: hash only the first BYTES bytes (default: the whole disk)
    #[arg(
        long,
        value_name = "BYTES",
        requires = "disk",
        conflicts_with = "image"
    )]
    pub length: Option<u64>,
}

/// `write IMAGE -d N`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(after_long_help = "\
Examples:
  rsdiskwrite write firmware.img -d 2 --dry-run
  rsdiskwrite write firmware.img.gz -d 2
  rsdiskwrite write firmware.img -d 2 --expect-sha256 <HEX>
  rsdiskwrite write firmware.img -d 2 --confirm \"ERASE 2\" --json

At a terminal you are asked to type ERASE <N>. When stdin is not a terminal, -d and
--confirm \"ERASE <N>\" are both required; the phrase is never read from a pipe.")]
pub struct WriteArgs {
    /// Image file (raw or gzip)
    #[arg(value_name = "IMAGE")]
    pub image: PathBuf,
    /// Target disk number from 'rsdiskwrite list' (prompted for at a terminal)
    #[arg(short = 'd', long, value_name = "N")]
    pub disk: Option<u32>,
    /// Allow disks larger than 2 GiB
    #[arg(short = 'u', long)]
    pub allow_large: bool,
    /// Allow non-removable disks that are not on a USB/SD/MMC bus
    #[arg(long)]
    pub allow_fixed: bool,
    /// Check every safety rule and show the plan without writing
    #[arg(long)]
    pub dry_run: bool,
    /// Skip reading the disk back after writing (verification is on by default)
    #[arg(long)]
    pub no_verify: bool,
    /// Refuse to write unless the image file's SHA-256 equals HEX
    #[arg(long, value_name = "HEX", value_parser = parse_digest)]
    pub expect_sha256: Option<Digest>,
    /// Confirm non-interactively; must be exactly "ERASE <N>" for the target disk
    #[arg(long, value_name = "PHRASE")]
    pub confirm: Option<String>,
}

impl WriteArgs {
    /// Arguments for writing `image` with every option at its default.
    pub fn new(image: impl Into<PathBuf>) -> Self {
        Self {
            image: image.into(),
            disk: None,
            allow_large: false,
            allow_fixed: false,
            dry_run: false,
            no_verify: false,
            expect_sha256: None,
            confirm: None,
        }
    }
}

/// `verify IMAGE -d N`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(after_long_help = "Examples:\n  rsdiskwrite verify firmware.img.gz -d 2")]
pub struct VerifyArgs {
    /// Image file (raw or gzip)
    #[arg(value_name = "IMAGE")]
    pub image: PathBuf,
    /// Disk number to read back (needs Administrator)
    #[arg(short = 'd', long, value_name = "N")]
    pub disk: u32,
}

/// `doctor` (no options).
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
#[command(after_long_help = "Examples:\n  rsdiskwrite doctor\n  rsdiskwrite doctor --json")]
pub struct DoctorArgs {}

/// `mcp <serve|config>`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCommand,
}

/// MCP subcommands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum McpCommand {
    /// Serve MCP over stdio (newline-delimited JSON-RPC)
    Serve(McpServeArgs),
    /// Print a configuration snippet for an MCP client (never edits files)
    Config(McpConfigArgs),
}

/// `mcp serve`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
#[command(after_long_help = "\
Examples:
  rsdiskwrite mcp serve
  rsdiskwrite mcp serve --allow-write

Without --allow-write only read-only tools are offered. With it, write_image is offered,
but every write still needs a plan_id from plan_write and a human clicking Yes in a
dialog on this computer.")]
pub struct McpServeArgs {
    /// Offer the destructive write_image tool (each write still needs human approval)
    #[arg(long)]
    pub allow_write: bool,
}

/// `mcp config --client C`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(
    after_long_help = "Examples:\n  rsdiskwrite mcp config --client claude-desktop\n  rsdiskwrite mcp config --client codex --allow-write"
)]
pub struct McpConfigArgs {
    /// Client to configure
    #[arg(long, value_enum, value_name = "CLIENT")]
    pub client: Client,
    /// Configure the server with --allow-write
    #[arg(long)]
    pub allow_write: bool,
    /// Executable path to put in the snippet (default: this executable)
    #[arg(long, value_name = "PATH")]
    pub exe: Option<PathBuf>,
}

/// `completions SHELL`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(
    after_long_help = "Examples:\n  rsdiskwrite completions powershell >> $PROFILE\n  rsdiskwrite completions bash > rsdiskwrite.bash"
)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum, value_name = "SHELL")]
    pub shell: clap_complete::Shell,
}

/// `gui` (no options).
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct GuiArgs {}

fn parse_digest(text: &str) -> std::result::Result<Digest, String> {
    crate::hash::parse_hex(text).map_err(|e| e.message)
}

impl Cli {
    /// Resolves the command to run: an explicit subcommand; the legacy
    /// `[-u] [-d N] IMAGE` form as `write`; no arguments at all as `gui`. Legacy flags
    /// without an image, or `--json` alone, are usage errors.
    pub fn into_command(self) -> Result<Command> {
        let legacy = self.legacy;
        if let Some(command) = self.command {
            if legacy.allow_large || legacy.disk.is_some() || legacy.image.is_some() {
                return Err(Error::usage(
                    "legacy -u/-d/IMAGE arguments cannot be combined with a subcommand",
                )
                .with_hint(
                    "Put the options after the subcommand, e.g. rsdiskwrite write IMAGE -d N",
                ));
            }
            return Ok(command);
        }
        match legacy.image {
            Some(image) => Ok(Command::Write(WriteArgs {
                disk: legacy.disk,
                allow_large: legacy.allow_large,
                ..WriteArgs::new(image)
            })),
            None if legacy.allow_large || legacy.disk.is_some() => {
                Err(Error::usage("an IMAGE is required with -u/-d")
                    .with_hint("Usage: rsdiskwrite write IMAGE -d N (see rsdiskwrite --help)"))
            }
            None if self.global.json => Err(Error::usage("no command given")
                .with_hint("Run 'rsdiskwrite --help' for the list of commands")),
            None => Ok(Command::Gui(GuiArgs {})),
        }
    }
}

/// Converts a clap parse error into a [`crate::ErrorKind::Usage`] error (first line of
/// clap's message, without the `error: ` prefix) for JSON rendering.
pub fn usage_error(error: &clap::Error) -> Error {
    let rendered = error.to_string();
    let first = rendered.lines().next().unwrap_or("invalid arguments");
    Error::usage(first.strip_prefix("error: ").unwrap_or(first).trim())
        .with_hint("Run 'rsdiskwrite --help' for usage")
}

/// Runs the resolved command.
pub fn run(cli: Cli, out: &Output) -> Result<()> {
    match cli.into_command()? {
        Command::List(args) => list::run(args, out),
        Command::Inspect(args) => inspect::run(args, out),
        Command::Hash(args) => hash::run(args, out),
        Command::Write(args) => write::run(args, out),
        Command::Verify(args) => verify::run(args, out),
        Command::Doctor(args) => doctor::run(args, out),
        Command::Mcp(args) => mcp::run(args, out),
        Command::Completions(args) => completions::run(args, out),
        Command::Gui(args) => gui::run(args, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> std::result::Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("rsdiskwrite").chain(args.iter().copied()))
    }

    fn command(args: &[&str]) -> Command {
        parse(args).unwrap().into_command().unwrap()
    }

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_open_the_gui() {
        assert!(matches!(command(&[]), Command::Gui(_)));
        assert!(matches!(command(&["gui"]), Command::Gui(_)));
    }

    #[test]
    fn legacy_form_maps_to_write() {
        let Command::Write(args) = command(&["-u", "-d", "1", "x.img"]) else {
            panic!("not write");
        };
        assert_eq!(args.image, PathBuf::from("x.img"));
        assert_eq!(args.disk, Some(1));
        assert!(args.allow_large);
        assert!(!args.no_verify && !args.dry_run && args.confirm.is_none());

        let Command::Write(args) = command(&["x.img"]) else {
            panic!("not write");
        };
        assert_eq!(args, WriteArgs::new("x.img"));

        let Command::Write(args) = command(&["x.img", "-d", "3"]) else {
            panic!("not write");
        };
        assert_eq!(args.disk, Some(3));
    }

    #[test]
    fn legacy_flags_without_image_are_usage_errors() {
        let error = parse(&["-d", "1"]).unwrap().into_command().unwrap_err();
        assert_eq!(error.exit_code(), 2);
        let error = parse(&["--json"]).unwrap().into_command().unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(parse(&["-d", "x", "a.img"]).is_err());
        assert!(parse(&["a.img", "b.img"]).is_err());
    }

    #[test]
    fn legacy_flags_before_a_subcommand_are_rejected() {
        let error = parse(&["-u", "list"]).unwrap().into_command().unwrap_err();
        assert_eq!(error.exit_code(), 2);
        let error = parse(&["-d", "1", "write", "x.img"])
            .unwrap()
            .into_command()
            .unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn write_subcommand_flags() {
        let hex = "ab".repeat(32);
        let cli = parse(&[
            "write",
            "fw.img.gz",
            "--disk",
            "2",
            "--allow-large",
            "--allow-fixed",
            "--dry-run",
            "--no-verify",
            "--expect-sha256",
            &hex,
            "--confirm",
            "ERASE 2",
            "--json",
        ])
        .unwrap();
        assert!(cli.global.json);
        let Command::Write(args) = cli.into_command().unwrap() else {
            panic!("not write");
        };
        assert_eq!(args.disk, Some(2));
        assert!(args.allow_large && args.allow_fixed && args.dry_run && args.no_verify);
        assert_eq!(args.expect_sha256.unwrap().to_hex(), hex);
        assert_eq!(args.confirm.as_deref(), Some("ERASE 2"));
        assert!(parse(&["write", "x.img", "--expect-sha256", "nothex"]).is_err());
        let Command::Write(args) = command(&["write", "x.img", "-u", "-d", "4"]) else {
            panic!("not write");
        };
        assert!(args.allow_large);
        assert_eq!(args.disk, Some(4));
    }

    #[test]
    fn global_flags_before_and_after_subcommands() {
        let cli = parse(&["--json", "--color", "never", "list"]).unwrap();
        assert!(cli.global.json);
        assert_eq!(cli.global.color, ColorChoice::Never);
        assert!(matches!(cli.into_command().unwrap(), Command::List(_)));
        let cli = parse(&["list", "--quiet", "--json"]).unwrap();
        assert!(cli.global.json && cli.global.quiet);
        let cli = parse(&["--json", "-d", "1", "x.img"]).unwrap();
        assert!(cli.global.json);
        assert!(matches!(cli.into_command().unwrap(), Command::Write(_)));
    }

    #[test]
    fn other_subcommands() {
        assert!(matches!(
            command(&["inspect", "a.img", "--sha256"]),
            Command::Inspect(InspectArgs { sha256: true, .. })
        ));
        assert!(matches!(
            command(&["hash", "a.img"]),
            Command::Hash(HashArgs {
                image: Some(_),
                disk: None,
                ..
            })
        ));
        assert!(matches!(
            command(&["hash", "--disk", "1", "--length", "512"]),
            Command::Hash(HashArgs {
                image: None,
                disk: Some(1),
                length: Some(512)
            })
        ));
        assert!(parse(&["hash"]).is_err());
        assert!(parse(&["hash", "a.img", "--disk", "1"]).is_err());
        assert!(parse(&["hash", "a.img", "--length", "5"]).is_err());
        assert!(matches!(
            command(&["verify", "a.img", "-d", "1"]),
            Command::Verify(VerifyArgs { disk: 1, .. })
        ));
        assert!(parse(&["verify", "a.img"]).is_err());
        assert!(matches!(command(&["doctor"]), Command::Doctor(_)));
        assert!(matches!(
            command(&["mcp", "serve", "--allow-write"]),
            Command::Mcp(McpArgs {
                command: McpCommand::Serve(McpServeArgs { allow_write: true })
            })
        ));
        for (name, client) in [
            ("claude-desktop", Client::ClaudeDesktop),
            ("claude-code", Client::ClaudeCode),
            ("codex", Client::Codex),
            ("openai-agents", Client::OpenaiAgents),
        ] {
            let Command::Mcp(McpArgs {
                command: McpCommand::Config(args),
            }) = command(&["mcp", "config", "--client", name])
            else {
                panic!("not mcp config");
            };
            assert_eq!(args.client, client);
        }
        assert!(parse(&["mcp", "config"]).is_err());
        assert!(matches!(
            command(&["completions", "powershell"]),
            Command::Completions(_)
        ));
        assert!(parse(&["completions", "nushell-ish"]).is_err());
    }

    #[test]
    fn clap_errors_become_usage_errors() {
        let error = usage_error(&parse(&["write"]).unwrap_err());
        assert_eq!(error.exit_code(), 2);
        assert!(!error.message.starts_with("error:"), "{}", error.message);
        let help = parse(&["--help"]).unwrap_err();
        assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
        let long = Cli::command().render_long_help().to_string();
        assert!(long.contains("Exit codes"));
        assert!(long.contains("Legacy"));
    }
}
