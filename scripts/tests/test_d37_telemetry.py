"""Sprint **M5 extension D37** — Pin tests for Mode-3 telemetry
instruments + dashboard.

Verrouillent que :
  * `mode3_instruments.rs` expose 4 instruments + register_all.
  * `register_all_mode3` appelé dans `runtime::Server::build`.
  * Dashboard `d37/01_mode3.json` valide + référence les bons
    instruments.
  * SLO panels (C-3 latency, C-5 k_max, C-12 dispersion) présents.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
MODE3_RS = REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "mode3_instruments.rs"
MOD_RS = REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "mod.rs"
RUNTIME_RS = REPO / "crates" / "efe-server" / "src" / "runtime.rs"
DASHBOARD = REPO / "deploy" / "observability" / "grafana" / "dashboards" / "d37" / "01_mode3.json"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# Mode-3 instruments (4)
# ─────────────────────────────────────────────────────────────────────

MODE3_INSTRUMENTS = [
    "efe_mode3_subswarms_spawned_total",
    "efe_mode3_dispersion",
    "efe_mode3_k_clamped_total",
    "efe_mode3_aggregation_seconds",
]


@pytest.mark.parametrize("instrument", MODE3_INSTRUMENTS)
def test_d37_instrument_declared(instrument: str) -> None:
    rs = _read(MODE3_RS)
    assert f'"{instrument}"' in rs


def test_d37_register_all_mode3_present() -> None:
    rs = _read(MODE3_RS)
    assert "pub fn register_all_mode3()" in rs


def test_d37_register_all_mode3_calls_each_instrument() -> None:
    rs = _read(MODE3_RS)
    body_start = rs.find("pub fn register_all_mode3()")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    expected_fns = [
        "mode3_subswarms_spawned_total()",
        "mode3_dispersion()",
        "mode3_k_clamped_total()",
        "mode3_aggregation_seconds()",
    ]
    for fn in expected_fns:
        assert fn in body, f"register_all_mode3() must call {fn}"


def test_d37_mode3_re_exported_from_mod() -> None:
    rs = _read(MOD_RS)
    assert "pub mod mode3_instruments;" in rs
    assert "pub use mode3_instruments::register_all_mode3;" in rs


def test_d37_register_all_mode3_wired_in_runtime() -> None:
    """Institutional anti-regression : register_all_mode3 MUST be
    called from `Server::build` so dashboards see zero-valued
    series at boot."""
    rs = _read(RUNTIME_RS)
    assert "register_all_mode3()" in rs


def test_d37_dispersion_is_gauge_not_histogram() -> None:
    """Architecture decision : dispersion is a GAUGE (last-value
    semantics) not a Histogram. The doc-comment in mode3_instruments
    explains the rationale ; pin test enforces it."""
    rs = _read(MODE3_RS)
    body_start = rs.find("once_metric!(")
    # Find the once_metric! call for mode3_dispersion specifically.
    dispersion_idx = rs.find("mode3_dispersion,")
    assert dispersion_idx >= 0
    # Look at the immediately following type to ensure it's GaugeVec.
    after = rs[dispersion_idx:dispersion_idx + 200]
    assert "GaugeVec" in after, "mode3_dispersion must be a GaugeVec, not Histogram"


def test_d37_aggregation_buckets_5_entries() -> None:
    """exponential_buckets(1e-4, 10.0, 5) — 5 buckets covering
    100µs → 1s. Anti-regression on bucket-count change that would
    break d37_mode3 latency panels."""
    rs = _read(MODE3_RS)
    assert "exponential_buckets(1e-4, 10.0, 5)" in rs


# ─────────────────────────────────────────────────────────────────────
# Dashboard d37/01_mode3.json
# ─────────────────────────────────────────────────────────────────────


def test_d37_dashboard_exists_and_valid() -> None:
    assert DASHBOARD.exists()
    with DASHBOARD.open("r", encoding="utf-8") as fh:
        data = json.load(fh)
    assert data["uid"] == "efe-d37-01-mode3"
    assert data["schemaVersion"] >= 38


def test_d37_dashboard_references_all_4_instruments() -> None:
    raw = DASHBOARD.read_text(encoding="utf-8")
    for instrument in MODE3_INSTRUMENTS:
        assert (
            instrument in raw
        ), f"d37_mode3 dashboard missing reference to {instrument}"


def test_d37_dashboard_has_dispersion_threshold_0_15() -> None:
    """SLO C-12 : dispersion ≤ 0.15. The threshold must appear in
    the dashboard for green-zone visual marker."""
    raw = DASHBOARD.read_text(encoding="utf-8")
    # The threshold "0.15" appears as a doc-comment annotation in
    # the panel description ; flexible match.
    assert "0.15" in raw, "d37 must surface the C-12 SLO threshold"


def test_d37_dashboard_has_k_clamped_panel() -> None:
    """SLO C-5 (k_max enforced) panel must be visible."""
    raw = DASHBOARD.read_text(encoding="utf-8")
    assert "efe_mode3_k_clamped_total" in raw
    assert "k_max" in raw or "K-Clamp" in raw or "clamp" in raw.lower()


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
