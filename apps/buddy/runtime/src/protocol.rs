use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const JSON_RPC_VERSION: &str = "2.0";
pub const RUNTIME_PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcRequest {
    jsonrpc: String,
    id: String,
    method: String,
    params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
enum RpcResponse {
    Success {
        jsonrpc: &'static str,
        id: String,
        result: Value,
    },
    Error {
        jsonrpc: &'static str,
        id: String,
        error: RpcErrorBody,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct RpcErrorBody {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcOutput {
    response: RpcResponse,
    shutdown: bool,
}

impl RpcOutput {
    pub fn response(id: impl Into<String>, result: Value) -> Self {
        Self {
            response: RpcResponse::Success {
                jsonrpc: JSON_RPC_VERSION,
                id: id.into(),
                result,
            },
            shutdown: false,
        }
    }

    pub fn shutdown(id: impl Into<String>) -> Self {
        Self {
            response: RpcResponse::Success {
                jsonrpc: JSON_RPC_VERSION,
                id: id.into(),
                result: json!({ "accepted": true }),
            },
            shutdown: true,
        }
    }

    pub fn error(id: impl Into<String>, code: i32, message: impl Into<String>) -> Self {
        Self::error_with_data(id, code, message, None)
    }

    pub fn error_with_data(
        id: impl Into<String>,
        code: i32,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Self {
        Self {
            response: RpcResponse::Error {
                jsonrpc: JSON_RPC_VERSION,
                id: id.into(),
                error: RpcErrorBody {
                    code,
                    message: message.into(),
                    data,
                },
            },
            shutdown: false,
        }
    }

    pub fn should_shutdown(&self) -> bool {
        self.shutdown
    }

    pub fn response_value(&self) -> Value {
        serde_json::to_value(&self.response).expect("RPC response must be serializable")
    }

    pub fn into_response_value(self) -> Value {
        serde_json::to_value(self.response).expect("RPC response must be serializable")
    }
}

impl RpcRequest {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_control_request(&self) -> bool {
        matches!(
            self.method.as_str(),
            "runtime.status"
                | "runtime.localState"
                | "runtime.shutdown"
                | "claude.status"
                | "chat.cancel"
                | "approvals.approveCodex"
                | "approvals.deny"
        )
    }

    pub fn into_parts(self) -> (String, String, Value) {
        (self.id, self.method, self.params)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct RpcParseError {
    code: i32,
    message: &'static str,
}

impl RpcParseError {
    pub fn code(&self) -> i32 {
        self.code
    }

    pub fn response_value(&self) -> Value {
        json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": Value::Null,
            "error": {
                "code": self.code,
                "message": self.message,
            },
        })
    }
}

pub fn parse_request(line: &str) -> Result<RpcRequest, RpcParseError> {
    let value = serde_json::from_str::<Value>(line).map_err(|_| RpcParseError {
        code: -32700,
        message: "Parse error",
    })?;
    let request = serde_json::from_value::<RpcRequest>(value).map_err(|_| RpcParseError {
        code: -32600,
        message: "Invalid Request",
    })?;

    if request.jsonrpc != JSON_RPC_VERSION || !request.params.is_object() {
        return Err(RpcParseError {
            code: -32600,
            message: "Invalid Request",
        });
    }

    Ok(request)
}

pub fn dispatch_request(request: RpcRequest) -> RpcOutput {
    match request.method.as_str() {
        "runtime.status" => RpcOutput::response(
            request.id,
            json!({
                "name": "lexora-buddy-runtime",
                "protocolVersion": RUNTIME_PROTOCOL_VERSION,
                "ready": true,
            }),
        ),
        "runtime.shutdown" => RpcOutput::shutdown(request.id),
        _ => RpcOutput::error(request.id, -32601, "Method not found"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{dispatch_request, parse_request, RpcOutput};

    #[test]
    fn runtime_status_returns_protocol_identity() {
        let request = parse_request(
            r#"{"jsonrpc":"2.0","id":"request-1","method":"runtime.status","params":{}}"#,
        )
        .expect("parse request");

        assert_eq!(
            dispatch_request(request),
            RpcOutput::response(
                "request-1",
                json!({
                    "name": "lexora-buddy-runtime",
                    "protocolVersion": 2,
                    "ready": true,
                }),
            )
        );
    }

    #[test]
    fn runtime_shutdown_requests_clean_process_exit() {
        let request = parse_request(
            r#"{"jsonrpc":"2.0","id":"request-2","method":"runtime.shutdown","params":{}}"#,
        )
        .expect("parse request");

        let output = dispatch_request(request);

        assert!(output.should_shutdown());
        assert_eq!(
            output.response_value(),
            json!({
                "jsonrpc": "2.0",
                "id": "request-2",
                "result": { "accepted": true },
            })
        );
    }

    #[test]
    fn rejects_requests_with_unknown_fields() {
        let error = parse_request(
            r#"{"jsonrpc":"2.0","id":"request-3","method":"runtime.status","params":{},"token":"secret"}"#,
        )
        .expect_err("reject unknown field");

        assert_eq!(error.code(), -32600);
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let request = parse_request(
            r#"{"jsonrpc":"2.0","id":"request-4","method":"runtime.missing","params":{}}"#,
        )
        .expect("parse request");

        assert_eq!(
            dispatch_request(request).response_value(),
            json!({
                "jsonrpc": "2.0",
                "id": "request-4",
                "error": {
                    "code": -32601,
                    "message": "Method not found",
                },
            })
        );
    }

    #[test]
    fn usage_snapshot_is_not_a_control_request() {
        let request = parse_request(
            r#"{"jsonrpc":"2.0","id":"request-5","method":"usage.snapshot","params":{}}"#,
        )
        .expect("parse request");

        assert!(!request.is_control_request());
    }
}
