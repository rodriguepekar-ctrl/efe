// SPDX-License-Identifier: UNLICENSED
//! Helpers HTTP/SSE — conversion `Receiver<StreamEvent>` → flux d'octets
//! conforme `text/event-stream`.
//!
//! Pas dépendant d'un framework HTTP particulier : retourne un
//! [`futures::Stream<Item = Result<Bytes, std::io::Error>>`] que vous pouvez
//! brancher dans Axum (`Sse::new`), Warp, Actix-web, etc.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use tokio::sync::mpsc::Receiver;

use crate::events::StreamEvent;

/// Header `Content-Type` à utiliser pour la réponse HTTP.
pub const CONTENT_TYPE_SSE: &str = "text/event-stream";

/// Préface optionnelle à émettre avant le 1er évènement
/// (`retry:` indique au client le délai de reconnexion en ms).
pub const SSE_PREFACE: &str = ":efe-coordinator SSE v1.0.0\nretry: 5000\n\n";

/// Adapter qui transforme un `Receiver<StreamEvent>` en stream d'octets SSE.
///
/// ## Format de sortie
///
/// ```text
/// event: SWARM_STARTED\n
/// data: {"type":"SWARM_STARTED",...}\n
/// \n
/// ```
///
/// (Conforme à la spec [SSE](https://html.spec.whatwg.org/multipage/server-sent-events.html).)
pub struct SseByteStream {
    rx: Receiver<StreamEvent>,
    sent_preface: bool,
}

impl SseByteStream {
    /// Construit l'adapter à partir d'un receiver de bus.
    #[must_use]
    pub fn new(rx: Receiver<StreamEvent>) -> Self {
        Self {
            rx,
            sent_preface: false,
        }
    }
}

impl Stream for SseByteStream {
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.sent_preface {
            self.sent_preface = true;
            return Poll::Ready(Some(Ok(bytes::Bytes::from(SSE_PREFACE))));
        }
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(ev)) => {
                let frame = ev.to_sse_frame();
                Poll::Ready(Some(Ok(bytes::Bytes::from(frame.into_bytes()))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Helper sync : sérialise une liste d'évènements en un blob SSE complet
/// (utile pour les tests + le replay engine).
#[must_use]
pub fn to_sse_blob(events: &[StreamEvent]) -> String {
    let mut out = String::with_capacity(events.len() * 128);
    out.push_str(SSE_PREFACE);
    for ev in events {
        out.push_str(&ev.to_sse_frame());
    }
    out
}

/// Parse un blob SSE (côté tests / client tooling) et retourne les frames.
///
/// Renvoie chaque `data: …` parsé en `StreamEvent` ; les lignes
/// `event:`, `retry:`, commentaires `:…` sont ignorées (compatibles spec).
pub fn parse_sse_blob(blob: &str) -> Vec<Result<StreamEvent, String>> {
    let mut out = Vec::new();
    for chunk in blob.split("\n\n") {
        for line in chunk.lines() {
            if let Some(json) = line.strip_prefix("data: ") {
                out.push(serde_json::from_str(json).map_err(|e| e.to_string()));
            }
        }
    }
    out
}

// On évite la dépendance forte à `bytes` en publicc : ré-export.
pub use bytes;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{InMemoryBus, EventBus};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn preface_indicates_protocol() {
        assert!(SSE_PREFACE.contains("efe-coordinator SSE v1.0.0"));
        assert!(SSE_PREFACE.contains("retry: 5000"));
    }

    #[test]
    fn to_sse_blob_starts_with_preface() {
        let blob = to_sse_blob(&[StreamEvent::Heartbeat { ts: Utc::now() }]);
        assert!(blob.starts_with(SSE_PREFACE));
        assert!(blob.contains("event: HEARTBEAT"));
    }

    #[test]
    fn parse_round_trips_simple_heartbeat() {
        let hb = StreamEvent::Heartbeat { ts: Utc::now() };
        let blob = to_sse_blob(&[hb.clone()]);
        let parsed = parse_sse_blob(&blob);
        assert_eq!(parsed.len(), 1);
        let back = parsed.into_iter().next().unwrap().unwrap();
        assert_eq!(back.tag(), "HEARTBEAT");
    }

    #[test]
    fn parse_skips_event_and_retry_lines() {
        let blob = "retry: 5000\n\nevent: HEARTBEAT\ndata: {\"type\":\"HEARTBEAT\",\"ts\":\"2026-05-25T00:00:00Z\"}\n\n";
        let parsed = parse_sse_blob(blob);
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].is_ok());
    }

    #[tokio::test]
    async fn byte_stream_emits_preface_first() {
        let bus = InMemoryBus::new();
        let (_t, rx) = bus.subscribe(Some("test"));
        let mut stream = SseByteStream::new(rx);
        let first = futures::StreamExt::next(&mut stream).await.unwrap().unwrap();
        let s = std::str::from_utf8(&first).unwrap();
        assert!(s.starts_with(":efe-coordinator SSE"));
    }

    #[tokio::test]
    async fn byte_stream_relays_events_then_closes() {
        let bus = std::sync::Arc::new(InMemoryBus::new());
        let (_t, rx) = bus.subscribe(Some("test"));
        let mut stream = SseByteStream::new(rx);
        // Préface
        let _ = futures::StreamExt::next(&mut stream).await.unwrap().unwrap();
        // Publie un évènement.
        bus.publish(StreamEvent::Heartbeat { ts: Utc::now() })
            .await
            .unwrap();
        let frame = futures::StreamExt::next(&mut stream).await.unwrap().unwrap();
        let s = std::str::from_utf8(&frame).unwrap();
        assert!(s.contains("event: HEARTBEAT"));
        // Drop le bus (et toutes les sender) → la receiver doit voir None.
        drop(bus);
        let end = futures::StreamExt::next(&mut stream).await;
        assert!(end.is_none());
    }

    #[test]
    fn content_type_constant_is_text_event_stream() {
        assert_eq!(CONTENT_TYPE_SSE, "text/event-stream");
    }

    #[test]
    fn parse_handles_swarm_started_with_uuid() {
        let s = Uuid::now_v7();
        let ev = StreamEvent::SwarmStarted {
            swarm_id: s,
            mission: crate::events::MissionTag::MissionA,
            user_text: "hi".into(),
            task_class: efe_nano_app_coordinator::types::router::TaskClass::FactualQa,
            ts: Utc::now(),
        };
        let blob = to_sse_blob(&[ev.clone()]);
        let parsed = parse_sse_blob(&blob);
        assert_eq!(parsed.len(), 1);
        let back = parsed.into_iter().next().unwrap().unwrap();
        assert_eq!(back.swarm_id(), Some(s));
    }
}
