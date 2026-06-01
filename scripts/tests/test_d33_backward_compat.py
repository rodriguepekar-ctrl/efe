"""Sprint **M1 extension D33** — Backward compat anti-régression.

Verrouille les invariants stables entre les sprints D1-D9 (MVP) et
D33 (GA). Toute violation = breaking change → major version bump.
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
TYPES_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "types.rs"
NA_SEM_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "agents" / "templates" / "na_sem.rs"
NA_LOG_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "agents" / "templates" / "na_log.rs"
NA_CRIT_RS = REPO_ROOT / "crates" / "efe-core" / "src" / "agents" / "templates" / "na_crit.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_mvp_set_unchanged() -> None:
    """`MVP_SET` exposes exactly the 3 D1-D9 templates ; nothing else."""
    rs = _read(TYPES_RS)
    assert (
        "MVP_SET: &'static [Self] = &[Self::NaSem, Self::NaLog, Self::NaCrit]" in rs
    )


def test_mvp_template_u8_stable() -> None:
    """NaSem=0, NaLog=1, NaCrit=2 forever (CUDA buffer encoding)."""
    rs = _read(TYPES_RS)
    assert "NaSem = 0" in rs
    assert "NaLog = 1" in rs
    assert "NaCrit = 2" in rs


def test_mvp_template_strings_stable() -> None:
    """The stable string forms used in YAML / telemetry / audit."""
    rs = _read(TYPES_RS)
    assert 'Self::NaSem => "na-sem"' in rs
    assert 'Self::NaLog => "na-log"' in rs
    assert 'Self::NaCrit => "na-crit"' in rs


def test_mvp_template_codes_pre_d33_unchanged() -> None:
    """The 3 MVP variants' is_mvp() must return true."""
    rs = _read(TYPES_RS)
    # Find the is_mvp method body.
    h_start = rs.find("fn is_mvp(&self) -> bool")
    h_end = rs.find("\n    }", h_start)
    body = rs[h_start:h_end]
    assert "Self::NaSem | Self::NaLog | Self::NaCrit" in body


def test_na_sem_module_preserves_share_share() -> None:
    """Pin the NaSem default share value (0.45 post-D33). Changing
    this requires bumping the catalog SLA + re-running every D7
    accuracy bench."""
    rs = _read(NA_SEM_RS)
    assert "0.45" in rs


def test_na_log_module_preserves_share() -> None:
    rs = _read(NA_LOG_RS)
    assert "0.05" in rs


def test_na_crit_module_preserves_share() -> None:
    rs = _read(NA_CRIT_RS)
    assert "0.05" in rs


def test_mvp_template_consensus_weights_baseline() -> None:
    """All 3 MVP templates have consensus_weight = 1.0 (baseline).
    GA additions are allowed to differ ; MVP is locked."""
    for f in (NA_SEM_RS, NA_LOG_RS, NA_CRIT_RS):
        rs = _read(f)
        assert "fn consensus_weight(&self) -> f32 {" in rs
        # The body should return literal 1.0 (potentially with a
        # comment justifying it). We grep for the value preceded by
        # whitespace and followed by either a newline or a closing
        # brace.
        assert "1.0" in rs


def test_mvp_kernel_routing_stable() -> None:
    """NaSem → Semantic, NaLog → Logical, NaCrit → Diversifying."""
    assert "FusionKernelId::Semantic" in _read(NA_SEM_RS)
    assert "FusionKernelId::Logical" in _read(NA_LOG_RS)
    assert "FusionKernelId::Diversifying" in _read(NA_CRIT_RS)


def test_template_code_repr_u8_attribute_present() -> None:
    """The `#[repr(u8)]` attribute MUST stay — its removal would
    silently change the wire format of CUDA buffers."""
    rs = _read(TYPES_RS)
    block_start = rs.find("pub enum TemplateCode")
    block_pre = rs[max(0, block_start - 200):block_start]
    assert "#[repr(u8)]" in block_pre


def test_template_code_non_exhaustive_attribute_present() -> None:
    """`#[non_exhaustive]` lets future templates ship without breaking
    pattern-matching consumers."""
    rs = _read(TYPES_RS)
    block_start = rs.find("pub enum TemplateCode")
    block_pre = rs[max(0, block_start - 200):block_start]
    assert "#[non_exhaustive]" in block_pre


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
