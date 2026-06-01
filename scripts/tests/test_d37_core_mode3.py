"""Sprint **M1 extension D37** — Pin tests pour Mode-3 backend dans
`efe-core`.

Verrouillent que :
  * Le module `swarm::mode3` existe avec mod.rs + aggregator.rs.
  * Trait `Mode3Aggregator` + 3 impls (Mean / Weighted / Median).
  * `K_MAX = 5` const + `clamp_k(k)` helper.
  * `Mode3Controller<B>` générique sur `B: ExecuteBackend`.
  * Dispatch sur `mode_per_call` ("mode1" / "mode2").
  * Mode3Verdict carries `k_clamped` + `k_effective`.
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
MODE3_DIR = REPO_ROOT / "crates" / "efe-core" / "src" / "swarm" / "mode3"
MODE3_MOD = MODE3_DIR / "mod.rs"
AGGREGATOR_RS = MODE3_DIR / "aggregator.rs"
LIB_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "lib.rs"
SWARM_MOD = REPO_ROOT / "crates" / "efe-core" / "src" / "swarm" / "mod.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# Module structure
# ─────────────────────────────────────────────────────────────────────

def test_mode3_dir_exists() -> None:
    assert MODE3_MOD.exists()
    assert AGGREGATOR_RS.exists()


def test_swarm_mod_exposes_mode3() -> None:
    rs = _read(SWARM_MOD)
    assert "pub mod mode3;" in rs


def test_lib_rs_exposes_mode3_top_level() -> None:
    rs = _read(LIB_RS)
    for sym in (
        "Mode3Controller",
        "Mode3Aggregator",
        "MeanAggregator",
        "WeightedAggregator",
        "MedianAggregator",
        "Mode3Verdict",
        "SubVerdictData",
        "AggregationMetadata",
    ):
        assert sym in rs, f"top-level re-export missing : {sym}"


# ─────────────────────────────────────────────────────────────────────
# Mode3Aggregator trait + 3 impls
# ─────────────────────────────────────────────────────────────────────

def test_mode3_aggregator_trait_present() -> None:
    rs = _read(AGGREGATOR_RS)
    assert "pub trait Mode3Aggregator" in rs
    assert "fn id(&self) -> &'static str" in rs
    assert "fn aggregate(" in rs


@pytest.mark.parametrize(
    "impl_name,id_str",
    [
        ("MeanAggregator", "mean"),
        ("WeightedAggregator", "weighted"),
        ("MedianAggregator", "median"),
    ],
)
def test_aggregator_impl_present(impl_name: str, id_str: str) -> None:
    rs = _read(AGGREGATOR_RS)
    assert f"pub struct {impl_name}" in rs
    assert f"impl Mode3Aggregator for {impl_name}" in rs
    assert f'"{id_str}"' in rs


def test_aggregator_ids_are_distinct() -> None:
    rs = _read(AGGREGATOR_RS)
    # We pin each id_string appears in the expected impl module.
    # Distinctness is enforced by the Rust unit tests
    # (`three_aggregators_have_distinct_ids`).
    assert '"mean"' in rs
    assert '"weighted"' in rs
    assert '"median"' in rs


def test_aggregation_error_typed() -> None:
    rs = _read(AGGREGATOR_RS)
    assert "pub enum AggregationError" in rs
    assert "Empty" in rs
    assert "UnknownStrategy" in rs


def test_aggregation_metadata_carries_dispersion() -> None:
    rs = _read(AGGREGATOR_RS)
    body = rs[rs.find("pub struct AggregationMetadata") :]
    body = body[: body.find("\n}\n")]
    for f in ("strategy", "total_confidence", "dispersion", "effective_n"):
        assert f in body, f"AggregationMetadata.{f} missing"


def test_sub_verdict_data_carries_seed_and_mode() -> None:
    rs = _read(AGGREGATOR_RS)
    body = rs[rs.find("pub struct SubVerdictData") :]
    body = body[: body.find("\n}\n")]
    for f in ("swarm_idx", "verdict", "confidence", "latency_ms", "mode_used", "seed_used"):
        assert f in body, f"SubVerdictData.{f} missing"


# ─────────────────────────────────────────────────────────────────────
# Mode3Controller
# ─────────────────────────────────────────────────────────────────────

def test_k_max_is_5() -> None:
    rs = _read(MODE3_MOD)
    assert "pub const K_MAX: u32 = 5" in rs


def test_clamp_k_function_present() -> None:
    rs = _read(MODE3_MOD)
    assert "pub fn clamp_k(k: u32) -> (u32, bool)" in rs


def test_mode3_controller_is_generic() -> None:
    rs = _read(MODE3_MOD)
    assert "pub struct Mode3Controller<B: ExecuteBackend + 'static>" in rs


def test_mode3_controller_holds_optional_mode2() -> None:
    rs = _read(MODE3_MOD)
    body = rs[rs.find("pub struct Mode3Controller") :]
    body = body[: body.find("\n}\n")]
    assert "mode2_controller: Option<Arc<Mode2Controller<B>>>" in body


def test_mode3_dispatch_on_mode_per_call() -> None:
    rs = _read(MODE3_MOD)
    # Locate the inner helper definition (after the `pub(crate) async
    # fn execute_one_sub_swarm` declaration). The body contains the
    # match-arm dispatch.
    helper_start = rs.find("pub(crate) async fn execute_one_sub_swarm")
    assert helper_start >= 0, "execute_one_sub_swarm helper missing"
    helper_end = rs.find("\n    }\n", helper_start)
    body = rs[helper_start:helper_end]
    assert '"mode1" =>' in body
    assert '"mode2" =>' in body
    # mode2 path must call the Mode-2 controller's execute method.
    # Be tolerant of formatting (multi-line argument list).
    assert ".execute(" in body
    assert "m2" in body  # variable that holds the mode2 controller


def test_mode3_dispatch_unknown_mode_returns_invalid_config() -> None:
    rs = _read(MODE3_MOD)
    helper_start = rs.find("pub(crate) async fn execute_one_sub_swarm")
    helper_end = rs.find("\n    }\n", helper_start)
    body = rs[helper_start:helper_end]
    # Anti-regression : the catch-all arm must surface a typed error,
    # not a panic or default.
    assert "EfeError::InvalidConfig" in body
    assert "unknown mode_per_call" in body


def test_mode3_default_aggregators_returns_three() -> None:
    rs = _read(MODE3_MOD)
    body = rs[rs.find("pub fn default_aggregators()") :]
    body = body[: body.find("\n    }\n")]
    assert '"mean"' in body
    assert '"weighted"' in body
    assert '"median"' in body


# ─────────────────────────────────────────────────────────────────────
# Mode3Verdict
# ─────────────────────────────────────────────────────────────────────

def test_mode3_verdict_carries_k_metadata() -> None:
    rs = _read(MODE3_MOD)
    body = rs[rs.find("pub struct Mode3Verdict") :]
    body = body[: body.find("\n}\n")]
    for f in ("final_verdict", "sub_verdicts", "aggregation", "request_id", "total_ms", "k_clamped", "k_effective"):
        assert f in body, f"Mode3Verdict.{f} missing"


# ─────────────────────────────────────────────────────────────────────
# Sequential vs Parallel
# ─────────────────────────────────────────────────────────────────────

def test_mode3_has_sequential_and_parallel_paths() -> None:
    rs = _read(MODE3_MOD)
    assert "pub async fn execute_sequential(" in rs
    assert "pub async fn execute_parallel(" in rs


def test_mode3_parallel_uses_run_k_swarms() -> None:
    rs = _read(MODE3_MOD)
    body = rs[rs.find("pub async fn execute_parallel(") :]
    body = body[: body.find("/// Execute one sub-swarm")]
    assert "parallel::run_k_swarms" in body


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
