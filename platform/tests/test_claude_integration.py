"""End-to-end integration proof: Anthropic ``ClaudeProvider`` ↔ EFE swarm.

This test does **not** require an ``ANTHROPIC_API_KEY`` — we intercept
the HTTP calls the ``anthropic`` SDK makes via ``respx``, serve canned
responses in the exact wire format Claude uses (``content`` blocks of
type ``tool_use`` / ``text``, ``usage`` counters) and run the full bot
loop.

What this proves end-to-end
---------------------------

1. :class:`ClaudeProvider` instantiates with only an API key
   (**no SDK compatibility drift** between our adapter and the
   installed ``anthropic`` version).

2. :class:`ClaudeProvider` satisfies the
   :class:`efe_platform.bot.planner.LlmProvider` protocol at runtime.

3. Our :func:`_convert_schemas_anthropic` produces the schema shape the
   Anthropic API accepts (we assert it on the intercepted request).

4. When Claude returns a ``tool_use`` block, the platform:
   - parses it into a :class:`PlannedCall`,
   - routes it through :class:`ToolRegistry` → ``swarm.classify``,
   - hands the result back to Claude in a Verify turn,
   - extracts the ``<json>...</json>`` recap and exits cleanly.

5. The :class:`BudgetLedger` correctly credits the ``input_tokens`` +
   ``output_tokens`` reported by Claude and the USD price we configured.
"""

from __future__ import annotations

import json
from collections.abc import AsyncIterator
from typing import Any

import httpx
import pytest

pytest.importorskip("anthropic")
respx = pytest.importorskip("respx")

from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.bot.budget import Budget
from efe_platform.bot.planner import LlmProvider
from efe_platform.bot.providers.claude import ClaudeProvider, _convert_schemas_anthropic
from efe_platform.bot.tool_registry import ToolSchema
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.protocol import (
    DecodedPayload,
    ExecuteResponse,
    Verdict,
)

# ---------------------------------------------------------------------------
#  Pre-canned Anthropic responses
# ---------------------------------------------------------------------------

_ANTHROPIC_URL = "https://api.anthropic.com/v1/messages"


def _plan_turn_payload() -> dict[str, Any]:
    """Claude's Plan turn: a single ``tool_use`` for ``swarm.classify``."""
    return {
        "id": "msg_plan_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-3-5-sonnet-latest",
        "stop_reason": "tool_use",
        "content": [
            {
                "type": "tool_use",
                "id": "tool_use_001",
                "name": "swarm.classify",
                "input": {
                    "tenant": "acme",
                    "task": "routing",
                    "cone_version": 1,
                    "text": "Je ne peux pas me connecter à mon compte",
                    "problem_embedding": [0.1 * i for i in range(16)],
                    "dim_latent": 16,
                    "budget_ms": 3000,
                },
            }
        ],
        "usage": {"input_tokens": 180, "output_tokens": 60},
    }


def _verify_turn_payload() -> dict[str, Any]:
    """Claude's Verify turn: final user text + ``<json>`` recap block."""
    recap = json.dumps(
        {"verdict": "routed", "confidence": 0.91, "notes": ["CONVERGED"]},
        ensure_ascii=False,
    )
    text = f"Votre ticket a été routé vers l'équipe Authentification.<json>{recap}</json>"
    return {
        "id": "msg_verify_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-3-5-sonnet-latest",
        "stop_reason": "end_turn",
        "content": [{"type": "text", "text": text}],
        "usage": {"input_tokens": 220, "output_tokens": 45},
    }


# ---------------------------------------------------------------------------
#  Schema converter — structural proof
# ---------------------------------------------------------------------------


def test_convert_schemas_anthropic_produces_api_shape() -> None:
    """Regression: the Anthropic API rejects anything other than
    ``{name, description, input_schema}`` — our converter must emit
    exactly those three keys per tool.
    """
    schemas = (
        ToolSchema(
            name="swarm.classify",
            description="Classify text",
            input_schema={
                "type": "object",
                "properties": {"tenant": {"type": "string"}},
                "required": ["tenant"],
            },
        ),
    )
    converted = _convert_schemas_anthropic(schemas)
    assert len(converted) == 1
    entry = converted[0]
    assert set(entry.keys()) == {"name", "description", "input_schema"}
    assert entry["input_schema"]["type"] == "object"
    assert entry["name"] == "swarm.classify"


# ---------------------------------------------------------------------------
#  Protocol conformance
# ---------------------------------------------------------------------------


def test_claude_provider_satisfies_llm_provider_protocol(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """``ClaudeProvider`` must be structurally compatible with the
    :class:`LlmProvider` Protocol used by :class:`Planner`."""
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-test-dummy")
    provider = ClaudeProvider()
    assert isinstance(provider, LlmProvider)


# ---------------------------------------------------------------------------
#  End-to-end: Claude (mocked HTTP) ↔ Swarm (in-memory)
# ---------------------------------------------------------------------------


@pytest.fixture
async def running_swarm() -> AsyncIterator[SwarmClient]:
    """In-memory swarm that returns a CONVERGED verdict for routing queries."""
    transport = InMemorySwarmTransport()
    transport.seed(
        tenant="acme",
        prompt_prefix="Je ne peux pas me connecter",
        response=ExecuteResponse(
            swarm_id="swarm-007",
            request_id="rid-seed",
            payload=DecodedPayload(text="auth_issue"),
            confidence=0.91,
            verdict=Verdict.CONVERGED,
            rounds_executed=4,
            total_ms=18.3,
            alignment_delta=0.06,
        ),
    )
    client = SwarmClient(SwarmEndpoint("in-mem"), transport=transport, max_retries=0)
    async with client:
        yield client


@pytest.mark.asyncio
async def test_claude_plan_execute_verify_end_to_end(
    running_swarm: SwarmClient,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The **proof**: Claude drives the essaim through tool-use."""
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-test-dummy")

    provider = ClaudeProvider(
        model="claude-3-5-sonnet-latest",
        price_per_input_token_usd=3.0 / 1_000_000,
        price_per_output_token_usd=15.0 / 1_000_000,
    )

    bot = Bot(
        BotConfig(budget=Budget(max_wall_clock_ms=5_000, max_llm_tokens=2_000)),
        swarm=running_swarm,
        provider=provider,
    )

    captured_requests: list[dict[str, Any]] = []

    async def _anthropic_handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        captured_requests.append(body)
        # 1st call = Plan turn, 2nd = Verify turn.
        payload = (
            _plan_turn_payload() if len(captured_requests) == 1 else _verify_turn_payload()
        )
        return httpx.Response(200, json=payload)

    with respx.mock(base_url="https://api.anthropic.com") as router:
        router.post("/v1/messages").mock(side_effect=_anthropic_handler)
        reply = await bot.answer(
            "Je ne peux pas me connecter à mon compte",
            tenant="acme",
        )

    # --- Behavioural assertions -----------------------------------------
    assert reply.ok is True
    assert reply.recap["verdict"] == "routed"
    assert reply.recap["confidence"] == pytest.approx(0.91)
    assert "CONVERGED" in reply.recap["notes"]
    assert "Authentification" in reply.final_text

    # --- Trace structure: Plan → Execute(swarm.classify) → Verify -------
    kinds = [e.kind for e in reply.trace]
    assert kinds == ["plan", "execute", "verify"]
    execute_event = reply.trace[1]
    assert execute_event.payload["tool_name"] == "swarm.classify"
    assert execute_event.payload["ok"] is True

    # --- Budget arithmetic ----------------------------------------------
    # Plan: 180 in + 60 out, Verify: 220 in + 45 out → 505 total tokens.
    assert reply.ledger["tokens_consumed"] == 505
    expected_usd = (180 + 220) * 3.0 / 1_000_000 + (60 + 45) * 15.0 / 1_000_000
    assert reply.ledger["usd_consumed"] == pytest.approx(expected_usd, rel=1e-6)
    assert reply.ledger["tool_calls_made"] == 1

    # --- Wire-format proof: Anthropic received the right schemas --------
    plan_body = captured_requests[0]
    assert plan_body["model"] == "claude-3-5-sonnet-latest"
    # Tools list contains our three swarm tools.
    tool_names = {t["name"] for t in plan_body["tools"]}
    assert "swarm.classify" in tool_names
    assert "swarm.describe" in tool_names
    # System prompt is the French Plan prompt (default BotConfig(lang="fr")).
    assert "coordinateur" in plan_body["system"]

    # --- Verify turn got tool_result block ------------------------------
    verify_body = captured_requests[1]
    # The Verify message history contains a user turn with a tool_result.
    verify_messages = verify_body["messages"]
    tool_result_blocks = [
        block
        for msg in verify_messages
        if msg["role"] == "user"
        for block in (msg["content"] if isinstance(msg["content"], list) else [])
        if isinstance(block, dict) and block.get("type") == "tool_result"
    ]
    assert len(tool_result_blocks) == 1
    assert tool_result_blocks[0]["tool_use_id"] == "tool_use_001"


@pytest.mark.asyncio
async def test_claude_budget_breach_short_circuits(
    running_swarm: SwarmClient,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A Claude response that exceeds the token budget short-circuits the
    bot with a ``budget_exceeded`` outcome — no Verify turn is issued."""
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-test-dummy")
    provider = ClaudeProvider(model="claude-3-5-sonnet-latest")
    # 500-token cap but the Plan turn already reports 240 tokens,
    # Verify would push us over on the following LLM charge.
    bot = Bot(
        BotConfig(budget=Budget(max_llm_tokens=240, max_wall_clock_ms=5_000)),
        swarm=running_swarm,
        provider=provider,
    )

    with respx.mock(base_url="https://api.anthropic.com") as router:
        router.post("/v1/messages").mock(
            side_effect=[
                httpx.Response(200, json=_plan_turn_payload()),  # 240 tokens
                httpx.Response(200, json=_verify_turn_payload()),  # would push over
            ]
        )
        reply = await bot.answer("Je ne peux pas me connecter", tenant="acme")

    # The tool call *did* happen (budget was checked only on charge_llm),
    # but the Verify-turn LLM charge trips the cap.
    assert reply.ok is False
    assert reply.error == "budget_exceeded"
    assert reply.recap["error_detail"]["dimension"] == "tokens"
