"""Sprint **M5 audit pass 1** — Pin tests anti-régression pour les
3 défauts (T-A1 / T-A2 / T-A3) trouvés post-livraison initiale.

| ID | Sévérité | Description | Couvert par |
|----|---|---|---|
| T-A1 | CRIT | Instruments Mode-2/Mode-3 declared but never bumped | `test_t_a1_*` |
| T-A2 | HIGH | Helm mirror ne provisionne pas les nouveaux dashboards | `test_t_a2_*` |
| T-A3 | MED  | Wiring nano-agent bloqué par circular dep efe-core ↔ telemetry | `test_t_a3_*` |
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
MODE2_RS = REPO / "crates" / "efe-server" / "src" / "services" / "mode2" / "mod.rs"
MODE3_RS = REPO / "crates" / "efe-server" / "src" / "services" / "mode3" / "mod.rs"
NANO_TELEMETRY_RS = REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "nano_agent_instruments.rs"
SYNC_SCRIPT = REPO / "scripts" / "observability" / "sync_helm_d13_assets.sh"
HELM_MIRROR_DIR = REPO / "deploy" / "helm" / "efe" / "d13_files" / "dashboards"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# T-A1 : Mode-2 instruments wired in efe-server::services::mode2
# ─────────────────────────────────────────────────────────────────────


def test_t_a1_mode2_cache_hit_total_wired() -> None:
    """`efe_mode2_cache_hit_total` MUST be incremented from
    Mode2ServiceImpl::execute on cache hit."""
    rs = _read(MODE2_RS)
    assert "mode2_instruments::mode2_cache_hit_total" in rs
    # The increment must be in the cache_hit branch.
    body = rs[rs.find("else if let Some(hit) = self.cache_get(cache_key)"):]
    body = body[: body.find("} else {")]
    assert "mode2_cache_hit_total" in body, (
        "mode2_cache_hit_total must be incremented in the cache-hit branch"
    )
    assert ".inc()" in body


def test_t_a1_mode2_cache_miss_total_wired() -> None:
    rs = _read(MODE2_RS)
    assert "mode2_instruments::mode2_cache_miss_total" in rs
    # Must be in the cache-miss branch (the `} else {` after cache_get).
    body = rs[rs.find("} else {"):]
    body = body[: body.find("self.cache_insert")]
    assert "mode2_cache_miss_total" in body, (
        "mode2_cache_miss_total must be incremented in the cache-miss branch"
    )


def test_t_a1_mode2_router_latency_observed() -> None:
    """The router call wall-clock duration must feed
    `efe_mode2_router_latency_seconds`."""
    rs = _read(MODE2_RS)
    assert "mode2_instruments::mode2_router_latency_seconds" in rs
    assert "started_router" in rs, (
        "the router call must be wrapped in Instant::now() for observability"
    )
    assert ".observe(started_router.elapsed().as_secs_f64())" in rs


def test_t_a1_mode2_anchors_generated_observed() -> None:
    rs = _read(MODE2_RS)
    assert "mode2_instruments::mode2_anchors_generated" in rs
    assert ".observe(out.anchors.len() as f64)" in rs


# ─────────────────────────────────────────────────────────────────────
# T-A1 : Mode-3 instruments wired in efe-server::services::mode3
# ─────────────────────────────────────────────────────────────────────


def test_t_a1_mode3_k_clamped_wired_in_execute() -> None:
    rs = _read(MODE3_RS)
    body = rs[rs.find("async fn execute(") :]
    body = body[: body.find("type SubscribeVerdictStream")]
    assert "mode3_k_clamped_total" in body, (
        "mode3_k_clamped_total must be incremented in the unary execute path"
    )
    # The increment must be guarded by `if k_clamped`.
    assert "if k_clamped {" in body


def test_t_a1_mode3_k_clamped_wired_in_subscribe() -> None:
    rs = _read(MODE3_RS)
    body = rs[rs.find("async fn subscribe_verdict(") :]
    body = body[: body.find("async fn cancel(")]
    assert "mode3_k_clamped_total" in body, (
        "mode3_k_clamped_total must be incremented in the streaming subscribe_verdict path"
    )


def test_t_a1_mode3_subswarms_spawned_total_wired() -> None:
    rs = _read(MODE3_RS)
    assert "mode3_subswarms_spawned_total" in rs
    # Must be incremented by the actual sub-verdict count, not a constant.
    assert ".inc_by(sub_verdicts.len() as u64)" in rs


def test_t_a1_mode3_aggregation_seconds_observed() -> None:
    rs = _read(MODE3_RS)
    assert "mode3_aggregation_seconds" in rs
    # The aggregator timer pattern.
    assert "agg_started.elapsed().as_secs_f64()" in rs


def test_t_a1_mode3_dispersion_set() -> None:
    rs = _read(MODE3_RS)
    assert "mode3_dispersion" in rs
    # Gauge `set` (not inc) — anti-regression.
    assert ".set(f64::from(agg_out.dispersion))" in rs


# ─────────────────────────────────────────────────────────────────────
# T-A2 : Helm dashboards mirror sync
# ─────────────────────────────────────────────────────────────────────


def test_t_a2_sync_script_handles_d29() -> None:
    body = _read(SYNC_SCRIPT)
    assert 'SRC_DASH_D29' in body
    assert "dashboards/d29" in body


def test_t_a2_sync_script_handles_d34() -> None:
    body = _read(SYNC_SCRIPT)
    assert 'SRC_DASH_D34' in body
    assert "dashboards/d34" in body


def test_t_a2_sync_script_handles_d37() -> None:
    body = _read(SYNC_SCRIPT)
    assert 'SRC_DASH_D37' in body
    assert "dashboards/d37" in body


def test_t_a2_sync_script_uses_sprint_prefix() -> None:
    """Anti-regression : the helm mirror filenames must use `dXX_`
    prefix to avoid collision with d13's `01_*.json` etc."""
    body = _read(SYNC_SCRIPT)
    for prefix in ("d29_", "d34_", "d37_"):
        assert (
            prefix in body
        ), f"sync script must rename copies with `{prefix}` prefix"


@pytest.mark.parametrize(
    "fname",
    ["d29_01_gpu_canary.json", "d34_01_templates.json",
     "d34_02_mode2.json", "d37_01_mode3.json"],
)
def test_t_a2_helm_mirror_has_m5_dashboard(fname: str) -> None:
    f = HELM_MIRROR_DIR / fname
    assert f.exists(), (
        f"helm mirror missing M5 dashboard {fname} ; "
        f"run scripts/observability/sync_helm_d13_assets.sh"
    )


# ─────────────────────────────────────────────────────────────────────
# T-A3 : DT markers documenting the unwired nano-agent metrics
# ─────────────────────────────────────────────────────────────────────


def test_t_a3_nano_agent_documents_circular_dep() -> None:
    """The nano_agent module doc-comment must explicitly call out
    the circular dep limitation (efe-core ↔ efe-telemetry) so a
    future engineer reading the doc understands why 3 of the 4
    instruments are at 0 in production."""
    rs = _read(NANO_TELEMETRY_RS)
    assert "circular" in rs.lower() or "cycle" in rs.lower()
    assert "TelemetryCollector" in rs or "DT-T-D34-WIRE-1" in rs


def test_t_a3_nano_agent_lists_unblock_options() -> None:
    """The doc-comment lists 3 future paths to unblock the wiring.
    Pin the presence of each option's DT marker."""
    rs = _read(NANO_TELEMETRY_RS)
    for marker in (
        "DT-T-D34-WIRE-1",
        "DT-T-D34-EVENT-BUS-1",
        "DT-T-D34-CALLBACK-1",
    ):
        assert marker in rs, f"DT marker {marker} missing from nano_agent doc"


def test_t_a3_nano_agent_safe_by_default_documented() -> None:
    rs = _read(NANO_TELEMETRY_RS)
    assert "safe-by-default" in rs.lower() or "safe by default" in rs.lower()


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
