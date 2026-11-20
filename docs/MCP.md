# MCP server (0.2.1)

`rsdiskwrite mcp serve [--allow-write]` is a Model Context Protocol server over stdio: newline-delimited JSON-RPC 2.0, one message per line. stdout carries only protocol messages; all logs go to stderr. Supported methods: `initialize` (protocol versions 2025-06-18, 2025-03-26, 2024-11-05; otherwise it answers 2025-06-18), `notifications/initialized`, `ping`, `tools/list`, `tools/call`. Unknown methods return -32601; notifications get no response; bad tool arguments return -32602. Each input line may be at most 1 MiB; a longer line is discarded and answered with -32600 (null id) and the server keeps serving.

Every tool result has `structuredContent` (same shape as the CLI JSON `data`) plus one text block. Failures set `isError: true` and `structuredContent = {code, message, hint}` with the same codes as the CLI ([CLI.md](CLI.md#exit-codes)). All input schemas are closed (`additionalProperties: false`).

## Tools

| Tool | Inputs | Notes |
|---|---|---|
| `list_disks` | none | read-only; works unelevated |
| `inspect_image` | `path`, `sha256?` | read-only; `sha256` reads the whole file |
| `doctor` | none | read-only diagnostics |
| `plan_write` | `image`, `disk`, `allow_large?`, `expect_sha256?` | dry run, writes nothing; applies all refusals; hashes the image; returns `allowed`, `write_enabled`, `identity`, `image_sha256`, `plan` and, only when allowed **and** the server has `--allow-write`, a `plan_id` with `expires_in_seconds` (300) |
| `verify_disk` | `image`, `disk` | read-only compare; needs Administrator; `verify_mismatch` with first offset on difference |
| `write_image` | `plan_id` | **destructive**; listed only with `--allow-write` |

Annotations: all tools except `write_image` have `readOnlyHint: true`; `write_image` has `destructiveHint: true`, `idempotentHint: false`. Pass absolute paths; relative ones resolve against the server's working directory. Over MCP, fixed disks (non-removable, non-USB/SD/MMC) are never writable and `--allow-fixed` cannot be requested.

Recommended agent workflow: `doctor` (if anything fails) -> `list_disks` -> show the user model/serial/size and ask which disk -> `inspect_image` -> `plan_write` (pass `expect_sha256` when the publisher gives one) -> show disk identity, warnings and refusals to the user -> `write_image` -> tell the user a dialog is waiting. `write_image` is verified by read-back automatically. Send `_meta.progressToken` to receive `notifications/progress`. Only one write runs at a time (`busy`); the write runs on a worker thread so `ping` stays responsive. `verify_disk` returns `busy` while a write runs, and at most 4 other tool calls run at once (further calls return `busy`; retry when one finishes).

## Write gating (four layers)

1. **Opt-in.** Without `--allow-write`, `write_image` is absent from `tools/list` and calling it by name returns `safety_refusal`; `plan_write` never issues a `plan_id`.
2. **Single-use plan.** `plan_id` is valid for 5 minutes, usable once (consumed even if the write is declined or fails), and bound to disk number, size, model, serial and the image SHA-256.
3. **Human dialog.** The server shows a native, topmost Yes/No Windows warning box (default button **No**; Enter or closing it declines) titled "an AI agent wants to ERASE a disk" with disk identity, image path, size and SHA-256. There is no flag, environment variable or tool argument to skip it, and no confirmation phrase an agent could compute. Declined means `not_confirmed` and nothing is written.
4. **Hard refusals and rechecks.** System, source and temp disks; fixed disks; disks over 2 GiB unless `allow_large`; oversize or misaligned images; the plan is recomputed after approval and its disk identity compared with the plan; the image is re-hashed on the same handle it is then written from and compared with the approved SHA-256 (`hash_mismatch` otherwise); and the identity is checked again after the volumes are locked. Requires Administrator.

## Setup

Print the exact snippet for your machine (it contains the path of the running executable; pass `--exe PATH` to override, `--allow-write` to include the flag). Nothing is written to your files.

### Claude Desktop
`rsdiskwrite mcp config --client claude-desktop`, then merge into `%APPDATA%\Claude\claude_desktop_config.json` and restart Claude Desktop:
```json
{
  "mcpServers": {
    "rsdiskwrite": {
      "command": "C:\\Tools\\rsdiskwrite.exe",
      "args": ["mcp", "serve"]
    }
  }
}
```

### Claude Code
`rsdiskwrite mcp config --client claude-code` prints (run once):
```powershell
claude mcp add --scope user rsdiskwrite -- "C:\Tools\rsdiskwrite.exe" mcp serve
```

### OpenAI Codex CLI
`rsdiskwrite mcp config --client codex`, add to `~/.codex/config.toml` and restart Codex:
```toml
[mcp_servers.rsdiskwrite]
command = "C:\\Tools\\rsdiskwrite.exe"
args = ["mcp", "serve"]
```

### OpenAI Agents SDK (Python)
`rsdiskwrite mcp config --client openai-agents`; use it as an async context manager and pass it in `Agent(mcp_servers=[rsdiskwrite])`:
```python
from agents.mcp import MCPServerStdio

rsdiskwrite = MCPServerStdio(
    params={"command": "C:\\Tools\\rsdiskwrite.exe", "args": ["mcp", "serve"]},
)
```

## Enabling writes (`--allow-write`)

Add `"--allow-write"` to the args (or `mcp config ... --allow-write`). Consequences:
- The client sees a destructive `write_image` tool and `plan_write` issues plan ids.
- The MCP process must be elevated to actually write, so launch the client (or the server) from an Administrator context; otherwise writes fail with `elevation_required`.
- Each write still needs your click on the dialog, on the machine the server runs on (so it cannot be approved remotely; headless sessions will simply decline).
- Gating applies to MCP only. A client that can also run shell commands can bypass it through the CLI; see [SAFETY.md](SAFETY.md) before enabling.

Leave `--allow-write` off unless you need agent-driven writes. Planning, inspecting and verifying work without it.
