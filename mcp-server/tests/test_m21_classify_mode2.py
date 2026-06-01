"""efe_classify_mode2 (M21) — pin tests."""
from __future__ import annotations

import pytest


@pytest.fixture
def fake_backend_with_mode2(fake_backend):
    """Augment FakeBackend with classify_mode2 + budget mocks."""

    async def classify_mode2(
        *, sentence, catalog, version=None, mode="MODE_2",
        top_k_anchors=10, threshold_override=None, auth,
    ):
        return {
            "label": "hardware_issue",
            "confidence": 0.92,
            "anchors_used": ["hardware", "boot", "device"],
            "mode": mode,
            "tokens_in": 250,
            "tokens_out": 45,
            "cost_usd": 0.0014,
            "provider_used": "anthropic:claude-sonnet-4-5",
            "model_used": "claude-sonnet-4-5",
            "cache_hit": False,
            "duration_ms": 587,
        }

    fake_backend.classify_mode2 = classify_mode2
    return fake_backend


@pytest.fixture
def m21_dispatcher(
    monkeypatch,
    fake_backend_with_mode2,
    auth_ctx,
    audit_emitter,
    rate_limiter,
    sampling_client,
    embedding_cache,
    bundle_cache,
    oci_factory,
):
    """Dispatcher with M21 flag ON."""
    from mcp_server.dispatcher import Dispatcher
    from mcp_server.profile import Profile
    from mcp_server.tools.registry import build_tool_registry

    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    return Dispatcher(
        registry=registry,
        backend=fake_backend_with_mode2,
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
def m21_off_dispatcher(
    monkeypatch,
    fake_backend,
    auth_ctx,
    audit_emitter,
    rate_limiter,
    sampling_client,
    embedding_cache,
    bundle_cache,
    oci_factory,
):
    from mcp_server.dispatcher import Dispatcher
    from mcp_server.profile import Profile
    from mcp_server.tools.registry import build_tool_registry

    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
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
async def test_classify_mode2_basic_path(m21_dispatcher):
    result = await m21_dispatcher.dispatch(
        tool_name="efe_classify_mode2",
        args={
            "sentence": "My laptop won't boot",
            "catalog": "support-routing",
        },
    )
    assert result.is_error is False
    body = result.content
    assert body["label"] == "hardware_issue"
    assert body["confidence"] == 0.92
    assert len(body["anchors_used"]) == 3
    assert body["mode"] == "MODE_2"


@pytest.mark.asyncio
async def test_classify_mode2_includes_cost_metadata(m21_dispatcher):
    result = await m21_dispatcher.dispatch(
        tool_name="efe_classify_mode2",
        args={"sentence": "test", "catalog": "x"},
    )
    body = result.content
    assert body["cost_usd"] == 0.0014
    assert body["tokens_in"] == 250
    assert body["tokens_out"] == 45
    assert body["provider_used"] == "anthropic:claude-sonnet-4-5"


@pytest.mark.asyncio
async def test_classify_mode2_mode_auto_esc(m21_dispatcher):
    result = await m21_dispatcher.dispatch(
        tool_name="efe_classify_mode2",
        args={
            "sentence": "test",
            "catalog": "x",
            "mode": "MODE_AUTO_ESC",
            "threshold_override": 0.7,
        },
    )
    assert result.is_error is False
    assert result.content["mode"] == "MODE_AUTO_ESC"


@pytest.mark.asyncio
async def test_classify_mode2_validates_top_k_range(m21_dispatcher):
    result = await m21_dispatcher.dispatch(
        tool_name="efe_classify_mode2",
        args={
            "sentence": "test",
            "catalog": "x",
            "top_k_anchors": 100,  # > max 20
        },
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_classify_mode2_blocks_when_feature_disabled(m21_off_dispatcher):
    result = await m21_off_dispatcher.dispatch(
        tool_name="efe_classify_mode2",
        args={"sentence": "test", "catalog": "x"},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_classify_mode2_emits_audit_event(m21_dispatcher, audit_emitter):
    audit_emitter.enabled = False
    before = len(audit_emitter.recent(limit=500))
    await m21_dispatcher.dispatch(
        tool_name="efe_classify_mode2",
        args={"sentence": "test", "catalog": "x"},
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1
    last = audit_emitter.recent(limit=1)[0]
    assert last.tool == "efe_classify_mode2"


@pytest.mark.asyncio
async def test_classify_mode2_rejects_invalid_mode(m21_dispatcher):
    result = await m21_dispatcher.dispatch(
        tool_name="efe_classify_mode2",
        args={
            "sentence": "test",
            "catalog": "x",
            "mode": "MODE_3",  # Not yet allowed in M21
        },
    )
    assert result.is_error is True
