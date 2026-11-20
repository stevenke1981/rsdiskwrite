//! End-to-end tests of `rsdiskwrite mcp serve` over stdio. They never write to a disk:
//! the only write_image calls use unknown plan ids or a server without --allow-write.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Runs `rsdiskwrite mcp serve [extra]`, sends `input` (raw lines), closes stdin and
/// returns every stdout line, each asserted to be a JSON-RPC 2.0 object.
fn session(extra: &[&str], input: &[String]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rsdiskwrite"))
        .args(["mcp", "serve"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start rsdiskwrite mcp serve");
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let lines: Vec<String> = BufReader::new(stdout).lines().map(Result::unwrap).collect();
        let _ = tx.send(lines);
    });
    {
        let mut stdin = child.stdin.take().unwrap();
        for line in input {
            stdin.write_all(line.as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    let lines = match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(lines) => lines,
        Err(_) => {
            let _ = child.kill();
            panic!("mcp serve did not exit after stdin closed");
        }
    };
    let status = child.wait().unwrap();
    assert!(status.success(), "mcp serve exit status {status}");
    lines
        .iter()
        .map(|line| {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"));
            assert_eq!(value["jsonrpc"], "2.0", "{line}");
            value
        })
        .collect()
}

fn request(id: i64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn initialize(id: i64) -> String {
    request(
        id,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {},
               "clientInfo": {"name": "mcp_stdio test", "version": "1"}}),
    )
}

fn initialized() -> String {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string()
}

fn response(lines: &[Value], id: i64) -> &Value {
    lines
        .iter()
        .find(|line| line["id"] == id)
        .unwrap_or_else(|| panic!("no response with id {id}: {lines:?}"))
}

fn tool_names(lines: &[Value], id: i64) -> Vec<String> {
    response(lines, id)["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn initialize_ping_and_read_only_tool_list() {
    let lines = session(
        &[],
        &[
            initialize(1),
            initialized(),
            request(2, "ping", json!({})),
            request(3, "tools/list", json!({})),
        ],
    );
    assert_eq!(lines.len(), 3, "{lines:?}");
    let init = &response(&lines, 1)["result"];
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["capabilities"]["tools"]["listChanged"], false);
    assert_eq!(init["serverInfo"]["name"], "rsdiskwrite");
    assert_eq!(init["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        init["instructions"]
            .as_str()
            .unwrap()
            .contains("plan_write")
    );
    assert_eq!(response(&lines, 2)["result"], json!({}));
    let names = tool_names(&lines, 3);
    for expected in [
        "list_disks",
        "inspect_image",
        "doctor",
        "plan_write",
        "verify_disk",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} in {names:?}"
        );
    }
    assert!(!names.iter().any(|n| n == "write_image"), "{names:?}");
    for tool in response(&lines, 3)["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert!(!tool["description"].as_str().unwrap().is_empty());
    }
}

#[test]
fn unsupported_protocol_version_gets_the_latest() {
    let lines = session(
        &[],
        &[request(
            1,
            "initialize",
            json!({"protocolVersion": "1999-01-01", "capabilities": {}}),
        )],
    );
    assert_eq!(
        response(&lines, 1)["result"]["protocolVersion"],
        "2025-06-18"
    );
}

#[test]
fn allow_write_lists_a_destructive_write_tool() {
    let lines = session(
        &["--allow-write"],
        &[
            initialize(1),
            initialized(),
            request(2, "tools/list", json!({})),
            request(
                3,
                "tools/call",
                json!({"name": "write_image", "arguments": {"plan_id": "plan_00000000000000000000000000000000"}}),
            ),
        ],
    );
    let tools = response(&lines, 2)["result"]["tools"].as_array().unwrap();
    let write = tools
        .iter()
        .find(|t| t["name"] == "write_image")
        .expect("write_image listed");
    assert_eq!(write["annotations"]["destructiveHint"], true);
    assert_eq!(write["annotations"]["readOnlyHint"], false);
    assert_eq!(write["annotations"]["idempotentHint"], false);
    assert_eq!(write["inputSchema"]["required"], json!(["plan_id"]));
    let result = &response(&lines, 3)["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "plan_invalid");
}

#[test]
fn write_image_by_name_without_allow_write_fails() {
    let lines = session(
        &[],
        &[
            initialize(1),
            request(
                2,
                "tools/call",
                json!({"name": "write_image", "arguments": {"plan_id": "anything"}}),
            ),
        ],
    );
    let result = &response(&lines, 2)["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "safety_refusal");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("--allow-write")
    );
}

#[test]
fn protocol_errors() {
    let lines = session(
        &[],
        &[
            "{not json".to_owned(),
            request(1, "no/such/method", json!({})),
            json!({"jsonrpc": "1.0", "id": 2, "method": "ping"}).to_string(),
            request(
                3,
                "tools/call",
                json!({"name": "no_such_tool", "arguments": {}}),
            ),
            request(
                4,
                "tools/call",
                json!({"name": "inspect_image", "arguments": {"bogus": true}}),
            ),
        ],
    );
    assert_eq!(lines.len(), 5, "{lines:?}");
    let parse = lines
        .iter()
        .find(|l| l["id"].is_null())
        .expect("parse error response with null id");
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(response(&lines, 1)["error"]["code"], -32601);
    assert_eq!(response(&lines, 2)["error"]["code"], -32600);
    assert_eq!(response(&lines, 3)["error"]["code"], -32602);
    assert_eq!(response(&lines, 4)["error"]["code"], -32602);
}

#[test]
fn notifications_are_never_answered() {
    let lines = session(
        &[],
        &[
            initialized(),
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 9}})
                .to_string(),
            json!({"jsonrpc": "2.0", "method": "no/such/notification"}).to_string(),
            request(1, "ping", json!({})),
        ],
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["id"], 1);
}

#[test]
fn read_only_tools_return_structured_content() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("x.img");
    std::fs::write(&image, vec![0u8; 1024]).unwrap();
    let lines = session(
        &[],
        &[
            initialize(1),
            request(
                2,
                "tools/call",
                json!({"name": "inspect_image", "arguments": {"path": image}}),
            ),
            request(3, "tools/call", json!({"name": "list_disks"})),
        ],
    );
    for id in [2, 3] {
        let result = &response(&lines, id)["result"];
        assert_eq!(result["content"][0]["type"], "text", "{result}");
        assert!(result["structuredContent"].is_object(), "{result}");
        assert!(result["isError"].is_boolean());
    }
    // image::inspect may still be a stub in this build ("unsupported"); when it works the
    // structured content is the ImageInfo.
    let inspect = &response(&lines, 2)["result"];
    if inspect["isError"] == false {
        assert_eq!(inspect["structuredContent"]["file_size"], 1024);
    } else {
        assert!(
            inspect["structuredContent"]["code"].is_string(),
            "{inspect}"
        );
    }
}
