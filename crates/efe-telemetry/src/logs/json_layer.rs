//! Custom `tracing_subscriber::Layer` emitting JSON Lines with PII
//! redaction baked in.
//!
//! The layer captures each event, merges the fields of every enclosing
//! span into the event's own fields (outermost-first, event-last so the
//! event's values win on conflict), runs them through the
//! [`FieldFilter`], and writes a single line of JSON to the configured
//! writer.
//!
//! # Schema
//!
//! ```json
//! {
//!   "ts": "2026-05-14T12:34:56.789Z",
//!   "level": "INFO",
//!   "target": "efe_core::swarm::arbiter",
//!   "message": "round complete",
//!   "fields": { "swarm_id": "abc", "round": 3, "variance": 0.01 }
//! }
//! ```
//!
//! Span-field capture (audit finding M11-#5) is implemented via
//! `Layer::on_new_span` — we visit a span's declared fields once at
//! creation and store them in its `span.extensions_mut()`. On event
//! emission, `ctx.event_scope(event)` yields the span stack outermost
//! first and we merge each span's snapshot into the event record.

use std::fmt::Debug;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Per-span snapshot of fields captured at span creation time.
#[derive(Clone, Debug, Default)]
struct SpanFieldSnapshot {
    fields: Map<String, Value>,
}

use crate::logs::field_filter::FieldFilter;

/// Writer target for the layer. Defaults to `std::io::stdout`, but can
/// be swapped for a test sink or a file.
pub type SharedWriter = Arc<Mutex<dyn Write + Send + Sync>>;

/// JSON-lines layer.
pub struct JsonLayer {
    writer: SharedWriter,
    filter: FieldFilter,
}

impl Debug for JsonLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonLayer")
            .field("filter", &self.filter)
            .finish()
    }
}

impl JsonLayer {
    /// Construct with a writer and a pre-configured [`FieldFilter`].
    #[must_use]
    pub fn new(writer: SharedWriter, filter: FieldFilter) -> Self {
        Self { writer, filter }
    }

    /// Convenience: write to stdout with default filter.
    #[must_use]
    pub fn stdout(allow_prompts: bool) -> Self {
        Self {
            writer: Arc::new(Mutex::new(std::io::stdout())),
            filter: FieldFilter::new(allow_prompts),
        }
    }

    /// Convenience: write to an in-memory buffer. Useful for tests.
    ///
    /// Returns the layer plus a concrete `Arc<Mutex<Vec<u8>>>` handle
    /// so callers can inspect the raw bytes without downcasting.
    #[must_use]
    pub fn in_memory() -> (Self, Arc<Mutex<Vec<u8>>>) {
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let writer: SharedWriter = buf.clone();
        let layer = Self {
            writer,
            filter: FieldFilter::default(),
        };
        (layer, buf)
    }
}

impl<S> Layer<S> for JsonLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    /// Snapshot the span's declared fields at creation time and store
    /// them in the span's extensions for later lookup in `on_event`.
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut visitor = JsonFieldVisitor::new(&self.filter);
        attrs.record(&mut visitor);
        let mut ext = span.extensions_mut();
        if ext.get_mut::<SpanFieldSnapshot>().is_none() {
            ext.insert(SpanFieldSnapshot {
                fields: visitor.fields,
            });
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Walk the span scope outermost-first, merging each snapshot
        // into an accumulator. The event's own fields are applied last
        // so they win on conflict.
        let mut merged: Map<String, Value> = Map::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(snap) = span.extensions().get::<SpanFieldSnapshot>() {
                    for (k, v) in &snap.fields {
                        merged.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        let mut visitor = JsonFieldVisitor::new(&self.filter);
        event.record(&mut visitor);
        for (k, v) in visitor.fields {
            merged.insert(k, v);
        }

        let metadata = event.metadata();
        let mut record = Map::new();
        record.insert("ts".into(), Value::String(iso8601_now()));
        record.insert("level".into(), Value::String(metadata.level().to_string()));
        record.insert(
            "target".into(),
            Value::String(metadata.target().to_string()),
        );
        if let Some(msg) = visitor.message.take() {
            record.insert("message".into(), Value::String(msg));
        }
        if !merged.is_empty() {
            record.insert("fields".into(), Value::Object(merged));
        }

        if let Ok(mut w) = self.writer.lock() {
            let _ = serde_json::to_writer(&mut *w, &Value::Object(record));
            let _ = w.write_all(b"\n");
        }
    }
}

/// RFC 3339 / ISO 8601 timestamp with millisecond precision. Produced
/// manually to avoid a `chrono` dependency.
fn iso8601_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    format_iso8601(secs, millis)
}

fn format_iso8601(unix_secs: i64, millis: u32) -> String {
    // Civil-from-days algorithm (Howard Hinnant's algorithm).
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let hour = (secs_of_day / 3600) as u32;
    let minute = ((secs_of_day % 3600) / 60) as u32;
    let second = (secs_of_day % 60) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y, m, d, hour, minute, second, millis
    )
}

struct JsonFieldVisitor<'a> {
    fields: Map<String, Value>,
    message: Option<String>,
    filter: &'a FieldFilter,
}

impl<'a> JsonFieldVisitor<'a> {
    fn new(filter: &'a FieldFilter) -> Self {
        Self {
            fields: Map::new(),
            message: None,
            filter,
        }
    }

    fn push(&mut self, field: &Field, value: Value) {
        let name = field.name();
        if name == "message" {
            if let Value::String(s) = value {
                self.message = Some(s);
            }
            return;
        }
        let filtered = self.filter.apply(name, value);
        self.fields.insert(name.to_string(), filtered);
    }
}

impl Visit for JsonFieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.push(field, Value::String(format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, Value::String(value.to_string()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, Value::Bool(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, Value::Number(value.into()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, Value::Number(value.into()));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        match serde_json::Number::from_f64(value) {
            Some(n) => self.push(field, Value::Number(n)),
            None => self.push(field, Value::String(format!("{value}"))),
        }
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.push(field, Value::String(value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::subscriber::with_default;
    use tracing_subscriber::{layer::SubscriberExt, Registry};

    fn read_lines(buf: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
        let guard = buf.lock().unwrap();
        let s = std::str::from_utf8(&guard).unwrap();
        s.lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn iso8601_format_is_well_formed() {
        let s = format_iso8601(1_715_000_000, 123);
        assert!(s.starts_with("2024-05-"));
        assert!(s.ends_with("Z"));
        assert!(s.contains('.'));
    }

    #[test]
    fn emits_one_line_per_event() {
        let (layer, buf) = JsonLayer::in_memory();
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, || {
            tracing::info!(swarm_id = "abc", round = 3, "round complete");
        });
        let lines = read_lines(&buf);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line["level"], "INFO");
        assert_eq!(line["message"], "round complete");
        assert_eq!(line["fields"]["swarm_id"], "abc");
        assert_eq!(line["fields"]["round"], 3);
    }

    #[test]
    fn blocked_fields_are_redacted() {
        let (layer, buf) = JsonLayer::in_memory();
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, || {
            tracing::info!(raw_prompt = "secret-content", "received");
        });
        let lines = read_lines(&buf);
        assert_eq!(lines[0]["fields"]["raw_prompt"], "<redacted>");
    }

    #[test]
    fn message_is_separated_from_fields() {
        let (layer, buf) = JsonLayer::in_memory();
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, || {
            tracing::info!(foo = 1, "hello world");
        });
        let lines = read_lines(&buf);
        assert_eq!(lines[0]["message"], "hello world");
        // The message must not appear inside `fields`.
        assert!(lines[0]["fields"].get("message").is_none());
    }

    // Regression for audit finding M11-#5: span fields must propagate
    // into event records so that cross-module logs emitted inside a
    // swarm span automatically carry `swarm_id`, `tenant_id`, etc.
    #[test]
    fn span_fields_are_merged_into_event_record() {
        let (layer, buf) = JsonLayer::in_memory();
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, || {
            let span = tracing::info_span!("swarm", swarm_id = "abc", tenant_id = "acme");
            let _guard = span.enter();
            tracing::info!(round = 2, "round complete");
        });
        let lines = read_lines(&buf);
        assert_eq!(lines[0]["message"], "round complete");
        let fields = lines[0]["fields"].as_object().expect("fields present");
        assert_eq!(fields["swarm_id"], "abc");
        assert_eq!(fields["tenant_id"], "acme");
        assert_eq!(fields["round"], 2);
    }

    #[test]
    fn event_field_wins_on_conflict_with_span() {
        let (layer, buf) = JsonLayer::in_memory();
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, || {
            let span = tracing::info_span!("outer", phase = "genesis");
            let _guard = span.enter();
            tracing::info!(phase = "fusion", "override");
        });
        let lines = read_lines(&buf);
        // Event's explicit field overrides the enclosing span's field.
        assert_eq!(lines[0]["fields"]["phase"], "fusion");
    }

    #[test]
    fn nested_spans_merge_outermost_first() {
        let (layer, buf) = JsonLayer::in_memory();
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, || {
            let outer = tracing::info_span!("outer", tenant_id = "acme", phase = "root");
            let _o = outer.enter();
            let inner = tracing::info_span!("inner", phase = "fusion");
            let _i = inner.enter();
            tracing::info!("note");
        });
        let lines = read_lines(&buf);
        let fields = lines[0]["fields"].as_object().unwrap();
        assert_eq!(fields["tenant_id"], "acme");
        // Inner span overrides outer for shared field.
        assert_eq!(fields["phase"], "fusion");
    }

    #[test]
    fn blocked_field_in_span_is_redacted() {
        let (layer, buf) = JsonLayer::in_memory();
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, || {
            let span = tracing::info_span!("req", raw_prompt = "leaked_content");
            let _guard = span.enter();
            tracing::info!("check");
        });
        let lines = read_lines(&buf);
        assert_eq!(lines[0]["fields"]["raw_prompt"], "<redacted>");
    }
}
