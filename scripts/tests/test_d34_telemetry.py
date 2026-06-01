"""Sprint **M5 extension D34** — Pin tests for nano-agent + Mode-2
telemetry instruments + 2 dashboards.

Verrouillent que :
  * `nano_agent_instruments.rs` expose 4 instruments + register_all.
  * `mode2_instruments.rs` expose 5 instruments + register_all.
  * `register_all_nano_agent` + `register_all_mode2` appelés dans
    `runtime::Server::build`.
  * Dashboards `d34/01_templates.json` + `d34/02_mode2.json`
    valides + référencent les bons instruments.
  * `consensus_weight` gauge pré-peuplée pour les 8 templates.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
NANO_RS = REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "nano_agent_instruments.rs"
MODE2_RS = REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "mode2_instruments.rs"
MOD_RS = REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "mod.rs"
RUNTIME_RS = REPO / "crates" / "efe-server" / "src" / "runtime.rs"
DASH_TEMPLATES = REPO / "deploy" / "observability" / "grafana" / "dashboards" / "d34" / "01_templates.json"
DASH_MODE2 = REPO / "deploy" / "observability" / "grafana" / "dashboards" / "d34" / "02_mode2.json"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# Nano-agent instruments (4)
# ─────────────────────────────────────────────────────────────────────

NANO_INSTRUMENTS = [
    "efe_nano_agent_population_total",
    "efe_nano_agent_consensus_weight",
    "efe_nano_agent_safety_triggered_total",
    "efe_nano_agent_kernel_routed_total",
]


@pytest.mark.parametrize("instrument", NANO_INSTRUMENTS)
def test_d34_nano_instrument_declared(instrument: str) -> None:
    rs = _read(NANO_RS)
    assert f'"{instrument}"' in rs


def test_d34_nano_register_all_present() -> None:
    rs = _read(NANO_RS)
    assert "pub fn register_all_nano_agent()" in rs


def test_d34_nano_register_all_pre_populates_consensus_weight() -> None:
    """**Audit invariant** — register_all_nano_agent MUST iterate
    over `ga_templates()` and `set` the consensus_weight gauge for
    each. Without this, the dashboard table panel shows 'no data'
    until the first swarm executes."""
    rs = _read(NANO_RS)
    body_start = rs.find("pub fn register_all_nano_agent()")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    assert "ga_templates()" in body
    assert ".set(" in body
    assert "consensus_weight" in body


def test_d34_nano_re_exported_from_mod() -> None:
    rs = _read(MOD_RS)
    assert "pub mod nano_agent_instruments;" in rs
    assert "pub use nano_agent_instruments::register_all_nano_agent;" in rs


# ─────────────────────────────────────────────────────────────────────
# Mode-2 instruments (5)
# ─────────────────────────────────────────────────────────────────────

MODE2_INSTRUMENTS = [
    "efe_mode2_cache_hit_total",
    "efe_mode2_cache_miss_total",
    "efe_mode2_router_latency_seconds",
    "efe_mode2_anchors_generated",
    "efe_mode2_seed_mix_collisions_total",
]


@pytest.mark.parametrize("instrument", MODE2_INSTRUMENTS)
def test_d34_mode2_instrument_declared(instrument: str) -> None:
    rs = _read(MODE2_RS)
    assert f'"{instrument}"' in rs


def test_d34_mode2_register_all_present() -> None:
    rs = _read(MODE2_RS)
    assert "pub fn register_all_mode2()" in rs


def test_d34_mode2_re_exported_from_mod() -> None:
    rs = _read(MOD_RS)
    assert "pub mod mode2_instruments;" in rs
    assert "pub use mode2_instruments::register_all_mode2;" in rs


def test_d34_mode2_router_latency_buckets_correct_arity() -> None:
    """exponential_buckets(0.05, 2.0, 8) — 8 buckets. Anti-regression
    on a refactor that changes the bucket count and breaks all
    histogram_quantile() panels in d34_mode2.json."""
    rs = _read(MODE2_RS)
    assert "exponential_buckets(0.05, 2.0, 8)" in rs


def test_d34_mode2_anchors_buckets_canonical() -> None:
    """The buckets [1, 3, 5, 10, 20] match the canonical top_k
    values supported by LlmRouter. Anti-regression on a future
    proto top_k cap change."""
    rs = _read(MODE2_RS)
    assert "vec![1.0, 3.0, 5.0, 10.0, 20.0]" in rs


# ─────────────────────────────────────────────────────────────────────
# Wiring runtime
# ─────────────────────────────────────────────────────────────────────


def test_d34_register_all_nano_agent_wired_in_runtime() -> None:
    rs = _read(RUNTIME_RS)
    assert "register_all_nano_agent()" in rs


def test_d34_register_all_mode2_wired_in_runtime() -> None:
    rs = _read(RUNTIME_RS)
    assert "register_all_mode2()" in rs


# ─────────────────────────────────────────────────────────────────────
# Dashboards
# ─────────────────────────────────────────────────────────────────────


def test_d34_templates_dashboard_exists_and_valid() -> None:
    assert DASH_TEMPLATES.exists()
    with DASH_TEMPLATES.open("r", encoding="utf-8") as fh:
        data = json.load(fh)
    assert data["uid"] == "efe-d34-01-templates"
    assert data["schemaVersion"] >= 38


def test_d34_mode2_dashboard_exists_and_valid() -> None:
    assert DASH_MODE2.exists()
    with DASH_MODE2.open("r", encoding="utf-8") as fh:
        data = json.load(fh)
    assert data["uid"] == "efe-d34-02-mode2"
    assert data["schemaVersion"] >= 38


def test_d34_dashboards_uids_unique() -> None:
    """Two dashboards in the same sprint MUST have different UIDs
    or Grafana will overwrite one with the other."""
    with DASH_TEMPLATES.open("r", encoding="utf-8") as fh:
        uid_a = json.load(fh)["uid"]
    with DASH_MODE2.open("r", encoding="utf-8") as fh:
        uid_b = json.load(fh)["uid"]
    assert uid_a != uid_b


def test_d34_templates_dashboard_references_nano_instruments() -> None:
    raw = DASH_TEMPLATES.read_text(encoding="utf-8")
    for instrument in NANO_INSTRUMENTS:
        assert (
            instrument in raw
        ), f"d34_templates dashboard missing reference to {instrument}"


def test_d34_mode2_dashboard_references_mode2_instruments() -> None:
    raw = DASH_MODE2.read_text(encoding="utf-8")
    for instrument in MODE2_INSTRUMENTS:
        assert (
            instrument in raw
        ), f"d34_mode2 dashboard missing reference to {instrument}"


def test_d34_mode2_dashboard_has_cache_hit_rate_slo_threshold() -> None:
    """SLO C-4 panel : cache hit rate ≥ 60%. The threshold value
    must appear in the dashboard for the green-zone marker."""
    raw = DASH_MODE2.read_text(encoding="utf-8")
    assert "0.6" in raw, "d34_mode2 must encode the C-4 SLO threshold (0.6)"


def test_d34_templates_dashboard_safety_panel_present() -> None:
    """NaSafety triggers panel is institutional — its presence
    proves operators can observe the C-11 SLO."""
    raw = DASH_TEMPLATES.read_text(encoding="utf-8")
    assert "efe_nano_agent_safety_triggered_total" in raw
    assert "NaSafety" in raw


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
