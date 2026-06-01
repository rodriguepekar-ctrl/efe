"""Sprint **M1 extension D32 cross-cutting** — Pin tests pour
`HybridFusion` / `CanaryHybridFusion`.

Verrouillent que :
  * `fusion::hybrid` module existe et expose `CanaryHybridFusion<P, S>`.
  * `HybridFusionReport` struct + `divergence_detected()` méthode.
  * `CanaryConfig` avec sampling_rate / divergence_l2 /
    log_on_divergence.
  * Sampling deterministic (`sample_gate(rate, round)`).
  * Fail-open : shadow erreur → primary verdict retourné.
  * 4 thresholds divergence : centroid_l2 + variance_ratio
    (low + high) + confidence_delta.
  * Implementation du trait FusionStrategy (drop-in).
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
HYBRID_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "fusion" / "hybrid.rs"
FUSION_MOD = REPO_ROOT / "crates" / "efe-core" / "src" / "fusion" / "mod.rs"
LIB_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "lib.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_hybrid_module_exists() -> None:
    assert HYBRID_RS.exists()


def test_fusion_mod_exposes_hybrid() -> None:
    rs = _read(FUSION_MOD)
    assert "pub mod hybrid;" in rs
    for sym in ("CanaryConfig", "CanaryHybridFusion", "HybridFusionReport"):
        assert sym in rs, f"fusion/mod.rs missing re-export {sym}"


def test_lib_rs_exposes_hybrid_top_level() -> None:
    rs = _read(LIB_RS)
    for sym in ("CanaryConfig", "CanaryHybridFusion", "HybridFusionReport"):
        assert sym in rs, f"lib.rs top-level missing {sym}"


# ─────────────────────────────────────────────────────────────────────
# CanaryHybridFusion
# ─────────────────────────────────────────────────────────────────────

def test_canary_struct_present() -> None:
    rs = _read(HYBRID_RS)
    assert "pub struct CanaryHybridFusion<P, S>" in rs


def test_canary_implements_fusion_strategy() -> None:
    """Drop-in replacement contract : CanaryHybridFusion implements
    the swarm-side FusionStrategy trait so the controller can use
    it without code change."""
    rs = _read(HYBRID_RS)
    assert "impl<P, S> FusionStrategy for CanaryHybridFusion<P, S>" in rs


def test_canary_run_round_calls_primary() -> None:
    """**Audit fix C-A1** — `run_round` must call primary, but shadow
    execution is disabled until DT-C-D32-SNAPSHOT-1 ships proper
    snapshot/restore. Pre-fix the method called both primary and
    shadow on the same `els: &mut`, which double-mutated the state
    and corrupted subsequent rounds.

    The pin test now :
      1. Verifies primary is called (excluding doc-comments).
      2. Verifies shadow is NOT called (anti-regression on the
         double-mutation bug). We strip doc-comments before grep
         because the audit-fix narrative explicitly mentions the
         old call site to document the regression.
    """
    rs = _read(HYBRID_RS)
    body = rs[rs.find("async fn run_round(") :]
    body = body[: body.find("\n    async fn inject_noise")]

    # Strip doc-comments (lines that begin with `//`) so our grep
    # only sees executable statements.
    code_lines = [
        line
        for line in body.splitlines()
        if not line.lstrip().startswith("//")
    ]
    code = "\n".join(code_lines)

    # (1) Primary IS called.
    assert "self.primary.run_round(" in code, "primary must be invoked"
    # (2) Shadow is NOT called in run_round CODE (excludes the
    # audit-fix documentation that references the pre-fix bug).
    assert "self.shadow.run_round(" not in code, (
        "shadow.run_round must NOT be invoked in run_round (C-A1 fix) ; "
        "would double-mutate els"
    )


def test_canary_inject_noise_forwards_to_primary_only() -> None:
    rs = _read(HYBRID_RS)
    body = rs[rs.find("async fn inject_noise(") :]
    body = body[: body.find("\n}\n")]
    assert "self.primary.inject_noise(" in body
    # Anti-regression : shadow must NOT be touched in inject_noise.
    assert "self.shadow.inject_noise(" not in body


# ─────────────────────────────────────────────────────────────────────
# Sampling gate
# ─────────────────────────────────────────────────────────────────────

def test_sample_gate_deterministic_per_round() -> None:
    rs = _read(HYBRID_RS)
    assert "fn sample_gate(rate: f32, round: u32) -> bool" in rs


def test_sample_gate_uses_multiplicative_hash() -> None:
    rs = _read(HYBRID_RS)
    body = rs[rs.find("fn sample_gate(") :]
    body = body[: body.find("\n}\n")]
    # Knuth's golden ratio constant 2^32 / phi = 2654435761.
    assert "2_654_435_761" in body or "2654435761" in body


# ─────────────────────────────────────────────────────────────────────
# Fail-open
# ─────────────────────────────────────────────────────────────────────

def test_canary_run_round_returns_primary_stats_unchanged() -> None:
    """**Audit fix C-A1** — `run_round` always returns primary_stats.
    Pre-fix on shadow error this was the "fail-open" path ; post-fix
    shadow never runs so primary_stats is the only return value."""
    rs = _read(HYBRID_RS)
    body = rs[rs.find("async fn run_round(") :]
    body = body[: body.find("\n    async fn inject_noise")]
    assert "Ok(primary_stats)" in body


def test_canary_documents_dt_c_d32_snapshot() -> None:
    """**Audit fix C-A1** — the disabled-shadow path must explicitly
    reference the DT identifier so future operators / engineers can
    grep for it."""
    rs = _read(HYBRID_RS)
    assert "DT-C-D32-SNAPSHOT-1" in rs, (
        "C-A1 fix must reference DT-C-D32-SNAPSHOT-1 in the disabled-shadow path"
    )


def test_canary_emits_debug_trace_event() -> None:
    """**Audit fix C-A1** — the shadow-disabled report still emits a
    cadence trace event so dashboards see a regular pulse even
    without divergence detection."""
    rs = _read(HYBRID_RS)
    body = rs[rs.find("async fn run_round(") :]
    body = body[: body.find("\n    async fn inject_noise")]
    assert "tracing::" in body


# ─────────────────────────────────────────────────────────────────────
# HybridFusionReport divergence thresholds
# ─────────────────────────────────────────────────────────────────────

def test_report_struct_has_required_fields() -> None:
    rs = _read(HYBRID_RS)
    body = rs[rs.find("pub struct HybridFusionReport") :]
    body = body[: body.find("\n}\n")]
    for f in (
        "round",
        "primary_stats",
        "shadow_stats",
        "centroid_l2_diff",
        "variance_ratio",
        "confidence_delta",
        "primary_ms",
        "shadow_ms",
        "shadow_invoked",
    ):
        assert f in body, f"HybridFusionReport.{f} missing"


def test_divergence_detected_method_present() -> None:
    rs = _read(HYBRID_RS)
    assert "pub fn divergence_detected(&self) -> bool" in rs


def test_divergence_thresholds_documented() -> None:
    rs = _read(HYBRID_RS)
    body = rs[rs.find("pub fn divergence_detected(&self) -> bool") :]
    body = body[: body.find("\n}\n")]
    # The 4-axis test : L2 > 0.05, variance_ratio < 0.5 OR > 2.0,
    # |confidence_delta| > 0.10.
    assert "0.05" in body
    assert "0.5" in body
    assert "2.0" in body
    assert "0.10" in body


def test_divergence_short_circuits_when_shadow_not_invoked() -> None:
    rs = _read(HYBRID_RS)
    body = rs[rs.find("pub fn divergence_detected(&self) -> bool") :]
    body = body[: body.find("\n}\n")]
    assert "self.shadow_invoked" in body
    assert "return false" in body


# ─────────────────────────────────────────────────────────────────────
# CanaryConfig
# ─────────────────────────────────────────────────────────────────────

def test_canary_config_default_10_percent() -> None:
    rs = _read(HYBRID_RS)
    body = rs[rs.find("impl Default for CanaryConfig") :]
    body = body[: body.find("\n}\n")]
    assert "0.1" in body  # sampling_rate
    assert "0.05" in body  # divergence_l2


def test_canary_config_from_env_present() -> None:
    rs = _read(HYBRID_RS)
    assert "pub fn from_env() -> Self" in rs
    assert "EFE_FUSION_CANARY_RATE" in rs
    assert "EFE_FUSION_CANARY_DIVERGENCE_L2" in rs
    assert "EFE_FUSION_CANARY_LOG_DIVERGENCE" in rs


def test_canary_config_clamps_rate_to_unit_interval() -> None:
    rs = _read(HYBRID_RS)
    body = rs[rs.find("pub fn from_env() -> Self") :]
    body = body[: body.find("\n    }\n")]
    assert ".clamp(0.0, 1.0)" in body


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
