"""Deterministic mock LLM provider — used in tests and offline CI.

The mock's behaviour is entirely driven by the input conversation:

* If the last user message starts with ``"plan:"``, the mock emits a
  single tool-use that the *user message suffix* describes, e.g.
  ``"plan: swarm.classify tenant=acme task=t1 cone_version=1 ..."``.

* If the conversation contains any ``tool`` role messages, we interpret
  the situation as a Verify phase and emit a Verify-style response with
  the canonical ``<json>...</json>`` recap block.

* Otherwise, we reply conversationally with a short canned greeting.

Token counts are deterministic: ``len(text) // 4 + 1`` per turn.
"""

from __future__ import annotations

import json
import uuid
from collections.abc import Mapping, Sequence
from typing import Any

from efe_platform.bot.planner import LlmProvider, LlmTurn, PlannedCall
from efe_platform.bot.tool_registry import ToolSchema


class MockProvider(LlmProvider):
    """Deterministic provider with zero I/O."""

    def __init__(
        self,
        *,
        price_per_token_usd: float = 0.0,
        force_kind: str | None = None,
    ) -> None:
        self._price = price_per_token_usd
        self._force_kind = force_kind

    async def run(
        self,
        *,
        phase: str,
        system: str,
        messages: Sequence[Mapping[str, Any]],
        tool_schemas: Sequence[ToolSchema],
        max_tokens: int,
    ) -> LlmTurn:
        if phase == "plan":
            return self._plan(messages, tool_schemas, max_tokens)
        if phase == "verify":
            return self._verify(messages, max_tokens)
        raise ValueError(f"unknown phase {phase!r}")

    # --------------------------------------------------------- Plan phase

    def _plan(
        self,
        messages: Sequence[Mapping[str, Any]],
        tool_schemas: Sequence[ToolSchema],
        max_tokens: int,
    ) -> LlmTurn:
        # Find the last *user* message.
        last_user = next(
            (m for m in reversed(messages) if m["role"] == "user"),
            None,
        )
        text = str(last_user["content"]) if last_user else ""
        if self._force_kind == "conversational":
            return _wrap(_CANNED_CONVERSATIONAL_REPLY, (), self._price)
        if text.startswith("plan:"):
            # Parse "plan: tool_name key=value key=value ..." form.
            calls = _parse_mock_plan(text, tool_schemas=tool_schemas)
            return _wrap("", calls, self._price, finish_reason="tool_use")
        if self._force_kind == "tool_uses" and tool_schemas:
            # Synthesize a trivial call to the first registered tool so
            # the bot loop has something to execute even without a
            # `plan:` prefix in the user message.
            first = tool_schemas[0]
            call = PlannedCall(
                tool_use_id=str(uuid.uuid4()),
                tool_name=first.name,
                arguments={},
            )
            return _wrap("", (call,), self._price, finish_reason="tool_use")
        # Default: conversational passthrough.
        return _wrap(_CANNED_CONVERSATIONAL_REPLY, (), self._price)

    # ------------------------------------------------------- Verify phase

    def _verify(
        self,
        messages: Sequence[Mapping[str, Any]],
        max_tokens: int,
    ) -> LlmTurn:
        tool_msgs = [m for m in messages if m["role"] == "tool"]
        if not tool_msgs:
            body = (
                "Je n'ai pas d'information supplémentaire à partager. "
                "<json>{\"verdict\":\"unknown\",\"confidence\":0.1,\"notes\":[]}</json>"
            )
            return _wrap(body, (), self._price)
        # Collapse tool results into a short summary.
        summaries: list[str] = []
        verdict_label = "ok"
        confidence = 0.8
        for t in tool_msgs:
            try:
                payload = json.loads(t["content"])
            except Exception:
                payload = {"raw": t["content"]}
            summaries.append(f"{t['tool_name']}→{payload.get('verdict', '?')}")
            c = payload.get("confidence")
            if isinstance(c, (int, float)):
                confidence = float(c)
            if payload.get("verdict") in ("DIVERGENCE", "MODE_COLLAPSE"):
                verdict_label = "uncertain"
                confidence = min(confidence, 0.3)
        recap_json = json.dumps(
            {
                "verdict": verdict_label,
                "confidence": confidence,
                "notes": summaries,
            },
            ensure_ascii=False,
        )
        body = f"Résultat: {', '.join(summaries)}. <json>{recap_json}</json>"
        return _wrap(body, (), self._price)


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


_CANNED_CONVERSATIONAL_REPLY = (
    "Bonjour — je suis le coordinateur EFE. Dites-moi ce que vous "
    "voulez classer ou router, et je délègue à l'essaim."
)


def _parse_mock_plan(
    text: str, *, tool_schemas: Sequence[ToolSchema]
) -> tuple[PlannedCall, ...]:
    """Parse a ``plan: tool_name k=v ...`` line into one :class:`PlannedCall`."""
    # Tolerate optional multi-line; take the first non-empty line.
    line = next((ln for ln in text.splitlines() if ln.strip()), "")
    if not line.startswith("plan:"):
        return ()
    body = line[len("plan:") :].strip()
    if not body:
        return ()
    parts = body.split()
    tool_name = parts[0]
    # Only route to known tools.
    known = {s.name for s in tool_schemas} if tool_schemas else set()
    if known and tool_name not in known:
        return ()
    args: dict[str, Any] = {}
    for kv in parts[1:]:
        if "=" not in kv:
            continue
        k, _, v = kv.partition("=")
        args[k] = _coerce_value(v)
    return (
        PlannedCall(
            tool_use_id=str(uuid.uuid4()),
            tool_name=tool_name,
            arguments=args,
        ),
    )


def _coerce_value(v: str) -> Any:
    # Arrays: [1.0,2.0,3.0]
    if v.startswith("[") and v.endswith("]"):
        return [_coerce_value(x) for x in v[1:-1].split(",") if x]
    # Booleans
    low = v.lower()
    if low in {"true", "false"}:
        return low == "true"
    # Numbers
    try:
        return int(v)
    except ValueError:
        pass
    try:
        return float(v)
    except ValueError:
        pass
    # Strings (strip quotes if present)
    return v.strip('"').strip("'")


def _wrap(
    text: str,
    calls: tuple[PlannedCall, ...],
    price: float,
    *,
    finish_reason: str = "stop",
) -> LlmTurn:
    tokens = max(1, len(text) // 4 + 2 * len(calls))
    return LlmTurn(
        text=text,
        tool_uses=calls,
        tokens_used=tokens,
        usd_used=tokens * price,
        finish_reason=finish_reason,
    )


__all__ = ["MockProvider"]
