"""Pin tests for M22 review queue tools (get / approve / reject) +
get_concept_stack."""
from __future__ import annotations

import pytest


@pytest.fixture
def fake_backend_with_review_queue(fake_backend):
    """Augment FakeBackend with mode3 review queue + concept_stack."""

    async def list_mode3_reviews(*, status, limit, offset, auth):
        if status == "all":
            reviews = [
                {"review_token": "ttt-pending-1", "status": "pending"},
                {"review_token": "ttt-approved-1", "status": "approved"},
            ]
        elif status == "pending":
            reviews = [{"review_token": "ttt-pending-1", "status": "pending"}]
        else:
            reviews = []
        return {"reviews": reviews[offset:offset + limit], "has_more": False}

    async def approve_mode3_review(*, review_token, final_label, review_notes, auth):
        return {
            "final_label": final_label or "billing",
            "reviewed_at": "2027-02-15T10:00:00Z",
            "reviewed_by": "operator@example.com",
        }

    async def reject_mode3_review(*, review_token, reason, auth):
        return {
            "reviewed_at": "2027-02-15T10:00:00Z",
            "reviewed_by": "operator@example.com",
        }

    async def get_concept_stack(*, audit_id, include_pii_unredacted, auth):
        return {
            "audit_id": audit_id,
            "tenant_id_hash": "abc123de",
            "catalog": "support-routing",
            "bundle_version": "v1.2.0",
            "mode": "MODE_3",
            "total_turns": 3,
            "total_cost_usd": 0.00275,
            "converged": True,
            "final_label": "fraud_review",
            "final_confidence": 0.91,
            "concept_stack": [{"turn": 0}, {"turn": 1}, {"turn": 2}],
            "created_at": "2027-02-15T10:00:00Z",
        }

    fake_backend.list_mode3_reviews = list_mode3_reviews
    fake_backend.approve_mode3_review = approve_mode3_review
    fake_backend.reject_mode3_review = reject_mode3_review
    fake_backend.get_concept_stack = get_concept_stack
    return fake_backend


@pytest.fixture
def m22_dispatcher_review(
    monkeypatch, fake_backend_with_review_queue, auth_ctx, audit_emitter,
    rate_limiter, sampling_client, embedding_cache, bundle_cache, oci_factory,
):
    from mcp_server.dispatcher import Dispatcher
    from mcp_server.profile import Profile
    from mcp_server.tools.registry import build_tool_registry

    monkeypatch.setenv("EFE_FEATURE_M22_MODE3_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    return Dispatcher(
        registry=registry,
        backend=fake_backend_with_review_queue,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


# ─── efe_get_mode3_reviews ─────────────────────────────────────────


@pytest.mark.asyncio
async def test_get_mode3_reviews_pending_default(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_get_mode3_reviews",
        args={},
    )
    assert result.is_error is False
    body = result.content
    assert body["count"] == 1


@pytest.mark.asyncio
async def test_get_mode3_reviews_all_status(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_get_mode3_reviews",
        args={"status": "all"},
    )
    assert result.content["count"] == 2


@pytest.mark.asyncio
async def test_get_mode3_reviews_invalid_status(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_get_mode3_reviews",
        args={"status": "weird_state"},
    )
    assert result.is_error is True


# ─── efe_approve_mode3_review ──────────────────────────────────────


@pytest.mark.asyncio
async def test_approve_review_requires_confirm(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_approve_mode3_review",
        args={"review_token": "00000000-0000-4000-8000-000000000001"},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_approve_review_basic_path(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_approve_mode3_review",
        args={
            "review_token": "00000000-0000-4000-8000-000000000001",
            "final_label": "fraud_review",
            "confirm": True,
        },
    )
    assert result.is_error is False
    body = result.content
    assert body["status"] == "approved"
    assert body["final_label"] == "fraud_review"


@pytest.mark.asyncio
async def test_approve_review_validates_token_length(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_approve_mode3_review",
        args={"review_token": "too-short", "confirm": True},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_approve_review_emits_audit(m22_dispatcher_review, audit_emitter):
    audit_emitter.enabled = False
    before = len(audit_emitter.recent(limit=500))
    await m22_dispatcher_review.dispatch(
        tool_name="efe_approve_mode3_review",
        args={
            "review_token": "00000000-0000-4000-8000-000000000001",
            "confirm": True,
        },
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1


# ─── efe_reject_mode3_review ───────────────────────────────────────


@pytest.mark.asyncio
async def test_reject_review_requires_confirm(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_reject_mode3_review",
        args={
            "review_token": "00000000-0000-4000-8000-000000000001",
            "reason": "Wrong label, manual override needed",
        },
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_reject_review_requires_min_reason_length(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_reject_mode3_review",
        args={
            "review_token": "00000000-0000-4000-8000-000000000001",
            "reason": "no",  # < 10 chars
            "confirm": True,
        },
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_reject_review_basic_path(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_reject_mode3_review",
        args={
            "review_token": "00000000-0000-4000-8000-000000000001",
            "reason": "LLM hallucinated, manual override",
            "confirm": True,
        },
    )
    assert result.is_error is False
    body = result.content
    assert body["status"] == "rejected"


# ─── efe_get_concept_stack ─────────────────────────────────────────


@pytest.mark.asyncio
async def test_get_concept_stack_basic(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_get_concept_stack",
        args={"audit_id": 12345},
    )
    assert result.is_error is False
    body = result.content
    assert body["audit_id"] == 12345
    assert body["mode"] == "MODE_3"
    assert body["total_turns"] == 3
    assert len(body["concept_stack"]) == 3


@pytest.mark.asyncio
async def test_get_concept_stack_validates_audit_id(m22_dispatcher_review):
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_get_concept_stack",
        args={"audit_id": -1},  # < min 1
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_get_concept_stack_pii_unredacted_blocked_in_basic_profile(
    m22_dispatcher_review,
):
    """include_pii_unredacted=true should be rejected in agent-native
    profile (admin-only)."""
    result = await m22_dispatcher_review.dispatch(
        tool_name="efe_get_concept_stack",
        args={"audit_id": 12345, "include_pii_unredacted": True},
    )
    assert result.is_error is True
    assert "forbidden" in str(result.content.get("code", "")).lower() or \
           "agent-native-deploy" in str(result.content.get("error", "")).lower()
