# AGENTS.md

Guide for AI agents. Part 1 is for agents working **on** this repo; part 2 is for agents **using** the tool.

## Part 1: working on rsdiskwrite

Windows-only Rust crate (library + thin binary), edition 2024, MSRV 1.88, branch work happens on feature branches off `main`. Version 0.2.1.

### Layout and ownership
| Path | Owns |
|---|---|
| `src/main.rs` | thin binary; console subsystem (MCP needs stdio); `compile_error!` off Windows |
| `src/error.rs` | `Error`/`ErrorKind`: JSON `code` strings and exit codes (frozen) |
| `src/ops.rs` | `plan` (all refusals, = dry run), `execute`, `verify`; the single place safety rules live |
| `src/disk/` | `model` (Disk, BusType, identity; cross-platform), `enumerate` (SetupAPI + probe), `ioctl`, `volume` (lock/dismount) |
| `src/image.rs`, `copy.rs`, `hash.rs`, `confirm.rs` | image inspect/decompress, copy/compare, SHA-256, `ERASE N` phrase |
| `src/output.rs` | human/JSON rendering, colour, progress sink |
| `src/cli/` | clap definitions in `mod.rs`, one file per subcommand |
| `src/mcp/` | `protocol`, `server`, `tools`, `gate` (plans, dialog), `client_config` |
| `src/doctor.rs`, `src/gui.rs` | diagnostics, AcmeUI GUI |
| `tests/cli.rs`, `tests/mcp_stdio.rs` | integration tests (temp files, spawned binary) |

Front ends (cli, gui, mcp) call `ops`; do not duplicate safety logic in a front end.

### Commands
```powershell
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```
All must pass before committing. GNU toolchain with a non-ASCII path: set `CARGO_TARGET_DIR` to an ASCII path. `Cargo.lock` is committed; AcmeUI deps are git-pinned (not publishable to crates.io).

### Rules
- **Never write to a physical disk** in tests, scripts or experiments, and never run `write`/`verify` against real `\\.\PhysicalDriveN`. Tests use temp files and mock `Confirmer`s. Do not launch the GUI. Read-only commands (`list`, `doctor`, `inspect`, `write --dry-run`) are fine.
- **MCP stdout hygiene:** nothing but JSON-RPC lines on stdout in `mcp serve`. No `println!`/`dbg!` reachable from it; log to stderr. `tests/mcp_stdio.rs` asserts every stdout line parses as JSON-RPC.
- **Frozen contract:** the `{"schema":1,...}` envelope, `ErrorKind` codes, exit codes 0-7, disk/plan/report JSON field names and MCP tool names/input schemas are public API. Add fields, do not rename or remove; breaking changes need a schema bump, CHANGELOG entry and doc updates (docs/CLI.md, docs/MCP.md).
- **Write gate:** never add a way to skip the MCP dialog (no flag, env var or argument). `write_image` must stay absent from `tools/list` without `--allow-write`. Fixed disks are never writable over MCP.
- Keep the experimental/data-destruction warnings in README and docs until real hardware is validated.
- No `unwrap` on IOCTL buffer slices without length checks. Keep MSRV 1.88 APIs.
- Commits: conventional commits; do not push unless asked.

## Part 2: using rsdiskwrite as an agent

- Prefer `--json`: stdout is exactly one `{"schema":1,"ok":...}` object; progress is on stderr. Branch on exit code: 0 ok, 1 I/O/internal, 2 usage, 3 safety refusal, 4 not confirmed, 5 needs Administrator, 6 verify/hash mismatch, 7 disk not found/identity changed. Error `code` and `hint` say what to do. Reference: [docs/CLI.md](docs/CLI.md).
- Workflow: `doctor` -> `list --json` -> **ask the human which disk** (never guess; show model, serial, size) -> `inspect IMAGE --sha256` -> `write IMAGE -d N --dry-run --json` -> have the human approve -> write -> check `verify.matched`.
- Do not run `rsdiskwrite write --confirm "ERASE N"` on your own. A human must choose the disk and approve that exact command. Never use `--allow-fixed` or `-u` unless the human explicitly asks for that disk. Pass `--expect-sha256` when a checksum is available.
- Prefer MCP when available ([docs/MCP.md](docs/MCP.md)): `list_disks`, `inspect_image`, `doctor`, `plan_write`, `verify_disk`, and `write_image` (only when the server has `--allow-write`; needs the `plan_id` from `plan_write` and a human clicking Yes in a dialog, so tell the user to watch for it). `plan_id` is single-use and expires in 5 minutes.
- Exit 5 means not elevated: report it to the user rather than trying to work around it.
- Safety background: [docs/SAFETY.md](docs/SAFETY.md).
