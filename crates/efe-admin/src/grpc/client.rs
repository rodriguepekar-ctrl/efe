//! REST-gateway-backed classify client used by the smoke test.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::auth::TokenProvider;
use crate::error::AdminError;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct GrpcClient {
    base_url: String,
    http: reqwest::Client,
    auth: TokenProvider,
}

#[derive(Debug, Serialize)]
pub struct ClassifyRequest<'a> {
    pub tenant: &'a str,
    pub catalog: &'a str,
    pub sentence: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct ClassifyResponse {
    pub label: Option<String>,
    pub confidence: Option<f32>,
    pub mode: Option<String>,
    #[serde(default)]
    pub anchors_used: Vec<String>,
}

impl GrpcClient {
    pub fn new(base_url: String, auth: TokenProvider) -> Result<Self, AdminError> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("efe-admin-grpc/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AdminError::Network(e.to_string()))?;
        Ok(Self { base_url: base_url.trim_end_matches('/').to_owned(), http, auth })
    }

    pub async fn classify(&self, req: ClassifyRequest<'_>) -> Result<ClassifyResponse, AdminError> {
        let url = format!("{}/v1/classify", self.base_url);
        let mut http_req = self.http.post(&url).json(&req);
        if let Some(tok) = self.auth.bearer()? {
            http_req = http_req.bearer_auth(tok);
        }
        let resp = http_req.send().await.map_err(|e| AdminError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AdminError::backend(
                format!("http_{}", status.as_u16()),
                format!("classify failed: {body}"),
            ));
        }
        let parsed: ClassifyResponse =
            resp.json().await.map_err(|e| AdminError::Network(format!("parse: {e}")))?;
        Ok(parsed)
    }
}
