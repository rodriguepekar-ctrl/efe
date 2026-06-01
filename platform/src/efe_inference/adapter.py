"""InferenceAdapter — top-level orchestrator.

Chains the entire request pipeline (see M07_ARCHITECTURE.md §5) :

    1. Safety.preflight  (PII scrub + moderation + injection)
    2. VerbatimCache.get
    3. SemanticCache.get
    4. BackendSelector.select  (fallback chain, skip open circuits)
    5. PromptTemplater.render
    6. RetryExecutor.run( adapter.generate )
    7. Safety.postflight (PII + moderation + policy)
    8. VerbatimCache.put + SemanticCache.put
    9. TokenAccount.bill + Budget.record
    10. Telemetry.emit

This file is intentionally long and explicit — it is the contract
between every component and the wire. Keep the method bodies
straight-line so the flow is readable at review time.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from decimal import Decimal
from typing import Any

from efe_inference.accounting.budget import BudgetEnforcer
from efe_inference.accounting.cost import CostCalculator
from efe_inference.accounting.tokenizer import TokenCounterRegistry
from efe_inference.backends.protocol import BackendRequest, BackendResponse
from efe_inference.backends.selector import BackendSelector
from efe_inference.cache.key import CacheKey, CacheKeyBuilder
from efe_inference.cache.semantic import SemanticCache
from efe_inference.cache.verbatim import VerbatimCache
from efe_inference.clock import Clock, SystemClock
from efe_inference.config import InferenceSettings
from efe_inference.errors import (
    AllBackendsDown,
    InferenceError,
    SafetyBlocked,
)
from efe_inference.hashing import sha256_text
from efe_inference.messages.request import ChatMessage, ChatRequest, Role, TextBlock
from efe_inference.messages.response import (
    ChatChoice,
    ChatChunk,
    ChatResponse,
    FinishReason,
)
from efe_inference.observability.metrics import MetricsCatalog
from efe_inference.resilience.circuit import CircuitBreaker
from efe_inference.resilience.dlq import DeadLetterQueue, DlqEntry
from efe_inference.resilience.retry import RetryExecutor
from efe_inference.safety.auditor import SafetyAuditor
from efe_inference.safety.policy import SafetyDecider, SafetyVerdict
from efe_inference.templating.families import family_for_backend
from efe_inference.templating.templater import PromptTemplater


@dataclass
class InferenceAdapter:
    """See module docstring."""

    settings: InferenceSettings
    selector: BackendSelector
    templater: PromptTemplater
    retry: RetryExecutor
    safety: SafetyDecider
    auditor: SafetyAuditor
    token_counters: TokenCounterRegistry
    cost: CostCalculator
    budget: BudgetEnforcer
    verbatim_cache: VerbatimCache
    semantic_cache: SemanticCache
    dlq: DeadLetterQueue
    metrics: MetricsCatalog
    key_builder: CacheKeyBuilder = field(default_factory=CacheKeyBuilder)
    clock: Clock = field(default_factory=SystemClock)

    # -------------------------------------------------------------- generate

    async def generate(self, req: ChatRequest) -> ChatResponse:
        started = self.clock.now_monotonic_s()
        self.metrics.requests_total.inc(
            tenant=req.tenant_id, task=req.task_id, outcome="start",
        )

        # --- Preflight safety ---
        scrubbed_req, pre_verdict = await self._preflight(req)
        if pre_verdict.decision == "block":
            await self._audit_and_raise(
                req=req, verdict=pre_verdict, severity="block",
                scrubbed=pre_verdict.scrubbed_text or "",
            )

        # --- Budget ---
        await self.budget.preflight(req.tenant_id)

        # --- Cache (verbatim then semantic) ---
        key = self.key_builder.build(scrubbed_req)
        cached = await self._try_cache(scrubbed_req, key)
        if cached is not None:
            self.metrics.cache_hits_total.inc(
                tenant=req.tenant_id, task=req.task_id, kind=cached.cached,
            )
            return cached

        self.metrics.cache_misses_total.inc(
            tenant=req.tenant_id, task=req.task_id,
        )

        # --- Backend dispatch with fallback + retry ---
        response = await self._dispatch(scrubbed_req, started)

        # --- Postflight safety ---
        final = await self._postflight(req, response)

        # --- Cache put (non-blocking, best-effort) ---
        await self._cache_put(scrubbed_req, key, final)

        # --- Accounting ---
        await self._account(scrubbed_req, final)

        return final

    # -------------------------------------------------------------- steps

    async def _preflight(
        self, req: ChatRequest,
    ) -> tuple[ChatRequest, SafetyVerdict]:
        """Run safety preflight.

        Fix C4 — emit audit events on *any* non-noop verdict (categories
        non-empty or decision != "allow"), not just on "block". PII
        scrubs and "review" outcomes now leave a signed trail.

        Fix M12 — increment the category counter **once per category**
        instead of joining categories into a comma-separated label. That
        comma-joined label caused combinatorial cardinality explosion on
        multi-PII requests.
        """
        from efe_inference.safety.policy import SafetyStage

        if not self.settings.safety.preflight:
            return req, SafetyVerdict(
                decision="allow", stage=SafetyStage.PREFLIGHT_PII,
            )
        # Fix M8 — scrub per-message (per TextBlock) instead of a single
        # concatenated blob. The earlier implementation re-injected the
        # whole concatenated scrub into the last user message, which
        # duplicated the system prompt text and broke message semantics.
        combined = req.total_text()
        verdict = await self.safety.preflight(combined)
        if verdict.decision == "block":
            return req, verdict
        scrubbed_req = _scrub_messages_per_block(req, self.safety)
        # Metrics — one increment per category (bounded cardinality).
        for cat in verdict.categories:
            self.metrics.safety_scrub_total.inc(
                tenant=req.tenant_id, category=cat,
            )
        # Audit — record PII / review outcomes. "allow" with empty
        # categories remains silent (the common happy path).
        if verdict.categories or verdict.decision != "allow":
            # ``block`` was already returned above; narrow to "allow" | "review".
            severity = (
                "warn" if verdict.decision == "review" else "info"
            )
            await self.auditor.record(
                request_id=req.request_id,
                tenant_id=req.tenant_id,
                verdict=verdict,
                severity=severity,
                scrubbed_text=verdict.scrubbed_text or "",
            )
        return scrubbed_req, verdict

    async def _try_cache(
        self, req: ChatRequest, key: CacheKey,
    ) -> ChatResponse | None:
        if self.settings.cache.verbatim_enabled:
            hit = await self.verbatim_cache.get(key)
            if hit is not None:
                return _as_cached(hit, "verbatim")
        if self.settings.cache.semantic_enabled:
            sem = await self.semantic_cache.get(
                req, threshold=self.settings.cache.semantic_cosine_threshold,
            )
            if sem is not None:
                return _as_cached(sem.response, "semantic")
        return None

    async def _dispatch(
        self, req: ChatRequest, started_mono: float,
    ) -> ChatResponse:
        candidates = await self.selector.select(
            tenant_id=req.tenant_id, task_id=req.task_id,
        )
        last_error: BaseException | None = None
        for entry, adapter in candidates:
            family = family_for_backend(adapter.kind)
            rendered = self.templater.render(req, family=family)
            backend_req = BackendRequest(
                request_id=req.request_id,
                tenant_id=req.tenant_id,
                task_id=req.task_id,
                model=req.model_hint or adapter.name,
                textual=rendered.textual,
                messages=rendered.messages,
                sampling=req.sampling,
                tools=req.tools,
                tool_choice=req.tool_choice,
                stream=req.stream,
                timeout_s=min(
                    entry.timeout_s,
                    req.timeout_budget_s,
                ),
            )
            breaker = self.selector.circuit.get(req.tenant_id, entry.backend_name)
            try:
                resp = await self._run_with_retry(
                    backend_req=backend_req, adapter=adapter,
                    breaker=breaker,
                )
                backend_overhead = self.clock.now_monotonic_s() - started_mono
                self.metrics.fallback_used_total.inc(
                    tenant=req.tenant_id,
                    backend=entry.backend_name,
                ) if entry != candidates[0][0] else None
                return _to_chat_response(
                    req, adapter_name=adapter.name,
                    kind=adapter.kind.value,
                    resp=resp,
                    total_overhead_s=backend_overhead,
                )
            except (
                AllBackendsDown, SafetyBlocked,
            ):
                raise
            except InferenceError as exc:
                last_error = exc
                self.metrics.backend_errors_total.inc(
                    tenant=req.tenant_id,
                    backend=entry.backend_name,
                    code=exc.code,
                )
                continue
            except Exception as exc:  # pragma: no cover — unexpected
                last_error = exc
                self.metrics.backend_errors_total.inc(
                    tenant=req.tenant_id,
                    backend=entry.backend_name,
                    code="unexpected",
                )
                continue
        # Exhausted the chain — DLQ + raise.
        prompt_hash_hex = sha256_text(req.total_text()).hex()
        await self.dlq.push(DlqEntry(
            request_id=req.request_id,
            tenant_id=req.tenant_id,
            task_id=req.task_id,
            prompt_hash_hex=prompt_hash_hex,
            last_error=str(last_error or "unknown"),
            enqueued_at=self.clock.now(),
        ))
        self.metrics.dlq_entries_total.inc(tenant=req.tenant_id)
        raise AllBackendsDown(
            "all backends failed",
            detail={"last_error": str(last_error or "unknown")},
        )

    # -------------------------------------------------------------- stream

    async def stream(
        self, req: ChatRequest,
    ) -> AsyncIterator[ChatChunk]:
        """Stream a response chunk-by-chunk (H8 + L1).

        Safety preflight happens exactly like in :meth:`generate`. The
        postflight safety runs on the accumulated text **after** the
        stream terminates; if the accumulated text violates policy, a
        synthetic final chunk with ``finish_reason=content_filter`` is
        emitted and the content is suppressed (the already-yielded
        chunks are the attacker's problem — we cannot un-send bytes).
        """
        scrubbed_req, pre_verdict = await self._preflight(req)
        if pre_verdict.decision == "block":
            await self._audit_and_raise(
                req=req, verdict=pre_verdict, severity="block",
                scrubbed=pre_verdict.scrubbed_text or "",
            )
        await self.budget.preflight(req.tenant_id)
        candidates = await self.selector.select(
            tenant_id=req.tenant_id, task_id=req.task_id,
        )
        entry, adapter = candidates[0]
        from efe_inference.templating.families import family_for_backend

        family = family_for_backend(adapter.kind)
        rendered = self.templater.render(scrubbed_req, family=family)
        backend_req = BackendRequest(
            request_id=req.request_id,
            tenant_id=req.tenant_id,
            task_id=req.task_id,
            model=req.model_hint or adapter.name,
            textual=rendered.textual,
            messages=rendered.messages,
            sampling=req.sampling,
            tools=req.tools,
            tool_choice=req.tool_choice,
            stream=True,
            timeout_s=min(entry.timeout_s, req.timeout_budget_s),
        )
        breaker = self.selector.circuit.get(
            req.tenant_id, entry.backend_name,
        )
        await breaker.allow()
        accumulator: list[str] = []
        try:
            async for chunk in adapter.stream(backend_req):
                if chunk.delta_text:
                    accumulator.append(chunk.delta_text)
                yield chunk
            await breaker.on_success()
        except Exception:
            await breaker.on_failure()
            raise
        # Postflight on accumulated text.
        final_text = "".join(accumulator)
        if self.settings.safety.postflight and final_text:
            verdict = await self.safety.postflight(final_text)
            if verdict.decision == "block":
                await self.auditor.record(
                    request_id=req.request_id,
                    tenant_id=req.tenant_id,
                    verdict=verdict,
                    severity="block",
                    scrubbed_text=verdict.scrubbed_text or "",
                )
                self.metrics.safety_block_total.inc(
                    tenant=req.tenant_id, stage=verdict.stage.value,
                )
                # Synthetic terminator chunk.
                yield ChatChunk(
                    delta_text="[SAFETY_BLOCK]",
                    finish_reason=FinishReason.CONTENT_FILTER,
                )

    async def _run_with_retry(
        self,
        *,
        backend_req: BackendRequest,
        adapter: Any,
        breaker: CircuitBreaker,
    ) -> BackendResponse:
        """Run a single backend's ``generate`` under retry + circuit.

        Fix C1 — re-check the breaker on **every** attempt. If a failed
        attempt flips the breaker to OPEN, the next attempt must refuse
        fast (raising :class:`CircuitOpen`) so the fallback chain can
        escalate to the secondary. Without this, retry.run keeps
        hammering a known-bad backend until max_attempts.
        """

        async def _do() -> BackendResponse:
            # C1: refuse before every attempt, not only at chain entry.
            await breaker.allow()
            try:
                resp: BackendResponse = await adapter.generate(backend_req)
            except Exception:
                await breaker.on_failure()
                raise
            else:
                await breaker.on_success()
                return resp

        result: BackendResponse = await self.retry.run(_do)
        return result

    async def _postflight(
        self, req: ChatRequest, resp: ChatResponse,
    ) -> ChatResponse:
        if not self.settings.safety.postflight:
            return resp
        first_text = (
            resp.choices[0].message.text() if resp.choices else ""
        )
        verdict = await self.safety.postflight(first_text)
        if verdict.decision == "block":
            await self._audit_and_raise(
                req=req, verdict=verdict, severity="block",
                scrubbed=verdict.scrubbed_text or "",
            )
        # Audit PII-scrub / review outcomes in postflight too (C4).
        if verdict.categories and verdict.decision != "block":
            await self.auditor.record(
                request_id=req.request_id,
                tenant_id=req.tenant_id,
                verdict=verdict,
                severity="info",
                scrubbed_text=verdict.scrubbed_text or "",
            )
            for cat in verdict.categories:
                self.metrics.safety_scrub_total.inc(
                    tenant=req.tenant_id, category=cat,
                )
        if verdict.scrubbed_text and verdict.scrubbed_text != first_text:
            # Rewrite the assistant's message with the scrubbed text.
            new_choices = list(resp.choices)
            if new_choices:
                first = new_choices[0]
                new_choices[0] = ChatChoice(
                    index=first.index,
                    message=ChatMessage(
                        role=first.message.role,
                        content=(TextBlock(text=verdict.scrubbed_text),),
                    ),
                    finish_reason=first.finish_reason,
                )
            flags = dict(resp.safety_flags)
            flags["postflight"] = list(verdict.categories) or ["scrubbed"]
            return ChatResponse(
                request_id=resp.request_id,
                model=resp.model,
                choices=tuple(new_choices),
                usage=resp.usage,
                backend_kind=resp.backend_kind,
                backend_latency_ms=resp.backend_latency_ms,
                adapter_overhead_ms=resp.adapter_overhead_ms,
                safety_flags=flags,
                cached=resp.cached,
            )
        return resp

    async def _cache_put(
        self, req: ChatRequest, key: CacheKey, resp: ChatResponse,
    ) -> None:
        if resp.cached != "no":
            return
        if self.settings.cache.verbatim_enabled:
            await self.verbatim_cache.put(
                key, resp, ttl_s=self.settings.redis.verbatim_ttl_s,
            )
        if self.settings.cache.semantic_enabled:
            await self.semantic_cache.put(
                req, resp,
                ttl_s=self.settings.redis.semantic_ttl_s,
            )

    async def _account(
        self, req: ChatRequest, resp: ChatResponse,
    ) -> None:
        self.metrics.prompt_tokens_total.inc(
            resp.usage.prompt_tokens,
            tenant=req.tenant_id, task=req.task_id, backend=resp.backend_kind,
        )
        self.metrics.completion_tokens_total.inc(
            resp.usage.completion_tokens,
            tenant=req.tenant_id, task=req.task_id, backend=resp.backend_kind,
        )
        cost = self.cost.calculate(
            resp.model,
            prompt_tokens=resp.usage.prompt_tokens,
            completion_tokens=resp.usage.completion_tokens,
        )
        # Fix H10 — accumulate in micro-cents (Decimal * 1_000_000
        # rounded to int) so long-running counters preserve precision.
        # A second gauge-like counter keeps the float form for
        # dashboards that expect USD.
        micro_cents = int((cost * Decimal(1_000_000)).to_integral_value())
        self.metrics.cost_usd_total.inc(
            float(cost),
            tenant=req.tenant_id, task=req.task_id, backend=resp.backend_kind,
        )
        self.metrics.cost_micro_cents_total.inc(
            float(micro_cents),
            tenant=req.tenant_id, task=req.task_id, backend=resp.backend_kind,
        )
        if cost > Decimal(0):
            await self.budget.record(req.tenant_id, cost)

    async def _audit_and_raise(
        self, *, req: ChatRequest, verdict: SafetyVerdict,
        severity: str, scrubbed: str,
    ) -> None:
        await self.auditor.record(
            request_id=req.request_id,
            tenant_id=req.tenant_id,
            verdict=verdict,
            severity=severity,
            scrubbed_text=scrubbed,
        )
        self.metrics.safety_block_total.inc(
            tenant=req.tenant_id, stage=verdict.stage.value,
        )
        raise SafetyBlocked(
            f"content policy violation at {verdict.stage.value}: "
            f"{verdict.reason}",
            detail={"categories": list(verdict.categories)},
        )


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _scrub_messages_per_block(
    req: ChatRequest, safety: SafetyDecider,
) -> ChatRequest:
    """Fix M8 — scrub each :class:`TextBlock` in place.

    The scrubber lives on :attr:`SafetyDecider.context`. We bypass the
    full ``preflight`` pipeline here (already ran on the combined text)
    and only apply the :class:`PiiScrubber` to each block's text so the
    structural shape of the request is preserved: system, user, tool
    messages keep their original boundaries.
    """
    scrubber = safety.context.scrubber
    new_messages = []
    for m in req.messages:
        new_content: list[Any] = []
        for b in m.content:
            if isinstance(b, TextBlock):
                result = scrubber.scrub(b.text)
                new_content.append(TextBlock(text=result.scrubbed_text))
            else:
                new_content.append(b)
        new_messages.append(ChatMessage(
            role=m.role,
            content=tuple(new_content),
            name=m.name,
            tool_call_id=m.tool_call_id,
        ))
    return ChatRequest(
        tenant_id=req.tenant_id,
        task_id=req.task_id,
        request_id=req.request_id,
        messages=tuple(new_messages),
        sampling=req.sampling,
        tools=req.tools,
        tool_choice=req.tool_choice,
        stream=req.stream,
        model_hint=req.model_hint,
        user_id=req.user_id,
        timeout_budget_s=req.timeout_budget_s,
        metadata=req.metadata,
    )


def _to_chat_response(
    req: ChatRequest,
    *,
    adapter_name: str,
    kind: str,
    resp: BackendResponse,
    total_overhead_s: float,
) -> ChatResponse:
    choice = ChatChoice(
        index=0,
        message=ChatMessage(
            role=Role.ASSISTANT,
            content=(TextBlock(text=resp.text),),
        ),
        finish_reason=resp.finish_reason,
    )
    # Fix H7 — clamp to >= 0. If backend self-reports latency larger
    # than the wall-clock elapsed (can happen with fake clocks or
    # streaming when first-byte arrives almost immediately), the
    # subtraction would be negative and pollute the histogram.
    adapter_overhead_ms = max(
        0.0, total_overhead_s * 1000.0 - resp.backend_latency_ms,
    )
    return ChatResponse(
        request_id=req.request_id,
        model=adapter_name,
        choices=(choice,),
        usage=resp.usage,
        backend_kind=kind,
        backend_latency_ms=resp.backend_latency_ms,
        adapter_overhead_ms=adapter_overhead_ms,
        safety_flags={},
        cached="no",
    )


def _as_cached(resp: ChatResponse, kind: str) -> ChatResponse:
    return ChatResponse(
        request_id=resp.request_id,
        model=resp.model,
        choices=resp.choices,
        usage=resp.usage,
        backend_kind=resp.backend_kind,
        backend_latency_ms=resp.backend_latency_ms,
        adapter_overhead_ms=resp.adapter_overhead_ms,
        safety_flags=resp.safety_flags,
        cached=kind,
    )


__all__ = ["InferenceAdapter"]
