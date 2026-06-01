//! Sprint **D13** — strongly-typed structured event emitters for the
//! 5 log streams documented in
//! `docs/bench/d13_observability_dashboards_architecture.md` § 6.
//!
//! Each emitter produces a `serde_json::Value` that conforms to the
//! corresponding JSON-Schema in
//! `deploy/observability/log-schemas/<stream>.json`. The emitters are
//! the **single point of truth** for the wire contract — handler code
//! that needs to log a structured event must go through one of these
//! functions, never construct the JSON manually.
//!
//! # Streams
//!
//! | Stream     | Builder              | Use case |
//! |------------|----------------------|----------|
//! | request    | [`RequestEvent`]     | one event per gRPC RPC |
//! | audit      | [`AuditEvent`]       | catalog/cone mutation |
//! | classify   | [`ClassifyEvent`]    | one event per Classify |
//! | evolution  | [`EvolutionEvent`]   | one event per EvolveCatalog |
//! | security   | [`SecurityEvent`]    | auth / authz check |
//!
//! # Common envelope
//!
//! All events share the [`CommonEnvelope`] fields. Handler code
//! constructs the envelope once per request (typically in an
//! interceptor) and threads it through.
//!
//! # Schema version
//!
//! Pinned at v1.0 in this release. A MAJOR bump is a wire-breaking
//! change ; a MINOR bump is additive. The
//! `scripts/tests/test_d13_log_schemas.py` test enforces the schema.

#![allow(missing_docs)]
// (this is internal helper code ; the public surface is documented at
// the function level above each builder)

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

// =====================================================================
// Common envelope
// =====================================================================

/// Fields shared by every event in the 5 D13 streams.
///
/// `timestamp` is **already RFC 3339 formatted** (UTC, ending in
/// `Z`). The builder accepts a pre-formatted string to avoid pulling
/// a date-time crate into this module ; callers that have chrono or
/// time available format it themselves, callers that don't can use
/// [`CommonEnvelope::now_rfc3339`] which uses `std::time::SystemTime`.
///
/// Construct once at request entry (e.g. inside the gRPC interceptor)
/// and reuse for every event emitted during the same request.
#[derive(Debug, Clone)]
pub struct CommonEnvelope {
    pub timestamp:       String,
    pub level:           Level,
    pub request_id:      Option<String>,
    pub trace_id:        Option<String>,
    pub span_id:         Option<String>,
    pub tenant_id:       Option<String>,
    pub principal:       Option<String>,
    pub service_version: String,
}

impl CommonEnvelope {
    /// Build with the bare minimum — caller fills in the rest later.
    /// Uses [`format_rfc3339_now`] for the timestamp.
    pub fn now(service_version: impl Into<String>) -> Self {
        Self {
            timestamp:       format_rfc3339_now(),
            level:           Level::Info,
            request_id:      None,
            trace_id:        None,
            span_id:         None,
            tenant_id:       None,
            principal:       None,
            service_version: service_version.into(),
        }
    }

    /// Render the common fields into a JSON Value for merging with
    /// per-stream fields.
    fn into_base(self, stream: &str) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::with_capacity(12);
        map.insert("schema_version".into(),  json!("1.0"));
        map.insert("stream".into(),          json!(stream));
        map.insert("timestamp".into(),       json!(self.timestamp));
        map.insert("level".into(),           json!(self.level.as_str()));
        map.insert("service".into(),         json!("efe-server"));
        map.insert("service_version".into(), json!(self.service_version));
        if let Some(v) = self.request_id { map.insert("request_id".into(), json!(v)); }
        if let Some(v) = self.trace_id   { map.insert("trace_id".into(),   json!(v)); }
        if let Some(v) = self.span_id    { map.insert("span_id".into(),    json!(v)); }
        if let Some(v) = self.tenant_id  { map.insert("tenant_id".into(),  json!(v)); }
        if let Some(v) = self.principal  { map.insert("principal".into(),  json!(v)); }
        map
    }
}

// =====================================================================
// Tiny RFC 3339 formatter (avoids chrono / time deps)
// =====================================================================

/// Format the current UTC time as RFC 3339 with nanosecond precision.
/// Returns a string of the form `2026-04-27T14:30:00.123456789Z`.
pub fn format_rfc3339_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_rfc3339_from_unix(dur.as_secs(), dur.subsec_nanos())
}

/// Format a `(unix_secs, subsec_nanos)` pair as RFC 3339 UTC.
///
/// Implementation : pure-std civil-day computation à la
/// Howard Hinnant. Range : year ∈ [1970, 9999]. Beyond 9999 the
/// formatter clamps the year to `9999` and emits a Z timestamp ;
/// callers that care are encouraged to use `chrono` or `time`.
pub fn format_rfc3339_from_unix(unix_secs: u64, subsec_nanos: u32) -> String {
    // Days since 1970-01-01 (civil day index, 0 = epoch).
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = (unix_secs % 86_400) as u32;
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;

    // Howard Hinnant's algorithm : days_from_civil ↔ civil_from_days.
    // Adapted from https://howardhinnant.github.io/date_algorithms.html
    // to avoid pulling a date-time crate.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    if m <= 2 { y += 1; }
    let y_clamped = y.clamp(1970, 9999) as u32;

    format!(
        "{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{ns:09}Z",
        y = y_clamped, m = m, d = d, hh = hh, mm = mm, ss = ss,
        ns = subsec_nanos,
    )
}

#[derive(Debug, Clone, Copy)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info  => "INFO",
            Self::Warn  => "WARN",
            Self::Error => "ERROR",
        }
    }
}

// =====================================================================
// Stream 1 — request
// =====================================================================

/// One event per gRPC request, emitted by the request_log interceptor.
#[derive(Debug, Clone)]
pub struct RequestEvent {
    pub envelope:           CommonEnvelope,
    pub rpc:                String,
    pub duration_ms:        u64,
    pub status_code:        String,
    pub client_ip:          Option<String>,
    pub user_agent:         Option<String>,
    pub request_size_bytes: Option<u64>,
    pub response_size_bytes: Option<u64>,
    pub error_message:      Option<String>,
}

impl RequestEvent {
    pub fn into_value(self) -> Value {
        let mut map = self.envelope.into_base("request");
        map.insert("rpc".into(),         json!(self.rpc));
        map.insert("duration_ms".into(), json!(self.duration_ms));
        map.insert("status_code".into(), json!(self.status_code));
        if let Some(v) = self.client_ip          { map.insert("client_ip".into(),           json!(v)); }
        if let Some(v) = self.user_agent         { map.insert("user_agent".into(),          json!(truncate_256(&v))); }
        if let Some(v) = self.request_size_bytes { map.insert("request_size_bytes".into(),  json!(v)); }
        if let Some(v) = self.response_size_bytes{ map.insert("response_size_bytes".into(), json!(v)); }
        match self.error_message {
            Some(v) => { map.insert("error_message".into(), json!(truncate_1024(&v))); }
            None    => { map.insert("error_message".into(), Value::Null); }
        }
        Value::Object(map)
    }
}

// =====================================================================
// Stream 2 — audit
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    Register,
    Evolve,
    Delete,
}

impl AuditAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Evolve   => "evolve",
            Self::Delete   => "delete",
        }
    }
}

/// One event per resource mutation (catalog or cone).
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub envelope:       CommonEnvelope,
    pub action:         AuditAction,
    pub resource_type:  String,
    pub resource_key:   String,
    pub parent_key:     Option<String>,
    pub op_tag:         String,
    pub content_sha256: Option<String>,
    pub description:    Option<String>,
    pub labels:         BTreeMap<String, String>,
}

impl AuditEvent {
    pub fn into_value(self) -> Value {
        let mut map = self.envelope.into_base("audit");
        map.insert("action".into(),        json!(self.action.as_str()));
        map.insert("resource_type".into(), json!(self.resource_type));
        map.insert("resource_key".into(),  json!(self.resource_key));
        match self.parent_key {
            Some(v) => { map.insert("parent_key".into(), json!(v)); }
            None    => { map.insert("parent_key".into(), json!("")); }
        }
        map.insert("op_tag".into(),        json!(self.op_tag));
        if let Some(v) = self.content_sha256 {
            map.insert("content_sha256".into(), json!(v));
        }
        if let Some(v) = self.description {
            map.insert("description".into(), json!(truncate_1024(&v)));
        }
        let labels_obj: serde_json::Map<String, Value> = self.labels
            .into_iter()
            .map(|(k, v)| (k, json!(v)))
            .collect();
        map.insert("labels".into(), Value::Object(labels_obj));
        Value::Object(map)
    }
}

// =====================================================================
// Stream 3 — classify
// =====================================================================

/// One event per Classify RPC. Strictly metadata — NEVER contains the
/// user's input text.
#[derive(Debug, Clone)]
pub struct ClassifyEvent {
    pub envelope:             CommonEnvelope,
    pub catalog_key:          String,
    pub predicted_class:      String,
    pub confidence:           f32,
    pub tier:                 ClassifyTier,
    pub n_classes:            u32,
    pub n_swarm_rounds:       Option<u32>,
    pub swarm_id:             Option<String>,
    pub embedder_fingerprint: Option<String>,
    pub elapsed_ms:           Option<u64>,
    pub rejected:             Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub enum ClassifyTier {
    High,
    Medium,
    Low,
    Reject,
}

impl ClassifyTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High   => "high",
            Self::Medium => "medium",
            Self::Low    => "low",
            Self::Reject => "reject",
        }
    }
}

impl ClassifyEvent {
    pub fn into_value(self) -> Value {
        let mut map = self.envelope.into_base("classify");
        map.insert("catalog_key".into(),     json!(self.catalog_key));
        map.insert("predicted_class".into(), json!(self.predicted_class));
        map.insert("confidence".into(),      json!(clamp_unit(self.confidence)));
        map.insert("tier".into(),            json!(self.tier.as_str()));
        map.insert("n_classes".into(),       json!(self.n_classes));
        if let Some(v) = self.n_swarm_rounds       { map.insert("n_swarm_rounds".into(),       json!(v)); }
        if let Some(v) = self.swarm_id             { map.insert("swarm_id".into(),             json!(v)); }
        if let Some(v) = self.embedder_fingerprint { map.insert("embedder_fingerprint".into(), json!(v)); }
        if let Some(v) = self.elapsed_ms           { map.insert("elapsed_ms".into(),           json!(v)); }
        if let Some(v) = self.rejected             { map.insert("rejected".into(),             json!(v)); }
        Value::Object(map)
    }
}

// =====================================================================
// Stream 4 — evolution
// =====================================================================

/// One event per EvolveCatalog RPC. Diff summary keys depend on op_tag.
#[derive(Debug, Clone)]
pub struct EvolutionEvent {
    pub envelope:               CommonEnvelope,
    pub from_key:               String,
    pub to_key:                 String,
    pub op_tag:                 String,
    pub diff_summary:           Value,
    pub validate_duration_ms:   Option<u64>,
    pub compile_duration_ms:    Option<u64>,
    pub content_sha256_before:  Option<String>,
    pub content_sha256_after:   Option<String>,
}

impl EvolutionEvent {
    pub fn into_value(self) -> Value {
        let mut map = self.envelope.into_base("evolution");
        map.insert("from_key".into(),     json!(self.from_key));
        map.insert("to_key".into(),       json!(self.to_key));
        map.insert("op_tag".into(),       json!(self.op_tag));
        map.insert("diff_summary".into(), self.diff_summary);
        if let Some(v) = self.validate_duration_ms  { map.insert("validate_duration_ms".into(),  json!(v)); }
        if let Some(v) = self.compile_duration_ms   { map.insert("compile_duration_ms".into(),   json!(v)); }
        if let Some(v) = self.content_sha256_before { map.insert("content_sha256_before".into(), json!(v)); }
        if let Some(v) = self.content_sha256_after  { map.insert("content_sha256_after".into(),  json!(v)); }
        Value::Object(map)
    }
}

// =====================================================================
// Stream 5 — security
// =====================================================================

#[derive(Debug, Clone, Copy)]
pub enum SecurityEventKind {
    JwtInvalid,
    JwtExpired,
    JwtSignature,
    ScopeMissing,
    TenantMismatch,
    RateLimited,
    GrafanaAdminAction,
    AuthOk,
}

impl SecurityEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JwtInvalid          => "jwt_invalid",
            Self::JwtExpired          => "jwt_expired",
            Self::JwtSignature        => "jwt_signature",
            Self::ScopeMissing        => "scope_missing",
            Self::TenantMismatch      => "tenant_mismatch",
            Self::RateLimited         => "rate_limited",
            Self::GrafanaAdminAction  => "grafana_admin_action",
            Self::AuthOk              => "auth_ok",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityEvent {
    pub envelope:        CommonEnvelope,
    pub event:           Option<SecurityEventKind>,
    pub reason:          Option<String>,
    pub claimed_tenant:  Option<String>,
    pub actual_tenant:   Option<String>,
    pub scope_required:  Option<String>,
    pub scope_present:   Vec<String>,
    pub client_ip:       Option<String>,
    pub user_agent:      Option<String>,
    pub actor:           Option<String>,
    pub action:          Option<String>,
    pub target:          Option<String>,
    pub diff_summary:    Option<String>,
}

impl SecurityEvent {
    pub fn new(envelope: CommonEnvelope, event: SecurityEventKind) -> Self {
        Self {
            envelope,
            event: Some(event),
            reason: None,
            claimed_tenant: None,
            actual_tenant: None,
            scope_required: None,
            scope_present: Vec::new(),
            client_ip: None,
            user_agent: None,
            actor: None,
            action: None,
            target: None,
            diff_summary: None,
        }
    }

    pub fn into_value(self) -> Value {
        let kind = self.event
            .expect("SecurityEvent.event is mandatory ; use SecurityEvent::new()")
            .as_str();
        let mut map = self.envelope.into_base("security");
        map.insert("event".into(), json!(kind));
        if let Some(v) = self.reason         { map.insert("reason".into(),         json!(truncate_256(&v))); }
        if let Some(v) = self.claimed_tenant { map.insert("claimed_tenant".into(), json!(v)); }
        if let Some(v) = self.actual_tenant  { map.insert("actual_tenant".into(),  json!(v)); }
        if let Some(v) = self.scope_required { map.insert("scope_required".into(), json!(v)); }
        if !self.scope_present.is_empty() {
            map.insert("scope_present".into(), json!(self.scope_present));
        }
        if let Some(v) = self.client_ip      { map.insert("client_ip".into(),      json!(v)); }
        if let Some(v) = self.user_agent     { map.insert("user_agent".into(),     json!(truncate_256(&v))); }
        if let Some(v) = self.actor          { map.insert("actor".into(),          json!(v)); }
        if let Some(v) = self.action         { map.insert("action".into(),         json!(v)); }
        if let Some(v) = self.target         { map.insert("target".into(),         json!(v)); }
        if let Some(v) = self.diff_summary   { map.insert("diff_summary".into(),   json!(truncate_512(&v))); }
        Value::Object(map)
    }
}

// =====================================================================
// Helpers
// =====================================================================

fn truncate_256(s: &str) -> String {
    if s.len() <= 256 { s.to_owned() } else { s.chars().take(256).collect() }
}

fn truncate_512(s: &str) -> String {
    if s.len() <= 512 { s.to_owned() } else { s.chars().take(512).collect() }
}

fn truncate_1024(s: &str) -> String {
    if s.len() <= 1024 { s.to_owned() } else { s.chars().take(1024).collect() }
}

fn clamp_unit(x: f32) -> f32 {
    if x.is_nan() { 0.0 }
    else if x < 0.0 { 0.0 }
    else if x > 1.0 { 1.0 }
    else { x }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_envelope() -> CommonEnvelope {
        CommonEnvelope {
            timestamp:       format_rfc3339_now(),
            level:           Level::Info,
            request_id:      Some("11111111-2222-3333-4444-555555555555".to_string()),
            trace_id:        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            span_id:         Some("bbbbbbbbbbbbbbbb".to_string()),
            tenant_id:       Some("tenant-acme".to_string()),
            principal:       Some("alice@acme.com".to_string()),
            service_version: "0.1.13".to_string(),
        }
    }

    #[test]
    fn rfc3339_format_includes_t_z_and_nanos() {
        let s = format_rfc3339_from_unix(1_700_000_000, 123_456_789);
        assert!(s.contains('T'));
        assert!(s.ends_with('Z'));
        assert!(s.contains(".123456789"));
        // 1700000000 = 2023-11-14T22:13:20Z
        assert!(s.starts_with("2023-11-14T22:13:20"));
    }

    #[test]
    fn rfc3339_format_handles_epoch() {
        let s = format_rfc3339_from_unix(0, 0);
        assert_eq!(s, "1970-01-01T00:00:00.000000000Z");
    }

    #[test]
    fn rfc3339_format_handles_y2038_boundary() {
        // 2 147 483 647 = 2038-01-19T03:14:07Z
        let s = format_rfc3339_from_unix(2_147_483_647, 0);
        assert!(s.starts_with("2038-01-19T03:14:07"));
    }

    #[test]
    fn rfc3339_format_now_succeeds() {
        let s = format_rfc3339_now();
        assert!(s.contains('T'));
        assert!(s.ends_with('Z'));
    }

    #[test]
    fn request_event_emits_required_fields() {
        let e = RequestEvent {
            envelope: sample_envelope(),
            rpc: "Classify".into(),
            duration_ms: 42,
            status_code: "OK".into(),
            client_ip: Some("1.2.3.4".into()),
            user_agent: Some("efe-tool/0.1.13".into()),
            request_size_bytes: Some(1024),
            response_size_bytes: Some(2048),
            error_message: None,
        };
        let v = e.into_value();
        assert_eq!(v["schema_version"], "1.0");
        assert_eq!(v["stream"], "request");
        assert_eq!(v["rpc"], "Classify");
        assert_eq!(v["duration_ms"], 42);
        assert_eq!(v["status_code"], "OK");
        assert_eq!(v["error_message"], Value::Null);
    }

    #[test]
    fn audit_event_emits_required_fields() {
        let mut labels = BTreeMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        let e = AuditEvent {
            envelope: sample_envelope(),
            action: AuditAction::Register,
            resource_type: "catalog".into(),
            resource_key: "tenant-acme/billing@v1".into(),
            parent_key: None,
            op_tag: "register".into(),
            content_sha256: Some("deadbeef".into()),
            description: Some("Initial import".into()),
            labels,
        };
        let v = e.into_value();
        assert_eq!(v["stream"], "audit");
        assert_eq!(v["action"], "register");
        assert_eq!(v["resource_type"], "catalog");
        assert_eq!(v["resource_key"], "tenant-acme/billing@v1");
        assert_eq!(v["op_tag"], "register");
        assert_eq!(v["parent_key"], "");
        assert_eq!(v["labels"]["env"], "prod");
    }

    #[test]
    fn classify_event_clamps_confidence() {
        let e = ClassifyEvent {
            envelope: sample_envelope(),
            catalog_key: "tenant-acme/triage@v3".into(),
            predicted_class: "billing".into(),
            confidence: 1.5, // out of [0, 1]
            tier: ClassifyTier::High,
            n_classes: 5,
            n_swarm_rounds: Some(8),
            swarm_id: None,
            embedder_fingerprint: Some("st:mini:384".into()),
            elapsed_ms: Some(45),
            rejected: Some(false),
        };
        let v = e.into_value();
        assert_eq!(v["stream"], "classify");
        // 1.5 clamped to 1.0
        assert!(v["confidence"].as_f64().unwrap() <= 1.0);
        assert_eq!(v["tier"], "high");
        assert_eq!(v["predicted_class"], "billing");
    }

    #[test]
    fn classify_event_clamps_negative_confidence() {
        let e = ClassifyEvent {
            envelope: sample_envelope(),
            catalog_key: "x/y@v1".into(),
            predicted_class: "z".into(),
            confidence: -0.3,
            tier: ClassifyTier::Reject,
            n_classes: 2,
            n_swarm_rounds: None,
            swarm_id: None,
            embedder_fingerprint: None,
            elapsed_ms: None,
            rejected: Some(true),
        };
        let v = e.into_value();
        assert_eq!(v["confidence"].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn classify_event_clamps_nan_confidence() {
        let e = ClassifyEvent {
            envelope: sample_envelope(),
            catalog_key: "x/y@v1".into(),
            predicted_class: "z".into(),
            confidence: f32::NAN,
            tier: ClassifyTier::Reject,
            n_classes: 2,
            n_swarm_rounds: None,
            swarm_id: None,
            embedder_fingerprint: None,
            elapsed_ms: None,
            rejected: Some(true),
        };
        let v = e.into_value();
        assert_eq!(v["confidence"].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn evolution_event_carries_diff_summary() {
        let e = EvolutionEvent {
            envelope: sample_envelope(),
            from_key: "tenant-acme/triage@v1".into(),
            to_key:   "tenant-acme/triage@v2".into(),
            op_tag:   "append_class".into(),
            diff_summary: json!({"classes_added": ["churn"]}),
            validate_duration_ms: Some(12),
            compile_duration_ms: Some(480),
            content_sha256_before: Some("aaaa".into()),
            content_sha256_after:  Some("bbbb".into()),
        };
        let v = e.into_value();
        assert_eq!(v["stream"], "evolution");
        assert_eq!(v["op_tag"], "append_class");
        assert_eq!(v["diff_summary"]["classes_added"][0], "churn");
    }

    #[test]
    fn security_event_emits_kind_as_event_field() {
        let e = SecurityEvent::new(sample_envelope(), SecurityEventKind::JwtExpired);
        let v = e.into_value();
        assert_eq!(v["stream"], "security");
        assert_eq!(v["event"], "jwt_expired");
    }

    #[test]
    fn security_event_with_scope_present() {
        let mut e = SecurityEvent::new(sample_envelope(), SecurityEventKind::ScopeMissing);
        e.scope_required = Some("admin:catalog:write".into());
        e.scope_present  = vec!["admin:catalog:read".into()];
        let v = e.into_value();
        assert_eq!(v["scope_required"], "admin:catalog:write");
        assert_eq!(v["scope_present"][0], "admin:catalog:read");
    }

    #[test]
    fn truncation_works() {
        let long = "x".repeat(2000);
        let e = RequestEvent {
            envelope: sample_envelope(),
            rpc: "X".into(),
            duration_ms: 0,
            status_code: "OK".into(),
            client_ip: None,
            user_agent: Some(long.clone()),
            request_size_bytes: None,
            response_size_bytes: None,
            error_message: Some(long),
        };
        let v = e.into_value();
        assert!(v["user_agent"].as_str().unwrap().len() <= 256);
        assert!(v["error_message"].as_str().unwrap().len() <= 1024);
    }

    #[test]
    fn timestamp_is_iso8601_utc() {
        let e = RequestEvent {
            envelope: sample_envelope(),
            rpc: "X".into(),
            duration_ms: 0,
            status_code: "OK".into(),
            client_ip: None,
            user_agent: None,
            request_size_bytes: None,
            response_size_bytes: None,
            error_message: None,
        };
        let v = e.into_value();
        let ts = v["timestamp"].as_str().unwrap();
        // Ends with Z (UTC), has T separator.
        assert!(ts.ends_with('Z'), "timestamp {ts:?} should end with Z");
        assert!(ts.contains('T'), "timestamp {ts:?} should have T separator");
    }

    #[test]
    fn audit_action_strings_match_schema_enum() {
        assert_eq!(AuditAction::Register.as_str(), "register");
        assert_eq!(AuditAction::Evolve.as_str(),   "evolve");
        assert_eq!(AuditAction::Delete.as_str(),   "delete");
    }

    #[test]
    fn classify_tier_strings_match_schema_enum() {
        assert_eq!(ClassifyTier::High.as_str(),   "high");
        assert_eq!(ClassifyTier::Medium.as_str(), "medium");
        assert_eq!(ClassifyTier::Low.as_str(),    "low");
        assert_eq!(ClassifyTier::Reject.as_str(), "reject");
    }

    #[test]
    fn security_event_kind_strings_match_schema_enum() {
        assert_eq!(SecurityEventKind::JwtInvalid.as_str(),         "jwt_invalid");
        assert_eq!(SecurityEventKind::JwtExpired.as_str(),         "jwt_expired");
        assert_eq!(SecurityEventKind::JwtSignature.as_str(),       "jwt_signature");
        assert_eq!(SecurityEventKind::ScopeMissing.as_str(),       "scope_missing");
        assert_eq!(SecurityEventKind::TenantMismatch.as_str(),     "tenant_mismatch");
        assert_eq!(SecurityEventKind::RateLimited.as_str(),        "rate_limited");
        assert_eq!(SecurityEventKind::GrafanaAdminAction.as_str(), "grafana_admin_action");
        assert_eq!(SecurityEventKind::AuthOk.as_str(),             "auth_ok");
    }

    #[test]
    fn level_strings_match_schema_enum() {
        assert_eq!(Level::Debug.as_str(), "DEBUG");
        assert_eq!(Level::Info.as_str(),  "INFO");
        assert_eq!(Level::Warn.as_str(),  "WARN");
        assert_eq!(Level::Error.as_str(), "ERROR");
    }
}
