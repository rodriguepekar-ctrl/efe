"""LLM orchestration layer — the "bot" that drives the swarm.

The bot's contract: take a natural-language user prompt, produce a
final answer **and** a structured machine-readable outcome, while
respecting a hard budget and producing a deterministic audit trail.

Exposed names:

* :class:`Bot` — top-level entry point.
* :class:`BotConfig` — immutable config.
* :class:`BotReply` — rich return value (final text + trace).
* :class:`TraceEvent` — one step in the :attr:`BotReply.trace`.
"""

from __future__ import annotations

from efe_platform.bot.agent import Bot, BotConfig, BotReply, TraceEvent
from efe_platform.bot.budget import Budget, BudgetLedger
from efe_platform.bot.memory import ConversationMemory, MemoryMessage
from efe_platform.bot.planner import PlannedCall, PlanStep, PlanVerdict
from efe_platform.bot.policy import GuardrailResult, PolicyEngine
from efe_platform.bot.tool_registry import Tool, ToolRegistry, ToolResult, ToolSchema

__all__ = [
    "Bot",
    "BotConfig",
    "BotReply",
    "Budget",
    "BudgetLedger",
    "ConversationMemory",
    "GuardrailResult",
    "MemoryMessage",
    "PlanStep",
    "PlanVerdict",
    "PlannedCall",
    "PolicyEngine",
    "Tool",
    "ToolRegistry",
    "ToolResult",
    "ToolSchema",
    "TraceEvent",
]
