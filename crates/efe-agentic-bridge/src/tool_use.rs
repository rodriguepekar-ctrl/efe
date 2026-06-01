// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : tool-use schema for Anthropic Messages API.
//!
//! ADR-6 + I11 + I12.  The single tool we expose to Claude is
//! `swarm_context` — its input schema is a structural description
//! of the `ContextBundle`.

use serde::{Deserialize, Serialize};

/// **N21 + I12** : the canonical tool name we expose to Claude.
pub const SWARM_CONTEXT_TOOL_NAME: &str = "swarm_context";

/// **N21** : a tool definition handed to Claude.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

impl ToolDefinition {
    /// **N21 + ADR-6** : the canonical `swarm_context` tool definition.
    ///
    /// The input schema is intentionally a single empty `object` —
    /// the tool takes no arguments from Claude ; the bridge returns
    /// the ContextBundle as the tool_result on Claude's first call.
    pub fn swarm_context() -> Self {
        Self {
            name: SWARM_CONTEXT_TOOL_NAME.into(),
            description: "Returns the pre-analysed structured context bundle produced by the swarm. Includes entities, redactions, classifications, language, summary, and arbitrary extras keyed by domain.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }
}

/// **N21** : a `tool_use` content block from a Claude response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUseBlock {
    /// Claude's id for this tool-use turn.
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

impl ToolUseBlock {
    /// **N21 + I12** : True if this block calls our canonical
    /// `swarm_context` tool.
    pub fn is_swarm_context(&self) -> bool {
        self.name == SWARM_CONTEXT_TOOL_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swarm_context_tool_definition_constant() {
        let t = ToolDefinition::swarm_context();
        assert_eq!(t.name, "swarm_context");
        assert!(!t.description.is_empty());
        assert_eq!(t.input_schema["type"], "object");
    }

    #[test]
    fn tool_definition_serde_round_trip() {
        let t = ToolDefinition::swarm_context();
        let json = serde_json::to_string(&t).unwrap();
        let back: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn tool_use_block_recognises_swarm_context() {
        let b = ToolUseBlock {
            id: "tu_1".into(),
            name: "swarm_context".into(),
            input: serde_json::json!({}),
        };
        assert!(b.is_swarm_context());
    }

    #[test]
    fn tool_use_block_other_name_not_recognised() {
        let b = ToolUseBlock {
            id: "tu_1".into(),
            name: "other_tool".into(),
            input: serde_json::json!({}),
        };
        assert!(!b.is_swarm_context());
    }

    #[test]
    fn swarm_context_tool_name_constant_stable() {
        assert_eq!(SWARM_CONTEXT_TOOL_NAME, "swarm_context");
    }
}
