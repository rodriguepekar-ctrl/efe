// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : line-based SSE parser for the Anthropic
//! Messages API streaming wire format.
//!
//! See architecture § 3 (protocol deep dive), § 9.1 (state
//! machine), § 19.1 (skeleton).  RFC reference : 8895 § 4.2.
//!
//! Implementation strategy : we consume `Bytes` chunks from a
//! `reqwest::Response::bytes_stream()`, scan line-by-line, and
//! emit one [`AnthropicSseEvent`] per `\n\n` boundary.
//!
//! Defense :
//! * `MAX_LINE_BYTES`   — single SSE line cap (64 KiB).
//! * `MAX_EVENT_BYTES`  — accumulated `data:` body cap (1 MiB).

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::stream::Stream;
use thiserror::Error;

use super::event::AnthropicSseEvent;

/// **N27** : max bytes for a single SSE line (anti-OOM).
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// **N27** : max accumulated `data:` body for one event (anti-OOM).
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// **N27** : closed taxonomy of SSE parse errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SseParseError {
    #[error("SSE line exceeded {MAX_LINE_BYTES} bytes")]
    LineTooLong,
    #[error("SSE event payload exceeded {MAX_EVENT_BYTES} bytes")]
    EventTooLarge,
    #[error("SSE payload was not valid UTF-8")]
    InvalidUtf8,
    #[error("SSE event JSON parse error : {0}")]
    Json(String),
    #[error("underlying network error : {0}")]
    Network(String),
}

/// **N27** : stateful line-based parser.  Holds the current event
/// accumulator across [`Bytes`] chunk boundaries.
pub struct SseParser {
    /// Optional `event: <name>` line value (we currently fall back
    /// to `data.type` per ADR-N27-2, but capture for forensics).
    current_event_name: Option<String>,
    /// Accumulated `data:` body.  Multi-line `data:` fields are
    /// joined with `\n` per RFC.
    data_buf: String,
    /// Bytes accumulated for the current line, until a `\n` is
    /// seen.  Capped at `MAX_LINE_BYTES`.
    line_buf: Vec<u8>,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            current_event_name: None,
            data_buf: String::new(),
            line_buf: Vec::with_capacity(4 * 1024),
        }
    }

    /// **N27** : feed a chunk of bytes ; emit parsed events into
    /// `out`.  Returns `Err` on first fatal parse error (caller
    /// should abort the stream).
    pub fn feed(
        &mut self,
        chunk: &[u8],
        out: &mut Vec<AnthropicSseEvent>,
    ) -> Result<(), SseParseError> {
        for &b in chunk {
            if b == b'\n' {
                // Strip optional trailing \r (CRLF tolerance).
                let line = if self.line_buf.last() == Some(&b'\r') {
                    &self.line_buf[..self.line_buf.len() - 1]
                } else {
                    &self.line_buf[..]
                };
                let line_str = std::str::from_utf8(line).map_err(|_| SseParseError::InvalidUtf8)?;
                if line_str.is_empty() {
                    // Event boundary.
                    if !self.data_buf.is_empty() {
                        let ev = parse_event_payload(&self.data_buf)?;
                        out.push(ev);
                    }
                    self.current_event_name = None;
                    self.data_buf.clear();
                } else if let Some(rest) = line_str.strip_prefix("event:") {
                    self.current_event_name = Some(rest.trim().to_string());
                } else if let Some(rest) = line_str.strip_prefix("data:") {
                    // Per RFC, trim a single leading space.
                    let d = rest.strip_prefix(' ').unwrap_or(rest);
                    if self.data_buf.len() + d.len() > MAX_EVENT_BYTES {
                        return Err(SseParseError::EventTooLarge);
                    }
                    if !self.data_buf.is_empty() {
                        self.data_buf.push('\n');
                    }
                    self.data_buf.push_str(d);
                } else if line_str.starts_with(':') {
                    // SSE comment (Cloudflare etc.).  Ignore.
                } else {
                    // Unknown field per RFC : silently ignore.
                }
                self.line_buf.clear();
            } else {
                if self.line_buf.len() >= MAX_LINE_BYTES {
                    return Err(SseParseError::LineTooLong);
                }
                self.line_buf.push(b);
            }
        }
        Ok(())
    }
}

fn parse_event_payload(data: &str) -> Result<AnthropicSseEvent, SseParseError> {
    serde_json::from_str(data).map_err(|e| SseParseError::Json(e.to_string()))
}

// ─────────────────────────────────────────────────────────────────────
//  Stream adapter
// ─────────────────────────────────────────────────────────────────────

/// **N27** : wrap a `reqwest::Response::bytes_stream` into a
/// `Stream<Item = Result<AnthropicSseEvent, SseParseError>>`.
///
/// Each input chunk may produce 0..N events ; the wrapper
/// buffers them and yields one per `poll_next`.
pub fn parse_sse_stream<S>(body: S) -> SseEventStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    SseEventStream {
        body,
        parser: SseParser::new(),
        buffered: Vec::new(),
        finished: false,
    }
}

pub struct SseEventStream<S> {
    body: S,
    parser: SseParser,
    buffered: Vec<AnthropicSseEvent>,
    finished: bool,
}

impl<S> Stream for SseEventStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    type Item = Result<AnthropicSseEvent, SseParseError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if !self.buffered.is_empty() {
                return Poll::Ready(Some(Ok(self.buffered.remove(0))));
            }
            if self.finished {
                return Poll::Ready(None);
            }
            match Pin::new(&mut self.body).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    self.finished = true;
                    if self.buffered.is_empty() {
                        return Poll::Ready(None);
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(SseParseError::Network(e.to_string()))));
                }
                Poll::Ready(Some(Ok(chunk))) => {
                    let mut new_events = Vec::new();
                    if let Err(e) = self.parser.feed(&chunk, &mut new_events) {
                        return Poll::Ready(Some(Err(e)));
                    }
                    self.buffered.extend(new_events);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_to_events(input: &[u8]) -> Result<Vec<AnthropicSseEvent>, SseParseError> {
        let mut parser = SseParser::new();
        let mut out = Vec::new();
        parser.feed(input, &mut out)?;
        Ok(out)
    }

    #[test]
    fn parse_single_message_start_event() {
        let input = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":42,\"output_tokens\":1}}}\n\n";
        let events = feed_to_events(input).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].label(), "message_start");
    }

    #[test]
    fn parse_three_events_in_one_chunk() {
        let input = b"event: ping\ndata: {\"type\":\"ping\"}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\n";
        let events = feed_to_events(input).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], AnthropicSseEvent::Ping);
        assert_eq!(events[1], AnthropicSseEvent::MessageStop);
        assert_eq!(events[2], AnthropicSseEvent::Ping);
    }

    #[test]
    fn split_event_across_chunks() {
        let mut parser = SseParser::new();
        let mut out = Vec::new();
        parser.feed(b"event: ping\ndata: {\"type\":", &mut out).unwrap();
        assert!(out.is_empty());
        parser.feed(b"\"ping\"}\n", &mut out).unwrap();
        assert!(out.is_empty());
        parser.feed(b"\n", &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], AnthropicSseEvent::Ping);
    }

    #[test]
    fn ignores_cloudflare_comment_lines() {
        let input =
            b": cf-cache-bypass\n: another\nevent: ping\ndata: {\"type\":\"ping\"}\n\n";
        let events = feed_to_events(input).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn ignores_unknown_field_per_rfc() {
        let input = b"id: 12\nretry: 500\nevent: ping\ndata: {\"type\":\"ping\"}\n\n";
        let events = feed_to_events(input).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn handles_crlf_line_endings() {
        let input = b"event: ping\r\ndata: {\"type\":\"ping\"}\r\n\r\n";
        let events = feed_to_events(input).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn rejects_oversized_line() {
        let mut huge = vec![b'x'; MAX_LINE_BYTES + 1];
        huge.push(b'\n');
        let r = feed_to_events(&huge);
        assert_eq!(r.unwrap_err(), SseParseError::LineTooLong);
    }

    #[test]
    fn rejects_oversized_event_data() {
        // Build many `data:` lines that together exceed MAX_EVENT_BYTES.
        let chunk = "x".repeat(1024);
        let line = format!("data: {chunk}\n");
        let times = (MAX_EVENT_BYTES / 1024) + 2;
        let mut input = String::with_capacity(line.len() * times);
        for _ in 0..times {
            input.push_str(&line);
        }
        let r = feed_to_events(input.as_bytes());
        assert_eq!(r.unwrap_err(), SseParseError::EventTooLarge);
    }

    #[test]
    fn rejects_invalid_json_data() {
        let input = b"event: ping\ndata: {not json\n\n";
        let r = feed_to_events(input);
        assert!(matches!(r, Err(SseParseError::Json(_))));
    }

    #[test]
    fn multi_line_data_joined_by_newlines() {
        // Per RFC, multiple `data:` fields are concatenated with \n.
        // Anthropic emits one JSON per event though, so this is a
        // corner case in the parser ; we test it works.
        let input = b"data: {\"type\":\ndata: \"ping\"}\n\n";
        let events = feed_to_events(input).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], AnthropicSseEvent::Ping);
    }

    #[test]
    fn blank_line_with_no_data_ignored() {
        // Trailing extra `\n\n` should not synthesize empty event.
        let input = b"\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\n\n\n";
        let events = feed_to_events(input).unwrap();
        assert_eq!(events.len(), 1);
    }
}
