// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : `StreamState` accumulator.
//! See architecture § 3.7 + § 8.5.

use std::collections::BTreeMap;
use std::time::Instant;

use efe_nano_sdk::AbortReason;

use super::event::{
    AnthropicContentBlock, AnthropicDelta, AnthropicSseEvent, AnthropicUsage,
};
use super::{MAX_PER_BLOCK_BYTES, MAX_TOOL_USE_JSON_BYTES};
use crate::error::BridgeError;
use crate::task::{AgenticEvent, StopReason};

/// **N27** : per-block accumulator.
#[derive(Debug, Clone)]
pub enum BlockBuilder {
    Text {
        buffer: String,
    },
    ToolUse {
        id: String,
        name: String,
        json_buffer: String,
        parsed: Option<serde_json::Value>,
    },
}

impl BlockBuilder {
    /// **N27** : called on `content_block_stop`.  For tool-use
    /// blocks this parses the accumulated JSON.
    pub fn finalize(&mut self) -> Result<(), BridgeError> {
        if let BlockBuilder::ToolUse {
            json_buffer,
            parsed,
            ..
        } = self
        {
            let value: serde_json::Value =
                serde_json::from_str(json_buffer).map_err(|e| BridgeError::ToolUseJsonInvalid {
                    reason: e.to_string(),
                })?;
            *parsed = Some(value);
        }
        Ok(())
    }
}

/// **N27** : streaming state for a single turn.
pub struct StreamState {
    /// 1 or 2 (which turn of the two-turn protocol).
    pub turn: u8,
    pub message_id: Option<String>,
    pub input_tokens: u32,
    pub final_output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
    pub blocks: BTreeMap<u32, BlockBuilder>,
    pub final_stop_reason: Option<StopReason>,
    pub started_at: Instant,
    pub first_text_delta_at: Option<Instant>,
    pub ended_at: Option<Instant>,
    pub finalized: bool,
    pub abort_reason: Option<AbortReason>,
    pub error_event_type: Option<String>,
}

impl StreamState {
    pub fn new(turn: u8) -> Self {
        Self {
            turn,
            message_id: None,
            input_tokens: 0,
            final_output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            blocks: BTreeMap::new(),
            final_stop_reason: None,
            started_at: Instant::now(),
            first_text_delta_at: None,
            ended_at: None,
            finalized: false,
            abort_reason: None,
            error_event_type: None,
        }
    }

    /// **N27** : compute total accumulated text in `index` order.
    pub fn accumulated_text(&self) -> String {
        let mut out = String::new();
        for b in self.blocks.values() {
            if let BlockBuilder::Text { buffer } = b {
                out.push_str(buffer);
            }
        }
        out
    }

    /// **N27** : extract parsed tool-use blocks (after finalize).
    pub fn tool_uses(&self) -> Vec<(String, String, serde_json::Value)> {
        let mut out = Vec::new();
        for b in self.blocks.values() {
            if let BlockBuilder::ToolUse {
                id,
                name,
                parsed: Some(v),
                ..
            } = b
            {
                out.push((id.clone(), name.clone(), v.clone()));
            }
        }
        out
    }

    /// **N27** : ms since `started_at`, capped at u32::MAX.
    pub fn elapsed_ms(&self) -> u32 {
        let end = self.ended_at.unwrap_or_else(Instant::now);
        end.saturating_duration_since(self.started_at).as_millis() as u32
    }

    /// **N27** : TTFT for this turn, if a text delta was seen.
    pub fn ttft_ms(&self) -> Option<u32> {
        self.first_text_delta_at.map(|t| {
            t.saturating_duration_since(self.started_at).as_millis() as u32
        })
    }

    /// **N27** : apply an SSE event.  Returns an optional
    /// `AgenticEvent` to be forwarded on the bridge's mpsc.
    /// Returns `Err` only on fatal protocol violations (which
    /// should map to `Completeness::Errored`).
    pub fn apply(
        &mut self,
        ev: AnthropicSseEvent,
    ) -> Result<Option<AgenticEvent>, BridgeError> {
        match ev {
            AnthropicSseEvent::MessageStart { message } => {
                self.message_id = Some(message.id.clone());
                self.input_tokens = message.usage.input_tokens;
                self.cache_creation_input_tokens = message.usage.cache_creation_input_tokens;
                self.cache_read_input_tokens = message.usage.cache_read_input_tokens;
                Ok(Some(AgenticEvent::MessageStart {
                    turn: self.turn,
                    provider_request_id: Some(message.id),
                    input_tokens: message.usage.input_tokens,
                }))
            }
            AnthropicSseEvent::ContentBlockStart { index, content_block } => {
                match content_block {
                    AnthropicContentBlock::Text { text } => {
                        self.blocks.insert(
                            index,
                            BlockBuilder::Text { buffer: text },
                        );
                        Ok(None)
                    }
                    AnthropicContentBlock::ToolUse { id, name, .. } => {
                        let started = AgenticEvent::ToolUseStart {
                            tool_id: id.clone(),
                            tool_name: name.clone(),
                        };
                        self.blocks.insert(
                            index,
                            BlockBuilder::ToolUse {
                                id,
                                name,
                                json_buffer: String::new(),
                                parsed: None,
                            },
                        );
                        Ok(Some(started))
                    }
                }
            }
            AnthropicSseEvent::ContentBlockDelta { index, delta } => match delta {
                AnthropicDelta::TextDelta { text } => {
                    if self.first_text_delta_at.is_none() {
                        self.first_text_delta_at = Some(Instant::now());
                    }
                    if let Some(BlockBuilder::Text { buffer }) = self.blocks.get_mut(&index) {
                        if buffer.len() + text.len() > MAX_PER_BLOCK_BYTES {
                            return Err(BridgeError::SseParseError {
                                reason: "text block exceeded MAX_PER_BLOCK_BYTES".into(),
                            });
                        }
                        buffer.push_str(&text);
                        Ok(Some(AgenticEvent::TextDelta(text)))
                    } else {
                        Err(BridgeError::SseParseError {
                            reason: format!("text_delta on non-text block index={index}"),
                        })
                    }
                }
                AnthropicDelta::InputJsonDelta { partial_json } => {
                    if let Some(BlockBuilder::ToolUse { json_buffer, .. }) =
                        self.blocks.get_mut(&index)
                    {
                        if json_buffer.len() + partial_json.len() > MAX_TOOL_USE_JSON_BYTES {
                            return Err(BridgeError::SseParseError {
                                reason: "tool_use JSON exceeded MAX_TOOL_USE_JSON_BYTES"
                                    .into(),
                            });
                        }
                        json_buffer.push_str(&partial_json);
                        Ok(Some(AgenticEvent::ToolUseDelta(partial_json)))
                    } else {
                        Err(BridgeError::SseParseError {
                            reason: format!(
                                "input_json_delta on non-tool_use block index={index}"
                            ),
                        })
                    }
                }
            },
            AnthropicSseEvent::ContentBlockStop { index } => {
                if let Some(b) = self.blocks.get_mut(&index) {
                    b.finalize()?;
                    if let BlockBuilder::ToolUse {
                        id, name, parsed, ..
                    } = b
                    {
                        if let Some(v) = parsed.clone() {
                            return Ok(Some(AgenticEvent::ToolUseComplete {
                                tool_id: id.clone(),
                                tool_name: name.clone(),
                                input: v,
                            }));
                        }
                    }
                    Ok(None)
                } else {
                    Err(BridgeError::SseParseError {
                        reason: format!("content_block_stop unknown index={index}"),
                    })
                }
            }
            AnthropicSseEvent::MessageDelta { delta, usage } => {
                if let Some(s) = delta.stop_reason {
                    self.final_stop_reason = Some(StopReason::from_anthropic(&s));
                }
                self.final_output_tokens = usage.output_tokens;
                Ok(None)
            }
            AnthropicSseEvent::MessageStop => {
                self.ended_at = Some(Instant::now());
                self.finalized = true;
                Ok(None)
            }
            AnthropicSseEvent::Ping => Ok(None),
            AnthropicSseEvent::Error { error } => {
                self.ended_at = Some(Instant::now());
                self.error_event_type = Some(error.error_type.clone());
                self.abort_reason = Some(AbortReason::UpstreamError {
                    error_type: error.error_type.clone(),
                });
                Err(BridgeError::HttpStatus {
                    status: 0, // not an HTTP status — mid-stream error
                    body: format!("anthropic error : {} : {}", error.error_type, error.message),
                })
            }
        }
    }

    /// **N27** : convert the running usage into the bridge's
    /// `TokenUsage` shape.  Combines `MessageStart` input and
    /// `MessageDelta` output.
    pub fn token_usage(&self) -> crate::task::TokenUsage {
        crate::task::TokenUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.final_output_tokens,
        }
    }

    /// **N27** : Anthropic-compatible `AnthropicUsage` shape.
    pub fn anthropic_usage(&self) -> AnthropicUsage {
        AnthropicUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.final_output_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_stream::event::{
        AnthropicMessage, AnthropicMessageDelta, AnthropicMessageDeltaUsage,
    };

    fn msg_start(input: u32) -> AnthropicSseEvent {
        AnthropicSseEvent::MessageStart {
            message: AnthropicMessage {
                id: "msg_1".into(),
                model: "claude-sonnet-4-6".into(),
                usage: AnthropicUsage {
                    input_tokens: input,
                    output_tokens: 1,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
            },
        }
    }

    #[test]
    fn single_text_block_accumulation() {
        let mut s = StreamState::new(1);
        s.apply(msg_start(42)).unwrap();
        s.apply(AnthropicSseEvent::ContentBlockStart {
            index: 0,
            content_block: AnthropicContentBlock::Text { text: String::new() },
        })
        .unwrap();
        let r = s
            .apply(AnthropicSseEvent::ContentBlockDelta {
                index: 0,
                delta: AnthropicDelta::TextDelta {
                    text: "Hello ".into(),
                },
            })
            .unwrap()
            .unwrap();
        assert!(matches!(r, AgenticEvent::TextDelta(_)));
        s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 0,
            delta: AnthropicDelta::TextDelta {
                text: "world!".into(),
            },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::ContentBlockStop { index: 0 })
            .unwrap();
        assert_eq!(s.accumulated_text(), "Hello world!");
        assert!(s.first_text_delta_at.is_some());
    }

    #[test]
    fn message_delta_captures_stop_and_usage() {
        let mut s = StreamState::new(1);
        s.apply(msg_start(50)).unwrap();
        s.apply(AnthropicSseEvent::MessageDelta {
            delta: AnthropicMessageDelta {
                stop_reason: Some("end_turn".into()),
                stop_sequence: None,
            },
            usage: AnthropicMessageDeltaUsage { output_tokens: 99 },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::MessageStop).unwrap();
        assert!(s.finalized);
        assert_eq!(s.final_stop_reason, Some(StopReason::EndTurn));
        assert_eq!(s.final_output_tokens, 99);
        let u = s.token_usage();
        assert_eq!(u.input_tokens, 50);
        assert_eq!(u.output_tokens, 99);
    }

    #[test]
    fn tool_use_block_assembled_and_parsed() {
        let mut s = StreamState::new(1);
        s.apply(msg_start(100)).unwrap();
        s.apply(AnthropicSseEvent::ContentBlockStart {
            index: 1,
            content_block: AnthropicContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "swarm_context".into(),
                input: serde_json::json!({}),
            },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 1,
            delta: AnthropicDelta::InputJsonDelta {
                partial_json: "{\"depth\":\"".into(),
            },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 1,
            delta: AnthropicDelta::InputJsonDelta {
                partial_json: "minimal\"}".into(),
            },
        })
        .unwrap();
        let r = s
            .apply(AnthropicSseEvent::ContentBlockStop { index: 1 })
            .unwrap()
            .unwrap();
        match r {
            AgenticEvent::ToolUseComplete { tool_name, input, .. } => {
                assert_eq!(tool_name, "swarm_context");
                assert_eq!(input, serde_json::json!({"depth":"minimal"}));
            }
            _ => panic!("expected ToolUseComplete"),
        }
        let tu = s.tool_uses();
        assert_eq!(tu.len(), 1);
        assert_eq!(tu[0].1, "swarm_context");
    }

    #[test]
    fn tool_use_invalid_json_errors() {
        let mut s = StreamState::new(1);
        s.apply(msg_start(100)).unwrap();
        s.apply(AnthropicSseEvent::ContentBlockStart {
            index: 1,
            content_block: AnthropicContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "swarm_context".into(),
                input: serde_json::json!({}),
            },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 1,
            delta: AnthropicDelta::InputJsonDelta {
                partial_json: "{not valid".into(),
            },
        })
        .unwrap();
        let r = s.apply(AnthropicSseEvent::ContentBlockStop { index: 1 });
        assert!(matches!(r, Err(BridgeError::ToolUseJsonInvalid { .. })));
    }

    #[test]
    fn error_event_records_abort_reason() {
        let mut s = StreamState::new(1);
        s.apply(msg_start(100)).unwrap();
        let r = s.apply(AnthropicSseEvent::Error {
            error: crate::claude_stream::event::AnthropicError {
                error_type: "overloaded_error".into(),
                message: "Overloaded".into(),
            },
        });
        assert!(matches!(r, Err(BridgeError::HttpStatus { .. })));
        assert!(matches!(
            s.abort_reason,
            Some(AbortReason::UpstreamError { .. })
        ));
        assert_eq!(s.error_event_type.as_deref(), Some("overloaded_error"));
    }

    #[test]
    fn text_block_size_cap_rejects_overflow() {
        let mut s = StreamState::new(1);
        s.apply(msg_start(0)).unwrap();
        s.apply(AnthropicSseEvent::ContentBlockStart {
            index: 0,
            content_block: AnthropicContentBlock::Text { text: String::new() },
        })
        .unwrap();
        let big = "x".repeat(MAX_PER_BLOCK_BYTES + 1);
        let r = s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 0,
            delta: AnthropicDelta::TextDelta { text: big },
        });
        assert!(matches!(r, Err(BridgeError::SseParseError { .. })));
    }

    #[test]
    fn ttft_captured_on_first_text_delta() {
        let mut s = StreamState::new(1);
        s.apply(msg_start(10)).unwrap();
        assert!(s.ttft_ms().is_none());
        s.apply(AnthropicSseEvent::ContentBlockStart {
            index: 0,
            content_block: AnthropicContentBlock::Text { text: String::new() },
        })
        .unwrap();
        // Add a tiny sleep so the delta isn't at the same instant.
        std::thread::sleep(std::time::Duration::from_millis(2));
        s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 0,
            delta: AnthropicDelta::TextDelta { text: "h".into() },
        })
        .unwrap();
        assert!(s.ttft_ms().is_some());
        // Subsequent deltas don't change TTFT.
        let first = s.first_text_delta_at;
        s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 0,
            delta: AnthropicDelta::TextDelta { text: "i".into() },
        })
        .unwrap();
        assert_eq!(s.first_text_delta_at, first);
    }

    #[test]
    fn ping_is_no_op() {
        let mut s = StreamState::new(1);
        let r = s.apply(AnthropicSseEvent::Ping).unwrap();
        assert!(r.is_none());
        assert!(!s.finalized);
    }
}
