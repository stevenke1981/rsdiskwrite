//! MCP tool definitions and dispatch.

use crate::disk::{self, Disk, Enumeration};
use crate::doctor::{self, CheckStatus, DoctorReport};
use crate::error::{Error, ErrorKind, Result};
use crate::hash::Digest;
use crate::image::{self, ImageInfo};
use crate::mcp::gate::{Confirmer, PlanStore, WriteConfirmPrompt};
use crate::ops::{self, Progress, VerifyReport, WritePlan, WriteReport, WriteRequest};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Tool names.
pub mod names {
    pub const LIST_DISKS: &str = "list_disks";
    pub const INSPECT_IMAGE: &str = "inspect_image";
    pub const DOCTOR: &str = "doctor";
    pub const PLAN_WRITE: &str = "plan_write";
    pub const VERIFY_DISK: &str = "verify_disk";
    /// Destructive; only listed/accepted with `--allow-write`.
    pub const WRITE_IMAGE: &str = "write_image";
}

/// MCP tool annotations (hints for clients).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    pub read_only_hint: bool,
    pub destructive_hint: bool,
    pub idempotent_hint: bool,
    pub open_world_hint: bool,
}

/// One entry of `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub annotations: ToolAnnotations,
}

/// A content block of a tool result (only text is used).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Content {
    /// Always `"text"`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

/// The `tools/call` result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<Content>,
    /// Same shape as the CLI's JSON `data` (or `{code,message,hint}` on error).
    pub structured_content: Value,
    pub is_error: bool,
}

impl CallToolResult {
    /// Success: `structuredContent = data` plus one text block.
    pub fn success(data: Value, text: impl Into<String>) -> Self {
        Self {
            content: vec![Content {
                kind: "text",
                text: text.into(),
            }],
            structured_content: data,
            is_error: false,
        }
    }

    /// Failure: `isError: true`, `structuredContent = {code,message,hint}`.
    pub fn failure(error: &Error) -> Self {
        let text = match &error.hint {
            Some(hint) => format!("{} ({}) Hint: {hint}", error.message, error.kind.code()),
            None => format!("{} ({})", error.message, error.kind.code()),
        };
        Self {
            content: vec![Content { kind: "text", text }],
            structured_content: serde_json::to_value(error).unwrap_or(Value::Null),
            is_error: true,
        }
    }
}

/// Everything a tool call may use.
pub struct ToolContext<'a> {
    /// Server started with `--allow-write`.
    pub allow_write: bool,
    pub plans: &'a PlanStore,
    pub confirmer: &'a dyn Confirmer,
    /// Present when the client supplied a `progressToken`.
    pub progress: Option<&'a dyn Fn(Progress)>,
}

/// Annotations of the read-only tools.
const READ_ONLY: ToolAnnotations = ToolAnnotations {
    read_only_hint: true,
    destructive_hint: false,
    idempotent_hint: true,
    open_world_hint: false,
};

/// Tool definitions for `tools/list`; `write_image` only when `allow_write`.
pub fn definitions(allow_write: bool) -> Vec<ToolDef> {
    let mut tools = vec![
        ToolDef {
            name: names::LIST_DISKS,
            title: "List physical disks",
            description: "List the physical disks (\\\\.\\PhysicalDriveN) attached to this Windows \
                machine with number, size in bytes, model, serial, bus, removable flag and whether \
                it holds the running Windows system. Read-only and safe; works without \
                Administrator rights. Use it to find the 'disk' number for plan_write and \
                verify_disk. Never guess a disk number: identify the target with the user by \
                model, serial and size.",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: READ_ONLY,
        },
        ToolDef {
            name: names::INSPECT_IMAGE,
            title: "Inspect a disk image",
            description: "Inspect a disk image file (raw .img or gzip .img.gz) without writing \
                anything: format, file size, decompressed size (the bytes a write would put on \
                the disk), gzip member count and trailing bytes. With sha256=true also computes \
                the SHA-256 of the file as stored and of the decompressed content (reads the whole \
                file; may take a while for large images). Read-only. Pass an absolute path; \
                relative paths resolve against the server's working directory.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path of the image file."
                    },
                    "sha256": {
                        "type": "boolean",
                        "default": false,
                        "description": "Also compute SHA-256 digests (reads the whole file)."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: READ_ONLY,
        },
        ToolDef {
            name: names::DOCTOR,
            title: "Diagnose the environment",
            description: "Run rsdiskwrite's environment diagnostics: version, OS, Administrator \
                elevation, system and temp disks, disk and volume enumeration, terminals and an \
                MCP self-test. Each check is ok/warn/fail with a hint. Read-only. Run it when \
                another tool fails unexpectedly (for example with elevation_required).",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: READ_ONLY,
        },
        ToolDef {
            name: names::PLAN_WRITE,
            title: "Plan (dry-run) writing an image to a disk",
            description: "Dry run of writing 'image' to PhysicalDrive'disk'. Writes nothing. \
                Applies every safety rule: refuses the Windows system disk, the disk holding the \
                image or the temp directory, fixed/internal disks (only removable media or \
                USB/SD/MMC disks are accepted over MCP), disks larger than 2 GiB unless \
                allow_large=true, and images larger than the disk or not sector-aligned. \
                Computes the image file's SHA-256 (reads the whole file) and reports the exact \
                disk identity (number, size, model, serial). When the write is allowed and the \
                server runs with --allow-write, returns a single-use plan_id valid for 5 minutes, \
                bound to that disk identity and image SHA-256, for write_image. Show the user the \
                disk identity, warnings and refusals before going further.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "image": {
                        "type": "string",
                        "description": "Absolute path of the image file (raw or gzip)."
                    },
                    "disk": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Target disk number N of \\\\.\\PhysicalDriveN, from list_disks."
                    },
                    "allow_large": {
                        "type": "boolean",
                        "default": false,
                        "description": "Permit a target larger than 2 GiB. Only set this when \
                            the user confirmed this specific large disk."
                    },
                    "expect_sha256": {
                        "type": "string",
                        "pattern": "^[0-9a-fA-F]{64}$",
                        "description": "Optional expected SHA-256 of the image file as stored \
                            (e.g. from the vendor's SHA256SUMS); planning fails with \
                            hash_mismatch if it differs."
                    }
                },
                "required": ["image", "disk"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: ToolAnnotations {
                idempotent_hint: false,
                ..READ_ONLY
            },
        },
        ToolDef {
            name: names::VERIFY_DISK,
            title: "Verify a disk against an image",
            description: "Read PhysicalDrive'disk' back and compare it byte for byte with 'image' \
                (gzip images are decompressed). Read-only; needs Administrator rights and reads \
                as many bytes as the image contains, which can take minutes. Fails with \
                verify_mismatch and the first differing offset if the disk differs.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "image": {
                        "type": "string",
                        "description": "Absolute path of the image file (raw or gzip)."
                    },
                    "disk": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Disk number N of \\\\.\\PhysicalDriveN."
                    }
                },
                "required": ["image", "disk"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: READ_ONLY,
        },
    ];
    if allow_write {
        tools.push(ToolDef {
            name: names::WRITE_IMAGE,
            title: "Write an image to a disk (DESTRUCTIVE)",
            description: "DESTRUCTIVE: overwrites an entire physical disk with the image of a plan \
                from plan_write; all existing data on that disk is destroyed. Only call this when \
                the user explicitly asked to write this image to this specific disk. \
                Preconditions: (1) 'plan_id' came from plan_write less than 5 minutes ago and \
                has not been used; every id works exactly once, even when the write is declined \
                or fails. (2) A human at this computer must click Yes in a native warning dialog \
                that shows the disk and image; there is no way to skip it, so tell the user to \
                watch for the dialog. If they click No or close it the result is not_confirmed \
                and nothing is written. (3) After confirmation the server re-checks the disk \
                identity and the image SHA-256, and again after locking the disk; any change \
                stops the write (identity_changed / hash_mismatch). Requires Administrator \
                rights. The disk is read back and verified after writing. Can take many \
                minutes; send _meta.progressToken to receive progress notifications. Only one \
                write runs at a time (busy otherwise).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "plan_id": {
                        "type": "string",
                        "description": "The plan_id returned by plan_write."
                    }
                },
                "required": ["plan_id"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: ToolAnnotations {
                read_only_hint: false,
                destructive_hint: true,
                idempotent_hint: false,
                open_world_hint: false,
            },
        });
    }
    tools
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectArgs {
    path: PathBuf,
    #[serde(default)]
    sha256: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanArgs {
    image: PathBuf,
    disk: u32,
    #[serde(default)]
    allow_large: bool,
    #[serde(default)]
    expect_sha256: Option<Digest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyArgs {
    image: PathBuf,
    disk: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArgs {
    plan_id: String,
}

/// Decodes `arguments` (`null` = `{}`); failures are usage errors (-32602).
fn parse_args<T: DeserializeOwned>(tool: &str, arguments: &Value) -> Result<T> {
    let value = if arguments.is_null() {
        json!({})
    } else {
        arguments.clone()
    };
    if !value.is_object() {
        return Err(Error::usage(format!(
            "Invalid arguments for {tool}: expected a JSON object"
        )));
    }
    serde_json::from_value(value).map_err(|e| {
        Error::usage(format!("Invalid arguments for {tool}: {e}"))
            .with_hint("See the tool's inputSchema in tools/list")
    })
}

/// Success result: `data` as structured content, text = `summary` + JSON of `data`.
fn success<T: Serialize>(data: &T, summary: impl Into<String>) -> Result<CallToolResult> {
    let data = serde_json::to_value(data)
        .map_err(|e| Error::internal(format!("cannot serialize tool result: {e}")))?;
    let json = serde_json::to_string_pretty(&data).unwrap_or_default();
    Ok(CallToolResult::success(
        data,
        format!("{}\n\n{json}", summary.into()),
    ))
}

/// The error returned for `write_image` when the server runs without `--allow-write`.
pub fn write_disabled_error() -> Error {
    Error::refusal(
        "write_image is disabled: this MCP server was started without --allow-write, so it \
         cannot write to any disk",
    )
    .with_hint(
        "Ask the user to restart the server as 'rsdiskwrite mcp serve --allow-write', or to \
         run 'rsdiskwrite write' themselves",
    )
}

/// Runs tool `name` with `arguments` (`null`/absent = `{}`). Unknown tools and
/// `write_image` without `allow_write` are errors. Tool-level failures are returned as
/// [`CallToolResult::failure`]; `Err` is reserved for invalid params (-32602).
pub fn call(name: &str, arguments: &Value, ctx: &ToolContext<'_>) -> Result<CallToolResult> {
    let outcome = match name {
        names::LIST_DISKS => {
            parse_args::<NoArgs>(name, arguments)?;
            list_disks()
        }
        names::INSPECT_IMAGE => inspect_image(parse_args(name, arguments)?),
        names::DOCTOR => {
            parse_args::<NoArgs>(name, arguments)?;
            run_doctor()
        }
        names::PLAN_WRITE => plan_write(parse_args(name, arguments)?, ctx),
        names::VERIFY_DISK => verify_disk(parse_args(name, arguments)?, ctx),
        names::WRITE_IMAGE => {
            if !ctx.allow_write {
                return Ok(CallToolResult::failure(&write_disabled_error()));
            }
            write_image(parse_args(name, arguments)?, ctx)
        }
        _ => {
            return Err(Error::usage(format!("Unknown tool '{name}'"))
                .with_hint("Call tools/list for the available tools"));
        }
    };
    Ok(outcome.unwrap_or_else(|error| CallToolResult::failure(&error)))
}

fn disk_line(disk: &Disk) -> String {
    let mut line = format!("- {} [{} bytes]", disk.summary(), disk.size);
    if disk.is_system {
        line.push_str(" (system disk: never writable)");
    } else if !disk.is_removable_target() {
        line.push_str(" (fixed disk: not writable over MCP)");
    }
    line
}

fn list_disks() -> Result<CallToolResult> {
    let enumeration: Enumeration = disk::enumerate()?;
    let mut text = format!("Found {} disk(s):", enumeration.disks.len());
    for disk in &enumeration.disks {
        text.push('\n');
        text.push_str(&disk_line(disk));
    }
    success(&enumeration, text)
}

fn inspect_image(args: InspectArgs) -> Result<CallToolResult> {
    let info: ImageInfo = image::inspect(&args.path, args.sha256)?;
    let text = format!(
        "{}: {:?} image, {} bytes as stored, {} bytes to write",
        info.path.display(),
        info.format,
        info.file_size,
        info.decompressed_size
    );
    success(&info, text)
}

fn run_doctor() -> Result<CallToolResult> {
    let report = DoctorReport::new(doctor::run());
    let mut text = format!("rsdiskwrite {} diagnostics:", report.version);
    for check in &report.checks {
        let status = match check.status {
            CheckStatus::Ok => "ok",
            CheckStatus::Warn => "WARN",
            CheckStatus::Fail => "FAIL",
        };
        text.push_str(&format!("\n- [{status}] {}: {}", check.id, check.detail));
        if let Some(hint) = &check.hint {
            text.push_str(&format!(" (hint: {hint})"));
        }
    }
    success(&report, text)
}

/// `plan_write` result.
#[derive(Serialize)]
struct PlanResult<'a> {
    /// Present only when the write is allowed and the server runs with `--allow-write`.
    plan_id: Option<String>,
    /// No refusals.
    allowed: bool,
    /// Server started with `--allow-write`.
    write_enabled: bool,
    expires_in_seconds: Option<u64>,
    image_sha256: Digest,
    identity: disk::DiskIdentity,
    plan: &'a WritePlan,
}

fn plan_write(args: PlanArgs, ctx: &ToolContext<'_>) -> Result<CallToolResult> {
    let mut request = WriteRequest::new(args.image, args.disk);
    request.allow_large = args.allow_large;
    // Fixed disks are never writable over MCP (hard refusal, PLAN.md layer 4).
    request.allow_fixed = false;
    request.verify = true;
    request.hash_image = true;
    request.expect_sha256 = args.expect_sha256;
    let plan = ops::plan(&request)?;
    let image_sha256 = plan
        .image
        .sha256
        .ok_or_else(|| Error::internal("planning did not compute the image SHA-256"))?;
    let allowed = plan.is_allowed();
    let plan_id = if allowed && ctx.allow_write {
        Some(ctx.plans.insert(plan.clone(), image_sha256)?.0)
    } else {
        None
    };
    let mut text = format!(
        "Plan: write {} ({} bytes, SHA-256 {image_sha256}) to {}.",
        plan.image.path.display(),
        plan.image.file_size,
        plan.disk.summary()
    );
    for warning in &plan.warnings {
        text.push_str(&format!("\nWarning: {warning}"));
    }
    if !allowed {
        text.push_str("\nREFUSED, this write is not allowed:");
        for refusal in &plan.refusals {
            text.push_str(&format!("\n- {}", refusal.message));
            if let Some(hint) = &refusal.hint {
                text.push_str(&format!(" (hint: {hint})"));
            }
        }
    } else if let Some(id) = &plan_id {
        text.push_str(&format!(
            "\nAllowed. plan_id {id} is valid once for {} seconds. Confirm the disk with the \
             user; write_image will ask a human to click Yes in a dialog.",
            ctx.plans.ttl().as_secs()
        ));
    } else {
        text.push_str(
            "\nAllowed, but this server was started without --allow-write, so no plan_id was \
             issued and write_image is unavailable.",
        );
    }
    let result = PlanResult {
        expires_in_seconds: plan_id.as_ref().map(|_| ctx.plans.ttl().as_secs()),
        plan_id,
        allowed,
        write_enabled: ctx.allow_write,
        image_sha256,
        identity: plan.disk.identity(),
        plan: &plan,
    };
    success(&result, text)
}

fn noop_progress(_: Progress) {}

fn verify_disk(args: VerifyArgs, ctx: &ToolContext<'_>) -> Result<CallToolResult> {
    let progress = ctx.progress.unwrap_or(&noop_progress);
    let report: VerifyReport = ops::verify(&args.image, args.disk, progress)?;
    if !report.matched {
        return Err(Error::new(
            ErrorKind::VerifyMismatch,
            format!(
                "PhysicalDrive{} differs from {} at byte {}",
                args.disk,
                args.image.display(),
                report.first_mismatch.unwrap_or(0)
            ),
        ));
    }
    let text = format!(
        "PhysicalDrive{} matches {} ({} bytes compared).",
        report.disk.number,
        report.image.display(),
        report.bytes
    );
    success(&report, text)
}

fn write_image(args: WriteArgs, ctx: &ToolContext<'_>) -> Result<CallToolResult> {
    // Layer 2: single-use, unexpired plan (removed from the store whatever happens next).
    let stored = ctx.plans.take(&args.plan_id)?;
    stored.plan.check()?;
    // Layer 3: a human must click Yes.
    let prompt = WriteConfirmPrompt {
        disk: stored.plan.disk.clone(),
        image: stored.plan.image.path.clone(),
        image_sha256: Some(stored.image_sha256),
        image_size: stored.plan.image.file_size,
    };
    if !ctx.confirmer.confirm(&prompt) {
        return Err(Error::new(
            ErrorKind::NotConfirmed,
            "The write was not confirmed in the dialog (No or closed); nothing was written",
        )
        .with_hint(
            "Ask the user whether they want to retry; a new plan_write is needed (plan ids are \
             single-use)",
        ));
    }
    // Layer 4: re-plan now, bound to the confirmed identity, so every refusal is evaluated
    // again. The image hash is not recomputed here: ops::execute hashes the very handle it
    // then writes from and compares it with the confirmed SHA-256 (so the file cannot be
    // swapped in between), and re-checks the disk identity once more after locking.
    let mut request = stored.plan.request.clone();
    request.image = stored.plan.image.path.clone();
    request.expected = Some(stored.identity());
    request.expect_sha256 = None;
    request.hash_image = false;
    request.allow_fixed = false;
    let mut fresh = ops::plan(&request)?;
    if fresh.disk.identity() != stored.identity() {
        return Err(Error::new(
            ErrorKind::IdentityChanged,
            format!("{} is no longer the planned disk", fresh.disk.name()),
        )
        .with_hint("Call list_disks and plan_write again"));
    }
    fresh.check()?;
    fresh.image.sha256 = Some(stored.image_sha256);
    let progress = ctx.progress.unwrap_or(&noop_progress);
    let report: WriteReport = ops::execute(&fresh, progress)?;
    if let Some(verify) = &report.verify
        && !verify.matched
    {
        return Err(Error::new(
            ErrorKind::VerifyMismatch,
            format!(
                "Read-back verification failed at byte {}",
                verify.first_mismatch.unwrap_or(0)
            ),
        ));
    }
    let verified = match &report.verify {
        Some(_) => "verified by read-back",
        None => "NOT verified",
    };
    let text = format!(
        "Wrote {} bytes from {} to {} in {} ms ({verified}).",
        report.bytes_written,
        report.image.display(),
        report.disk.summary(),
        report.elapsed_ms
    );
    success(&report, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::gate::tests::fake_plan;

    struct Never;
    impl Confirmer for Never {
        fn confirm(&self, _: &WriteConfirmPrompt) -> bool {
            panic!("confirmer must not be reached")
        }
    }

    #[test]
    fn write_tool_is_listed_only_with_allow_write() {
        let read_only = definitions(false);
        assert!(read_only.iter().all(|t| t.name != names::WRITE_IMAGE));
        assert!(read_only.iter().all(|t| t.annotations.read_only_hint
            && !t.annotations.destructive_hint
            && !t.annotations.open_world_hint));
        let all = definitions(true);
        assert_eq!(all.len(), read_only.len() + 1);
        let write = all.iter().find(|t| t.name == names::WRITE_IMAGE).unwrap();
        assert!(write.annotations.destructive_hint && !write.annotations.read_only_hint);
        assert!(!write.annotations.idempotent_hint);
        for tool in &all {
            assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
            assert_eq!(tool.input_schema["additionalProperties"], false);
            let value = serde_json::to_value(tool).unwrap();
            assert!(value["inputSchema"].is_object());
            assert!(value["annotations"]["readOnlyHint"].is_boolean());
            assert!(value.get("outputSchema").is_none());
        }
    }

    #[test]
    fn write_without_allow_write_and_unknown_tools_are_rejected() {
        let plans = PlanStore::default();
        let digest = Digest([2; 32]);
        let id = plans
            .insert(fake_plan("x.img".as_ref(), digest), digest)
            .unwrap();
        let ctx = ToolContext {
            allow_write: false,
            plans: &plans,
            confirmer: &Never,
            progress: None,
        };
        let result = call(names::WRITE_IMAGE, &json!({"plan_id": id.0}), &ctx).unwrap();
        assert!(result.is_error);
        assert_eq!(result.structured_content["code"], "safety_refusal");
        assert_eq!(plans.len(), 1, "the plan is not consumed");
        assert_eq!(
            call("format_c", &Value::Null, &ctx).unwrap_err().kind,
            ErrorKind::Usage
        );
        assert_eq!(
            call(names::LIST_DISKS, &json!({"bogus": 1}), &ctx)
                .unwrap_err()
                .kind,
            ErrorKind::Usage
        );
        assert_eq!(
            call(names::PLAN_WRITE, &json!({"image": "x.img"}), &ctx)
                .unwrap_err()
                .kind,
            ErrorKind::Usage
        );
    }
}
