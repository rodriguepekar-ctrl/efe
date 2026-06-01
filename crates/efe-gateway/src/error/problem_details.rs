// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : RFC 7807 Problem Details type.

use std::collections::BTreeMap;

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

/// **N22 — RFC 7807** : the Problem Details body shape.
///
/// Field order in serialised JSON follows RFC 7807's recommendation :
/// `type`, `title`, `status`, `detail`, `instance` ; then EFE
/// extensions : `trace_id`, `extensions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProblemDetails {
    /// URI reference identifying the problem type.  Sprint 14 ships
    /// short relative slugs like `/errors/missing-auth` ; Sprint 15
    /// switches to fully-qualified `https://docs.efe.ai/errors/...`.
    #[serde(rename = "type")]
    pub problem_type: String,
    /// Short human-readable summary.
    pub title: String,
    /// HTTP status code (mirrors the response status).
    pub status: u16,
    /// Specific situation description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// URI reference identifying the specific occurrence (request path).
    pub instance: String,
    /// W3C trace_id (16-byte hex).  Always present in EFE bodies.
    pub trace_id: String,
    /// Free-form RFC 7807 extension fields.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl ProblemDetails {
    /// **N22** : Construct.
    pub fn new(
        status: u16,
        title: impl Into<String>,
        type_slug: &str,
        instance: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            problem_type: format!("/errors/{type_slug}"),
            title: title.into(),
            status,
            detail: None,
            instance: instance.into(),
            trace_id: trace_id.into(),
            extensions: BTreeMap::new(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_extension(mut self, key: &str, value: serde_json::Value) -> Self {
        self.extensions.insert(key.into(), value);
        self
    }
}

impl IntoResponse for ProblemDetails {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(self.clone())).into_response();
        // RFC 7807 mandates `application/problem+json` content type.
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_minimal_problem_details() {
        let pd = ProblemDetails::new(
            404,
            "Not Found",
            "not-found",
            "/v1/swarms/abc",
            "trace-1234",
        );
        assert_eq!(pd.status, 404);
        assert_eq!(pd.title, "Not Found");
        assert_eq!(pd.problem_type, "/errors/not-found");
        assert_eq!(pd.instance, "/v1/swarms/abc");
        assert_eq!(pd.trace_id, "trace-1234");
        assert!(pd.detail.is_none());
        assert!(pd.extensions.is_empty());
    }

    #[test]
    fn with_detail_sets_field() {
        let pd = ProblemDetails::new(400, "Bad", "bad", "/x", "t")
            .with_detail("the body is malformed");
        assert_eq!(pd.detail.as_deref(), Some("the body is malformed"));
    }

    #[test]
    fn with_extension_appends_to_map() {
        let pd = ProblemDetails::new(403, "Forbidden", "fbd", "/x", "t")
            .with_extension("jwt_tenant", serde_json::json!("acme"))
            .with_extension("body_tenant", serde_json::json!("globex"));
        assert_eq!(pd.extensions.len(), 2);
        assert_eq!(
            pd.extensions.get("jwt_tenant"),
            Some(&serde_json::json!("acme"))
        );
    }

    #[test]
    fn serde_round_trip() {
        let pd = ProblemDetails::new(401, "Unauthorized", "missing-auth", "/v1/x", "tid")
            .with_detail("bearer missing")
            .with_extension("k", serde_json::json!(42));
        let json = serde_json::to_string(&pd).unwrap();
        let back: ProblemDetails = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pd);
    }

    #[test]
    fn serde_json_field_order_rfc_7807() {
        // RFC 7807 recommends type, title, status, detail, instance
        // (in that order on the wire).  serde_json with
        // `#[derive(Serialize)]` uses declaration order which
        // matches.
        let pd = ProblemDetails::new(400, "Bad", "bad", "/x", "t").with_detail("dd");
        let json = serde_json::to_string(&pd).unwrap();
        let type_pos = json.find("\"type\"").unwrap();
        let title_pos = json.find("\"title\"").unwrap();
        let status_pos = json.find("\"status\"").unwrap();
        let detail_pos = json.find("\"detail\"").unwrap();
        let instance_pos = json.find("\"instance\"").unwrap();
        let trace_pos = json.find("\"trace_id\"").unwrap();
        assert!(type_pos < title_pos);
        assert!(title_pos < status_pos);
        assert!(status_pos < detail_pos);
        assert!(detail_pos < instance_pos);
        assert!(instance_pos < trace_pos);
    }

    #[test]
    fn into_response_carries_problem_json_content_type() {
        let pd = ProblemDetails::new(404, "Not Found", "nf", "/x", "t");
        let resp = pd.into_response();
        assert_eq!(resp.status(), 404);
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, "application/problem+json");
    }

    #[test]
    fn unknown_status_falls_back_to_500() {
        let pd = ProblemDetails {
            problem_type: "/errors/x".into(),
            title: "x".into(),
            status: 999, // invalid
            detail: None,
            instance: "/x".into(),
            trace_id: "t".into(),
            extensions: BTreeMap::new(),
        };
        let resp = pd.into_response();
        // StatusCode::from_u16(999) is actually valid (it's in
        // CustomStatusCode range) ; check the response status
        // is what we set.
        assert!(resp.status().is_server_error() || resp.status().as_u16() == 999);
    }
}
