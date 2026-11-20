# CLI reference (0.2.0)

```text
rsdiskwrite [GLOBAL OPTIONS] <COMMAND>
rsdiskwrite [-u] [-d N] IMAGE          legacy 0.1.x form, same as: write IMAGE [-u] [-d N]
rsdiskwrite                            no arguments: open the GUI
```

Writing, verifying and hashing a disk need an elevated (Administrator) terminal. `list`, `inspect`, `hash IMAGE`, `write --dry-run` (planning), `doctor` and `mcp config` work unelevated. The tool never raises a UAC prompt; it exits 5 instead.

## Global options

| Option | Meaning |
|---|---|
| `--json` | stdout carries exactly one JSON object (see envelope below); progress goes to stderr as NDJSON |
| `-q`, `--quiet` | suppress progress and informational messages |
| `--color auto\|always\|never` | default `auto`: colour only on a terminal and when `NO_COLOR` is unset |
| `-h`, `--help` / `-V`, `--version` | |

Human mode shows a progress bar (speed/ETA) only when stderr is a terminal.

## Commands

### list
`rsdiskwrite list [--json]` lists physical disks: number, bus, removable, size (and exact bytes), model, serial, flags `SYSTEM` / `FIXED`. Works without elevation.

### inspect
`rsdiskwrite inspect IMAGE [--sha256]` shows format (`raw` or `gzip`), file size, decompressed size (what a write puts on the disk), gzip member count and trailing bytes. `--sha256` also computes the SHA-256 of the stored file and of the decompressed content (reads the whole file).

### hash
`rsdiskwrite hash IMAGE` SHA-256 of the file as stored (compressed bytes for `.gz`).
`rsdiskwrite hash --disk N [--length BYTES]` SHA-256 of a physical disk (default the whole disk; needs Administrator).

### write
`rsdiskwrite write IMAGE -d N [options]` **destroys all data on disk N.**

| Option | Meaning |
|---|---|
| `-d`, `--disk N` | target disk number from `list`; prompted for only at a terminal, otherwise required |
| `-u`, `--allow-large` | allow disks over 2 GiB |
| `--allow-fixed` | allow non-removable disks not on a USB/SD/MMC bus |
| `--dry-run` | check every safety rule, print the plan, write nothing |
| `--no-verify` | skip the read-back (verification is on by default) |
| `--expect-sha256 HEX` | refuse unless the image file's SHA-256 equals HEX (exit 6 otherwise) |
| `--confirm "ERASE N"` | non-interactive confirmation; exact, case-sensitive match |

Flow: plan (all refusals, exit 3 before any prompt) -> confirmation -> lock and dismount volumes -> identity re-check -> write -> flush -> refresh partition info -> read-back verify. Confirmation: at a terminal you are asked to type `ERASE N`; when stdin is **not** a terminal both `-d` and `--confirm` are mandatory, and the phrase is never read from a pipe. Refusals: system disk, the disk holding the image, the temp-dir disk (gzip images), fixed disks (unless `--allow-fixed`), disks over 2 GiB (unless `-u`), image larger than the disk, image size not a multiple of the logical sector size. Gzip images are decompressed to a temp file first (stops when larger than the disk).

### verify
`rsdiskwrite verify IMAGE -d N` reads disk N back and compares it with the image (read-only; needs Administrator). Exit 6 with the first differing offset on mismatch.

### doctor
`rsdiskwrite doctor` checks: version, executable, OS, elevation (warn if not elevated), system disk, temp dir and free space, disk enumeration, volume enumeration, terminals/`NO_COLOR`, and an in-process MCP self-test. Exit 1 if any check fails; warnings do not fail.

### mcp
- `rsdiskwrite mcp serve [--allow-write]` serve MCP on stdio (see [MCP.md](MCP.md)). Logs go to stderr only.
- `rsdiskwrite mcp config --client claude-desktop|claude-code|codex|openai-agents [--allow-write] [--exe PATH]` prints a snippet; never edits files. With `--json`: `{client, format, location, text, notes[]}`.

### gui
`rsdiskwrite gui` opens the graphical interface (same as no arguments).

### completions
`rsdiskwrite completions bash|elvish|fish|powershell|zsh` prints a completion script, e.g. `rsdiskwrite completions powershell >> $PROFILE`. With `--json` the script is wrapped as `{"shell":...,"script":...}` in the envelope.

## Exit codes

| Code | Meaning | JSON `error.code` |
|---|---|---|
| 0 | success | |
| 1 | I/O, internal, invalid image, unsupported, other failure (also `doctor` with a failed check) | `io`, `internal`, `invalid_image`, `unsupported`, `plan_invalid`, `busy` |
| 2 | usage error | `usage` |
| 3 | safety refusal | `safety_refusal` |
| 4 | not confirmed | `not_confirmed` |
| 5 | Administrator required | `elevation_required` |
| 6 | verify or SHA-256 mismatch | `verify_mismatch`, `hash_mismatch` |
| 7 | disk not found or identity changed | `disk_not_found`, `identity_changed` |

## JSON envelope (schema 1)

Success: `{"schema":1,"ok":true,"data":{...}}`. Failure: `{"schema":1,"ok":false,"error":{"code":"...","message":"...","hint":null}}`. Sizes are integer bytes. The `code` strings and exit codes are a frozen contract; additive fields may appear, existing ones will not change without a schema bump. Usage errors (exit 2) are also reported as JSON when `--json` is present. Failures with details (a refused `write`, a verify mismatch after writing) add the plan or report next to `error`.

`list --json`:
```json
{"schema":1,"ok":true,"data":{"disks":[{"number":2,"size":8004304896,"model":"SanDisk Ultra","serial":"4C530001","bus":"usb","removable":true,"sector":512,"is_system":false}],"method":"setup_api"}}
```
`method` is `setup_api` or `probe` (fallback). `serial` may be `null`; bus values are snake_case (`usb`, `sd`, `sata`, `nvme`, ...).

`inspect --json`: `{path, format, file_size, decompressed_size, gzip_members, trailing_bytes, sha256, content_sha256}` (the two hashes are `null` without `--sha256`).

`hash --json`: `{image, disk, bytes, sha256}` (`image` or `disk` is `null`).

`write --dry-run --json`: the plan: `{request, disk, image{path,format,file_size,...,sha256}, refusals[{code,message,hint}], warnings[]}`. Refusal codes: `system_disk`, `source_disk`, `temp_disk`, `fixed_disk`, `large_disk`, `image_too_large`, `sector_misaligned`.

`write --json` success: `{disk, image, format, bytes_written, content_sha256, verify{disk,image,bytes,matched,first_mismatch,content_sha256}|null, elapsed_ms}`.

`verify --json`: `{disk, image, bytes, matched, first_mismatch, content_sha256}`.

`doctor --json`: `{version, checks:[{id,status:"ok|warn|fail",detail,hint}], ...}`.

Progress lines on stderr in JSON mode look like `{"type":"progress","phase":"hashing","done":5,"total":5}`.

Failure example:
```powershell
PS> rsdiskwrite write x.img -d 99 --json
{"schema":1,"ok":false,"error":{"code":"disk_not_found","message":"PhysicalDrive99 does not exist","hint":"Run 'rsdiskwrite list' to see available disks"}}
PS> $LASTEXITCODE
7
```

## Examples

```powershell
rsdiskwrite list --json
rsdiskwrite inspect firmware.img.gz --sha256 --json
rsdiskwrite write firmware.img -d 2 --dry-run --json
rsdiskwrite write firmware.img.gz -d 2 --expect-sha256 <HEX>          # prompts: type ERASE 2
rsdiskwrite write firmware.img -d 2 --confirm "ERASE 2" --json        # non-interactive
rsdiskwrite verify firmware.img -d 2
rsdiskwrite hash --disk 2 --length 268435456
```
