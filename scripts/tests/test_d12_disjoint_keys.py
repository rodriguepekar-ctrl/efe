"""Sprint D12 — pin the disjoint-key invariant after adding the
``catalog::`` namespace.

Mirrors the structure of ``test_d11_disjoint_keys.py``. Catches:

  1. RESERVED_PREFIXES gains/loses an entry without explicit
     review (test fails, forcing a docs + BENCHMARKS update).
  2. A new SLO definition lands under a non-registered prefix.
  3. A D12 SLO accidentally collides with a previous-sprint key.
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
# 1. RESERVED_PREFIXES inventory after D12
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
    # Sprint D13 addition — see test_d13_disjoint_keys.py.
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
        f"prefix's owner in `BENCHMARKS.md` § 14."
    )


# ─────────────────────────────────────────────────────────────────────
# 2. D12 SLOs inventory
# ─────────────────────────────────────────────────────────────────────

def test_d12_slos_present():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    ids = {slo["id"] for slo in parse_criterion.SLO_DEFINITIONS}
    expected = {
        "SLO-D12-CT-1", "SLO-D12-CT-2", "SLO-D12-CT-3",
        "SLO-D12-CT-4", "SLO-D12-CT-5", "SLO-D12-CT-6",
        "SLO-D12-CT-7",
    }
    missing = expected - ids
    assert not missing, f"missing D12 SLOs: {missing}"


def test_d12_slos_have_well_formed_fields():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d12 = [s for s in parse_criterion.SLO_DEFINITIONS
           if s["id"].startswith("SLO-D12-")]
    assert len(d12) == 7, f"expected 7 D12 SLOs, got {len(d12)}"
    for slo in d12:
        assert slo["bench_key"], slo
        assert slo["target_ns"] >= 0, slo
        assert slo.get("comparator", "lte") in ("lte", "gte"), slo


def test_d12_slo_keys_under_catalog_namespace():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    for slo in parse_criterion.SLO_DEFINITIONS:
        if not slo["id"].startswith("SLO-D12-"):
            continue
        key = slo["bench_key"]
        assert key.startswith("catalog::"), (
            f"D12 SLO {slo['id']} bench_key {key!r} must start with "
            f"'catalog::'"
        )


def test_d12_slo_ids_unique():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d12_ids = [s["id"] for s in parse_criterion.SLO_DEFINITIONS
               if s["id"].startswith("SLO-D12-")]
    assert len(d12_ids) == len(set(d12_ids)), \
        f"duplicate D12 SLO IDs: {d12_ids}"


# ─────────────────────────────────────────────────────────────────────
# 3. Cross-sprint disjointness
# ─────────────────────────────────────────────────────────────────────

def test_d12_bench_keys_disjoint_from_d6_through_d11():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d12_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                if s["id"].startswith("SLO-D12-")}
    other_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                  if not s["id"].startswith("SLO-D12-")}
    overlap = d12_keys & other_keys
    assert not overlap, (
        f"D12 bench keys collide with earlier sprints: {overlap}"
    )


def test_total_slo_count_includes_d12():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d12 = sum(1 for s in parse_criterion.SLO_DEFINITIONS
              if s["id"].startswith("SLO-D12-"))
    assert d12 == 7, f"expected exactly 7 D12 SLOs ; got {d12}"


# ─────────────────────────────────────────────────────────────────────
# 4. SLO targets sanity
# ─────────────────────────────────────────────────────────────────────

def test_d12_latency_slos_have_lte_comparator():
    """D12-CT-1..5 are wire-latency budgets — they must be `lte`."""
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    latency_slos = ("SLO-D12-CT-1", "SLO-D12-CT-2", "SLO-D12-CT-3",
                    "SLO-D12-CT-4", "SLO-D12-CT-5")
    for slo in parse_criterion.SLO_DEFINITIONS:
        if slo["id"] in latency_slos:
            assert slo.get("comparator", "lte") == "lte", (
                f"{slo['id']} must use 'lte' comparator (lower-is-better)"
            )


def test_d12_correctness_slos_have_gte_comparator():
    """D12-CT-6 (serialization pass %) and D12-CT-7 (sha256 match %)
    are higher-is-better."""
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    correctness_slos = ("SLO-D12-CT-6", "SLO-D12-CT-7")
    for slo in parse_criterion.SLO_DEFINITIONS:
        if slo["id"] in correctness_slos:
            assert slo["comparator"] == "gte", (
                f"{slo['id']} must use 'gte' comparator "
                f"(higher-is-better)"
            )
            # Both correctness SLOs should target 1.0 (100 %).
            assert slo["target_ns"] == 1_000_000_000, (
                f"{slo['id']} target should be 1e9 (= 1.0 ratio)"
            )
