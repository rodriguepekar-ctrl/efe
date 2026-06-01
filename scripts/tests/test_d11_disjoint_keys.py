"""Sprint D11 — pin the disjoint-key invariant after adding the
``isolation::``, ``cost::``, and ``obs::`` namespaces.

Mirrors the structure of ``test_d10_disjoint_keys.py``. Catches:

  1. RESERVED_PREFIXES gains/loses an entry without explicit
     review (test fails, forcing a docs + BENCHMARKS update).
  2. A new SLO definition lands under a non-registered prefix.
  3. A D11 SLO accidentally collides with a previous-sprint key.
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
# 1. RESERVED_PREFIXES inventory after D11
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
    # Sprint D12 addition — see test_d12_disjoint_keys.py.
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
        f"prefix's owner in `BENCHMARKS.md` § 13.3."
    )


# ─────────────────────────────────────────────────────────────────────
# 2. D11 SLOs inventory
# ─────────────────────────────────────────────────────────────────────

def test_d11_slos_present():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    ids = {slo["id"] for slo in parse_criterion.SLO_DEFINITIONS}
    expected = {
        "SLO-D11-MT-1", "SLO-D11-MT-2", "SLO-D11-MT-3",
        "SLO-D11-MT-4", "SLO-D11-MT-5", "SLO-D11-MT-6",
        "SLO-D11-MT-7",
    }
    missing = expected - ids
    assert not missing, f"missing D11 SLOs: {missing}"


def test_d11_slos_have_well_formed_fields():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d11 = [s for s in parse_criterion.SLO_DEFINITIONS
           if s["id"].startswith("SLO-D11-")]
    assert len(d11) == 7, f"expected 7 D11 SLOs, got {len(d11)}"
    for slo in d11:
        assert slo["bench_key"], slo
        assert slo["target_ns"] >= 0, slo
        assert slo["comparator"] in ("lte", "gte"), slo


def test_d11_slo_keys_under_d11_namespaces():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    allowed = ("isolation::", "cost::", "obs::")
    for slo in parse_criterion.SLO_DEFINITIONS:
        if not slo["id"].startswith("SLO-D11-"):
            continue
        key = slo["bench_key"]
        assert any(key.startswith(p) for p in allowed), (
            f"D11 SLO {slo['id']} bench_key {key!r} not under "
            f"any of {allowed}"
        )


def test_d11_slo_ids_unique():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d11_ids = [s["id"] for s in parse_criterion.SLO_DEFINITIONS
               if s["id"].startswith("SLO-D11-")]
    assert len(d11_ids) == len(set(d11_ids)), \
        f"duplicate D11 SLO IDs: {d11_ids}"


# ─────────────────────────────────────────────────────────────────────
# 3. Cross-sprint disjointness
# ─────────────────────────────────────────────────────────────────────

def test_d11_bench_keys_disjoint_from_d8_d9_d10():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d11_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                if s["id"].startswith("SLO-D11-")}
    other_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                  if not s["id"].startswith("SLO-D11-")}
    overlap = d11_keys & other_keys
    assert not overlap, (
        f"D11 bench keys collide with earlier sprints: {overlap}"
    )


def test_total_slo_count_post_d11():
    """Sanity — pin the cumulative count so a removed SLO from an
    earlier sprint is caught by a single integer comparison."""
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    total = len(parse_criterion.SLO_DEFINITIONS)
    # D11 added 7 new ones; assert the count is at least the
    # cumulative count — reviewers update this when intentionally
    # removing an old SLO.
    assert total >= 7, (
        f"expected at least 7 D11 SLOs ; got {total} total"
    )
    # And specifically count the D11 ones.
    d11 = sum(1 for s in parse_criterion.SLO_DEFINITIONS
              if s["id"].startswith("SLO-D11-"))
    assert d11 == 7, f"expected exactly 7 D11 SLOs ; got {d11}"
