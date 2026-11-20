# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-11-20

### Added
- Subcommand CLI (clap): `list`, `inspect`, `hash`, `write`, `verify`, `doctor`, `mcp serve`, `mcp config`, `gui`, `completions`. Global `--json`, `--quiet`, `--color`.
- Machine-readable output: `--json` prints exactly one `{"schema":1,"ok":...}` object on stdout; progress goes to stderr (NDJSON in JSON mode). Stable error codes and exit codes 0-7.
- `write --dry-run` runs every safety check and prints the plan without writing.
- Read-back verification after every write (on by default, `--no-verify` to skip) and a standalone `verify` command.
- SHA-256 support: `inspect --sha256`, `hash` (image or disk), `write --expect-sha256`.
- `inspect`: format, file size, decompressed size, gzip member count, trailing bytes.
- Fixed-disk guard: non-removable disks not on USB/SD/MMC buses are refused unless `--allow-fixed`.
- Structured disk identity (number, size, model, serial, bus) re-checked before writing and again after locking volumes.
- Enumeration through SetupAPI (`GUID_DEVINTERFACE_DISK`) with a probe fallback; listing works without Administrator.
- Volume dismount after locking and a partition-table refresh after writing.
- `doctor` environment diagnostics (also exposed over MCP).
- MCP server over stdio (hand-rolled JSON-RPC 2.0): `list_disks`, `inspect_image`, `doctor`, `plan_write`, `verify_disk`, and opt-in `write_image` behind `--allow-write` with four gating layers (see docs/MCP.md and docs/SAFETY.md).
- `mcp config --client claude-desktop|claude-code|codex|openai-agents` prints setup snippets without editing files.
- Shell completions (`completions <shell>`).
- GUI: verify and allow-fixed toggles, disk identity rows, refusal display, per-phase percent progress.
- Release workflow (tag `v*`): zip, bare exe and `SHA256SUMS.txt` on GitHub Releases; MSRV (1.88) CI job; build cache.
- `completions --json` wraps the script in the envelope; `hash --disk --length` no longer silently clamps; strict hex parsing for digests; `mcp serve` errors go to stderr only; the GUI releases its own auto-created console; release workflow runs tests; `package.ps1` defaults `-Target` to the rustc host.
- Documentation: README rewrite, docs/CLI.md, docs/MCP.md, docs/SAFETY.md, AGENTS.md, CHANGELOG.

### Changed
- Code split into a library plus thin binary (`cli`, `ops`, `disk`, `image`, `copy`, `hash`, `mcp`, `doctor`, `gui`, ...).
- Disk serial is a separate field instead of being folded into the model string.
- Disk handles for enumeration/queries are opened with zero access rights, so listing no longer needs elevation.
- The confirmation phrase is only prompted for when stdin is a terminal; otherwise `-d` and `--confirm "ERASE N"` are required. It is never read from a pipe.
- The legacy form `rsdiskwrite [-u] [-d N] IMAGE` is kept and maps to `write`; no arguments still opens the GUI.

### Known limitations
- Real-hardware writes are still unvalidated. Signatures on signed images are not authenticated (use `--expect-sha256`). An agent with shell access can run `write --confirm` directly. See docs/SAFETY.md.

## [0.1.0] - 2026-11-15

### Added
- Initial release: Windows physical-disk image writer with an AcmeUI Native GUI and a basic CLI (`rsdiskwrite [-u] [-d N] IMAGE`).
- Raw and gzip images detected by content; concatenated gzip members and trailing data tolerated.
- Refusal of the Windows system disk, the source-image disk and (for gzip) the temp-directory disk; 2 GiB default disk-size limit; identity re-check; volume locking; typed `ERASE <N>` confirmation.

[0.2.0]: https://github.com/stevenke1981/rsdiskwrite/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/stevenke1981/rsdiskwrite/releases/tag/v0.1.0
