"""Sprint **M1 extension D38** — Pin tests pour la parallélisation
k-swarm via `tokio::task::JoinSet`.

Verrouillent que :
  * `swarm::mode3::parallel::run_k_swarms<F>` existe et est public.
  * Utilise `tokio::task::JoinSet`.
  * Fail-fast pattern (premier erreur abort le reste).
  * Map `JoinError` panics → `EfeError::Internal`.
  * `Mode3Controller::execute_parallel` consomme run_k_swarms.
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
PARALLEL_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "swarm" / "mode3" / "parallel.rs"
MODE3_MOD = REPO_ROOT / "crates" / "efe-core" / "src" / "swarm" / "mode3" / "mod.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_parallel_module_exists() -> None:
    assert PARALLEL_RS.exists()


def test_run_k_swarms_signature() -> None:
    rs = _read(PARALLEL_RS)
    assert "pub async fn run_k_swarms<T, F, Fut>" in rs


def test_run_k_swarms_uses_join_set() -> None:
    rs = _read(PARALLEL_RS)
    assert "tokio::task::JoinSet" in rs or "JoinSet::new()" in rs


def test_run_k_swarms_fail_fast_aborts_remaining() -> None:
    rs = _read(PARALLEL_RS)
    body = rs[rs.find("pub async fn run_k_swarms") :]
    body = body[: body.find("\nfn map_join_error")]
    # Both error paths must call abort_handles.
    n_aborts = body.count("h.abort()")
    assert n_aborts >= 2, f"expected ≥2 abort calls, got {n_aborts}"


def test_run_k_swarms_handles_join_error() -> None:
    rs = _read(PARALLEL_RS)
    assert "JoinError" in rs
    assert "fn map_join_error(" in rs


def test_join_error_to_efe_error_internal() -> None:
    rs = _read(PARALLEL_RS)
    body = rs[rs.find("fn map_join_error") :]
    body = body[: body.find("\n}\n")]
    assert "EfeError::Internal" in body
    assert "is_cancelled()" in body


def test_run_k_swarms_arrival_order_documented() -> None:
    rs = _read(PARALLEL_RS)
    assert "arrival order" in rs.lower()


def test_run_k_swarms_zero_tasks_returns_empty_documented() -> None:
    rs = _read(PARALLEL_RS)
    # The unit test `run_k_swarms_zero_tasks_returns_empty_vec`
    # documents this contract.
    assert "run_k_swarms_zero_tasks_returns_empty_vec" in rs


# ─────────────────────────────────────────────────────────────────────
# Mode3Controller integration
# ─────────────────────────────────────────────────────────────────────

def test_mode3_execute_parallel_calls_run_k_swarms() -> None:
    rs = _read(MODE3_MOD)
    body = rs[rs.find("pub async fn execute_parallel(") :]
    body = body[: body.find("/// Execute one sub-swarm")]
    assert "parallel::run_k_swarms" in body


def test_mode3_execute_parallel_takes_arc_self() -> None:
    """The parallel path needs `Arc<Self>` so each spawned task can
    clone the controller handle."""
    rs = _read(MODE3_MOD)
    sig = rs[rs.find("pub async fn execute_parallel(") : rs.find(") -> Result<Mode3Verdict>", rs.find("pub async fn execute_parallel("))]
    assert "self: Arc<Self>" in sig


def test_parallel_mod_exposed_in_mode3_mod() -> None:
    rs = _read(MODE3_MOD)
    assert "pub mod parallel;" in rs


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
