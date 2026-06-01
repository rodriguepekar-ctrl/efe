//! HTTP/JSON client to the mcp-server (over its standard `tools/call`
//! endpoint, which the mcp-server exposes via stdio in MCP mode and
//! via HTTP gateway when running in « http » transport).
//!
//! Exposes a single `call_tool(name, args)` method returning a JSON
//! Value. Error mapping respects `ToolError.code` from the Python side.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::TokenProvider;
use crate::error::AdminError;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct McpClient {
    base_url: String,
    http: reqwest::Client,
    auth: TokenProvider,
}

#[derive(Debug, Serialize)]
struct ToolCallRequest<'a> {
    name: &'a str,
    arguments: &'a Value,
}

#[derive(Debug, Deserialize)]
struct ToolCallResponse {
    /// Successful payload returned by the tool handler.
    #[serde(default)]
    result: Option<Value>,
    /// Optional error envelope (mcp-server returns either result OR error).
    #[serde(default)]
    error: Option<ToolErrorEnvelope>,
}

#[derive(Debug, Deserialize)]
struct ToolErrorEnvelope {
    code: String,
    message: String,
}

impl McpClient {
    pub fn new(base_url: String, auth: TokenProvider) -> Result<Self, AdminError> {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("efe-admin/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AdminError::Network(e.to_string()))?;
        Ok(Self { base_url: base_url.trim_end_matches('/').to_owned(), http, auth })
    }

    /// Invoke an MCP tool with a JSON value as args. Returns the
    /// successful payload as `serde_json::Value`. Maps backend-side
    /// `ToolError` to `AdminError::Backend` preserving the code.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, AdminError> {
        let body = ToolCallRequest { name, arguments: &args };
        let url = format!("{}/mcp/tools/call", self.base_url);

        let mut req = self.http.post(&url).json(&body);
        if let Some(tok) = self.auth.bearer()? {
            req = req.bearer_auth(tok);
        }

        let resp = req.send().await.map_err(|e| AdminError::Network(e.to_string()))?;
        let status = resp.status();
        let envelope: ToolCallResponse = resp
            .json()
            .await
            .map_err(|e| AdminError::Network(format!("parse response: {e}")))?;

        if let Some(err) = envelope.error {
            return Err(AdminError::Backend {
                code: err.code,
                message: err.message,
            });
        }

        if !status.is_success() {
            return Err(AdminError::Backend {
                code: format!("http_{}", status.as_u16()),
                message: format!("non-200 from mcp-server: {status}"),
            });
        }

        envelope
            .result
            .ok_or_else(|| AdminError::Internal("mcp-server returned neither result nor error".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, ServerGuard};

    async fn server() -> ServerGuard {
        mockito::Server::new_async().await
    }

    #[tokio::test]
    async fn happy_path_returns_result() {
        let mut s = server().await;
        let m = s
            .mock("POST", "/mcp/tools/call")
            .match_header("authorization", Matcher::Regex("Bearer .*".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result": {"tenant_id": "acme"}}"#)
            .create_async()
            .await;

        let client = McpClient::new(s.url(), TokenProvider::from_static("tok")).unwrap();
        let res = client
            .call_tool("efe_create_tenant", serde_json::json!({"name": "acme"}))
            .await
            .unwrap();
        assert_eq!(res["tenant_id"], "acme");
        m.assert_async().await;
    }

    #[tokio::test]
    async fn error_envelope_maps_to_backend_err() {
        let mut s = server().await;
        let m = s
            .mock("POST", "/mcp/tools/call")
            .with_status(200)
            .with_body(
                r#"{"error": {"code": "conflict", "message": "tenant 'acme' already exists"}}"#,
            )
            .create_async()
            .await;
        let client = McpClient::new(s.url(), TokenProvider::none()).unwrap();
        let err = client
            .call_tool("efe_create_tenant", serde_json::json!({}))
            .await
            .unwrap_err();
        match err {
            AdminError::Backend { code, message } => {
                assert_eq!(code, "conflict");
                assert!(message.contains("already exists"));
            }
            other => panic!("expected Backend, got {other:?}"),
        }
        m.assert_async().await;
    }

    #[tokio::test]
    async fn http_500_without_envelope_is_backend_err() {
        let mut s = server().await;
        s.mock("POST", "/mcp/tools/call")
            .with_status(500)
            .with_body(r#"{}"#)
            .create_async()
            .await;
        let client = McpClient::new(s.url(), TokenProvider::none()).unwrap();
        let err = client
            .call_tool("efe_create_tenant", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, AdminError::Internal(_) | AdminError::Backend { .. }));
    }
}
