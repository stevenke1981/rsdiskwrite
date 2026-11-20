//! MCP tool definitions and dispatch.

use crate::error::{Error, Result};
use crate::mcp::gate::{Confirmer, PlanStore};
use crate::ops::Progress;
use serde::Serialize;
use serde_json::Value;

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

/// Tool definitions for `tools/list`; `write_image` only when `allow_write`.
pub fn definitions(allow_write: bool) -> Vec<ToolDef> {
    // TODO(phase2-B): define all tools with input/output schemas and annotations.
    let _ = allow_write;
    Vec::new()
}

/// Runs tool `name` with `arguments` (`null`/absent = `{}`). Unknown tools and
/// `write_image` without `allow_write` are errors. Tool-level failures are returned as
/// [`CallToolResult::failure`]; `Err` is reserved for invalid params (-32602).
pub fn call(name: &str, arguments: &Value, ctx: &ToolContext<'_>) -> Result<CallToolResult> {
    // TODO(phase2-B): dispatch to list_disks, inspect_image, doctor, plan_write,
    // verify_disk, write_image.
    let _ = (arguments, ctx);
    Ok(CallToolResult::failure(&Error::unsupported(format!(
        "MCP tool '{name}'"
    ))))
}
