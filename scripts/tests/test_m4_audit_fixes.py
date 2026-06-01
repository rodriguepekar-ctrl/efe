"""Sprint **M4 audit pass 1** — pin tests anti-régression pour les
9 défauts trouvés dans le code drop M4 initial (D21/D22/D27/D32/D34/D37).

Chaque test vérifie qu'un fix spécifique est landé et reste landé. Une
régression sur l'un de ces points fait échouer le test correspondant.

| ID | Sévérité | Catégorie | Couvert par |
|----|---|---|---|
| M4-A1 | CRIT | Wiring forgotten | `test_a1_*` |
| M4-A2 | CRIT | Backend non-câblé | `test_a2_*` |
| M4-A3 | HIGH | Backend non-câblé | `test_a3_*` |
| M4-A4 | HIGH | UX | `test_a4_*` |
| M4-A5 | HIGH | UX | `test_a5_*` |
| M4-A6 | MED | Wiring forgotten | `test_a6_*` |
| M4-A7 | MED | Telemetry gap | `test_a7_*` |
| M4-A8 | MED | Audit log | `test_a8_*` |
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
ADMIN_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "admin.rs"
MODE2_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode2" / "mod.rs"
MODE3_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode3" / "mod.rs"
RUNTIME_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "runtime.rs"
SYNTH_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "synthetic_spec.rs"
REQUEST_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "swarm" / "request.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# M4-A1 — runtime.rs admin catalog + bundle wiring
# ─────────────────────────────────────────────────────────────────────


def test_a1_runtime_chains_with_catalog_registry() -> None:
    """`AdminServiceImpl::with_catalog_registry()` is called from
    runtime.rs — pre-fix the builder method existed but was never
    called, so all 8 D21 catalog RPCs returned FailedPrecondition."""
    rs = _read(RUNTIME_RS)
    assert "with_catalog_registry(self.catalog_registry.clone())" in rs


def test_a1_runtime_conditionally_chains_with_bundle_reload() -> None:
    """`with_bundle_reload(...)` is called when trust_store is wired."""
    rs = _read(RUNTIME_RS)
    assert "with_bundle_reload(" in rs
    # Gated on `embedder_trust_store.is_some()` — verified via the
    # `if let Some(trust)` pattern.
    block_start = rs.find("if let Some(trust) = self.embedder_trust_store")
    assert block_start >= 0, "bundle reload wiring is not gated on trust store presence"


def test_a1_server_state_holds_active_bundle_slot() -> None:
    rs = _read(RUNTIME_RS)
    assert "active_bundle_slot: Arc<crate::embedder::ActiveBundleSlot>" in rs


def test_a1_server_state_holds_trust_store() -> None:
    rs = _read(RUNTIME_RS)
    assert "embedder_trust_store: Option<Arc<crate::embedder::TrustStore>>" in rs


def test_a1_build_embedder_registry_returns_bundle_wiring_struct() -> None:
    """The factory now returns a typed triple instead of just the
    registry."""
    rs = _read(RUNTIME_RS)
    assert "struct BundleWiring" in rs
    assert "active_bundle_slot:" in rs
    assert "trust_store:" in rs


# ─────────────────────────────────────────────────────────────────────
# M4-A2 — Mode-3 dispatch + backend invocation
# ─────────────────────────────────────────────────────────────────────


def test_a2_execute_sub_swarm_takes_backend_handle() -> None:
    """The function signature must include the backend handles ;
    pre-fix it took only `(swarm_idx, seed, mode)`."""
    rs = _read(MODE3_RS)
    sig_start = rs.find("async fn execute_sub_swarm(")
    assert sig_start >= 0
    sig_end = rs.find(") -> Result<SubVerdictData", sig_start)
    sig = rs[sig_start:sig_end]
    for required in (
        "query_text:",
        "tenant_id:",
        "mode1_backend:",
        "mode2:",
        "router_id:",
    ):
        assert required in sig, f"execute_sub_swarm signature missing parameter `{required}`"


def test_a2_execute_sub_swarm_calls_mode1_backend() -> None:
    rs = _read(MODE3_RS)
    body_start = rs.find("async fn execute_sub_swarm(")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    assert "mode1_backend" in body
    assert "mode1_backend\n                .execute(" in body or "mode1_backend\n                .execute(req)" in body or ".execute(req)" in body


def test_a2_execute_sub_swarm_dispatches_on_mode_per_call() -> None:
    rs = _read(MODE3_RS)
    body_start = rs.find("async fn execute_sub_swarm(")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    assert '"mode1" =>' in body
    assert '"mode2" =>' in body
    # Pre-fix both arms were identical synthesis ; now mode2 calls
    # `m2.execute_inner(...)`.
    assert "execute_inner(" in body


def test_a2_no_synthesis_constant_in_execute_sub_swarm() -> None:
    """Anti-regression : the old `0.5 + (swarm_idx as f32) * 0.1`
    formula must NOT reappear."""
    rs = _read(MODE3_RS)
    body_start = rs.find("async fn execute_sub_swarm(")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    assert "0.5 + (swarm_idx as f32) * 0.1" not in body, (
        "regression to old synthesis ; Mode-3 sub-swarm must call backend"
    )


def test_a2_mode2_exposes_execute_inner_helper() -> None:
    """Mode-2's internal helper must be public-in-crate so Mode-3
    can call it without going through the tonic Request wrapping."""
    rs = _read(MODE2_RS)
    assert "pub(crate) async fn execute_inner(" in rs
    # Signature must include the seed parameter so Mode-3 can
    # propagate per-sub-swarm seeds.
    sig_start = rs.find("pub(crate) async fn execute_inner(")
    sig_end = rs.find(") -> Result<", sig_start)
    sig = rs[sig_start:sig_end]
    for arg in ("query_text:", "top_k_anchors:", "router_id:", "tenant_id:", "seed:"):
        assert arg in sig, f"execute_inner missing parameter `{arg}`"


# ─────────────────────────────────────────────────────────────────────
# M4-A3 — Mode-2 invoke_backend really calls the backend
# ─────────────────────────────────────────────────────────────────────


def test_a3_invoke_backend_signature_no_underscore_prefix() -> None:
    """Pre-fix the function had `_backend: &DynExecuteBackend, _query: &str` —
    the underscore prefix silenced the unused-param warning. Both
    must be live params now."""
    rs = _read(MODE2_RS)
    sig_start = rs.find("async fn invoke_backend_with_anchors(")
    sig_end = rs.find(") -> Result<", sig_start)
    sig = rs[sig_start:sig_end]
    assert " backend: &DynExecuteBackend" in sig, (
        "backend param must be live (no underscore prefix)"
    )
    assert " query: &str" in sig, "query param must be live"
    assert " tenant_id: &str" in sig, "tenant_id param must be live"


def test_a3_invoke_backend_calls_backend_execute() -> None:
    rs = _read(MODE2_RS)
    body_start = rs.find("async fn invoke_backend_with_anchors(")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    assert "backend\n        .execute(" in body or "backend.execute(" in body, (
        "invoke_backend_with_anchors must actually call backend.execute()"
    )


def test_a3_no_synthesis_from_anchor_confidence_only() -> None:
    """Pre-fix the function did `let confidence = top.confidence` and
    returned a hardcoded `Verdict::Converged as i32`. The fix calls
    backend and uses `resp.confidence` + `verdict_to_proto_i32(&resp.verdict)`."""
    rs = _read(MODE2_RS)
    body_start = rs.find("async fn invoke_backend_with_anchors(")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    assert "verdict_to_proto_i32(&resp.verdict)" in body, (
        "verdict must come from real swarm response, not synthesis"
    )


# ─────────────────────────────────────────────────────────────────────
# M4-A4 — ListAuditEvents returns Unimplemented
# ─────────────────────────────────────────────────────────────────────


def test_a4_list_audit_events_returns_unimplemented() -> None:
    rs = _read(ADMIN_RS)
    h_start = rs.find("async fn list_audit_events(")
    h_end = rs.find("\n    async fn breach_history", h_start)
    body = rs[h_start:h_end]
    assert "Status::unimplemented(" in body
    # Anti-regression : the old empty-stream pattern must NOT be back.
    assert "drop(tx);" not in body, (
        "list_audit_events must NOT silently drop tx ; that returned an "
        "empty stream indistinguishable from 'no events'"
    )


# ─────────────────────────────────────────────────────────────────────
# M4-A5 — EvolveCatalog points at CatalogService alternative
# ─────────────────────────────────────────────────────────────────────


def test_a5_evolve_catalog_message_is_actionable() -> None:
    """The Unimplemented status must point at the working alternative
    so operators know what to do RIGHT NOW."""
    rs = _read(ADMIN_RS)
    h_start = rs.find("async fn evolve_catalog(")
    h_end = rs.find("\n    async fn reload_catalog", h_start)
    body = rs[h_start:h_end]
    assert "Status::unimplemented(format!" in body
    assert "CatalogService" in body or "catalog.CatalogService" in body
    assert "from_key" in body or "evolution_kind" in body, (
        "error message must echo the input fields for grpcurl re-invocation"
    )


def test_a5_evolve_catalog_emits_warn_log() -> None:
    rs = _read(ADMIN_RS)
    h_start = rs.find("async fn evolve_catalog(")
    h_end = rs.find("\n    async fn reload_catalog", h_start)
    body = rs[h_start:h_end]
    assert "tracing::warn!" in body, (
        "EvolveCatalog must emit a structured tracing event for ops correlation"
    )


# ─────────────────────────────────────────────────────────────────────
# M4-A6 — health reporter set_serving for Mode2/3
# ─────────────────────────────────────────────────────────────────────


def test_a6_health_reporter_set_serving_for_mode2() -> None:
    rs = _read(RUNTIME_RS)
    assert "set_serving::<Mode2ServiceServer<Mode2ServiceImpl>>()" in rs


def test_a6_health_reporter_set_serving_for_mode3() -> None:
    rs = _read(RUNTIME_RS)
    assert "set_serving::<Mode3ServiceServer<Mode3ServiceImpl>>()" in rs


def test_a6_health_reporter_set_not_serving_for_mode2_3() -> None:
    rs = _read(RUNTIME_RS)
    assert "set_not_serving::<Mode2ServiceServer<Mode2ServiceImpl>>()" in rs
    assert "set_not_serving::<Mode3ServiceServer<Mode3ServiceImpl>>()" in rs


# ─────────────────────────────────────────────────────────────────────
# M4-A7 — Mode-2/3 metrics on early-return paths
# ─────────────────────────────────────────────────────────────────────


def test_a7_mode2_execute_emits_metric_on_scope_check_failure() -> None:
    """Pre-fix `ScopeGuard::require(...)?` returned without emitting
    a metric. Now there must be an explicit emit before return."""
    rs = _read(MODE2_RS)
    h_start = rs.find("async fn execute(")
    h_end = rs.find("type ExecuteStreamingStream", h_start)
    body = rs[h_start:h_end]
    # Anti-regression : the bare `?` after ScopeGuard::require must
    # NOT reappear in this handler.
    assert "ScopeGuard::require(&claims.0, BroadScope::DATA)?;" not in body
    # Positive : the explicit-emit pattern must be present.
    assert "if let Err(e) = ScopeGuard::require(&claims.0, BroadScope::DATA)" in body


def test_a7_mode3_execute_emits_metric_on_aggregator_failure() -> None:
    rs = _read(MODE3_RS)
    h_start = rs.find("async fn execute(")
    h_end = rs.find("type SubscribeVerdictStream", h_start)
    body = rs[h_start:h_end]
    # The fix wraps `Self::resolve_aggregator` in a `match Ok/Err`
    # with explicit emit.
    assert "match Self::resolve_aggregator(" in body
    # And similarly for the join_set error paths.
    assert "emit_request_completion(EXECUTE_PATH, &tenant, &Err(&status), started)" in body


def test_a7_mode3_execute_emits_metric_on_unknown_mode() -> None:
    rs = _read(MODE3_RS)
    h_start = rs.find("async fn execute(")
    h_end = rs.find("type SubscribeVerdictStream", h_start)
    body = rs[h_start:h_end]
    # The unknown-mode branch must emit the metric.
    assert 'unknown mode_per_call' in body
    assert "emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started)" in body


# ─────────────────────────────────────────────────────────────────────
# M4-A8 — soft_delete logging
# ─────────────────────────────────────────────────────────────────────


def test_a8_soft_delete_emits_warn_log() -> None:
    rs = _read(ADMIN_RS)
    h_start = rs.find("async fn delete_catalog(")
    h_end = rs.find("\n    async fn reload_bundle", h_start)
    body = rs[h_start:h_end]
    # The pre-fix `let _ = inner.soft_delete;` must be replaced by an
    # `if inner.soft_delete { tracing::warn!(... DT-S-D21-SOFT-1 ...) }`
    # pattern.
    assert "if inner.soft_delete" in body
    assert "tracing::warn!" in body
    assert "DT-S-D21-SOFT-1" in body


# ─────────────────────────────────────────────────────────────────────
# Synthetic spec helper (M4-A2/A3 dependency)
# ─────────────────────────────────────────────────────────────────────


def test_synthetic_spec_module_exists() -> None:
    rs = _read(SYNTH_RS)
    assert "pub fn build_synthetic_request(" in rs
    assert "pub fn verdict_to_proto_i32(" in rs


def test_request_synthetic_constructor_added_to_efe_core() -> None:
    """A new public `Request::synthetic` constructor must be added
    to efe-core so server-side synthesis doesn't depend on
    `Request::for_tests` (which is `#[doc(hidden)]`)."""
    rs = _read(REQUEST_RS)
    assert "pub fn synthetic(" in rs
    assert "tenant_id: String" in rs
    assert "request_id: RequestId" in rs


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
