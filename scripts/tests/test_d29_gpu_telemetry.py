"""Sprint **M5 extension D29** — Pin tests for GPU telemetry
instruments + dashboard.

Verrouillent que :
  * `gpu_instruments.rs` expose 6 once_metric! macros.
  * `register_all_gpu()` est défini et appelé dans `runtime::Server`.
  * Dashboard `d29/01_gpu_canary.json` existe et est valide JSON.
  * Les 6 instruments référencés par le dashboard matchent les
    noms déclarés en Rust (anti-régression sur les renames).
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
GPU_RS = REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "gpu_instruments.rs"
RUNTIME_RS = REPO / "crates" / "efe-server" / "src" / "runtime.rs"
DASHBOARD = REPO / "deploy" / "observability" / "grafana" / "dashboards" / "d29" / "01_gpu_canary.json"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# 6 instruments declared in gpu_instruments.rs
# ─────────────────────────────────────────────────────────────────────

EXPECTED_INSTRUMENTS = [
    "efe_gpu_kernel_duration_seconds",
    "efe_gpu_memory_used_bytes",
    "efe_gpu_utilization_ratio",
    "efe_gpu_kernel_calls_total",
    "efe_gpu_oom_total",
    "efe_gpu_nccl_allreduce_seconds",
]


@pytest.mark.parametrize("instrument", EXPECTED_INSTRUMENTS)
def test_d29_instrument_declared(instrument: str) -> None:
    """The 6 D29 instruments must be declared as Prometheus
    `Opts::new("...", ...)` strings in `gpu_instruments.rs`."""
    rs = _read(GPU_RS)
    assert (
        f'"{instrument}"' in rs
    ), f"D29 instrument {instrument} not declared in gpu_instruments.rs"


def test_d29_register_all_gpu_present() -> None:
    rs = _read(GPU_RS)
    assert "pub fn register_all_gpu()" in rs


def test_d29_register_all_gpu_calls_each_instrument() -> None:
    rs = _read(GPU_RS)
    body_start = rs.find("pub fn register_all_gpu()")
    body_end = rs.find("\n}\n", body_start)
    body = rs[body_start:body_end]
    expected_fns = [
        "gpu_kernel_duration_seconds()",
        "gpu_memory_used_bytes()",
        "gpu_utilization_ratio()",
        "gpu_kernel_calls_total()",
        "gpu_oom_total()",
        "gpu_nccl_allreduce_seconds()",
    ]
    for fn in expected_fns:
        assert (
            fn in body
        ), f"register_all_gpu() must call accessor {fn}"


def test_d29_register_all_gpu_wired_in_runtime() -> None:
    """The institutional anti-regression : `register_all_gpu()`
    MUST be called from `Server::build` so dashboards see zero-
    valued series at boot rather than 'no data'."""
    rs = _read(RUNTIME_RS)
    assert "register_all_gpu()" in rs


# ─────────────────────────────────────────────────────────────────────
# Dashboard d29/01_gpu_canary.json
# ─────────────────────────────────────────────────────────────────────


def test_d29_dashboard_exists() -> None:
    assert DASHBOARD.exists(), f"D29 dashboard missing at {DASHBOARD}"


def test_d29_dashboard_is_valid_json() -> None:
    with DASHBOARD.open("r", encoding="utf-8") as fh:
        data = json.load(fh)
    assert isinstance(data, dict)


def test_d29_dashboard_uid_stable() -> None:
    with DASHBOARD.open("r", encoding="utf-8") as fh:
        data = json.load(fh)
    assert data["uid"] == "efe-d29-01-gpu-canary"


def test_d29_dashboard_schema_version_recent() -> None:
    """Grafana 10.x ships schemaVersion 38 / 39. Anti-regression on
    older format that breaks panel rendering."""
    with DASHBOARD.open("r", encoding="utf-8") as fh:
        data = json.load(fh)
    assert data["schemaVersion"] >= 38


def test_d29_dashboard_references_all_6_instruments() -> None:
    """Every panel queries one of the 6 D29 instruments. A panel
    that references an instrument we never declare = silent
    'no data' panel."""
    with DASHBOARD.open("r", encoding="utf-8") as fh:
        raw = fh.read()
    for instrument in EXPECTED_INSTRUMENTS:
        assert (
            instrument in raw
        ), f"D29 dashboard does not reference instrument {instrument}"


def test_d29_dashboard_has_canary_divergence_panel() -> None:
    """SLO C-8 (divergence detection) requires a dashboard panel
    that surfaces the rate. The panel title / target combination
    is grep-able for anti-regression."""
    with DASHBOARD.open("r", encoding="utf-8") as fh:
        raw = fh.read()
    assert "efe_gpu_canary_divergence_total" in raw
    assert "efe_gpu_canary_divergence_l2" in raw


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
