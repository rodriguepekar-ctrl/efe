#!/usr/bin/env python3
"""bench/load/parse_ghz_report.py — Sprint D9.3 ghz → bench-format adapter.

Reads one or more ghz JSON reports (output of `ghz --format=json`),
extracts p50 / p95 / p99 / mean / throughput / error-rate, computes
the spike-recovery time when the spike scenario is present, and
emits two artefacts:

1. A bench-format JSON file ingestible by
   ``scripts/parse_criterion_report.py`` (the SLO evaluator). Each
   ghz scenario contributes a ``load_<scenario>::<metric>`` group.
   The numeric values follow the bench-format convention:
   ``median_ns`` for raw nanosecond measurements, ``median_ns``
   pre-scaled by ``×1e9`` for ratios (matches the parser's
   ``display_as: ratio`` and ``scale: 1e9`` semantics).

2. A markdown summary appended to whatever ``--summary-out`` points
   at, suitable for a CI PR comment.

Exit code:
   0 — every requested SLO PASSED on the parsed data.
   1 — at least one SLO FAILED.
   2 — operational failure (file missing, JSON malformed, …).

This module is intentionally **stdlib-only** (no ``numpy``, no
``pandas``) so it runs on the same minimal Docker / CI image as
``run_load_test.sh``.

The four D9-LOAD-* SLOs evaluated here are also defined statically
in ``scripts/parse_criterion_report.py::SLO_DEFINITIONS`` — this
adapter pre-emits them so the aggregator can consume the load test
artefact uniformly with the rest of the bench-format ecosystem.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import sys
import time
from pathlib import Path
from typing import Any


# ─────────────────────────────────────────────────────────────────────
# ghz field extractors (defensive against schema drift)
# ─────────────────────────────────────────────────────────────────────

def _ghz_safe(report: dict[str, Any], path: list[str], default: Any = None) -> Any:
    """Walk a JSON path, returning ``default`` if any key is missing.

    ghz versions occasionally rename top-level fields between minor
    releases ; we keep this adapter resilient by never raising on
    missing keys. A missing required field maps to a SLO SKIP rather
    than a hard parser failure, which is the right semantic when CI
    runs against a ghz version we haven't pinned.
    """
    cur = report
    for key in path:
        if not isinstance(cur, dict) or key not in cur:
            return default
        cur = cur[key]
    return cur


def _ghz_scenario_label(report: dict[str, Any]) -> str:
    """Recover the scenario label (smoke/sustained/stress/spike).

    ghz emits the metadata map under ``options.metadata`` ; we set
    ``x-load-test-scenario`` on every config so the parser can
    associate a ghz report with the bench-format key without
    relying on filename conventions.
    """
    md = _ghz_safe(report, ["options", "metadata"], {}) or {}
    label = md.get("x-load-test-scenario") or md.get("x_load_test_scenario")
    if label:
        return str(label).strip().lower()
    # Fallback — ghz CLI version pre-0.117 does not echo metadata in
    # the report. Inspect the load schedule to make a best-effort
    # guess (sufficient for D9 ; the metadata path is the canonical
    # one once the matrix moves to ghz ≥ 0.117).
    schedule = _ghz_safe(report, ["options", "loadSchedule"], "")
    if schedule == "step":
        return "spike"
    if _ghz_safe(report, ["options", "concurrency"], 0) >= 16:
        return "stress"
    if _ghz_safe(report, ["options", "concurrency"], 0) >= 4:
        return "sustained"
    return "smoke"


def _ghz_count(report: dict[str, Any]) -> int:
    """Total response count (success + error)."""
    return int(_ghz_safe(report, ["count"], 0) or 0)


def _ghz_p_ns(report: dict[str, Any], pct: float) -> float | None:
    """Return latency at percentile ``pct`` (e.g. 99) in nanoseconds.

    ghz emits the latency distribution as
    ``latencyDistribution: [{percentage: int, latency: int_ns}, ...]``.
    The percentile values are integers (50, 75, 90, 95, 99) — we
    expose only the ones present, returning ``None`` when the
    distribution is empty (e.g. all responses errored).
    """
    dist = _ghz_safe(report, ["latencyDistribution"], []) or []
    target = int(pct)
    for entry in dist:
        if int(entry.get("percentage", -1)) == target:
            # ghz emits latency as nanoseconds since v0.117.
            return float(entry.get("latency", 0))
    return None


def _ghz_throughput_rps(report: dict[str, Any]) -> float:
    """Achieved RPS = count / wallclock duration (seconds)."""
    duration_ns = float(_ghz_safe(report, ["total"], 0) or 0)
    count = _ghz_count(report)
    if duration_ns <= 0 or count == 0:
        return 0.0
    return count / (duration_ns / 1e9)


def _ghz_error_ratio(report: dict[str, Any]) -> float:
    """Failed-RPC ratio = (count - successes) / count."""
    count = _ghz_count(report)
    if count == 0:
        return 1.0
    # ghz reports `statusCodeDistribution: { "OK": <n>, "...": <m> }`.
    by_code = _ghz_safe(report, ["statusCodeDistribution"], {}) or {}
    ok = int(by_code.get("OK", 0))
    return max(0.0, min(1.0, 1.0 - ok / count))


# ─────────────────────────────────────────────────────────────────────
# Spike recovery time — the only metric that requires sample-level
# inspection rather than ghz's pre-aggregated summary.
# ─────────────────────────────────────────────────────────────────────

def _ghz_spike_recovery_seconds(
    report: dict[str, Any],
    *,
    p99_budget_ns: float,
    spike_start_s: float = 30.0,
    window_s: float = 10.0,
    earliest_recovery_s: float | None = None,
) -> float | None:
    """Estimate the time from spike onset until p99 drops back below
    ``p99_budget_ns`` measured in a sliding ``window_s`` window of
    individual sample latencies.

    Falls back to ``None`` when ghz did not emit per-sample details
    (the ``--enable-trace`` flag is required to capture them ; if
    absent we cannot compute recovery and the SLO ``D9-LOAD-4``
    is reported as SKIPPED rather than FAIL).

    Window-size choice (audit fix L21)
    ----------------------------------
    The window must be SHORT enough that a window placed entirely
    inside the post-spike steady-state contains only post-spike
    samples (so its p99 reflects post-spike behaviour). With the
    previous default ``window_s=60`` AND a 90 s scenario, the FIRST
    valid window starts at ``t=30`` (spike onset) and spans the
    full remaining duration — that window mixes spike samples with
    recovery samples and the parser would either fail to detect
    recovery or return ``0`` (incorrect: the system has not yet
    recovered at t=30+epsilon).

    The default is now ``window_s=10`` and we walk the spike
    duration in 1 s increments. With ~64 RPS in stress mode the
    10 s window contains ~640 samples — enough for a meaningful
    p99 estimate.

    Optionally callers can pass ``earliest_recovery_s`` to require
    the window to start strictly *after* this offset (e.g. exclude
    the first 5 s of the spike during which no recovery is possible
    even with HPA scale-up).

    Performance
    -----------
    O(N²) in number of samples (N≤~3 000 for a 90 s scenario), but
    the inner loop is a list-comp + sort, executing in well under a
    second. Switch to a rolling deque if scenarios grow past 30 s.
    """
    details = _ghz_safe(report, ["details"], None)
    if not isinstance(details, list) or not details:
        return None

    # Each detail is `{timestamp: ISO-8601, latency: int_ns,
    # status: str, error?: str}`. We need timestamps relative to
    # the first sample.
    samples = []
    t0_iso = details[0].get("timestamp")
    if not t0_iso:
        return None
    try:
        t0 = _parse_iso(t0_iso)
    except ValueError:
        return None

    for d in details:
        try:
            t = _parse_iso(d.get("timestamp", ""))
        except ValueError:
            continue
        rel = (t - t0).total_seconds()
        lat = float(d.get("latency", 0))
        if lat > 0 and rel >= 0:
            samples.append((rel, lat))

    if not samples:
        return None

    # Slide a window of size `window_s`; for each window starting at
    # t ≥ spike_start_s, compute p99 over the window's latencies.
    # Recovery = first window such that ALL its samples are post-
    # spike-onset AND its p99 ≤ p99_budget_ns.
    spike_end_max = max(s[0] for s in samples)
    if spike_end_max < spike_start_s + window_s:
        # Spike scenario didn't run long enough to measure recovery
        # over even a single window post-onset.
        return None

    # The minimum time the window can start: ``spike_start_s + 1`` so
    # the window contains only post-spike samples (audit fix L21).
    min_t = max(spike_start_s + 1.0, earliest_recovery_s or 0.0)

    # Walk in 1-second steps for a deterministic, bounded answer.
    t = min_t
    while t + window_s <= spike_end_max + 1.0:
        window_lats = [lat for (rel, lat) in samples if t <= rel < t + window_s]
        if window_lats:
            window_lats.sort()
            # ceil(0.99 * n) - 1 indexes the highest sample below the
            # 1 % tail. For n=10, idx = ceil(9.9)-1 = 9 → p99 ≈ max.
            idx = max(0, math.ceil(0.99 * len(window_lats)) - 1)
            p99 = window_lats[idx]
            if p99 <= p99_budget_ns:
                return max(0.0, t - spike_start_s)
        t += 1.0
    # Did not recover within the scenario duration → return the
    # post-spike duration so the SLO clearly fails (recovery > budget).
    return spike_end_max - spike_start_s


def _parse_iso(s: str):
    """Tiny ISO-8601 → datetime helper (no dateutil dep)."""
    from datetime import datetime
    # Python ≥ 3.11 accepts trailing 'Z' in fromisoformat.
    if sys.version_info >= (3, 11):
        return datetime.fromisoformat(s.replace("Z", "+00:00"))
    # Fallback for 3.10 — strip 'Z', append +00:00.
    s = s.rstrip("Z")
    return datetime.fromisoformat(s + "+00:00")


# ─────────────────────────────────────────────────────────────────────
# Bench-format emitter
# ─────────────────────────────────────────────────────────────────────

def _to_bench_format(
    scenarios: dict[str, dict[str, Any]],
    *,
    git_sha: str,
    spike_recovery_s: float | None,
) -> dict[str, Any]:
    """Compose the **flat** eval-bench-format expected by
    ``scripts/parse_criterion_report.py --eval-bench-format``.

    The aggregator does ``results.update(eval_payload)`` directly, so
    the file must be a flat mapping
    ``{key: {mean_ns, median_ns, std_dev_ns, mad_ns, approx_p95_ns, jitter_ns}}``.
    Wrapping in a ``results`` envelope (as the criterion bench reports
    do) would break the ingestion silently.
    """
    out: dict[str, dict[str, float]] = {}

    def _entry(value_ns: float, jitter_ns: float = 0.0) -> dict[str, float]:
        v = float(value_ns)
        return {
            "mean_ns": v,
            "median_ns": v,
            "std_dev_ns": float(jitter_ns),
            "mad_ns": float(jitter_ns),
            "approx_p95_ns": v + float(jitter_ns),
            "jitter_ns": float(jitter_ns),
        }

    for scenario, data in scenarios.items():
        prefix = f"load_{scenario}::"
        for pct in (50, 95, 99):
            ns = data.get(f"p{pct}_ns")
            if ns is not None:
                out[f"{prefix}p{pct}_ns"] = _entry(ns)
        rps = data.get("throughput_rps")
        if rps is not None:
            out[f"{prefix}throughput_rps"] = _entry(rps * 1e9)
        err = data.get("error_ratio")
        if err is not None:
            out[f"{prefix}error_ratio"] = _entry(err * 1e9)
        cnt = data.get("count")
        if cnt is not None:
            out[f"{prefix}count"] = _entry(cnt)

    if spike_recovery_s is not None:
        out["load_spike::recovery_seconds"] = _entry(spike_recovery_s * 1e9)

    return out


# ─────────────────────────────────────────────────────────────────────
# SLO evaluation (mirrors scripts/parse_criterion_report.py D9-LOAD-*)
# ─────────────────────────────────────────────────────────────────────

def _eval_slos(
    bench: dict[str, Any],
    *,
    p99_budget_ms: float,
    error_budget_ratio: float,
    throughput_min_rps: float,
    recovery_budget_s: float,
) -> tuple[bool, list[dict[str, Any]]]:
    """Return ``(all_passed, [{id, name, status, …}, …])``.

    The parsing logic mirrors what `scripts/parse_criterion_report.py`
    does at the top-level; running it locally here lets the load-test
    script `exit 1` quickly without a follow-up aggregator step.

    Accepts both shapes:
      * flat eval-bench-format ``{key: {median_ns, …}}``
      * wrapped report-format ``{"results": {key: {median_ns, …}}}``
    """
    results = bench.get("results") if "results" in bench else bench

    def _val(key: str) -> float | None:
        entry = results.get(key)
        if entry is None:
            return None
        return float(entry.get("median_ns", 0))

    rows: list[dict[str, Any]] = []
    all_passed = True

    p99_budget_ns = p99_budget_ms * 1e6

    # D9-LOAD-1 — sustained p99 ≤ budget
    sustained_p99 = _val("load_sustained::p99_ns")
    rows.append(_slo_row(
        "D9-LOAD-1",
        "Sustained p99 latency",
        measured=sustained_p99,
        target=p99_budget_ns,
        comparator="lte",
        unit="ns",
    ))

    # D9-LOAD-2 — sustained error rate ≤ budget
    sustained_err = _val("load_sustained::error_ratio")
    err_target_scaled = error_budget_ratio * 1e9
    rows.append(_slo_row(
        "D9-LOAD-2",
        "Sustained error ratio",
        measured=sustained_err,
        target=err_target_scaled,
        comparator="lte",
        unit="ratio",
    ))

    # D9-LOAD-3 — stress throughput ≥ budget
    stress_rps = _val("load_stress::throughput_rps")
    sustained_rps = _val("load_sustained::throughput_rps")
    # Use the max of stress and sustained — sustained measures the
    # achievable steady-state, stress checks the saturation ceiling.
    actual_rps_scaled = max(stress_rps or 0.0, sustained_rps or 0.0)
    rows.append(_slo_row(
        "D9-LOAD-3",
        "Throughput RPS (sustained or stress)",
        measured=actual_rps_scaled if actual_rps_scaled > 0 else None,
        target=throughput_min_rps * 1e9,
        comparator="gte",
        unit="rps",
    ))

    # D9-LOAD-4 — spike recovery time ≤ budget
    recovery_ns = _val("load_spike::recovery_seconds")
    rows.append(_slo_row(
        "D9-LOAD-4",
        "Spike recovery time",
        measured=recovery_ns,
        target=recovery_budget_s * 1e9,
        comparator="lte",
        unit="s",
    ))

    for r in rows:
        if r["status"] == "FAIL":
            all_passed = False

    return all_passed, rows


def _slo_row(
    slo_id: str,
    name: str,
    *,
    measured: float | None,
    target: float,
    comparator: str,
    unit: str,
) -> dict[str, Any]:
    if measured is None:
        return {
            "id": slo_id, "name": name,
            "status": "SKIP",
            "measured": None, "target": target,
            "comparator": comparator, "unit": unit,
            "reason": "metric absent (scenario didn't run or didn't emit it)",
        }
    if comparator == "lte":
        passed = measured <= target
    elif comparator == "gte":
        passed = measured >= target
    else:
        return {
            "id": slo_id, "name": name,
            "status": "ERROR",
            "measured": measured, "target": target,
            "comparator": comparator, "unit": unit,
            "reason": f"unknown comparator {comparator!r}",
        }
    return {
        "id": slo_id, "name": name,
        "status": "PASS" if passed else "FAIL",
        "measured": measured, "target": target,
        "comparator": comparator, "unit": unit,
    }


def _format_value(v: float | None, unit: str) -> str:
    """Render a stored bench-format value back to a human-readable
    string, given an explicit ``unit``.

    Audit fix L10/L13/X3 — the previous heuristic ``v > 1e6`` to
    decide between "%" and raw fraction conflated error_ratio
    (which IS a percentage in [0, 1]) with throughput (RPS, in
    arbitrary units up to many thousands when scaled by ×1e9). The
    new version takes ``unit`` as a strict tag.

    Supported units:
      ``ns``     — value is raw nanoseconds. Render as ms.
      ``ratio``  — value is ratio × 1e9, in [0, 1e9]. Render as
                   percentage with 3 decimals.
      ``rps``    — value is RPS × 1e9. Render as RPS with 2 decimals.
      ``s``      — value is seconds × 1e9. Render as seconds with 1 decimal.
    Unknown units fall through to repr.
    """
    if v is None:
        return "—"
    if unit == "ns":
        return f"{v / 1e6:.1f} ms"
    if unit == "ratio":
        return f"{v / 1e9 * 100:.3f} %"
    if unit == "rps":
        return f"{v / 1e9:.2f} RPS"
    if unit == "s":
        return f"{v / 1e9:.1f} s"
    return f"{v}"


def _emit_summary(
    summary_path: str,
    bench: dict[str, Any],
    rows: list[dict[str, Any]],
    *,
    overall_passed: bool,
) -> None:
    """Append the human-friendly markdown summary to ``summary_path``.

    The file is appended (not overwritten) so the orchestrator's
    header (endpoint, scenario list, timestamp) is preserved.
    """
    with open(summary_path, "a", encoding="utf-8") as f:
        f.write("\n## SLO compliance — D9-LOAD-*\n\n")
        f.write("| ID | Name | Comparator | Target | Measured | Status |\n")
        f.write("|----|------|:----------:|-------:|---------:|:------:|\n")
        for r in rows:
            f.write(
                f"| `{r['id']}` "
                f"| {r['name']} "
                f"| {r['comparator']} "
                f"| {_format_value(r['target'], r['unit'])} "
                f"| {_format_value(r['measured'], r['unit'])} "
                f"| {'✅' if r['status'] == 'PASS' else '❌' if r['status'] == 'FAIL' else '⏭'} {r['status']} |\n"
            )
        f.write("\n")
        f.write(f"**Overall:** {'✅ PASS' if overall_passed else '❌ FAIL'}\n")
        f.write("\n")
        # Compact per-scenario stats.
        f.write("## Per-scenario raw stats\n\n")
        results = bench["results"] if "results" in bench else bench
        scenarios = sorted({
            k.split("::", 1)[0].removeprefix("load_")
            for k in results.keys()
            if k.startswith("load_")
        })
        for sc in scenarios:
            f.write(f"### `{sc}`\n\n")
            keys = sorted(
                k for k in results
                if k.startswith(f"load_{sc}::")
            )
            for k in keys:
                short = k.split("::", 1)[1]
                v = results[k]["median_ns"]
                if short.startswith("p") and short.endswith("_ns"):
                    f.write(f"  * **{short}**: {v / 1e6:.1f} ms\n")
                elif short == "throughput_rps":
                    f.write(f"  * **throughput**: {v / 1e9:.2f} RPS\n")
                elif short == "error_ratio":
                    f.write(f"  * **error rate**: {v / 1e9 * 100:.3f} %\n")
                elif short == "count":
                    f.write(f"  * **samples**: {int(v)}\n")
                elif short == "recovery_seconds":
                    f.write(f"  * **recovery**: {v / 1e9:.1f} s\n")
                else:
                    f.write(f"  * **{short}**: {v}\n")
            f.write("\n")


# ─────────────────────────────────────────────────────────────────────
# Disjoint-key invariant — pinned regression test (cf. supplement §12)
# ─────────────────────────────────────────────────────────────────────

ACCURACY_PREFIXES = ("eval_accuracy::", "eval_latency::")

# Sprint D10 §12.3 — extended namespace registry. Each emitter
# (parse_ghz_report.py, leak_detector.py, validate_quickstart.sh,
# docs.yml workflow) owns exactly one prefix and MUST NOT emit keys
# under a sibling's prefix. Pinned by
# scripts/tests/test_d10_disjoint_keys.py.
RESERVED_PREFIXES = (
    "eval_accuracy::",   # eval/scripts/run_eval*.py (D6/D7/D8)
    "eval_latency::",    # eval/scripts/run_eval*.py
    "load_",             # bench/load/parse_ghz_report.py (D9)
    "soak::",            # bench/soak/leak_detector.py   (D9)
    "docs::",            # .github/workflows/docs.yml    (D10)
    "dx::",              # scripts/validate_quickstart.sh (D10)
    "isolation::",       # crates/efe-server/tests/d11_isolation_* (D11)
    "cost::",            # scripts/billing/aggregate_usage.py     (D11)
    "obs::",             # Prometheus tenant cardinality probe    (D11)
    "catalog::",         # bench-catalog.yml + d12_catalog_*       (D12)
    "obs::dash::",       # bench-observability.yml + dashboards d13 (D13)
)


def assert_disjoint_keys(bench: dict[str, Any]) -> None:
    """Reject any load_*:: key that collides with the accuracy
    namespace. Pinned by tests/test_parse_ghz_report.py.

    Accepts both shapes (flat or wrapped) for forward compatibility.
    """
    keys = bench["results"] if "results" in bench else bench
    for key in keys:
        for bad in ACCURACY_PREFIXES:
            if key.startswith(bad):
                raise ValueError(
                    f"load test bench-format emitted accuracy-namespace key: {key!r}"
                )
        # Sprint D10 — each load test emitter must stay in `load_*::`.
        # We allow `load_<scenario>::<metric>` only.
        if not (key.startswith("load_") or key in ("results",)):
            raise ValueError(
                f"load test bench-format emitted out-of-namespace key: {key!r}"
            )


# ─────────────────────────────────────────────────────────────────────
# CLI
# ─────────────────────────────────────────────────────────────────────

def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    p.add_argument("--reports", nargs="+", required=True,
                   help="ghz JSON reports to ingest (one per scenario).")
    p.add_argument("--bench-out", required=True,
                   help="Output bench-format JSON path.")
    p.add_argument("--summary-out", required=True,
                   help="Output markdown summary path (appended).")
    p.add_argument("--p99-budget-ms", type=float, default=5000.0,
                   help="SLO D9-LOAD-1 budget in milliseconds.")
    p.add_argument("--error-budget-ratio", type=float, default=0.01,
                   help="SLO D9-LOAD-2 budget (ratio 0..1).")
    p.add_argument("--throughput-min-rps", type=float, default=5.0,
                   help="SLO D9-LOAD-3 minimum throughput in RPS.")
    p.add_argument("--recovery-budget-s", type=float, default=30.0,
                   help="SLO D9-LOAD-4 recovery budget in seconds.")
    p.add_argument("--git-sha", default=os.environ.get("GITHUB_SHA", "local"),
                   help="Git SHA stamp for traceability.")
    args = p.parse_args(argv)

    scenarios: dict[str, dict[str, Any]] = {}
    spike_recovery_s: float | None = None

    for path in args.reports:
        if not os.path.exists(path) or os.path.getsize(path) == 0:
            print(f"::warning::skipping empty report: {path}", file=sys.stderr)
            continue
        try:
            with open(path, encoding="utf-8") as f:
                report = json.load(f)
        except json.JSONDecodeError as exc:
            print(f"::error::malformed JSON in {path}: {exc}", file=sys.stderr)
            return 2

        scenario = _ghz_scenario_label(report)
        scenarios[scenario] = {
            "p50_ns": _ghz_p_ns(report, 50),
            "p95_ns": _ghz_p_ns(report, 95),
            "p99_ns": _ghz_p_ns(report, 99),
            "throughput_rps": _ghz_throughput_rps(report),
            "error_ratio": _ghz_error_ratio(report),
            "count": _ghz_count(report),
        }

        if scenario == "spike":
            spike_recovery_s = _ghz_spike_recovery_seconds(
                report,
                p99_budget_ns=args.p99_budget_ms * 1e6,
            )

    if not scenarios:
        print("::error::no usable ghz reports", file=sys.stderr)
        return 2

    bench = _to_bench_format(
        scenarios,
        git_sha=args.git_sha,
        spike_recovery_s=spike_recovery_s,
    )
    assert_disjoint_keys(bench)

    Path(args.bench_out).parent.mkdir(parents=True, exist_ok=True)
    with open(args.bench_out, "w", encoding="utf-8") as f:
        json.dump(bench, f, indent=2)

    # Local SLO evaluation — the aggregator does its own pass against
    # the canonical SLO_DEFINITIONS table; this local pass exists so
    # the load-test script can fail-fast in CI without a round-trip.
    overall, rows = _eval_slos(
        bench,
        p99_budget_ms=args.p99_budget_ms,
        error_budget_ratio=args.error_budget_ratio,
        throughput_min_rps=args.throughput_min_rps,
        recovery_budget_s=args.recovery_budget_s,
    )
    _emit_summary(args.summary_out, bench, rows, overall_passed=overall)

    # We DO NOT mutate the bench-format file with a wrapped envelope
    # here — the upstream aggregator (`scripts/parse_criterion_report.py`)
    # consumes a flat ``{key: {…}}`` mapping. Adding wrappers would
    # silently break ingestion (the aggregator does
    # ``results.update(json.load(f))`` straight onto the file).

    return 0 if overall else 1


if __name__ == "__main__":
    sys.exit(main())
