// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : Anthropic SSE event types.
//! See architecture § 3.3 (event taxonomy) and § 8.4 (Rust shape).

use serde::Deserialize;

/// **N27 + § 3.3** : a single Anthropic SSE event after JSON
/// parsing.  The tag is `type` so we get an exact 1:1 with the
/// wire shape.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicSseEvent {
    /// First event of any stream.  Carries `message.id` +
    /// `message.usage.input_tokens` (final input count).
    MessageStart { message: AnthropicMessage },
    /// A new content block began at `index`.  Variant of
    /// [`AnthropicContentBlock`] dictates what comes next :
    /// text deltas vs input-JSON deltas.
    ContentBlockStart {
        index: u32,
        content_block: AnthropicContentBlock,
    },
    /// Incremental delta for the block at `index`.
    ContentBlockDelta { index: u32, delta: AnthropicDelta },
    /// Block at `index` is complete.
    ContentBlockStop { index: u32 },
    /// Penultimate event : carries the final `stop_reason` +
    /// final `usage.output_tokens`.
    MessageDelta {
        delta: AnthropicMessageDelta,
        usage: AnthropicMessageDeltaUsage,
    },
    /// Last event ; closes the stream.
    MessageStop,
    /// Heartbeat ; ignored by the bridge (ADR-N27-6).
    Ping,
    /// Mid-stream terminal error.  Carries Anthropic's `error.type`
    /// + human-readable `message`.
    Error { error: AnthropicError },
}

impl AnthropicSseEvent {
    /// **N27** : stable label for metrics / logs.
    pub fn label(&self) -> &'static str {
        match self {
            Self::MessageStart { .. } => "message_start",
            Self::ContentBlockStart { .. } => "content_block_start",
            Self::ContentBlockDelta { .. } => "content_block_delta",
            Self::ContentBlockStop { .. } => "content_block_stop",
            Self::MessageDelta { .. } => "message_delta",
            Self::MessageStop => "message_stop",
            Self::Ping => "ping",
            Self::Error { .. } => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnthropicMessage {
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub usage: AnthropicUsage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct AnthropicUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnthropicMessageDelta {
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub stop_sequence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct AnthropicMessageDeltaUsage {
    pub output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnthropicError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_start() {
        let j = r#"{
            "type":"message_start",
            "message":{
                "id":"msg_01ABC",
                "model":"claude-sonnet-4-6",
                "usage":{"input_tokens":1247,"output_tokens":1}
            }
        }"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        match ev {
            AnthropicSseEvent::MessageStart { message } => {
                assert_eq!(message.id, "msg_01ABC");
                assert_eq!(message.usage.input_tokens, 1247);
            }
            _ => panic!("expected MessageStart"),
        }
    }

    #[test]
    fn parse_content_block_start_text() {
        let j = r#"{
            "type":"content_block_start",
            "index":0,
            "content_block":{"type":"text","text":""}
        }"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        match ev {
            AnthropicSseEvent::ContentBlockStart { index, content_block } => {
                assert_eq!(index, 0);
                assert!(matches!(content_block, AnthropicContentBlock::Text { .. }));
            }
            _ => panic!("expected ContentBlockStart"),
        }
    }

    #[test]
    fn parse_content_block_start_tool_use() {
        let j = r#"{
            "type":"content_block_start",
            "index":1,
            "content_block":{
                "type":"tool_use",
                "id":"toolu_01XYZ",
                "name":"swarm_context",
                "input":{}
            }
        }"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        match ev {
            AnthropicSseEvent::ContentBlockStart { index, content_block } => {
                assert_eq!(index, 1);
                match content_block {
                    AnthropicContentBlock::ToolUse { id, name, .. } => {
                        assert_eq!(id, "toolu_01XYZ");
                        assert_eq!(name, "swarm_context");
                    }
                    _ => panic!("expected ToolUse"),
                }
            }
            _ => panic!("expected ContentBlockStart"),
        }
    }

    #[test]
    fn parse_content_block_delta_text() {
        let j = r#"{
            "type":"content_block_delta",
            "index":0,
            "delta":{"type":"text_delta","text":"Hello"}
        }"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        match ev {
            AnthropicSseEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 0);
                match delta {
                    AnthropicDelta::TextDelta { text } => assert_eq!(text, "Hello"),
                    _ => panic!("expected TextDelta"),
                }
            }
            _ => panic!("expected ContentBlockDelta"),
        }
    }

    #[test]
    fn parse_content_block_delta_input_json() {
        let j = r#"{
            "type":"content_block_delta",
            "index":1,
            "delta":{"type":"input_json_delta","partial_json":"{\"depth\":\""}
        }"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        match ev {
            AnthropicSseEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 1);
                match delta {
                    AnthropicDelta::InputJsonDelta { partial_json } => {
                        assert_eq!(partial_json, "{\"depth\":\"");
                    }
                    _ => panic!("expected InputJsonDelta"),
                }
            }
            _ => panic!("expected ContentBlockDelta"),
        }
    }

    #[test]
    fn parse_content_block_stop() {
        let j = r#"{"type":"content_block_stop","index":0}"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        assert_eq!(
            ev,
            AnthropicSseEvent::ContentBlockStop { index: 0 }
        );
    }

    #[test]
    fn parse_message_delta_with_usage() {
        let j = r#"{
            "type":"message_delta",
            "delta":{"stop_reason":"end_turn","stop_sequence":null},
            "usage":{"output_tokens":342}
        }"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        match ev {
            AnthropicSseEvent::MessageDelta { delta, usage } => {
                assert_eq!(delta.stop_reason.as_deref(), Some("end_turn"));
                assert_eq!(usage.output_tokens, 342);
            }
            _ => panic!("expected MessageDelta"),
        }
    }

    #[test]
    fn parse_message_stop() {
        let j = r#"{"type":"message_stop"}"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        assert_eq!(ev, AnthropicSseEvent::MessageStop);
    }

    #[test]
    fn parse_ping() {
        let j = r#"{"type":"ping"}"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        assert_eq!(ev, AnthropicSseEvent::Ping);
    }

    #[test]
    fn parse_error_event() {
        let j = r#"{
            "type":"error",
            "error":{"type":"overloaded_error","message":"Overloaded"}
        }"#;
        let ev: AnthropicSseEvent = serde_json::from_str(j).unwrap();
        match ev {
            AnthropicSseEvent::Error { error } => {
                assert_eq!(error.error_type, "overloaded_error");
                assert_eq!(error.message, "Overloaded");
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn label_stable_across_variants() {
        assert_eq!(AnthropicSseEvent::MessageStop.label(), "message_stop");
        assert_eq!(AnthropicSseEvent::Ping.label(), "ping");
    }
}
