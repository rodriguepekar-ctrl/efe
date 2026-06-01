"""Dispatcher pin tests (M16) — happy paths + write confirm + isolation.

Each test exercises the dispatcher with the FakeBackend fixture.
"""
from __future__ import annotations

import pytest


# ─────────────────────────────────────────────────────────────────────
# Read tools
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_classify_happy_path(dispatcher, fake_backend):
    # Pre-register a catalog so classify finds it.
    await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "greetings",
            "classes": [{"name": "hi", "examples": ["hello"]}],
            "confirm": True,
        },
    )
    res = await dispatcher.dispatch(
        tool_name="efe_classify",
        args={"catalog": "greetings", "text": "hi there", "top_k": 3},
    )
    assert res.is_error is False
    assert res.content["catalog_name"] == "greetings"
    assert res.content["catalog_version"] == 1
    assert len(res.content["top_k"]) == 1


@pytest.mark.asyncio
async def test_classify_unknown_catalog_returns_not_found(dispatcher):
    res = await dispatcher.dispatch(
        tool_name="efe_classify",
        args={"catalog": "missing", "text": "hi"},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_found"


@pytest.mark.asyncio
async def test_list_catalogs_happy_path(dispatcher):
    await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "a",
            "classes": [{"name": "x", "examples": ["a"]}],
            "confirm": True,
        },
    )
    await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "b",
            "classes": [{"name": "y", "examples": ["b"]}],
            "confirm": True,
        },
    )
    res = await dispatcher.dispatch(tool_name="efe_list_catalogs", args={})
    assert res.is_error is False
    assert res.content["count"] == 2


@pytest.mark.asyncio
async def test_get_audit_log_returns_events(dispatcher):
    res = await dispatcher.dispatch(
        tool_name="efe_get_audit_log", args={"limit": 10}
    )
    assert res.is_error is False
    assert "events" in res.content
    assert res.content["count"] == 0  # FakeBackend has empty audit by default


@pytest.mark.asyncio
async def test_get_admin_quotas_returns_d12_limits(dispatcher):
    res = await dispatcher.dispatch(
        tool_name="efe_get_admin_quotas", args={}
    )
    assert res.is_error is False
    limits = res.content["limits"]
    assert limits["max_names_per_tenant"] == 100
    assert limits["max_classes_per_catalog"] == 256
    assert limits["max_sentences_per_class"] == 64


# ─────────────────────────────────────────────────────────────────────
# Write tools — confirm-required pattern (TM-M16-T-01)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_register_without_confirm_is_rejected(dispatcher):
    res = await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "x",
            "classes": [{"name": "c1", "examples": ["a"]}],
            "confirm": False,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "confirm_required"


@pytest.mark.asyncio
async def test_register_missing_confirm_field_fails_schema(dispatcher):
    res = await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "x",
            "classes": [{"name": "c1", "examples": ["a"]}],
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_register_with_confirm_succeeds(dispatcher, fake_backend):
    res = await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "approved-cat",
            "classes": [{"name": "c1", "examples": ["a"]}],
            "confirm": True,
        },
    )
    assert res.is_error is False
    assert res.content["registered"] is True
    assert res.content["version"] == 1
    assert any(c["op"] == "register" for c in fake_backend.calls)


@pytest.mark.asyncio
async def test_evolve_without_confirm_is_rejected(dispatcher):
    # Catalog must exist first
    await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "evol",
            "classes": [{"name": "c1", "examples": ["a"]}],
            "confirm": True,
        },
    )
    res = await dispatcher.dispatch(
        tool_name="efe_evolve_catalog",
        args={
            "name": "evol",
            "additional_examples_per_class": {"c1": ["new"]},
            "confirm": False,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "confirm_required"


@pytest.mark.asyncio
async def test_evolve_with_confirm_succeeds(dispatcher):
    await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "evol",
            "classes": [{"name": "c1", "examples": ["a"]}],
            "confirm": True,
        },
    )
    res = await dispatcher.dispatch(
        tool_name="efe_evolve_catalog",
        args={
            "name": "evol",
            "additional_examples_per_class": {"c1": ["new"]},
            "confirm": True,
        },
    )
    assert res.is_error is False
    assert res.content["evolved"] is True
    assert res.content["version"] == 2


# ─────────────────────────────────────────────────────────────────────
# Unknown tool / argument injection
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_unknown_tool_returns_unknown_tool_error(dispatcher):
    res = await dispatcher.dispatch(tool_name="nonexistent", args={})
    assert res.is_error is True
    assert res.content["code"] == "unknown_tool"


@pytest.mark.asyncio
async def test_extra_args_are_rejected_by_schema(dispatcher):
    """TM-M16-T-02 — additionalProperties:false defends arg injection."""
    res = await dispatcher.dispatch(
        tool_name="efe_classify",
        args={"catalog": "x", "text": "y", "tenant_id": "evil"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_dispatcher_forwards_caller_token_to_backend(dispatcher, fake_backend, auth_ctx):
    await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "auth-test",
            "classes": [{"name": "c1", "examples": ["a"]}],
            "confirm": True,
        },
    )
    last = fake_backend.calls[-1]
    assert last["token"] == auth_ctx.raw_token, "dispatcher must forward the caller's token"
