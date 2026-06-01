"""Sprint D13 — pin the disjoint-key invariant after adding the
``obs::dash::`` namespace.

Mirrors the structure of ``test_d12_disjoint_keys.py``. Catches:

  1. RESERVED_PREFIXES gains/loses an entry without explicit
     review (test fails, forcing a docs + BENCHMARKS update).
  2. A new SLO definition lands under a non-registered prefix.
  3. A D13 SLO accidentally collides with a previous-sprint key.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]


def _load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


# ─────────────────────────────────────────────────────────────────────
# 1. RESERVED_PREFIXES inventory after D13
# ─────────────────────────────────────────────────────────────────────

EXPECTED_PREFIXES = frozenset({
    # D6/D7/D8
    "eval_accuracy::",
    "eval_latency::",
    # D9
    "load_",
    "soak::",
    # D10
    "docs::",
    "dx::",
    # D11
    "isolation::",
    "cost::",
    "obs::",
    # D12
    "catalog::",
    # D13 NEW
    "obs::dash::",
})


def test_reserved_prefixes_match_expected():
    parse_ghz_report = _load_module(
        "parse_ghz_report",
        _REPO / "bench" / "load" / "parse_ghz_report.py",
    )
    actual = frozenset(parse_ghz_report.RESERVED_PREFIXES)
    assert actual == EXPECTED_PREFIXES, (
        f"RESERVED_PREFIXES drift:\n"
        f"  expected: {sorted(EXPECTED_PREFIXES)}\n"
        f"  actual:   {sorted(actual)}\n"
        f"Update both the constant in `parse_ghz_report.py` AND "
        f"`EXPECTED_PREFIXES` in this test, AND document the new "
        f"prefix's owner in `BENCHMARKS.md` § 15."
    )


# ─────────────────────────────────────────────────────────────────────
# 2. D13 SLOs inventory
# ─────────────────────────────────────────────────────────────────────

def test_d13_slos_present():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    ids = {slo["id"] for slo in parse_criterion.SLO_DEFINITIONS}
    expected = {
        "SLO-D13-OBS-1", "SLO-D13-OBS-2", "SLO-D13-OBS-3",
        "SLO-D13-OBS-4", "SLO-D13-OBS-5", "SLO-D13-OBS-6",
        "SLO-D13-OBS-7",
    }
    missing = expected - ids
    assert not missing, f"missing D13 SLOs: {missing}"


def test_d13_slos_have_well_formed_fields():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d13 = [s for s in parse_criterion.SLO_DEFINITIONS
           if s["id"].startswith("SLO-D13-")]
    assert len(d13) == 7, f"expected 7 D13 SLOs, got {len(d13)}"
    for slo in d13:
        assert slo["bench_key"], slo
        assert slo["target_ns"] >= 0, slo
        assert slo.get("comparator", "lte") in ("lte", "gte"), slo


def test_d13_slo_keys_under_obs_dash_namespace():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    for slo in parse_criterion.SLO_DEFINITIONS:
        if not slo["id"].startswith("SLO-D13-"):
            continue
        key = slo["bench_key"]
        assert key.startswith("obs::dash::"), (
            f"D13 SLO {slo['id']} bench_key {key!r} must start with "
            f"'obs::dash::'"
        )


def test_d13_slo_ids_unique():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d13_ids = [s["id"] for s in parse_criterion.SLO_DEFINITIONS
               if s["id"].startswith("SLO-D13-")]
    assert len(d13_ids) == len(set(d13_ids)), \
        f"duplicate D13 SLO IDs: {d13_ids}"


# ─────────────────────────────────────────────────────────────────────
# 3. Cross-sprint disjointness
# ─────────────────────────────────────────────────────────────────────

def test_d13_bench_keys_disjoint_from_d6_through_d12():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d13_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                if s["id"].startswith("SLO-D13-")}
    other_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                  if not s["id"].startswith("SLO-D13-")}
    overlap = d13_keys & other_keys
    assert not overlap, (
        f"D13 bench keys collide with earlier sprints: {overlap}"
    )


def test_total_slo_count_includes_d13():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d13 = sum(1 for s in parse_criterion.SLO_DEFINITIONS
              if s["id"].startswith("SLO-D13-"))
    assert d13 == 7, f"expected exactly 7 D13 SLOs ; got {d13}"


def test_obs_dash_is_subnamespace_of_obs():
    """`obs::dash::` is a sub-namespace of `obs::` (D11 still owns
    plain `obs::*`). Verify both exist and are distinct prefixes."""
    parse_ghz_report = _load_module(
        "parse_ghz_report",
        _REPO / "bench" / "load" / "parse_ghz_report.py",
    )
    actual = frozenset(parse_ghz_report.RESERVED_PREFIXES)
    assert "obs::" in actual
    assert "obs::dash::" in actual
    # And they're NOT the same string.
    assert "obs::" != "obs::dash::"
