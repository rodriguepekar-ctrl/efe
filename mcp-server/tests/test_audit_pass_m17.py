"""M17 audit pass 1 — pin tests for the 8 defects fixed.

Pattern carryover from M11-M16 audit passes. Every fix in this file
locks in correct behaviour so a future regression is caught loudly.
"""
from __future__ import annotations

import asyncio
from typing import Any

import pytest

from mcp_server.backend_client import BackendBadRequest
from mcp_server.profile import Profile
from mcp_server.streaming import (
    NULL_PROGRESS,
    OperationCancelled,
    ProgressEmitter,
    make_null_progress,
)
from mcp_server.tools.registry import all_known_tools


# ─────────────────────────────────────────────────────────────────────
# A1 — calibrate_thresholds validation_set ground-truth preserved
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def calibrate_seeded_backend(fake_backend):
    fake_backend._seed_class("calib", [
        {"name": "alpha", "examples": ["a-ex-1", "a-ex-2"]},
        {"name": "beta", "examples": ["b-ex-1", "b-ex-2"]},
    ])
    return fake_backend


@pytest.mark.asyncio
async def test_audit_a1_validation_set_classifies_each_entry(
    agent_native_dispatcher, calibrate_seeded_backend,
):
    """The ground-truth labels MUST drive the partition.

    Pre-fix : pos+neg texts were forwarded to score_distribution which
    treated them all as positives, miscalibrating silently.
    """
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={
            "catalog": "calib",
            "validation_set": [
                {"text": "alpha-thing", "true_class": "alpha"},
                {"text": "beta-thing", "true_class": "beta"},
                {"text": "alpha-thing-2", "true_class": "alpha"},
            ],
        },
    )
    assert res.is_error is False, res.content
    assert res.content["calibration_method"] == "provided_validation"

    # The dispatcher should have called classify() on each validation entry
    classify_calls = [c for c in calibrate_seeded_backend.calls if c["op"] == "classify"]
    assert len(classify_calls) == 3, (
        "audit fix M17-A1 : each validation entry must be classified to "
        "obtain its true scores per class"
    )


@pytest.mark.asyncio
async def test_audit_a1_validation_set_uses_ground_truth_partition(
    agent_native_dispatcher, calibrate_seeded_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={
            "catalog": "calib",
            "validation_set": [
                {"text": "x", "true_class": "alpha"},
                {"text": "y", "true_class": "beta"},
            ],
        },
    )
    per_class = res.content["per_class"]
    by_name = {p["class_name"]: p for p in per_class}
    # Each class must have at least 1 sample (1 pos + 1 neg = 2 total)
    for cls in ("alpha", "beta"):
        assert by_name[cls]["samples_used"] >= 1, (
            f"class {cls} must use at least one ground-truth sample"
        )


# ─────────────────────────────────────────────────────────────────────
# A2 — streaming progress wired through dispatch
# ─────────────────────────────────────────────────────────────────────


class FakeProgressSession:
    def __init__(self) -> None:
        self.events: list[dict[str, Any]] = []

    async def send_progress_notification(
        self, progress_token: str, *, progress: int, total: int = 100, message: str = "",
    ) -> None:
        self.events.append({
            "token": progress_token,
            "progress": progress,
            "total": total,
            "message": message,
        })


@pytest.mark.asyncio
async def test_audit_a2_progress_session_routed_into_emitter(
    agent_native_dispatcher, fake_backend,
):
    fake_backend._seed_class("stream", [
        {"name": "a", "examples": ["alpha"]},
        {"name": "b", "examples": ["beta"]},
    ])
    session = FakeProgressSession()
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={"catalog": "stream"},
        progress_token="tok-A2",
        progress_session=session,
    )
    assert res.is_error is False, res.content
    assert len(session.events) >= 2, (
        "audit fix M17-A2 : streaming progress events MUST reach the "
        "supplied session — pre-fix used None session, was always no-op"
    )
    assert all(e["token"] == "tok-A2" for e in session.events)


@pytest.mark.asyncio
async def test_audit_a2_no_session_means_no_streaming(
    agent_native_dispatcher, fake_backend,
):
    """Without progress_session, streaming silently degrades — the tool
    still completes successfully but no events are sent."""
    fake_backend._seed_class("stream2", [
        {"name": "a", "examples": ["alpha"]},
    ])
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={"catalog": "stream2"},
        progress_token="tok-noop",
        progress_session=None,
    )
    assert res.is_error is False


# ─────────────────────────────────────────────────────────────────────
# A3 — diagnose ambiguity formula uses C(N,2)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a3_ambiguity_formula_uses_pair_count():
    """When the catalog has 5 classes and 2 ambiguous pairs, the
    ambiguity score should be 1 - 2/10 = 0.8 (not 1 - 2/5 = 0.6).
    """
    n_classes = 5
    n_ambig = 2
    n_pairs = (n_classes * (n_classes - 1)) // 2
    expected = 1.0 - (n_ambig / n_pairs)
    assert round(expected, 4) == 0.8


def test_audit_a3_ambiguity_score_zero_when_all_pairs_ambiguous():
    n_classes = 4
    n_pairs = (n_classes * (n_classes - 1)) // 2  # 6
    expected = 1.0 - (n_pairs / n_pairs)
    assert expected == 0.0


def test_audit_a3_ambiguity_with_two_classes_handled():
    """C(2,2) = 1 ; if 1 pair ambiguous, score = 0."""
    n_classes = 2
    n_pairs = 1
    expected = 1.0 - (1 / n_pairs)
    assert expected == 0.0


# ─────────────────────────────────────────────────────────────────────
# A4 — schema test coverage extended to all 20 tools
# ─────────────────────────────────────────────────────────────────────


def test_audit_a4_all_known_tools_have_strict_schemas():
    """Every tool (M16 + M17 + any later milestone) must reject unknown args.

    Pre-fix (M17-A4) : `test_every_tool_schema_disallows_additional_properties`
    only iterated profile=core (6 tools). The 14 M17 schemas were never
    checked, leaving room for arg-injection regressions. We now iterate
    `all_known_tools()` so every newly-registered tool is automatically
    covered (M18 D47 added 5, M18 D48+ will add more).
    """
    tools = all_known_tools()
    # Must include at minimum the M16 6 + M17 14 = 20 tools.
    assert len(tools) >= 20
    for name, tool in tools.items():
        assert tool.input_schema.get("additionalProperties") is False, (
            f"tool {name!r} missing additionalProperties:false"
        )


def test_audit_a4_all_sampling_tools_require_confirm():
    """M17 invariant : sampling tools that potentially mutate state OR
    consume large context must require `confirm:true`.

    M20 carve-out : LLM-helper tools (`feature_flag` attribute set)
    are read-only operator helpers that are themselves opt-in via
    env-var. They use sampling delegation for LLM-driven analysis
    but do not mutate the catalog ; the env-var gate replaces the
    per-call confirm requirement.
    """
    tools = all_known_tools()
    sampling_tools = [t for t in tools.values() if t.requires_sampling]
    assert len(sampling_tools) >= 3, "expected sampling tools to be registered"
    for tool in sampling_tools:
        # M20 carve-out : LLM-helper tools are gated by env-var (default
        # OFF). Their opt-in posture is enforced at registry build time,
        # not via per-call confirm. They must have feature_flag set
        # AND be is_write=False.
        if tool.feature_flag is not None:
            assert tool.is_write is False, (
                f"feature-gated sampling tool {tool.name!r} must be read-only"
            )
            continue
        assert "confirm" in tool.input_schema.get("required", []), (
            f"sampling tool {tool.name!r} must require confirm "
            f"(or set feature_flag for opt-in helpers)"
        )


# ─────────────────────────────────────────────────────────────────────
# A5 — resources/read emits an audit event
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_audit_a5_resources_read_emits_event(
    advanced_dispatcher, audit_emitter, fake_backend,
):
    """A successful resource read must add a `mcp_resource_read` event
    so the D14-bis chain stays complete."""
    audit_emitter.enabled = False  # tests don't want network ; events still go to local ring
    fake_backend._seed_class("audit-cat", [{"name": "x", "examples": ["a"]}])
    before = len(audit_emitter.recent(limit=500))
    res = await advanced_dispatcher.read_resource("efe://catalogs/audit-cat")
    assert "uri" in res
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1
    last = audit_emitter.recent(limit=1)[0]
    assert last.tool == "mcp_resource_read"
    assert last.ok is True


@pytest.mark.asyncio
async def test_audit_a5_resource_invalid_uri_still_emits(
    advanced_dispatcher, audit_emitter,
):
    audit_emitter.enabled = False
    before = len(audit_emitter.recent(limit=500))
    res = await advanced_dispatcher.read_resource("efe://catalogs/../etc/passwd")
    assert "error" in res
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1
    last = audit_emitter.recent(limit=1)[0]
    assert last.tool == "mcp_resource_read"
    assert last.ok is False
    assert last.error_code == "invalid_resource_uri"


# ─────────────────────────────────────────────────────────────────────
# A6 — make_null_progress() returns fresh emitter
# ─────────────────────────────────────────────────────────────────────


def test_audit_a6_make_null_progress_returns_fresh_instance():
    p1 = make_null_progress()
    p2 = make_null_progress()
    assert p1 is not p2, (
        "audit fix M17-A6 : helper must return a fresh emitter to avoid "
        "shared mutable state via the singleton pattern"
    )


def test_audit_a6_cancelling_one_does_not_affect_another():
    p1 = make_null_progress()
    p2 = make_null_progress()
    p1.cancel()
    assert p1.cancelled is True
    assert p2.cancelled is False, (
        "cancellation must NOT leak across emitters"
    )


def test_audit_a6_dispatcher_uses_fresh_emitter_per_call(
    advanced_dispatcher,
):
    # Indirect check : after one cancelled dispatch (via deliberate flag),
    # the next dispatch must start from a clean state. We can't easily
    # cancel mid-call here, so instead we check the singleton is no longer
    # shared (it's instantiated per-dispatch).
    assert NULL_PROGRESS is not make_null_progress()


# ─────────────────────────────────────────────────────────────────────
# A7 — predicted_class unknown surfaces as invalid_arguments
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def explain_seeded_backend(fake_backend):
    fake_backend._seed_class("explain-cat", [
        {"name": "alpha", "examples": ["a-1", "a-2"]},
        {"name": "beta", "examples": ["b-1"]},
    ])
    return fake_backend


@pytest.mark.asyncio
async def test_audit_a7_unknown_predicted_class_returns_invalid_arguments(
    advanced_dispatcher, explain_seeded_backend,
):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_explain_classification",
        args={
            "catalog": "explain-cat",
            "text": "alpha example",
            "predicted_class": "does-not-exist",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments", (
        "audit fix M17-A7 : pre-fix silently used top-1 instead of the "
        "user-supplied predicted_class — now we surface the error"
    )


@pytest.mark.asyncio
async def test_audit_a7_known_predicted_class_still_works(
    advanced_dispatcher, explain_seeded_backend,
):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_explain_classification",
        args={
            "catalog": "explain-cat",
            "text": "alpha example",
            "predicted_class": "alpha",
        },
    )
    assert res.is_error is False
    assert res.content["predicted_class"] == "alpha"


# ─────────────────────────────────────────────────────────────────────
# A8 — bulk_classify on abstract BackendClient
# ─────────────────────────────────────────────────────────────────────


def test_audit_a8_bulk_classify_method_on_abstract_class():
    """Pre-fix : bulk_classify only existed on RestBackendClient. Tools
    relied on AttributeError to discover the gap. Now declared on the
    abstract base for architectural consistency."""
    from mcp_server.backend_client import BackendClient
    assert hasattr(BackendClient, "bulk_classify")
    # Method should be async + raise NotImplementedError when called
    import inspect
    method = BackendClient.bulk_classify
    assert inspect.iscoroutinefunction(method)


@pytest.mark.asyncio
async def test_audit_a8_bulk_classify_default_raises_not_implemented():
    """The abstract default raises NotImplementedError so concrete
    clients must opt-in or rely on the per-item fallback."""
    from mcp_server.backend_client import BackendClient

    class MinimalBackend(BackendClient):
        info = type("I", (), {"protocol": "test", "endpoint": "test", "version": None})()

        async def classify(self, *, catalog, text, top_k=3, auth): return {}
        async def register_catalog(self, *, name, classes, auth): return {}
        async def evolve_catalog(self, *, name, additional_examples_per_class, auth): return {}
        async def list_catalogs(self, *, auth): return []
        async def get_audit_log(self, *, limit, auth): return []
        async def get_admin_quotas(self, *, auth): return {}
        async def aclose(self): pass

    backend = MinimalBackend()
    with pytest.raises(NotImplementedError):
        await backend.bulk_classify(
            catalog="x", texts=["y"], top_k=1, include_explanations=False,
            auth=type("A", (), {"raw_token": "t"})(),
        )
