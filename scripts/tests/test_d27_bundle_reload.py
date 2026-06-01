"""Sprint **D27** (M4 § 11) — pin tests for the live bundle reload.

Verifies the 3-phase contract :
  * Phase 1 — verify (signature + per-component SHA + smoke test)
  * Phase 2 — atomic Arc swap
  * Phase 3 — drain via Arc::strong_count probe
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
RELOAD_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "embedder" / "reload.rs"
EMBEDDER_MOD_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "embedder" / "mod.rs"
ADMIN_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "admin.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_active_bundle_slot_struct_present() -> None:
    rs = _read(RELOAD_RS)
    assert "pub struct ActiveBundleSlot" in rs


def test_smoke_test_fixtures_hardcoded() -> None:
    rs = _read(RELOAD_RS)
    assert "pub const SMOKE_FIXTURES" in rs
    # Architecture mandates ≥ 5 fixtures (M4 § 11.2 / § 11.3).
    # Find the array and count.
    start = rs.find("pub const SMOKE_FIXTURES")
    block = rs[start : start + 500]
    n_quotes = block.count('"')
    # Each string contributes two quotes ; ≥ 5 strings ⇒ ≥ 10 quotes.
    assert n_quotes >= 10, f"expected ≥5 fixtures, found {n_quotes // 2}"


def test_smoke_l2_tolerance_is_documented_constant() -> None:
    rs = _read(RELOAD_RS)
    assert "pub const SMOKE_L2_TOLERANCE: f32 = 1e-3" in rs


def test_three_phase_pipeline_in_reload_method() -> None:
    rs = _read(RELOAD_RS)
    # The reload() method must call Phase 1 (ensure_loaded), Phase 1.5
    # (smoke_test), Phase 2 (atomic swap with replace), Phase 3 (drain).
    body = rs[rs.find("pub async fn reload(") :]
    assert "ensure_loaded(" in body, "Phase 1 missing"
    assert "smoke_test(" in body, "Phase 1.5 missing"
    assert "guard.replace(" in body, "Phase 2 atomic swap missing"
    assert "drain_old(" in body, "Phase 3 drain missing"


def test_singleflight_gate_via_in_flight_mutex() -> None:
    rs = _read(RELOAD_RS)
    body = rs[rs.find("pub async fn reload(") :]
    assert "in_flight" in body
    assert "ReloadInProgress" in body


def test_drain_uses_arc_strong_count() -> None:
    rs = _read(RELOAD_RS)
    body = rs[rs.find("async fn drain_old(") :]
    assert "Arc::strong_count(" in body


def test_default_drain_timeout_is_30s() -> None:
    rs = _read(RELOAD_RS)
    assert "DEFAULT_DRAIN_TIMEOUT" in rs
    assert "Duration::from_secs(30)" in rs


def test_reload_module_re_exported_from_embedder_mod() -> None:
    rs = _read(EMBEDDER_MOD_RS)
    assert "pub mod reload;" in rs
    assert "pub use reload::" in rs


def test_admin_reload_bundle_handler_uses_active_bundle_slot() -> None:
    rs = _read(ADMIN_RS)
    # The handler must accept the wired `(slot, registry, trust)` triple
    # AND call slot.reload(...).
    assert "with_bundle_reload(" in rs
    body = rs[rs.find("async fn reload_bundle(") :]
    assert "slot" in body, "ReloadBundle handler should reference the slot"
    assert ".reload(" in body, "ReloadBundle handler should invoke reload()"


def test_oci_pull_returns_unimplemented_d27() -> None:
    """OCI pull is DT-S-D27-OCI-1 ; D27 ships filesystem-only loads."""
    rs = _read(ADMIN_RS)
    assert "DT-S-D27-OCI-1" in rs


def test_reload_status_proto_enum_present() -> None:
    proto_path = REPO_ROOT / "protos" / "efe" / "v1" / "admin.proto"
    proto = proto_path.read_text(encoding="utf-8")
    for variant in (
        "RELOAD_STATUS_UNSPECIFIED",
        "RELOAD_STATUS_PENDING",
        "RELOAD_STATUS_LOADING",
        "RELOAD_STATUS_SMOKE_OK",
        "RELOAD_STATUS_ACTIVE",
        "RELOAD_STATUS_FAILED",
    ):
        assert variant in proto, f"ReloadStatus::{variant} missing"


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
