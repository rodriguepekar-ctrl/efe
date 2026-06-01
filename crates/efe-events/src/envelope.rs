//! Event envelope : the common shape every event carries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use ulid::Ulid;

/// ULID-based event identifier. Lexicographically sortable, monotonic-ish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(pub Ulid);

impl EventId {
    /// Generate a new ULID using the current time + crypto-secure RNG.
    pub fn new() -> Self {
        Self(Ulid::new())
    }

    /// Decode from a 26-char string. Returns None on malformed input.
    pub fn parse(s: &str) -> Option<Self> {
        Ulid::from_string(s).ok().map(EventId)
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<Ulid> for EventId {
    fn as_ref(&self) -> &Ulid {
        &self.0
    }
}

/// Generic envelope. `T` is the topic-specific payload type.
///
/// JSON wire format example (`topic = "classify.completed"`) :
///
/// ```json
/// {
///   "$schema": "https://schemas.efe.ai/event/v1.json",
///   "id": "01HE...",
///   "tenant_id": "acme",
///   "topic": "classify.completed",
///   "occurred_at": "2026-05-11T12:34:56.789Z",
///   "schema_version": 1,
///   "trace_id": "00-...-01",
///   "data": { ... }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event<T> {
    /// Common JSON Schema URI (informational ; not enforced at parse).
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema_uri: Option<String>,

    /// Unique event ID.
    pub id: EventId,

    /// Tenant slug.
    pub tenant_id: String,

    /// Hierarchical topic, e.g. `classify.completed`, `agent.run.completed`.
    pub topic: String,

    /// Event time. Server-generated at publish time.
    pub occurred_at: DateTime<Utc>,

    /// Schema version for the `data` payload. Today always `1`.
    pub schema_version: u32,

    /// Optional W3C trace context (`00-<traceid>-<spanid>-<flags>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,

    /// Topic-specific payload.
    pub data: T,
}

impl<T> Event<T>
where
    T: Serialize,
{
    /// Build a new event with a fresh ID + `occurred_at = now()`.
    pub fn new(tenant_id: impl Into<String>, topic: impl Into<String>, data: T) -> Self {
        Self {
            schema_uri: Some("https://schemas.efe.ai/event/v1.json".into()),
            id: EventId::new(),
            tenant_id: tenant_id.into(),
            topic: topic.into(),
            occurred_at: Utc::now(),
            schema_version: 1,
            trace_id: None,
            data,
        }
    }

    /// Same as `new` but attach a W3C trace context.
    pub fn new_traced(
        tenant_id: impl Into<String>,
        topic: impl Into<String>,
        data: T,
        trace_id: impl Into<String>,
    ) -> Self {
        let mut e = Self::new(tenant_id, topic, data);
        e.trace_id = Some(trace_id.into());
        e
    }

    /// Build the NATS subject for this event.
    pub fn subject(&self) -> String {
        if self.tenant_id == "system" {
            format!("efe.system.{}", self.topic)
        } else {
            format!("efe.tenant.{}.{}", self.tenant_id, self.topic)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct DummyData {
        text: String,
        n: u32,
    }

    #[test]
    fn event_id_is_ulid_format() {
        let e = EventId::new();
        let s = e.to_string();
        assert_eq!(s.len(), 26);
        let back = EventId::parse(&s).expect("parse roundtrip");
        assert_eq!(back, e);
    }

    #[test]
    fn event_id_invalid_returns_none() {
        assert!(EventId::parse("not-a-ulid").is_none());
    }

    #[test]
    fn subject_for_tenant() {
        let e: Event<DummyData> = Event::new(
            "acme",
            "classify.completed",
            DummyData { text: "x".into(), n: 1 },
        );
        assert_eq!(e.subject(), "efe.tenant.acme.classify.completed");
    }

    #[test]
    fn subject_for_system() {
        let e: Event<DummyData> = Event::new(
            "system",
            "audit.cache_poisoned",
            DummyData { text: "y".into(), n: 0 },
        );
        assert_eq!(e.subject(), "efe.system.audit.cache_poisoned");
    }

    #[test]
    fn json_roundtrip_preserves_fields() {
        let original: Event<serde_json::Value> = Event::new(
            "acme",
            "drift.detected",
            json!({"class_id": "c1", "kl": 0.42}),
        );
        let bytes = serde_json::to_vec(&original).unwrap();
        let back: Event<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.id, original.id);
        assert_eq!(back.tenant_id, "acme");
        assert_eq!(back.topic, "drift.detected");
        assert_eq!(back.schema_version, 1);
        assert_eq!(back.data["class_id"], "c1");
    }

    #[test]
    fn traced_event_preserves_trace_id() {
        let e: Event<DummyData> = Event::new_traced(
            "acme",
            "foo",
            DummyData { text: "x".into(), n: 1 },
            "00-deadbeef-cafef00d-01",
        );
        assert_eq!(e.trace_id.as_deref(), Some("00-deadbeef-cafef00d-01"));
    }
}
