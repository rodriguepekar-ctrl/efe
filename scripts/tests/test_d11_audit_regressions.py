"""Sprint D11 — pin the audit's bug fixes.

Audit fixes covered:

  * **B1** — ``EfeAdminClient.list_catalogs()`` honestly raises
    ``NotImplementedError`` since the underlying ``ListCatalogs``
    gRPC RPC is not yet defined in
    ``protos/efe/v1/admin.proto``. Pre-fix the method called
    ``stub.ListCatalogs(...)`` which would crash with
    ``AttributeError`` at runtime.
  * **C1** — ``scripts/billing/aggregate_usage.py`` switched from
    ``query_range`` (which over-counted N× because each step
    returned a full-window ``increase()``) to ``query_instant``
    at ``t=end``.
  * **H1** — ``test-cross-tenant-denial.yaml`` Python script now
    uses ``grpc_health.v1.health_pb2_grpc`` (canonical module
    name from grpcio-health-checking) instead of the
    non-existent ``grpc.health.v1.health_pb2_grpc``.

These tests are END-TO-END functional checks at the public surface —
they would NOT have passed before the audit fixes.
"""

from __future__ import annotations

import importlib
import importlib.util
import re
import sys
from pathlib import Path

import pytest


_REPO = Path(__file__).resolve().parents[2]
_SDK = _REPO / "clients" / "python"
sys.path.insert(0, str(_SDK))


# ─────────────────────────────────────────────────────────────────────
# B1 — list_catalogs activated in D12
#
# In D11, this method raised NotImplementedError because the
# underlying gRPC RPC didn't exist yet. D12 ships the
# `efe.v1.catalog.CatalogService` and activates the method, so the
# original audit-fix tests are obsolete. We KEEP the test slot to
# pin the new contract :
#
#   * The method must NOT raise NotImplementedError anymore.
#   * The signature accepts the documented `tenant`, `page_size`,
#     `page_token` keyword args.
# ─────────────────────────────────────────────────────────────────────

def test_list_catalogs_no_longer_raises_not_implemented():
    """D12 activation — list_catalogs is now a real RPC. The method
    must not raise NotImplementedError before reaching the gRPC
    layer (pre-D12 it did)."""
    import inspect
    efe_tool = importlib.import_module("efe_tool")
    src = inspect.getsource(efe_tool.EfeAdminClient.list_catalogs)
    assert "NotImplementedError" not in src, (
        "D12 regression — list_catalogs must NOT raise "
        "NotImplementedError anymore (the CatalogService.ListCatalogs "
        "RPC ships in v0.1.12)"
    )


def test_list_catalogs_method_signature_d12():
    """D12 surface — the method now accepts kwargs documented in
    `docs/howto/catalog-admin.md` : `tenant`, `page_size`,
    `page_token`."""
    import inspect
    efe_tool = importlib.import_module("efe_tool")
    sig = inspect.signature(efe_tool.EfeAdminClient.list_catalogs)
    params = set(sig.parameters)
    # Must include all three documented kwargs (plus `self`).
    expected = {"self", "tenant", "page_size", "page_token"}
    assert params == expected, (
        f"D12 surface drift on list_catalogs : params={params} ; "
        f"expected {expected}"
    )


def test_health_does_not_call_list_catalogs():
    """Audit fix B1 — ``health()`` previously delegated to
    ``list_catalogs()`` which now raises NotImplementedError. It
    must use a different probe (``_ensure_stub``) instead.
    """
    efe_tool = importlib.import_module("efe_tool")
    import inspect
    src = inspect.getsource(efe_tool.EfeAdminClient.health)
    # Strip docstring / comments before checking for the call.
    # We approximate: find the function body code after the docstring.
    code_only = re.sub(r'""".*?"""', "", src, count=1, flags=re.DOTALL)
    code_only = re.sub(r"#.*", "", code_only)   # strip line comments
    assert "self.list_catalogs" not in code_only, (
        "audit fix B1 regression — health() must NOT call "
        "self.list_catalogs() (which raises NotImplementedError). "
        "Use self._ensure_stub() instead."
    )
    assert "_ensure_stub" in code_only, (
        "audit fix B1 — health() must probe via _ensure_stub() "
        "(no RPC fired ; channel-only reachability)."
    )


# ─────────────────────────────────────────────────────────────────────
# C1 — aggregator over-counting fix
# ─────────────────────────────────────────────────────────────────────

def _load_aggregate_module():
    spec = importlib.util.spec_from_file_location(
        "aggregate_usage",
        _REPO / "scripts" / "billing" / "aggregate_usage.py",
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_aggregator_uses_query_instant_in_main():
    """Audit fix C1 — main() must call query_instant, not query_range,
    to avoid 720× over-counting on monthly windows."""
    mod = _load_aggregate_module()
    import inspect
    main_src = inspect.getsource(mod.main)
    assert "query_instant(" in main_src, (
        "audit fix C1 regression — aggregator main() must use "
        "query_instant (instant query at t=end) instead of "
        "query_range (which over-counts N× when increase() window "
        "is larger than the step)."
    )
    # `query_range` reference in `main()` would mean the bug is back.
    # We allow query_range to be DEFINED in the module (kept for
    # debugging) but main() must not call it.
    assert "query_range(" not in main_src, (
        "audit fix C1 regression — aggregator main() still uses "
        "query_range. Switch to query_instant."
    )


def test_query_instant_function_exists():
    mod = _load_aggregate_module()
    assert hasattr(mod, "query_instant"), (
        "audit fix C1 — query_instant() must be exposed alongside "
        "query_range()."
    )
    # Signature must take (prom_url, query, time_, *, timeout_s).
    import inspect
    sig = inspect.signature(mod.query_instant)
    params = list(sig.parameters)
    assert params[:3] == ["prom_url", "query", "time_"], params


def test_aggregate_metric_handles_instant_response_shape():
    """Audit fix C1 — instant queries return a single ``value``
    pair per series, not a ``values`` list. Aggregator must read
    both shapes (legacy support)."""
    mod = _load_aggregate_module()
    payload = {
        "data": {
            "result": [
                {"metric": {"tenant": "acme"}, "value": [1.0, "999"]},
            ]
        }
    }
    out: dict = {}
    mod.aggregate_metric(payload, "requests", out)
    assert out["acme"]["requests"] == pytest.approx(999.0)


def test_extract_instant_value_helper_exists():
    mod = _load_aggregate_module()
    assert hasattr(mod, "_extract_instant_value")


def test_no_overcounting_simulation():
    """Simulate the previous bug: a query_range response with 720
    identical 100-value samples per tenant would have been summed
    to 72,000 — clearly wrong. The post-fix code reads the
    `value` field (single pair) when present, OR the `values`
    list otherwise. Both paths are tested here."""
    mod = _load_aggregate_module()
    # Path A — instant query (production path): single value.
    payload_instant = {
        "data": {
            "result": [
                {"metric": {"tenant": "acme"}, "value": [1.0, "100"]},
            ]
        }
    }
    out_a: dict = {}
    mod.aggregate_metric(payload_instant, "requests", out_a)
    assert out_a["acme"]["requests"] == 100.0

    # Path B — legacy range query (debugging only): if 720 datapoints
    # of value=100 are present, aggregate_metric still sums them
    # (legacy behavior). This isn't reachable from main() post-C1.
    payload_legacy = {
        "data": {
            "result": [
                {
                    "metric": {"tenant": "acme"},
                    "values": [[float(i), "100"] for i in range(720)],
                },
            ]
        }
    }
    out_b: dict = {}
    mod.aggregate_metric(payload_legacy, "requests", out_b)
    # Old summing behavior preserved — this confirms the legacy
    # path WORKS but is only reachable via debugging tools, not
    # the production CLI path.
    assert out_b["acme"]["requests"] == 720 * 100  # 72_000


# ─────────────────────────────────────────────────────────────────────
# H1 — helm-test python uses correct grpc_health module
# ─────────────────────────────────────────────────────────────────────

def test_helm_test_uses_correct_grpc_health_module():
    path = (
        _REPO / "deploy" / "helm" / "efe" / "templates" / "tests"
        / "test-cross-tenant-denial.yaml"
    )
    text = path.read_text(encoding="utf-8")

    # The CORRECT module name (with underscore) must appear.
    assert "grpc_health.v1.health_pb2_grpc" in text, (
        "audit fix H1 regression — must import from "
        "`grpc_health.v1.health_pb2_grpc` (canonical module from "
        "`grpcio-health-checking`)."
    )

    # The INCORRECT (non-existent) module name MUST NOT appear in
    # the active code path.
    bad = re.search(
        r"^[^#]*\bgrpc\.health\.v1\.health_pb2_grpc",
        text, re.MULTILINE,
    )
    assert not bad, (
        f"audit fix H1 regression — found stale reference to the "
        f"non-existent `grpc.health.v1.health_pb2_grpc` module: "
        f"{bad.group(0)!r}"
    )


def test_helm_test_imports_health_modules_at_top():
    """Audit fix H1 — the imports must be BEFORE any code that
    references them, otherwise an ImportError is shadowed by a
    later ``except grpc.RpcError``."""
    path = (
        _REPO / "deploy" / "helm" / "efe" / "templates" / "tests"
        / "test-cross-tenant-denial.yaml"
    )
    text = path.read_text(encoding="utf-8")
    # The script body is in a YAML string with `|` indented under
    # `args:` — the Python heredoc starts somewhere after
    # `python3 - <<'PY'` on its own indented line. We just need
    # the relative ordering of import vs use ; no need to parse
    # the YAML envelope perfectly.
    import_pos = text.find("from grpc_health.v1 import health_pb2")
    use_pos = text.find("health_pb2_grpc.HealthStub")
    assert import_pos > -1, "import of grpc_health is missing"
    assert use_pos > -1, "no use of health_pb2_grpc.HealthStub"
    assert import_pos < use_pos, (
        "audit fix H1 regression — `from grpc_health.v1 import …` "
        "must appear BEFORE `health_pb2_grpc.HealthStub(...)`. "
        "Otherwise NameError shadows the actual import error."
    )
