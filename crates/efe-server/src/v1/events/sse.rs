//! Server-Sent Events stream of customer events.
//!
//! `GET /v1/events/stream` opens a long-lived HTTP connection ; the
//! server emits each event as an SSE frame. Browser-friendly fallback
//! to NATS for non-gRPC clients.

use super::handlers::{EventLogEntry, EventsError, EventsHandlerState};
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::time::Duration;

/// SSE stream of events. Polls the DB-backed event log every 500 ms
/// (cheap : tail of a tenant's `customer_event_log` table is
/// well-indexed). For production at scale, a future variant taps
/// directly into NATS and pushes events on receive.
pub async fn sse_stream<S: EventsHandlerState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> impl IntoResponse {
    let (parts, _) = req.into_parts();
    let tenant = match state.auth_tenant_from_request(&parts) {
        Some(t) => t,
        None => {
            return axum::response::Response::builder()
                .status(401)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"code":"unauthenticated","message":"missing bearer"}"#,
                ))
                .unwrap();
        }
    };

    let stream = async_stream::stream! {
        let mut since = chrono::Utc::now();
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let page = state
                .list_events(&tenant, Some(since), None, None, 100, None)
                .await;
            match page {
                Ok(p) => {
                    for entry in &p.events {
                        if let Ok(ev) = entry_to_sse(entry) {
                            yield Ok::<_, Infallible>(ev);
                        }
                        // Advance the watermark.
                        since = entry.occurred_at;
                    }
                }
                Err(e) => {
                    let payload = serde_json::json!({
                        "error": e.to_string(),
                    });
                    if let Ok(ev) = Event::default().event("error").json_data(payload) {
                        yield Ok(ev);
                    }
                    // Avoid hot-loop on persistent errors.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn entry_to_sse(entry: &EventLogEntry) -> Result<Event, EventsError> {
    Event::default()
        .id(entry.id.to_string())
        .event(entry.topic.clone())
        .json_data(entry)
        .map_err(|e| EventsError::Db(format!("sse encode : {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn entry_serializes_as_sse() {
        let e = EventLogEntry {
            id: Uuid::new_v4(),
            tenant_id: "acme".into(),
            topic: "classify.completed".into(),
            occurred_at: Utc::now(),
            schema_version: 1,
            trace_id: None,
            data: serde_json::json!({"foo": "bar"}),
        };
        let _ = entry_to_sse(&e).expect("should encode");
        // ensure compile-time path through Stream<Item = Result<Event, Infallible>>
        let _typed: Result<Event, EventsError> = entry_to_sse(&e);
    }
}
