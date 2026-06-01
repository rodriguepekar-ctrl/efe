"""efe_classify_mode3 (M22) — pin tests."""
from __future__ import annotations

import pytest


@pytest.fixture
def fake_backend_with_mode3(fake_backend):
    """Augment FakeBackend with classify_mode3 mock."""

    async def classify_mode3(
        *, sentence, catalog, version=None, max_turns=None, threshold=None,
        top_k_anchors=10, auth,
    ):
        # Simulate convergence at turn 2
        return {
            "final_label": "fraud_review",
            "final_confidence": 0.91,
            "mode": "MODE_3",
            "total_turns": 3,
            "total_cost_usd": 0.00275,
            "converged": True,
            "requires_human_review": False,
            "review_token": None,
            "audit_id": 12345,
            "concept_stack": [
                {
                    "turn": 0, "candidates_in": ["a", "b", "c"],
                    "candidates_out": ["a", "b"],
                    "llm_reasoning": "reasoning turn 0",
                    "swarm_label_attempt": None, "swarm_confidence": None,
                    "cost_usd": 0.0014, "duration_ms": 587,
                    "provider_used": "anthropic:claude-sonnet-4-5",
                    "timestamp": "2027-02-15T10:00:00Z",
                },
                {
                    "turn": 1, "candidates_in": ["a", "b"],
                    "candidates_out": ["a"],
                    "llm_reasoning": "reasoning turn 1",
                    "swarm_label_attempt": "billing", "swarm_confidence": 0.62,
                    "cost_usd": 0.00135, "duration_ms": 542,
                    "provider_used": "anthropic:claude-sonnet-4-5",
                    "timestamp": "2027-02-15T10:00:01Z",
                },
                {
                    "turn": 2, "candidates_in": ["a"], "candidates_out": [],
                    "llm_reasoning": None,
                    "swarm_label_attempt": "fraud_review", "swarm_confidence": 0.91,
                    "cost_usd": 0.0, "duration_ms": 187,
                    "provider_used": None,
                    "timestamp": "2027-02-15T10:00:02Z",
                },
            ],
            "degraded_reason": None,
        }

    fake_backend.classify_mode3 = classify_mode3
    return fake_backend


@pytest.fixture
def m22_dispatcher(
    monkeypatch, fake_backend_with_mode3, auth_ctx, audit_emitter,
    rate_limiter, sampling_client, embedding_cache, bundle_cache, oci_factory,
):
    from mcp_server.dispatcher import Dispatcher
    from mcp_server.profile import Profile
    from mcp_server.tools.registry import build_tool_registry

    monkeypatch.setenv("EFE_FEATURE_M22_MODE3_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    return Dispatcher(
        registry=registry,
        backend=fake_backend_with_mode3,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


@pytest.fixture
def m22_off_dispatcher(
    monkeypatch, fake_backend, auth_ctx, audit_emitter, rate_limiter,
    sampling_client, embedding_cache, bundle_cache, oci_factory,
):
    from mcp_server.dispatcher import Dispatcher
    from mcp_server.profile import Profile
    from mcp_server.tools.registry import build_tool_registry

    monkeypatch.delenv("EFE_FEATURE_M22_MODE3_ENABLED", raising=False)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    return Dispatcher(
        registry=registry,
        backend=fake_backend,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


@pytest.mark.asyncio
async def test_classify_mode3_basic_path(m22_dispatcher):
    result = await m22_dispatcher.dispatch(
        tool_name="efe_classify_mode3",
        args={"sentence": "test", "catalog": "support-routing"},
    )
    assert result.is_error is False
    body = result.content
    assert body["final_label"] == "fraud_review"
    assert body["final_confidence"] == 0.91
    assert body["total_turns"] == 3
    assert body["converged"] is True
    assert len(body["concept_stack"]) == 3


@pytest.mark.asyncio
async def test_classify_mode3_returns_concept_stack(m22_dispatcher):
    result = await m22_dispatcher.dispatch(
        tool_name="efe_classify_mode3",
        args={"sentence": "x", "catalog": "x"},
    )
    stack = result.content["concept_stack"]
    assert len(stack) == 3
    assert stack[0]["turn"] == 0
    assert stack[1]["turn"] == 1
    assert stack[2]["turn"] == 2


@pytest.mark.asyncio
async def test_classify_mode3_validates_max_turns_range(m22_dispatcher):
    result = await m22_dispatcher.dispatch(
        tool_name="efe_classify_mode3",
        args={"sentence": "x", "catalog": "x", "max_turns": 99},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_classify_mode3_validates_threshold_range(m22_dispatcher):
    result = await m22_dispatcher.dispatch(
        tool_name="efe_classify_mode3",
        args={"sentence": "x", "catalog": "x", "threshold": 1.5},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_classify_mode3_blocks_when_feature_disabled(m22_off_dispatcher):
    result = await m22_off_dispatcher.dispatch(
        tool_name="efe_classify_mode3",
        args={"sentence": "test", "catalog": "x"},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_classify_mode3_emits_audit(m22_dispatcher, audit_emitter):
    audit_emitter.enabled = False
    before = len(audit_emitter.recent(limit=500))
    await m22_dispatcher.dispatch(
        tool_name="efe_classify_mode3",
        args={"sentence": "test", "catalog": "x"},
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1


@pytest.mark.asyncio
async def test_classify_mode3_returns_audit_id(m22_dispatcher):
    result = await m22_dispatcher.dispatch(
        tool_name="efe_classify_mode3",
        args={"sentence": "test", "catalog": "x"},
    )
    assert result.content["audit_id"] == 12345
