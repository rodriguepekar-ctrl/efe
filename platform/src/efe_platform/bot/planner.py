"""Plan / Execute / Verify loop.

The planner is the **engine** the agent drives. Given:

* a :class:`~efe_platform.bot.tool_registry.ToolRegistry`,
* a :class:`~efe_platform.bot.budget.BudgetLedger`,
* a :class:`PolicyEngine`,
* and an :class:`LlmProvider` (supplied by the agent),

it executes a deterministic three-phase state machine:

1. **Plan** — one LLM turn asking "given this user prompt, which
   tools should we call?". The response is parsed to a list of
   :class:`PlannedCall`. If the model produces text without tool-uses,
   we short-circuit and return the text directly (conversational
   passthrough).

2. **Execute** — each :class:`PlannedCall` is dispatched through the
   tool registry, producing :class:`ToolResult` entries appended to the
   conversation memory.

3. **Verify** — one LLM turn that reads the tool results and produces
   the final user-facing answer plus a structured JSON recap extracted
   by :func:`extract_recap`.

The planner itself is I/O-agnostic with respect to the LLM provider —
it takes an :class:`LlmProvider` protocol.
"""

from __future__ import annotations

import json
import re
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any, Literal, Protocol, runtime_checkable

from efe_platform.bot.budget import BudgetLedger
from efe_platform.bot.memory import ConversationMemory
from efe_platform.bot.policy import PolicyEngine
from efe_platform.bot.tool_registry import ToolRegistry, ToolResult, ToolSchema
from efe_platform.errors import VerifyError

# ---------------------------------------------------------------------------
#  Provider protocol
# ---------------------------------------------------------------------------


PhaseKind = Literal["plan", "verify"]


@dataclass(frozen=True, slots=True)
class LlmTurn:
    """Provider-agnostic LLM response."""

    text: str
    tool_uses: tuple[PlannedCall, ...]
    tokens_used: int
    usd_used: float
    finish_reason: str = "stop"


@runtime_checkable
class LlmProvider(Protocol):
    """Narrow contract implemented by each LLM bridge.

    The concrete implementations live in :mod:`efe_platform.bot.providers`
    (mock, claude, openai). Each implementation is responsible for
    translating the **provider-neutral** :class:`ToolSchema` sequence
    into the SDK-specific shape (e.g. Anthropic's ``{name, description,
    input_schema}`` vs OpenAI's ``{type: "function", function: {...}}``).
    """

    async def run(
        self,
        *,
        phase: PhaseKind,
        system: str,
        messages: Sequence[Mapping[str, Any]],
        tool_schemas: Sequence[ToolSchema],
        max_tokens: int,
    ) -> LlmTurn: ...


# ---------------------------------------------------------------------------
#  Planner data types
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class PlannedCall:
    """A single tool-use requested by the LLM."""

    tool_use_id: str
    tool_name: str
    arguments: Mapping[str, Any]

    def to_dict(self) -> dict[str, Any]:
        return {
            "tool_use_id": self.tool_use_id,
            "tool_name": self.tool_name,
            "arguments": dict(self.arguments),
        }


@dataclass(frozen=True, slots=True)
class PlanVerdict:
    """Outcome of the Plan phase."""

    kind: Literal["conversational", "tool_uses"]
    text: str = ""
    calls: tuple[PlannedCall, ...] = ()
    tokens_used: int = 0
    usd_used: float = 0.0


@dataclass(frozen=True, slots=True)
class VerifyVerdict:
    """Outcome of the Verify phase."""

    user_text: str
    recap: Mapping[str, Any]
    tokens_used: int
    usd_used: float


@dataclass(frozen=True, slots=True)
class PlanStep:
    """One record in the planner trace."""

    phase: Literal["plan", "execute", "verify"]
    summary: str
    payload: Mapping[str, Any] = field(default_factory=dict)


# ---------------------------------------------------------------------------
#  Planner
# ---------------------------------------------------------------------------


@dataclass
class Planner:
    """Orchestrates the PEV loop.

    The planner emits :class:`PlanStep` events through an optional
    callback — the agent wires this to the audit log + tracing spans.
    """

    provider: LlmProvider
    registry: ToolRegistry
    policy: PolicyEngine
    plan_system_prompt: str
    verify_system_prompt: str
    max_plan_tokens: int = 1024
    max_verify_tokens: int = 1024
    on_step: Callable[[PlanStep], None] | None = None

    # ----------------------------------------------------- Plan phase

    async def plan(
        self,
        *,
        memory: ConversationMemory,
        ledger: BudgetLedger,
        tenant: str,
    ) -> PlanVerdict:
        ledger.check_wall_clock()
        messages = _memory_to_messages(memory)
        turn = await self.provider.run(
            phase="plan",
            system=self.plan_system_prompt,
            messages=messages,
            tool_schemas=self.registry.schemas(),
            max_tokens=self.max_plan_tokens,
        )
        ledger.charge_llm(tokens=turn.tokens_used, usd=turn.usd_used)
        if not turn.tool_uses:
            verdict = PlanVerdict(
                kind="conversational",
                text=turn.text,
                tokens_used=turn.tokens_used,
                usd_used=turn.usd_used,
            )
        else:
            self.policy.check_plan(
                planned_calls=turn.tool_uses,
                registry=self.registry,
                ledger=ledger,
                expected_tenant=tenant,
            )
            verdict = PlanVerdict(
                kind="tool_uses",
                calls=turn.tool_uses,
                tokens_used=turn.tokens_used,
                usd_used=turn.usd_used,
            )
        self._emit(
            PlanStep(
                phase="plan",
                summary=(
                    f"{verdict.kind} ({len(verdict.calls)} calls, "
                    f"{verdict.tokens_used} tokens)"
                ),
                payload={
                    "kind": verdict.kind,
                    "calls": [c.to_dict() for c in verdict.calls],
                    "tokens": verdict.tokens_used,
                    "usd": verdict.usd_used,
                },
            )
        )
        return verdict

    # --------------------------------------------------- Execute phase

    async def execute(
        self,
        calls: Sequence[PlannedCall],
        *,
        memory: ConversationMemory,
        ledger: BudgetLedger,
    ) -> tuple[ToolResult, ...]:
        results: list[ToolResult] = []
        for call in calls:
            ledger.charge_tool_call()
            ledger.check_wall_clock()
            result = await self.registry.call(
                name=call.tool_name,
                tool_use_id=call.tool_use_id,
                args=call.arguments,
            )
            memory.add_tool_result(
                tool_use_id=call.tool_use_id,
                tool_name=call.tool_name,
                result=result.value,
            )
            self._emit(
                PlanStep(
                    phase="execute",
                    summary=(
                        f"tool={call.tool_name} ok={result.ok} "
                        f"dt={result.duration_ms:.1f}ms"
                    ),
                    payload=result.to_dict(),
                )
            )
            results.append(result)
        return tuple(results)

    # ----------------------------------------------------- Verify phase

    async def verify(
        self,
        *,
        memory: ConversationMemory,
        ledger: BudgetLedger,
    ) -> VerifyVerdict:
        ledger.check_wall_clock()
        messages = _memory_to_messages(memory)
        turn = await self.provider.run(
            phase="verify",
            system=self.verify_system_prompt,
            messages=messages,
            tool_schemas=(),
            max_tokens=self.max_verify_tokens,
        )
        ledger.charge_llm(tokens=turn.tokens_used, usd=turn.usd_used)
        user_text, recap = extract_recap(turn.text)
        self.policy.check_output(user_text)
        verdict = VerifyVerdict(
            user_text=user_text,
            recap=recap,
            tokens_used=turn.tokens_used,
            usd_used=turn.usd_used,
        )
        self._emit(
            PlanStep(
                phase="verify",
                summary=f"final len={len(user_text)} verdict={recap.get('verdict')}",
                payload={
                    "verdict": recap.get("verdict"),
                    "confidence": recap.get("confidence"),
                    "notes": recap.get("notes"),
                    "tokens": verdict.tokens_used,
                    "usd": verdict.usd_used,
                },
            )
        )
        return verdict

    # --------------------------------------------------------- Internal

    def _emit(self, step: PlanStep) -> None:
        if self.on_step is not None:
            self.on_step(step)


# ---------------------------------------------------------------------------
#  Recap extraction
# ---------------------------------------------------------------------------

_JSON_BLOCK_RE = re.compile(r"<json>(.*?)</json>", re.DOTALL)


def extract_recap(text: str) -> tuple[str, Mapping[str, Any]]:
    """Split a Verify-phase message into ``(user_text, recap_json)``.

    The Verify prompt instructs the LLM to append a ``<json>…</json>``
    block. We parse it leniently: if absent / malformed, we return a
    permissive recap with ``verdict='unknown'`` rather than failing
    hard, but we raise :class:`VerifyError` if the JSON block exists
    but contains invalid JSON.
    """
    m = _JSON_BLOCK_RE.search(text)
    if not m:
        return text.strip(), {"verdict": "unknown", "confidence": 0.0, "notes": []}
    raw_json = m.group(1).strip()
    before = text[: m.start()].strip()
    after = text[m.end() :].strip()
    user_text = (before + "\n" + after).strip() if after else before
    try:
        recap = json.loads(raw_json)
    except json.JSONDecodeError as exc:
        raise VerifyError(f"invalid JSON recap: {exc.msg}") from exc
    if not isinstance(recap, dict):
        raise VerifyError(f"recap must be an object, got {type(recap).__name__}")
    _sanitize_recap(recap)
    return user_text, recap


def _sanitize_recap(recap: dict[str, Any]) -> None:
    """In-place: coerce defaults, strip anything the platform doesn't ingest."""
    recap.setdefault("verdict", "unknown")
    if not isinstance(recap.get("confidence"), (int, float)):
        recap["confidence"] = 0.0
    recap["confidence"] = max(0.0, min(1.0, float(recap["confidence"])))
    notes = recap.get("notes") or []
    if not isinstance(notes, list):
        raise VerifyError("recap.notes must be a list")
    recap["notes"] = [str(n) for n in notes][:20]


# ---------------------------------------------------------------------------
#  Memory → provider messages
# ---------------------------------------------------------------------------


def _memory_to_messages(memory: ConversationMemory) -> list[dict[str, Any]]:
    """Convert memory to the generic ``role/content`` message list.

    Provider-specific translation (Anthropic tool_use blocks, OpenAI
    function_call) happens inside each provider adapter — the planner
    stays agnostic.
    """
    out: list[dict[str, Any]] = []
    for msg in memory.snapshot():
        if msg.role == "tool":
            out.append(
                {
                    "role": "tool",
                    "tool_use_id": msg.tool_use_id,
                    "tool_name": msg.tool_name,
                    "content": msg.content,
                }
            )
        else:
            out.append({"role": msg.role, "content": msg.content})
    return out


# ---------------------------------------------------------------------------
#  Helper to ensure mypy sees the coroutine typing
# ---------------------------------------------------------------------------


_Callable = Callable[..., Awaitable[Any]]  # re-exported for type context


__all__ = [
    "LlmProvider",
    "LlmTurn",
    "PlanStep",
    "PlanVerdict",
    "PlannedCall",
    "Planner",
    "VerifyVerdict",
    "extract_recap",
]
