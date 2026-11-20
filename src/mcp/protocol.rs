//! JSON-RPC 2.0 message types (one JSON object per line).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `jsonrpc` member value.
pub const JSONRPC_VERSION: &str = "2.0";

/// Standard JSON-RPC error codes.
pub mod codes {
    /// Invalid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// Not a valid request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// Unknown method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid method parameters.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal error.
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// A request id (number or string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

/// A request (has an `id`; must be answered with a [`Response`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A notification (no `id`; never answered).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Notification {
    /// A notification with `jsonrpc: "2.0"`.
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// An error with no `data`.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// A response: exactly one of `result` / `error` is present. `id` is `null` only when the
/// request id could not be determined (parse errors).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A success response.
    pub fn success(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    pub fn error(id: Option<RequestId>, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A parsed incoming line.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request(Request),
    Notification(Notification),
    /// A response to a request we sent (ignored by this server).
    Response(Response),
}

/// Parses one line. On failure returns the error [`Response`] to send back
/// (`PARSE_ERROR` for invalid JSON, `INVALID_REQUEST` for anything else malformed, with
/// the request id when one could be read).
pub fn parse_line(line: &str) -> Result<Incoming, Box<Response>> {
    let value: Value = serde_json::from_str(line).map_err(|e| {
        Box::new(Response::error(
            None,
            RpcError::new(codes::PARSE_ERROR, e.to_string()),
        ))
    })?;
    let id = value
        .get("id")
        .and_then(|id| serde_json::from_value::<RequestId>(id.clone()).ok());
    let invalid = |message: String| {
        Box::new(Response::error(
            id.clone(),
            RpcError::new(codes::INVALID_REQUEST, message),
        ))
    };
    if value.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Err(invalid("jsonrpc must be \"2.0\"".into()));
    }
    let has_method = value.get("method").is_some();
    let has_id = value.get("id").is_some_and(|id| !id.is_null());
    match (has_method, has_id) {
        (true, true) => serde_json::from_value(value)
            .map(Incoming::Request)
            .map_err(|e| invalid(e.to_string())),
        (true, false) => serde_json::from_value(value)
            .map(Incoming::Notification)
            .map_err(|e| invalid(e.to_string())),
        (false, _) if value.get("result").is_some() || value.get("error").is_some() => {
            serde_json::from_value(value)
                .map(Incoming::Response)
                .map_err(|e| invalid(e.to_string()))
        }
        _ => Err(invalid("missing method".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_requests_and_notifications() {
        match parse_line(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap() {
            Incoming::Request(r) => {
                assert_eq!(r.id, RequestId::Number(1));
                assert_eq!(r.method, "ping");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap(),
            Incoming::Notification(_)
        ));
        assert!(matches!(
            parse_line(r#"{"jsonrpc":"2.0","id":"a","result":{}}"#).unwrap(),
            Incoming::Response(_)
        ));
    }

    #[test]
    fn malformed_lines_produce_error_responses() {
        let parse = parse_line("{not json").unwrap_err();
        assert_eq!(parse.error.unwrap().code, codes::PARSE_ERROR);
        assert_eq!(parse.id, None);
        let invalid = parse_line(r#"{"jsonrpc":"1.0","id":7,"method":"x"}"#).unwrap_err();
        assert_eq!(invalid.error.unwrap().code, codes::INVALID_REQUEST);
        assert_eq!(invalid.id, Some(RequestId::Number(7)));
    }

    #[test]
    fn responses_serialize_without_absent_members() {
        let ok = Response::success(RequestId::String("x".into()), json!({}));
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"jsonrpc": "2.0", "id": "x", "result": {}})
        );
        let err = Response::error(None, RpcError::new(codes::METHOD_NOT_FOUND, "nope"));
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32601, "message": "nope"}})
        );
    }
}
