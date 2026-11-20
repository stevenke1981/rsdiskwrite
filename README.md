# rsdiskwrite

Write raw or gzip-compressed disk images (`.img`, `.img.gz`) to USB sticks and SD cards on Windows. It has a desktop GUI, a scriptable CLI with JSON output, and an optional MCP server so AI agents can plan and (with human approval) perform writes. It is an independent Rust implementation inspired by [physdiskwrite](https://m0n0.ch/wall/physdiskwrite.php); the GUI uses [AcmeUI Native](https://github.com/stevenke1981/AcmeUI-Native).

> **WARNING: experimental, and writing DESTROYS ALL DATA on the target disk.**
> Physical-disk writes have **not yet been validated on real hardware** (only on temp files and unit/integration tests). Check the disk number, model, serial and size every time, keep backups, and try a disposable device first. See [docs/SAFETY.md](docs/SAFETY.md).

## Install

- **Release zip:** download `rsdiskwrite-0.2.1-x86_64-pc-windows-msvc.zip` from [GitHub Releases](https://github.com/stevenke1981/rsdiskwrite/releases). Each release also has the bare `.exe` and `SHA256SUMS.txt`. Verify before running:
  ```powershell
  Get-FileHash .\rsdiskwrite-0.2.1-x86_64-pc-windows-msvc.zip -Algorithm SHA256
  Get-Content .\SHA256SUMS.txt   # the hashes must match
  ```
- **From source:** needs Windows x86-64 and Rust 1.88+ (see [Build and test](#build-and-test)).

Writing, verifying and hashing a disk need an **elevated** (Administrator) terminal. Listing disks does not.

## Quick start

**GUI:** run `rsdiskwrite.exe` with no arguments (as Administrator). Pick an image, pick a disk (number, model, serial and size are shown), leave read-back verification on, and confirm.

**CLI:** (run `rsdiskwrite <command> --help` for all flags)

```powershell
rsdiskwrite list                                   # find the disk number
rsdiskwrite inspect firmware.img.gz --sha256       # format, bytes that will be written, checksums
rsdiskwrite write firmware.img.gz -d 2 --dry-run   # check every safety rule, write nothing
rsdiskwrite write firmware.img.gz -d 2             # asks you to type: ERASE 2
rsdiskwrite verify firmware.img.gz -d 2            # optional later re-check (read-only)
```

`write` reads the disk back and compares it after writing (disable with `--no-verify`). For scripts: `--json`, `--confirm "ERASE 2"` and exit codes; see [docs/CLI.md](docs/CLI.md). The 0.1.x form `rsdiskwrite [-u] [-d N] IMAGE` still works.

Built-in refusals: the Windows system disk, the disk holding the image (or the temp dir for `.gz`), non-removable non-USB/SD disks (override: `--allow-fixed`), disks over 2 GiB (override: `-u`/`--allow-large`), images larger than the disk or not a multiple of the sector size.

**Image integrity:** m0n0wall-style signatures appended to images are tolerated but **not authenticated**. Compare against the publisher's checksum with `--expect-sha256 <HEX>`.

## Use with AI agents

`rsdiskwrite mcp serve` is an MCP server (stdio). By default it is read-only (list disks, inspect images, diagnose, dry-run plans, verify). `--allow-write` adds a `write_image` tool, and every write then still needs a single-use plan and a human clicking **Yes** in a dialog on your PC. Print client setup snippets (never edits files):

```powershell
rsdiskwrite mcp config --client claude-code     # also: claude-desktop, codex, openai-agents
```

Details and setup for each client: [docs/MCP.md](docs/MCP.md). Read the known limitation in [docs/SAFETY.md](docs/SAFETY.md): an agent that can run shell commands can use the CLI directly.

## Troubleshooting

- Run `rsdiskwrite doctor` first (add `--json` for scripts). It checks elevation, system and temp disk, disk and volume enumeration, terminals and the MCP server.
- **Exit 5 / `elevation_required`:** open an Administrator terminal. rsdiskwrite never triggers a UAC prompt itself.
- **Volume cannot be locked:** close Explorer windows, terminals, antivirus scans or any program using the card, then retry. Writing fails *before* any data is written if a volume cannot be locked or dismounted. Encrypted or virtual volumes and some card readers can block writes.
- **Disk not found / identity changed (exit 7):** the disk number changed or the device was re-plugged. Run `list` again; do not swap drives during a write.
- **Refused (exit 3):** read the message; it names the rule. Do not reach for `--allow-fixed` unless you are certain.
- **Piped input does not confirm:** when stdin is not a terminal you must pass `-d N --confirm "ERASE N"`.
- GNU toolchain builds with a non-ASCII workspace path can break `dlltool`; set `CARGO_TARGET_DIR` to an ASCII path.

## Build and test

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked           # temp files only; never touches physical disks
cargo build --release --locked
pwsh scripts/package.ps1      # dist/: zip, bare exe, SHA256SUMS.txt
```

AcmeUI Native is pinned to a git commit in `Cargo.toml`; dependencies are locked in `Cargo.lock`. Releases are built by `.github/workflows/release.yml` on tags `v*`.

## Project layout

```text
src/main.rs        thin binary (console subsystem; MCP needs stdio)
src/lib.rs         library root
src/cli/           clap commands, one file per subcommand
src/ops.rs         plan / execute / verify (all safety rules live here)
src/disk/          Windows disk enumeration, IOCTLs, volume lock/dismount, Disk model
src/image.rs       raw/gzip detection, inspect, decompression
src/copy.rs hash.rs confirm.rs output.rs error.rs
src/doctor.rs      diagnostics
src/mcp/           JSON-RPC protocol, server, tools, write gate, client config snippets
src/gui.rs         AcmeUI Native GUI
tests/             CLI and MCP stdio integration tests
docs/              CLI.md, MCP.md, SAFETY.md
```

Contributing agents: see [AGENTS.md](AGENTS.md). Changes: [CHANGELOG.md](CHANGELOG.md).

## License

MIT. AcmeUI Native and other dependencies keep their own licenses.
