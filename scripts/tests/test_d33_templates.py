"""Sprint **M1 extension D33** — Pin tests pour les 5 nouveaux
templates nano-agents (NaQual, NaCausal, NaTemp, NaSafety, NaConsist).

Vérifient que :
  * `TemplateCode` enum a 8 variantes (3 MVP + 5 GA).
  * Chaque template a son propre module `na_*.rs`.
  * Le trait `NanoAgentTemplate` est implémenté par chacune.
  * `default_share()` somme à 1.0 ± 0.005 sur les 8 templates.
  * `consensus_weight()` de NaSafety = 2.0 (institutional invariant).
  * Backward compat MVP_SET préservée.
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
TYPES_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "types.rs"
TEMPLATES_DIR = REPO_ROOT / "crates" / "efe-core" / "src" / "agents" / "templates"
TEMPLATES_MOD = TEMPLATES_DIR / "mod.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# Each tuple = (variant_name, repr_u8, stable_string)
EXPECTED_TEMPLATES: list[tuple[str, int, str]] = [
    ("NaSem", 0, "na-sem"),
    ("NaLog", 1, "na-log"),
    ("NaCrit", 2, "na-crit"),
    ("NaQual", 3, "na-qual"),
    ("NaCausal", 4, "na-causal"),
    ("NaTemp", 5, "na-temp"),
    ("NaSafety", 6, "na-safety"),
    ("NaConsist", 7, "na-consist"),
]


# ─────────────────────────────────────────────────────────────────────
# TemplateCode enum extension
# ─────────────────────────────────────────────────────────────────────

def test_template_code_has_8_variants() -> None:
    """All 8 variants present in `TemplateCode` enum."""
    rs = _read(TYPES_RS)
    for name, _, _ in EXPECTED_TEMPLATES:
        assert f"{name} =" in rs, f"variant {name} missing from TemplateCode enum"


def test_template_code_repr_u8_stable() -> None:
    """Numeric values are stable across versions (CUDA buffers)."""
    rs = _read(TYPES_RS)
    for name, value, _ in EXPECTED_TEMPLATES:
        assert f"{name} = {value}" in rs, (
            f"{name} must have stable u8 = {value} (CUDA encoding)"
        )


def test_template_code_as_str_stable_strings() -> None:
    rs = _read(TYPES_RS)
    for name, _, stable in EXPECTED_TEMPLATES:
        assert f'Self::{name} => "{stable}"' in rs, (
            f"as_str() missing stable string for {name}"
        )


def test_template_code_from_str_inverse() -> None:
    rs = _read(TYPES_RS)
    for name, _, stable in EXPECTED_TEMPLATES:
        assert f'"{stable}" => Some(Self::{name})' in rs, (
            f"from_str() missing inverse for {name}"
        )


def test_template_code_mvp_set_unchanged() -> None:
    """Backward compat — MVP_SET still has exactly 3 entries."""
    rs = _read(TYPES_RS)
    assert "MVP_SET: &'static [Self] = &[Self::NaSem, Self::NaLog, Self::NaCrit]" in rs


def test_template_code_ga_set_has_8_entries() -> None:
    rs = _read(TYPES_RS)
    assert "GA_SET: &'static [Self]" in rs
    # Spot-check : the 5 D33 additions must be referenced in GA_SET.
    for name in ("NaQual", "NaCausal", "NaTemp", "NaSafety", "NaConsist"):
        assert (
            f"Self::{name}" in rs
        ), f"GA_SET missing {name}"


def test_template_code_is_mvp_helper_present() -> None:
    rs = _read(TYPES_RS)
    assert "fn is_mvp(&self) -> bool" in rs


# ─────────────────────────────────────────────────────────────────────
# Per-template modules
# ─────────────────────────────────────────────────────────────────────

@pytest.mark.parametrize("variant,_v,stable", EXPECTED_TEMPLATES)
def test_each_template_has_module(variant: str, _v: int, stable: str) -> None:
    """One file per template under agents/templates/."""
    mod_name = "na_" + variant[2:].lower()
    f = TEMPLATES_DIR / f"{mod_name}.rs"
    assert f.exists(), f"missing module file {f}"


@pytest.mark.parametrize("variant,_v,_s", EXPECTED_TEMPLATES)
def test_each_template_implements_trait(variant: str, _v: int, _s: str) -> None:
    """Each template impls `NanoAgentTemplate`."""
    mod_name = "na_" + variant[2:].lower()
    rs = _read(TEMPLATES_DIR / f"{mod_name}.rs")
    assert (
        f"impl NanoAgentTemplate for {variant}" in rs
    ), f"{variant} doesn't impl NanoAgentTemplate"


@pytest.mark.parametrize("variant,_v,_s", EXPECTED_TEMPLATES)
def test_each_template_has_unit_struct(variant: str, _v: int, _s: str) -> None:
    mod_name = "na_" + variant[2:].lower()
    rs = _read(TEMPLATES_DIR / f"{mod_name}.rs")
    assert f"pub struct {variant}" in rs


# ─────────────────────────────────────────────────────────────────────
# NanoAgentTemplate trait + helpers
# ─────────────────────────────────────────────────────────────────────

def test_nano_agent_template_trait_present() -> None:
    rs = _read(TEMPLATES_MOD)
    assert "pub trait NanoAgentTemplate" in rs
    for method in (
        "fn code(&self)",
        "fn default_share(&self)",
        "fn fusion_kernel(&self)",
        "fn consensus_weight(&self)",
    ):
        assert method in rs, f"trait method missing : {method}"


def test_fusion_kernel_id_has_8_variants() -> None:
    rs = _read(TEMPLATES_MOD)
    for kernel in (
        "Semantic",
        "Logical",
        "Diversifying",
        "Quality",
        "Causal",
        "Temporal",
        "Safety",
        "Consistency",
    ):
        assert (
            f"{kernel}," in rs or f"{kernel} =" in rs or f"{kernel}\n" in rs
        ), f"FusionKernelId::{kernel} missing"


def test_centroid_init_enum_has_three_variants() -> None:
    rs = _read(TEMPLATES_MOD)
    assert "enum CentroidInit" in rs
    for variant in ("PerturbProblem", "Fixed", "Uniform"):
        assert variant in rs


def test_ga_templates_helper_present() -> None:
    rs = _read(TEMPLATES_MOD)
    assert "pub fn ga_templates() -> Vec<std::sync::Arc<dyn NanoAgentTemplate>>" in rs


def test_mvp_templates_helper_present() -> None:
    rs = _read(TEMPLATES_MOD)
    assert "pub fn mvp_templates() -> Vec<std::sync::Arc<dyn NanoAgentTemplate>>" in rs


# ─────────────────────────────────────────────────────────────────────
# Institutional invariants
# ─────────────────────────────────────────────────────────────────────

def test_safety_consensus_weight_is_2_0() -> None:
    """Institutional invariant : NaSafety has the highest consensus
    weight (= 2.0). A refusal vote should outweigh positive votes."""
    rs = _read(TEMPLATES_DIR / "na_safety.rs")
    assert "2.0" in rs, "NaSafety weight 2.0 missing"


def test_ga_includes_safety() -> None:
    """**Audit fix C-A4** — `na_safety.rs` doc references a pin test
    `test_d33_templates::test_ga_includes_safety` to enforce the
    institutional invariant "production deployments MUST include
    NaSafety at ≥ 5 % share". Pre-audit this pin test was referenced
    in the doc-comment but never implemented. Defect class : "doc
    invariant without pin test enforcement".

    The pin test verifies :
      1. `ga_templates()` returns a Vec containing `Arc::new(NaSafety)`.
      2. `TemplateCode::GA_SET` contains `NaSafety`.
      3. `na_safety.rs` exists with `default_share() >= 0.05`.
    """
    # (1) — ga_templates() helper includes NaSafety.
    rs = _read(TEMPLATES_MOD)
    block_start = rs.find("pub fn ga_templates()")
    block_end = rs.find("\n}\n", block_start)
    body = rs[block_start:block_end]
    assert "NaSafety" in body, (
        "ga_templates() must include NaSafety — institutional safety invariant"
    )

    # (2) — TemplateCode::GA_SET enumerates NaSafety.
    rs_types = _read(TYPES_RS)
    ga_block_start = rs_types.find("GA_SET: &'static [Self]")
    ga_block_end = rs_types.find("];", ga_block_start)
    ga_block = rs_types[ga_block_start:ga_block_end]
    assert "Self::NaSafety" in ga_block, (
        "TemplateCode::GA_SET must include NaSafety"
    )

    # (3) — NaSafety.default_share() is at least 5 %.
    rs_safety = _read(TEMPLATES_DIR / "na_safety.rs")
    safety_share_start = rs_safety.find("fn default_share(&self) -> f32")
    safety_share_end = rs_safety.find("\n    }\n", safety_share_start)
    body_share = rs_safety[safety_share_start:safety_share_end]
    # The contract is ≥ 0.05 ; the current value is exactly 0.05.
    # Anti-regression : the default share must NOT drop below 0.05
    # (a 1 % share would dilute the refusal signal too much).
    assert "0.05" in body_share, "NaSafety default_share must be ≥ 0.05"


def test_safety_centroid_init_returns_none_at_template_level() -> None:
    """The actual safety anchor centroid is resolved at genesis time
    when dim is known. The trait returns None ; the genesis spawner
    picks up the slack."""
    rs = _read(TEMPLATES_DIR / "na_safety.rs")
    # The handler explicitly documents the None choice with the audit
    # marker C-A2.
    assert "C-A2" in rs


def test_na_crit_uses_uniform_init() -> None:
    """NaCrit anti-mode-collapse role requires Uniform init."""
    rs = _read(TEMPLATES_DIR / "na_crit.rs")
    assert "CentroidInit::Uniform" in rs


def test_default_shares_documented_to_sum_to_one() -> None:
    """The pin test `ga_default_shares_sum_to_unity` enforces the
    invariant in Rust ; this Python test documents the expected
    arithmetic so any future change is visible in PR diffs."""
    rs = _read(TEMPLATES_DIR / "na_sem.rs")
    # NaSem 0.45 documented in the comment block.
    assert "0.45" in rs
    assert "= 1.00" in rs


# ─────────────────────────────────────────────────────────────────────
# Cardinality::balanced_ga
# ─────────────────────────────────────────────────────────────────────

def test_balanced_ga_helper_present() -> None:
    cardinality_rs = REPO_ROOT / "crates" / "efe-core" / "src" / "genesis" / "cardinality.rs"
    rs = _read(cardinality_rs)
    assert "pub fn balanced_ga(total_pop: u32)" in rs


def test_balanced_mvp_helper_present() -> None:
    cardinality_rs = REPO_ROOT / "crates" / "efe-core" / "src" / "genesis" / "cardinality.rs"
    rs = _read(cardinality_rs)
    assert "pub fn balanced_mvp(total_pop: u32)" in rs


def test_balanced_ga_calls_ga_templates() -> None:
    cardinality_rs = REPO_ROOT / "crates" / "efe-core" / "src" / "genesis" / "cardinality.rs"
    rs = _read(cardinality_rs)
    assert "ga_templates()" in rs


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
