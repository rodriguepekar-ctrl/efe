"""Sprint D10 — pin the disjoint-key invariant across all SLO
namespaces.

Each SLO emitter (``parse_ghz_report.py``, ``leak_detector.py``,
``docs.yml`` workflow, ``validate_quickstart.sh``) owns exactly one
prefix and MUST NOT emit keys under a sibling's prefix.

The invariant is:

    * ``eval_accuracy::*``  — owned by ``eval/scripts/run_eval*.py``
    * ``eval_latency::*``   — owned by ``eval/scripts/run_eval*.py``
    * ``load_*::*``         — owned by ``bench/load/parse_ghz_report.py``
    * ``soak::*``           — owned by ``bench/soak/leak_detector.py``
    * ``docs::*``           — owned by ``.github/workflows/docs.yml``
    * ``dx::*``             — owned by ``scripts/validate_quickstart.sh``

If a regression introduces a key under the wrong prefix, the
aggregator silently overwrites or skips. This test catches the
class of bugs introduced in the D8 audit (CRITICAL bug #1: SLOs
silently SKIPPED).
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
# 1. Each registered prefix is documented in RESERVED_PREFIXES
# ─────────────────────────────────────────────────────────────────────

EXPECTED_PREFIXES = frozenset({
    "eval_accuracy::",
    "eval_latency::",
    "load_",
    "soak::",
    "docs::",
    "dx::",
    # Sprint D11 additions — see test_d11_disjoint_keys.py for the
    # canonical inventory.
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
        f"Update both ``parse_ghz_report.RESERVED_PREFIXES`` and "
        f"``EXPECTED_PREFIXES`` in this test, AND document the new "
        f"prefix's owner in `BENCHMARKS.md` § 12.3."
    )


# ─────────────────────────────────────────────────────────────────────
# 2. SLO_DEFINITIONS bench_keys all live under a registered prefix
# ─────────────────────────────────────────────────────────────────────

def test_every_slo_bench_key_under_registered_prefix():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )

    # Some legacy SLOs use criterion-style keys (group_name::id).
    # We exempt those with a known prefix list — these are the keys
    # the criterion benches emit (one prefix per bench group, named
    # after the criterion group). New SLOs MUST use one of the
    # RESERVED_PREFIXES; legacy ones are grandfathered.
    LEGACY_OK_PREFIXES = (
        # Sprint C / S-series criterion benches
        "swarm_e2e_",
        "memory_e2e_",
        "real_e2e_",
        "real_strategies_e2e_",
        # Sprint D2 — memory pool / arena
        "pool_acquire_",
        "pool_concurrent_",
        "arena_alloc_",
        "arena_reset",
        "realistic_swarm_alloc_pattern",
        "memory_real_arena",
        # Sprint D3 — compute pipeline
        "compute_pipeline",
        # Sprint D4 — gRPC server
        "server_rpc",
        "rpc_e2e",
    )

    violations: list[tuple[str, str]] = []
    for slo in parse_criterion.SLO_DEFINITIONS:
        key = slo["bench_key"]
        # Either matches a RESERVED_PREFIXES entry OR a legacy criterion key.
        if any(key.startswith(p) for p in EXPECTED_PREFIXES):
            continue
        if any(key.startswith(p) for p in LEGACY_OK_PREFIXES):
            continue
        violations.append((slo["id"], key))

    assert not violations, (
        f"SLO bench_keys outside RESERVED_PREFIXES:\n"
        + "\n".join(f"  {sid}: {key!r}" for sid, key in violations)
        + "\nEither move them to a registered prefix or add the prefix "
          "to LEGACY_OK_PREFIXES in this test."
    )


# ─────────────────────────────────────────────────────────────────────
# 3. SLO IDs from each sprint family stay disjoint
# ─────────────────────────────────────────────────────────────────────

def test_d10_slos_present():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    ids = {slo["id"] for slo in parse_criterion.SLO_DEFINITIONS}
    expected = {
        "SLO-D10-DOC-1", "SLO-D10-DOC-2", "SLO-D10-DOC-3",
        "SLO-D10-DX-1", "SLO-D10-DX-2",
    }
    missing = expected - ids
    assert not missing, f"missing D10 SLOs: {missing}"


def test_d10_slos_have_well_formed_fields():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d10 = [s for s in parse_criterion.SLO_DEFINITIONS
           if s["id"].startswith("SLO-D10-")]
    assert len(d10) == 5, f"expected 5 D10 SLOs, got {len(d10)}"
    for slo in d10:
        assert slo["bench_key"], slo
        assert slo["target_ns"] >= 0, slo
        assert slo["comparator"] in ("lte", "gte"), slo


def test_d10_slo_keys_under_docs_or_dx_prefix():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    for slo in parse_criterion.SLO_DEFINITIONS:
        if not slo["id"].startswith("SLO-D10-"):
            continue
        key = slo["bench_key"]
        assert key.startswith("docs::") or key.startswith("dx::"), (
            f"D10 SLO {slo['id']} bench_key {key!r} not under docs::/dx::"
        )


def test_d10_slo_ids_unique():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d10_ids = [s["id"] for s in parse_criterion.SLO_DEFINITIONS
               if s["id"].startswith("SLO-D10-")]
    assert len(d10_ids) == len(set(d10_ids)), \
        f"duplicate D10 SLO IDs: {d10_ids}"


# ─────────────────────────────────────────────────────────────────────
# 4. Cross-sprint: D10 keys disjoint from D8/D9
# ─────────────────────────────────────────────────────────────────────

def test_d10_bench_keys_disjoint_from_d8_d9():
    parse_criterion = _load_module(
        "parse_criterion_report",
        _REPO / "scripts" / "parse_criterion_report.py",
    )
    d10_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                if s["id"].startswith("SLO-D10-")}
    other_keys = {s["bench_key"] for s in parse_criterion.SLO_DEFINITIONS
                  if not s["id"].startswith("SLO-D10-")}
    overlap = d10_keys & other_keys
    assert not overlap, f"D10 bench keys collide with earlier sprints: {overlap}"


# ─────────────────────────────────────────────────────────────────────
# 5. parse_ghz_report assert_disjoint_keys works end-to-end
# ─────────────────────────────────────────────────────────────────────

def test_assert_disjoint_keys_rejects_docs_namespace_in_load_artefact():
    """A load artefact must NOT contain `docs::*` keys — that would
    indicate a workflow misconfiguration."""
    parse_ghz_report = _load_module(
        "parse_ghz_report",
        _REPO / "bench" / "load" / "parse_ghz_report.py",
    )
    bench = {"docs::build_strict_warnings": {"median_ns": 0}}
    import pytest
    with pytest.raises(ValueError, match="out-of-namespace"):
        parse_ghz_report.assert_disjoint_keys(bench)


def test_assert_disjoint_keys_rejects_eval_in_load_artefact():
    """The classic D8 audit bug: a load artefact emitting eval_*
    keys would silently overwrite accuracy measurements."""
    parse_ghz_report = _load_module(
        "parse_ghz_report",
        _REPO / "bench" / "load" / "parse_ghz_report.py",
    )
    bench = {"eval_accuracy::top1": {"median_ns": 0.9 * 1e9}}
    import pytest
    with pytest.raises(ValueError, match="accuracy-namespace"):
        parse_ghz_report.assert_disjoint_keys(bench)
