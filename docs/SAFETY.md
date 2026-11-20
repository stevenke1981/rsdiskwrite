# Safety model

rsdiskwrite overwrites whole physical disks. This page explains what it refuses, how it checks it is writing to the disk you meant, and what it cannot protect you from.

> **Status: experimental.** Real-hardware writes have not yet been validated on disposable devices. Everything below is verified by code review and tests against temp files and mocks only.

## Threat model

Protected against: choosing the wrong disk by mistake (human or AI), a disk renumbering between planning and writing, an agent writing without a human approving, piped input silently confirming a write, and a tampered or swapped image between planning and writing (MCP).

Not protected against: a malicious or compromised program running as Administrator, a malicious image (the contents are written as is), power loss or I/O failure mid-write (the target is left partially written), and the shell-bypass limitation below.

## Refusals (evaluated by one function, `ops::plan`, for CLI, GUI and MCP)

| Rule | Override |
|---|---|
| target holds `%SystemRoot%` (Windows system disk) | none |
| target holds the source image | none |
| target holds the temp dir (gzip images are decompressed there) | none |
| not removable and not on a USB/SD/MMC bus (fixed disk) | CLI/GUI `--allow-fixed`; **never over MCP** |
| larger than 2 GiB | CLI `-u`/`--allow-large`; MCP `allow_large` on `plan_write` |
| image larger than the disk | none |
| image size not a multiple of the logical sector size | none |

Refusals are reported all at once and the process exits 3 before any prompt. `write --dry-run` and MCP `plan_write` show them without writing.

## Identity checks

A disk is identified by number, size, model and serial (serial is its own field, not folded into the model). The identity is captured when planning and compared again before writing, and once more after the volumes are locked. A mismatch aborts with `identity_changed` (exit 7). Device identifiers can be missing or duplicated on some bridges and card readers, so do not unplug or swap devices during an operation. If any volume on the disk cannot be locked and dismounted, nothing is written. After writing, the partition table is refreshed and the disk is read back and compared (on by default).

## Confirmation

- **CLI:** at a terminal you must type `ERASE <N>`. When stdin is not a terminal, `-d N` and `--confirm "ERASE N"` are both required; the phrase is never read from a pipe. Wrong phrase: exit 4.
- **GUI:** explicit confirmation of the irreversible overwrite.
- **MCP:** disabled unless the server is started with `--allow-write`; then a single-use 5-minute `plan_id` bound to disk identity and image SHA-256, a native topmost Yes/No dialog (default No) on the local desktop that no flag or environment variable can bypass, and rechecks after approval. See [MCP.md](MCP.md#write-gating-four-layers).
- Elevation: writes need Administrator; the tool reports `elevation_required` (exit 5) rather than prompting UAC.

## Known limitation: agents with shell access bypass the MCP gate

The dialog gate protects the MCP path only. An AI agent that can run arbitrary shell commands can call `rsdiskwrite write IMAGE -d N --confirm "ERASE N"` itself, because the confirmation phrase is derivable from the disk number and the CLI is designed to be scriptable. The built-in refusals (system, source, temp, fixed, large disks) still apply, but a wrongly chosen removable disk would be erased without a human click.

Recommendations:
- In your AI client, require approval for shell commands (for example Claude Code permission rules that do not auto-allow `rsdiskwrite`, Codex approval policy `untrusted`/on-request) and review any `write` command, especially its `-d` value, before approving. Never auto-approve `rsdiskwrite write`.
- Run agents in a non-elevated session. Without Administrator rights `write`, `verify` and disk `hash` fail with exit 5, while planning, `list` and `inspect` still work.
- Keep `--allow-write` off unless needed; in read-only mode agents can still plan and verify.
- Prefer a dedicated test device and always compare `list` output (model, serial, size) with the physical device.

## Image authenticity

Gzip trailing data is ignored so that m0n0wall-style signed images work, but **signatures are not authenticated**; no key is trusted. Use `--expect-sha256 <HEX>` (CLI) or `expect_sha256` (MCP `plan_write`) with a checksum obtained from the publisher over a trusted channel. `inspect` reports trailing bytes so you can see appended data. The same applies to rsdiskwrite releases: verify `SHA256SUMS.txt` (a checksum next to a download only detects corruption, not a compromised release page).

## Operational advice

Close programs using the source image. Treat the first write on any machine as a test with a disposable device. Report problems at the project's GitHub issues, including the output of `rsdiskwrite doctor`.
