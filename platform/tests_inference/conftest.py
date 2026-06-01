"""Fixtures shared by every efe_inference test."""

from __future__ import annotations

import asyncio
from collections.abc import Iterator
from datetime import datetime, timezone
from decimal import Decimal

import pytest

from efe_inference.accounting.budget import (
    BudgetEnforcer,
    InMemoryBudgetStore,
)
from efe_inference.accounting.cost import CostCalculator, CostRate
from efe_inference.accounting.tokenizer import (
    TokenCounterRegistry,
)
from efe_inference.adapter import InferenceAdapter
from efe_inference.backends.mock import MockBackend
from efe_inference.backends.registry import BackendRegistry
from efe_inference.backends.selector import (
    BackendSelector,
    FallbackChain,
    FallbackEntry,
)
from efe_inference.cache.embed import DeterministicEmbedder
from efe_inference.cache.key import CacheKeyBuilder
from efe_inference.cache.semantic import InMemorySemanticCache
from efe_inference.cache.verbatim import InMemoryVerbatimCache
from efe_inference.clock import FakeClock
from efe_inference.config import (
    ApiSettings,
    CacheSettings,
    CircuitSettings,
    InferenceSettings,
    NatsSettings,
    PiiSettings,
    ResilienceSettings,
    RetrySettings,
    SafetySettings,
    TimeoutSettings,
)
from efe_inference.observability.metrics import MetricsCatalog
from efe_inference.resilience.circuit import CircuitBreakerRegistry
from efe_inference.resilience.dlq import InMemoryDeadLetterQueue
from efe_inference.resilience.retry import RetryExecutor
from efe_inference.safety.auditor import InMemorySafetyAuditor
from efe_inference.safety.injection import PromptInjectionDetector
from efe_inference.safety.moderation import (
    LocalBlocklistModerator,
    ModerationChain,
)
from efe_inference.safety.pii import RegexPiiScrubber
from efe_inference.safety.policy import SafetyContext, SafetyDecider
from efe_inference.templating.templater import PromptTemplater


@pytest.fixture(scope="session")
def event_loop() -> Iterator[asyncio.AbstractEventLoop]:
    loop = asyncio.new_event_loop()
    yield loop
    loop.close()


@pytest.fixture
def fake_clock() -> FakeClock:
    return FakeClock(datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc))


@pytest.fixture
def settings() -> InferenceSettings:
    return InferenceSettings(
        environment="test",
        safety=SafetySettings(pii=PiiSettings(mode="redact")),
        cache=CacheSettings(
            verbatim_enabled=True,
            semantic_enabled=True,
            semantic_cosine_threshold=0.95,
        ),
        resilience=ResilienceSettings(
            retry=RetrySettings(max_attempts=2, backoff_base_s=0.01),
            circuit=CircuitSettings(
                failure_threshold=2, window_s=60, cool_down_s=5,
            ),
            timeout=TimeoutSettings(total_s=5.0),
        ),
        nats=NatsSettings(hmac_key_hex="11" * 32),
        api=ApiSettings(),
    )


@pytest.fixture
def registry_with_mocks() -> BackendRegistry:
    reg = BackendRegistry()
    reg.register(MockBackend(name="primary"))
    reg.register(MockBackend(name="secondary"))
    return reg


@pytest.fixture
def adapter(
    settings: InferenceSettings,
    registry_with_mocks: BackendRegistry,
    fake_clock: FakeClock,
) -> InferenceAdapter:
    circuit = CircuitBreakerRegistry(
        settings=settings.resilience.circuit, clock=fake_clock,
    )
    chain = FallbackChain(entries=(
        FallbackEntry(backend_name="primary", timeout_s=5.0),
        FallbackEntry(backend_name="secondary", timeout_s=5.0),
    ))
    selector = BackendSelector(
        registry=registry_with_mocks, circuit=circuit,
        default_chain=chain,
    )
    templater = PromptTemplater()
    retry = RetryExecutor(settings=settings.resilience.retry)
    scrubber = RegexPiiScrubber(settings.safety.pii)
    moderator = ModerationChain(
        providers=(LocalBlocklistModerator(),),
        settings=settings.safety.moderation,
    )
    injector = PromptInjectionDetector()
    safety_ctx = SafetyContext(
        scrubber=scrubber, moderator=moderator, injector=injector,
        injection_threshold=settings.safety.injection.threshold,
        moderation_thresholds=dict(settings.safety.moderation.thresholds),
    )
    safety = SafetyDecider(context=safety_ctx)
    auditor = InMemorySafetyAuditor(
        hmac_key=bytes.fromhex(settings.safety.audit_hmac_key_hex),
        clock=fake_clock,
    )
    token_counters = TokenCounterRegistry.default()
    cost = CostCalculator((
        CostRate(
            backend_name="primary",
            prompt_usd_per_1k=Decimal("0.001"),
            completion_usd_per_1k=Decimal("0.002"),
        ),
    ))
    budget = BudgetEnforcer(
        store=InMemoryBudgetStore(),
        default_daily_budget_usd=Decimal("100"),
        clock=fake_clock,
    )
    verbatim = InMemoryVerbatimCache()
    embedder = DeterministicEmbedder(dim=16)
    semantic = InMemorySemanticCache(
        embedder=embedder, key_builder=CacheKeyBuilder(),
    )
    dlq = InMemoryDeadLetterQueue()
    metrics = MetricsCatalog()
    return InferenceAdapter(
        settings=settings,
        selector=selector,
        templater=templater,
        retry=retry,
        safety=safety,
        auditor=auditor,
        token_counters=token_counters,
        cost=cost,
        budget=budget,
        verbatim_cache=verbatim,
        semantic_cache=semantic,
        dlq=dlq,
        metrics=metrics,
        clock=fake_clock,
    )
