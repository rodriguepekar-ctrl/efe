"""The ``Bot`` — top-level orchestrator exposing :meth:`answer`.

The bot wires together:

* the :class:`~efe_platform.swarm.SwarmClient` (data plane),
* the :class:`~efe_platform.bot.tool_registry.ToolRegistry` (dispatch
  surface for the LLM),
* the :class:`~efe_platform.bot.memory.ConversationMemory` (bounded
  context),
* the :class:`~efe_platform.bot.budget.BudgetLedger` (cost guard),
* the :class:`~efe_platform.bot.policy.PolicyEngine` (safety gate),
* the :class:`~efe_platform.bot.planner.Planner` (PEV state machine),
* an :class:`LlmProvider` (chosen at construction: mock / claude / openai).

The exposed API is small by design:

```python
reply = await bot.answer("Classe ce ticket: ...", tenant="acme")
print(reply.final_text)
for event in reply.trace: ...
```

Everything else happens under the hood with strict typing, audit
records, and determinism guarantees.
"""

from __future__ import annotations

import time
import uuid
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_platform.bot.budget import Budget, BudgetLedger
from efe_platform.bot.memory import ConversationMemory
from efe_platform.bot.planner import LlmProvider, Planner, PlanStep
from efe_platform.bot.policy import PolicyEngine
from efe_platform.bot.prompts import get_prompt
from efe_platform.bot.tool_registry import ToolRegistry, ToolSchema
from efe_platform.errors import (
    BotError,
    BudgetExceeded,
    EfePlatformError,
    LlmProviderUnavailable,
)
from efe_platform.swarm.client import SwarmClient
from efe_platform.swarm.cone import ConeRegistryClient
from efe_platform.swarm.spec_builder import SpecBuilder

# ---------------------------------------------------------------------------
#  Trace event
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class TraceEvent:
    """One entry in a :attr:`BotReply.trace`."""

    kind: str
    summary: str
    ts_ms: int
    payload: Mapping[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "summary": self.summary,
            "ts_ms": self.ts_ms,
            "payload": dict(self.payload),
        }


# ---------------------------------------------------------------------------
#  Reply
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class BotReply:
    """Rich bot output exposed to callers."""

    request_id: str
    tenant: str
    final_text: str
    recap: Mapping[str, Any]
    trace: tuple[TraceEvent, ...]
    ledger: Mapping[str, float | int]
    duration_ms: float
    ok: bool
    error: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "request_id": self.request_id,
            "tenant": self.tenant,
            "final_text": self.final_text,
            "recap": dict(self.recap),
            "trace": [e.to_dict() for e in self.trace],
            "ledger": dict(self.ledger),
            "duration_ms": round(self.duration_ms, 2),
            "ok": self.ok,
            "error": self.error,
        }


# ---------------------------------------------------------------------------
#  Bot config
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class BotConfig:
    """Immutable bot configuration."""

    lang: str = "fr"
    budget: Budget = field(default_factory=Budget)
    max_memory_messages: int = 24
    max_memory_tokens: int = 12_000
    max_plan_tokens: int = 1024
    max_verify_tokens: int = 1024
    tenant_default: str = "default"

    @classmethod
    def from_env(cls) -> BotConfig:
        """Read the simplest env overrides — mirrors :class:`PlatformConfig`."""
        import os

        return cls(
            lang=os.environ.get("EFE_PLATFORM_BOT_LANG", "fr"),
            budget=Budget(
                max_wall_clock_ms=int(
                    os.environ.get("EFE_PLATFORM_BUDGET_MS", "30000")
                ),
                max_llm_tokens=int(
                    os.environ.get("EFE_PLATFORM_BUDGET_TOKENS", "16000")
                ),
                max_usd=float(os.environ.get("EFE_PLATFORM_BUDGET_USD", "0.25")),
                max_tool_calls=int(
                    os.environ.get("EFE_PLATFORM_BUDGET_CALLS", "8")
                ),
            ),
            tenant_default=os.environ.get("EFE_PLATFORM_TENANT", "default"),
        )


# ---------------------------------------------------------------------------
#  Optional sinks
# ---------------------------------------------------------------------------


class TraceSink(Protocol):
    """Anything that consumes :class:`TraceEvent` in real time."""

    def __call__(self, event: TraceEvent) -> None: ...


# ---------------------------------------------------------------------------
#  Bot
# ---------------------------------------------------------------------------


class Bot:
    """Top-level orchestrator."""

    def __init__(
        self,
        config: BotConfig,
        *,
        swarm: SwarmClient,
        provider: LlmProvider | None = None,
        cones: ConeRegistryClient | None = None,
        trace_sink: TraceSink | None = None,
        # Hook used by tests to inject a custom tool registry.
        registry_factory: Callable[[SwarmClient, ConeRegistryClient | None], ToolRegistry]
        | None = None,
    ) -> None:
        self._config = config
        self._swarm = swarm
        self._cones = cones
        self._trace_sink = trace_sink
        self._provider = provider or self._default_provider()
        self._registry = (registry_factory or _default_registry_factory)(swarm, cones)
        self._policy = PolicyEngine()

    # ------------------------------------------------------------ Factories

    @staticmethod
    def _default_provider() -> LlmProvider:
        """The mock provider — guarantees the bot is always importable.

        Real-world callers pass ``provider=ClaudeProvider(...)`` etc.
        """
        from efe_platform.bot.providers.mock import MockProvider

        return MockProvider()

    # ---------------------------------------------------------- Public API

    async def answer(
        self,
        prompt: str,
        *,
        tenant: str | None = None,
        request_id: str | None = None,
        budget: Budget | None = None,
    ) -> BotReply:
        """Run one Plan / Execute / Verify cycle and return a :class:`BotReply`."""
        if not prompt.strip():
            raise BotError("prompt must not be empty")
        tenant = tenant or self._config.tenant_default
        request_id = request_id or str(uuid.uuid4())
        ledger = BudgetLedger(budget=budget or self._config.budget)
        memory = ConversationMemory(
            max_messages=self._config.max_memory_messages,
            max_approx_tokens=self._config.max_memory_tokens,
        )
        memory.add_user(prompt)

        trace: list[TraceEvent] = []
        t_start = time.monotonic()

        def _emit(step: PlanStep) -> None:
            ev = TraceEvent(
                kind=step.phase,
                summary=step.summary,
                ts_ms=int((time.monotonic() - t_start) * 1000),
                payload=dict(step.payload),
            )
            trace.append(ev)
            if self._trace_sink is not None:
                self._trace_sink(ev)

        planner = Planner(
            provider=self._provider,
            registry=self._registry,
            policy=self._policy,
            plan_system_prompt=get_prompt("plan", self._config.lang),  # type: ignore[arg-type]
            verify_system_prompt=get_prompt("verify", self._config.lang),  # type: ignore[arg-type]
            max_plan_tokens=self._config.max_plan_tokens,
            max_verify_tokens=self._config.max_verify_tokens,
            on_step=_emit,
        )

        try:
            plan_verdict = await planner.plan(
                memory=memory, ledger=ledger, tenant=tenant
            )
            if plan_verdict.kind == "conversational":
                memory.add_assistant(plan_verdict.text)
                return self._finish(
                    request_id=request_id,
                    tenant=tenant,
                    final_text=plan_verdict.text,
                    recap={"verdict": "conversational", "confidence": 1.0, "notes": []},
                    trace=trace,
                    ledger=ledger,
                    t_start=t_start,
                )
            # assistant message embeds the plan (provider reconstructs it)
            memory.add_assistant(_plan_echo(plan_verdict.calls))
            await planner.execute(
                plan_verdict.calls, memory=memory, ledger=ledger
            )
            verify_verdict = await planner.verify(memory=memory, ledger=ledger)
            memory.add_assistant(verify_verdict.user_text)
            return self._finish(
                request_id=request_id,
                tenant=tenant,
                final_text=verify_verdict.user_text,
                recap=verify_verdict.recap,
                trace=trace,
                ledger=ledger,
                t_start=t_start,
            )
        except BudgetExceeded as exc:
            return self._finish_error(
                request_id=request_id,
                tenant=tenant,
                error=exc,
                trace=trace,
                ledger=ledger,
                t_start=t_start,
            )
        except EfePlatformError as exc:
            return self._finish_error(
                request_id=request_id,
                tenant=tenant,
                error=exc,
                trace=trace,
                ledger=ledger,
                t_start=t_start,
            )

    # ----------------------------------------------------- Tool registry accessor

    @property
    def registry(self) -> ToolRegistry:
        """Expose the registry so callers can add custom tools."""
        return self._registry

    # ----------------------------------------------------------- Internals

    def _finish(
        self,
        *,
        request_id: str,
        tenant: str,
        final_text: str,
        recap: Mapping[str, Any],
        trace: list[TraceEvent],
        ledger: BudgetLedger,
        t_start: float,
    ) -> BotReply:
        return BotReply(
            request_id=request_id,
            tenant=tenant,
            final_text=final_text,
            recap=recap,
            trace=tuple(trace),
            ledger=ledger.to_dict(),
            duration_ms=(time.monotonic() - t_start) * 1000,
            ok=True,
        )

    def _finish_error(
        self,
        *,
        request_id: str,
        tenant: str,
        error: EfePlatformError,
        trace: list[TraceEvent],
        ledger: BudgetLedger,
        t_start: float,
    ) -> BotReply:
        # Preserve the structured detail (dimension of a budget breach,
        # offending tool name, etc.) so the HTTP layer + metrics can
        # route on it later.
        recap: dict[str, Any] = {
            "verdict": "error",
            "confidence": 0.0,
            "notes": [error.message],
            "error_code": error.code,
        }
        if isinstance(error.detail, dict):
            recap["error_detail"] = dict(error.detail)
        return BotReply(
            request_id=request_id,
            tenant=tenant,
            final_text="",
            recap=recap,
            trace=tuple(trace),
            ledger=ledger.to_dict(),
            duration_ms=(time.monotonic() - t_start) * 1000,
            ok=False,
            error=error.code,
        )


# ---------------------------------------------------------------------------
#  Default tool registry wiring
# ---------------------------------------------------------------------------


def _default_registry_factory(
    swarm: SwarmClient, cones: ConeRegistryClient | None
) -> ToolRegistry:
    """Register the MVP toolset."""
    registry = ToolRegistry()

    classify_schema = ToolSchema(
        name="swarm.classify",
        description=(
            "Run an EFE swarm inference to classify a short text against a "
            "pre-registered alignment cone. Returns a decoded text payload "
            "plus confidence and verdict."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "tenant": {"type": "string"},
                "task": {"type": "string"},
                "cone_version": {"type": "integer"},
                "text": {"type": "string"},
                "problem_embedding": {"type": "array"},
                "dim_latent": {"type": "integer"},
                "budget_ms": {"type": "integer"},
            },
            "required": [
                "tenant",
                "task",
                "cone_version",
                "problem_embedding",
                "dim_latent",
            ],
        },
    )

    async def classify_handler(args: Mapping[str, Any]) -> dict[str, Any]:
        from efe_platform.swarm.protocol import ConeRef

        spec = (
            SpecBuilder()
            .with_problem_embedding(
                tuple(float(x) for x in args["problem_embedding"]),
                dim_latent=int(args["dim_latent"]),
            )
            .with_cone_ref(
                ConeRef(
                    tenant=str(args["tenant"]),
                    task=str(args["task"]),
                    version=int(args["cone_version"]),
                )
            )
            .with_budget_ms(int(args.get("budget_ms", 3000)))
            .build()
        )
        outcome = await swarm.execute(
            spec,
            raw_prompt=str(args.get("text", "")),
            tenant=str(args["tenant"]),
        )
        return outcome.to_dict()

    registry.register(classify_schema, classify_handler)

    describe_schema = ToolSchema(
        name="swarm.describe",
        description=(
            "Dry-run a swarm spec against the server. Returns `{valid, errors, "
            "hbm_bytes_estimate, rounds_expected}` without executing the "
            "swarm. Useful before paying for a full run."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "tenant": {"type": "string"},
                "task": {"type": "string"},
                "cone_version": {"type": "integer"},
                "problem_embedding": {"type": "array"},
                "dim_latent": {"type": "integer"},
            },
            "required": [
                "tenant",
                "task",
                "cone_version",
                "problem_embedding",
                "dim_latent",
            ],
        },
    )

    async def describe_handler(args: Mapping[str, Any]) -> dict[str, Any]:
        from efe_platform.swarm.protocol import ConeRef

        spec = (
            SpecBuilder()
            .with_problem_embedding(
                tuple(float(x) for x in args["problem_embedding"]),
                dim_latent=int(args["dim_latent"]),
            )
            .with_cone_ref(
                ConeRef(
                    tenant=str(args["tenant"]),
                    task=str(args["task"]),
                    version=int(args["cone_version"]),
                )
            )
            .build()
        )
        return await swarm.describe(spec)

    registry.register(describe_schema, describe_handler)

    if cones is not None:
        list_schema = ToolSchema(
            name="swarm.list_cones",
            description=(
                "List registered alignment cones for the given tenant + task. "
                "Returns the canonical keys sorted ascending."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "tenant": {"type": "string"},
                    "task": {"type": "string"},
                },
                "required": ["tenant", "task"],
            },
        )

        async def list_handler(args: Mapping[str, Any]) -> dict[str, Any]:
            versions = await cones.list_versions(
                tenant=str(args["tenant"]), task=str(args["task"])
            )
            return {"versions": list(versions)}

        registry.register(list_schema, list_handler)
    return registry


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _plan_echo(calls: tuple[Any, ...]) -> str:
    """Human-readable echo of a plan written to memory as the assistant turn.

    The exact format is opaque — providers format tool_use blocks
    natively; this string is only for memory replay fidelity.
    """
    if not calls:
        return ""
    names = ", ".join(c.tool_name for c in calls)
    return f"[plan] calling: {names}"


__all__ = [
    "Bot",
    "BotConfig",
    "BotReply",
    "LlmProviderUnavailable",
    "TraceEvent",
    "TraceSink",
]
