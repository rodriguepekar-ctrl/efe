// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : finalize a [`StreamState`] into a
//! ready-to-record summary.  See architecture § 6 + § 19.3.

use efe_nano_sdk::{AbortReason, Completeness};

use super::state::StreamState;
use crate::task::{StopReason, TokenUsage};

/// **N27** : the consolidated result of a single (turn-level)
/// finalize step.  Two-turn flows combine two of these into the
/// final provenance record.
#[derive(Debug, Clone)]
pub struct StreamFinalized {
    pub text_output: String,
    pub tool_uses: Vec<ToolUseFinalized>,
    pub usage: TokenUsage,
    pub stop_reason: StopReason,
    pub completeness: Completeness,
    pub provider_request_id: Option<String>,
    pub latency_ms: u32,
    pub ttft_ms: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ToolUseFinalized {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

impl StreamFinalized {
    /// **N27** : build from a successfully-completed [`StreamState`]
    /// (i.e. `message_stop` was received).
    pub fn from_complete(state: &StreamState, expected_max_tokens: u32) -> Self {
        let stop = state.final_stop_reason.unwrap_or(StopReason::Other);
        let completeness = match stop {
            StopReason::MaxTokens => Completeness::Partial {
                received_tokens: state.final_output_tokens,
                expected_tokens: expected_max_tokens,
            },
            _ => Completeness::Complete,
        };
        Self {
            text_output: state.accumulated_text(),
            tool_uses: state
                .tool_uses()
                .into_iter()
                .map(|(id, name, input)| ToolUseFinalized { id, name, input })
                .collect(),
            usage: state.token_usage(),
            stop_reason: stop,
            completeness,
            provider_request_id: state.message_id.clone(),
            latency_ms: state.elapsed_ms(),
            ttft_ms: state.ttft_ms(),
        }
    }

    /// **N27 + ADR-N27-4** : build for an aborted stream.
    /// Carries `Completeness::Aborted` + a partial token count.
    pub fn from_aborted(state: &StreamState, reason: AbortReason) -> Self {
        let stop = state.final_stop_reason.unwrap_or(StopReason::Other);
        let received = state.final_output_tokens;
        Self {
            text_output: state.accumulated_text(),
            tool_uses: state
                .tool_uses()
                .into_iter()
                .map(|(id, name, input)| ToolUseFinalized { id, name, input })
                .collect(),
            usage: state.token_usage(),
            stop_reason: stop,
            completeness: Completeness::Aborted {
                received_tokens: received,
                reason,
            },
            provider_request_id: state.message_id.clone(),
            latency_ms: state.elapsed_ms(),
            ttft_ms: state.ttft_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_stream::event::{
        AnthropicContentBlock, AnthropicDelta, AnthropicMessage, AnthropicMessageDelta,
        AnthropicMessageDeltaUsage, AnthropicSseEvent, AnthropicUsage,
    };

    fn full_flow_state(input: u32, output: u32, stop: &str, text: &str) -> StreamState {
        let mut s = StreamState::new(1);
        s.apply(AnthropicSseEvent::MessageStart {
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
        })
        .unwrap();
        s.apply(AnthropicSseEvent::ContentBlockStart {
            index: 0,
            content_block: AnthropicContentBlock::Text { text: String::new() },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::ContentBlockDelta {
            index: 0,
            delta: AnthropicDelta::TextDelta { text: text.into() },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::ContentBlockStop { index: 0 }).unwrap();
        s.apply(AnthropicSseEvent::MessageDelta {
            delta: AnthropicMessageDelta {
                stop_reason: Some(stop.into()),
                stop_sequence: None,
            },
            usage: AnthropicMessageDeltaUsage { output_tokens: output },
        })
        .unwrap();
        s.apply(AnthropicSseEvent::MessageStop).unwrap();
        s
    }

    #[test]
    fn from_complete_end_turn_gives_complete() {
        let s = full_flow_state(50, 99, "end_turn", "Hello");
        let f = StreamFinalized::from_complete(&s, 4_096);
        assert_eq!(f.text_output, "Hello");
        assert_eq!(f.usage.input_tokens, 50);
        assert_eq!(f.usage.output_tokens, 99);
        assert_eq!(f.stop_reason, StopReason::EndTurn);
        assert_eq!(f.completeness, Completeness::Complete);
        assert_eq!(f.provider_request_id.as_deref(), Some("msg_1"));
    }

    #[test]
    fn from_complete_max_tokens_gives_partial() {
        let s = full_flow_state(20, 4_096, "max_tokens", "x");
        let f = StreamFinalized::from_complete(&s, 4_096);
        match f.completeness {
            Completeness::Partial { received_tokens, expected_tokens } => {
                assert_eq!(received_tokens, 4_096);
                assert_eq!(expected_tokens, 4_096);
            }
            _ => panic!("expected Partial"),
        }
    }

    #[test]
    fn from_aborted_carries_reason_and_partial_tokens() {
        let s = full_flow_state(20, 17, "end_turn", "He");
        let f = StreamFinalized::from_aborted(&s, AbortReason::ClientDisconnect);
        match f.completeness {
            Completeness::Aborted { received_tokens, reason } => {
                assert_eq!(received_tokens, 17);
                assert_eq!(reason, AbortReason::ClientDisconnect);
            }
            _ => panic!("expected Aborted"),
        }
    }
}
