#!/usr/bin/env python3
"""bench/soak/leak_detector.py — Sprint D9.4 OLS leak detector.

Inputs a CSV of (t_seconds, rss_bytes) samples (typically one per
minute over 24 h, 1 440 rows) and decides whether the binary leaks
memory using a one-sided ordinary least squares regression.

Mathematical foundation
-----------------------
Given a stream ``(t_i, RSS_i)`` we fit ``RSS(t) ≈ β₀ + β₁ · t``
and test the hypothesis ``H₀: β₁ = 0`` (no leak) against
``H₁: β₁ > 0`` (leak).

The slope estimator and its standard error are::

    β̂₁ = Σ((t_i − t̄)(RSS_i − RSS̄)) / Σ((t_i − t̄)²)
    σ̂²  = Σ((RSS_i − β̂₀ − β̂₁ t_i)²) / (n − 2)
    se(β̂₁) = sqrt(σ̂² / Σ((t_i − t̄)²))

The default leak threshold ``870 B/s`` was chosen so that a
sustained leak above the bound would saturate the
``resources.limits.memory = 1Gi`` Helm default in under 14 days
(see ``docs/bench/d9_deep_analysis_supplement.md`` §2.2).

D9.A3 amendments
----------------
The deep-analysis supplement mandated three corrections over the
naïve OLS:

1. **Drop warmup window** (``--warmup-seconds 1800``, default 30 min)
   to avoid heap fragmentation polluting the slope.
2. **Median-of-10 sliding window** smoothing (``--smooth-window 10``)
   to neutralise glibc malloc-trim jitter.
3. **One-sided test** on the slope (we only care about leaks, not
   negative slopes which can occur with allocator return-to-OS).

CLI
---
::
    python3 bench/soak/leak_detector.py \
        --csv target/soak/rss.csv \
        --slope-threshold-bytes-per-sec 870 \
        --warmup-seconds 1800 \
        --smooth-window 10 \
        --report-out target/soak/leak_report.md

Exit code:
   0 — no leak detected (slope below threshold OR slope insignificant)
   1 — leak confirmed (slope above threshold AND statistically significant)
   2 — operational failure (CSV malformed, too few samples, …)
"""

from __future__ import annotations

import argparse
import csv
import math
import os
import statistics
import sys
from pathlib import Path
from typing import Any


# ─────────────────────────────────────────────────────────────────────
# Statistics helpers (stdlib-only, no numpy)
# ─────────────────────────────────────────────────────────────────────

def ols_slope_and_se(t: list[float], y: list[float]) -> tuple[float, float, float, int]:
    """Return ``(β₀, β̂₁, se(β̂₁), df)``.

    Uses the formulae from §1 of the file docstring. Raises ``ValueError``
    when ``n < 3`` (we need at least 3 points for a meaningful regression
    and a non-zero residual variance estimate).
    """
    n = len(t)
    if n < 3 or len(y) != n:
        raise ValueError(
            f"OLS requires n ≥ 3 paired samples (got n={n}, y={len(y)})"
        )

    t_bar = sum(t) / n
    y_bar = sum(y) / n
    sxx = sum((ti - t_bar) ** 2 for ti in t)
    if sxx == 0.0:
        raise ValueError("OLS denominator is zero (all timestamps identical?)")

    sxy = sum((t[i] - t_bar) * (y[i] - y_bar) for i in range(n))
    beta1 = sxy / sxx
    beta0 = y_bar - beta1 * t_bar

    # Residuals + unbiased σ̂² estimate
    rss = sum((y[i] - (beta0 + beta1 * t[i])) ** 2 for i in range(n))
    df = n - 2
    sigma2 = rss / df if df > 0 else 0.0
    se_beta1 = math.sqrt(sigma2 / sxx) if sxx > 0 else float("inf")

    return beta0, beta1, se_beta1, df


def median_of_window(values: list[float], window_size: int) -> list[float]:
    """Return a list of medians, one per non-overlapping window of
    ``window_size`` elements. Used to smooth glibc-trim jitter.
    The trailing partial window (if any) is dropped to keep the
    output index/value alignment unambiguous.
    """
    if window_size <= 1:
        return list(values)
    out: list[float] = []
    for i in range(0, len(values) - window_size + 1, window_size):
        out.append(statistics.median(values[i : i + window_size]))
    return out


# ─────────────────────────────────────────────────────────────────────
# Sample loading
# ─────────────────────────────────────────────────────────────────────

def load_csv(path: str) -> tuple[list[float], list[float]]:
    """Load a 2-column CSV ``(t_seconds, rss_bytes)``.

    Lines starting with ``#`` are treated as comments. The header
    line, if present, is detected by attempting to parse the first
    cell as a float (fail → header).
    """
    t_values: list[float] = []
    y_values: list[float] = []
    with open(path, newline="", encoding="utf-8") as f:
        reader = csv.reader(f)
        for i, row in enumerate(reader):
            if not row:
                continue
            if row[0].startswith("#"):
                continue
            try:
                t = float(row[0])
                y = float(row[1])
            except (ValueError, IndexError):
                if i == 0:
                    # First row, can't parse → header. Skip silently.
                    continue
                raise ValueError(f"row {i}: cannot parse {row!r}")
            t_values.append(t)
            y_values.append(y)
    return t_values, y_values


def apply_warmup(
    t: list[float], y: list[float], *, warmup_s: float
) -> tuple[list[float], list[float]]:
    """Drop samples with ``t_i < warmup_s``."""
    out_t, out_y = [], []
    for ti, yi in zip(t, y, strict=True):
        if ti >= warmup_s:
            out_t.append(ti)
            out_y.append(yi)
    return out_t, out_y


# ─────────────────────────────────────────────────────────────────────
# Decision logic
# ─────────────────────────────────────────────────────────────────────

def decide(
    *,
    slope_bps: float,
    se_bps: float,
    df: int,
    threshold_bps: float,
    significance_t_value: float = 1.96,
) -> dict[str, Any]:
    """Decide PASS / FAIL given the slope estimate, its SE, and the
    threshold. The decision is FAIL only if **both**:

    * ``slope > threshold`` (effect size meets the operational bar)
    * ``slope / se > significance_t_value`` (effect is statistically
      significant). Default cutoff t = 1.96 ≈ 95 % one-sided.

    A slope that exceeds the threshold but is statistically
    indistinguishable from zero (e.g. tiny n) → SUSPECT (status
    "INCONCLUSIVE", exit code 2 from the caller's perspective).
    """
    if df < 1:
        return {"status": "INCONCLUSIVE", "reason": "insufficient samples"}
    t_obs = slope_bps / se_bps if se_bps > 0 else float("inf")
    significant = t_obs >= significance_t_value
    over_threshold = slope_bps >= threshold_bps
    if over_threshold and significant:
        return {
            "status": "FAIL",
            "t_obs": t_obs,
            "reason": (
                f"slope {slope_bps:.1f} B/s ≥ threshold {threshold_bps:.1f} "
                f"AND |t|={t_obs:.2f} ≥ {significance_t_value:.2f}"
            ),
        }
    if over_threshold and not significant:
        return {
            "status": "INCONCLUSIVE",
            "t_obs": t_obs,
            "reason": (
                f"slope {slope_bps:.1f} B/s ≥ threshold {threshold_bps:.1f} "
                f"BUT |t|={t_obs:.2f} < {significance_t_value:.2f} "
                "(possible noise)"
            ),
        }
    return {
        "status": "PASS",
        "t_obs": t_obs,
        "reason": (
            f"slope {slope_bps:.1f} B/s < threshold {threshold_bps:.1f}"
            if not over_threshold
            else f"slope significant but below threshold"
        ),
    }


# ─────────────────────────────────────────────────────────────────────
# Markdown report
# ─────────────────────────────────────────────────────────────────────

def render_report(
    *,
    raw_n: int,
    used_n: int,
    warmup_s: float,
    smooth_window: int,
    beta0: float,
    beta1: float,
    se_beta1: float,
    df: int,
    threshold_bps: float,
    decision: dict[str, Any],
    duration_total_s: float,
    projected_growth_24h_bytes: float,
    projected_growth_7d_bytes: float,
) -> str:
    return f"""# D9.4 Soak leak report

**Decision:** {decision['status']}
**Reason:** {decision['reason']}

## Sample summary

* Raw samples loaded: {raw_n}
* Samples after warmup (t ≥ {warmup_s:.0f} s) and smoothing
  (window={smooth_window}): {used_n}
* Soak total wallclock duration: {duration_total_s / 3600:.1f} h
  ({duration_total_s:.0f} s)
* Regression degrees of freedom: {df}

## OLS fit

* β₀ (intercept): {beta0:.0f} bytes
* **β̂₁ (slope): {beta1:.2f} B/s**
* SE(β̂₁): {se_beta1:.2f} B/s
* t-statistic: {decision.get('t_obs', float('nan')):.2f} (one-sided, H₀: β₁=0)

## Threshold

* Operational threshold: **{threshold_bps:.0f} B/s**
* Rationale: above this, RSS grows faster than the
  ``resources.limits.memory=1Gi`` budget can absorb in < 14 days
  (cf. d9_deep_analysis_supplement §2.2).

## Projection

If the observed slope persists:

* +24 h growth: ~{projected_growth_24h_bytes / 1024 / 1024:.1f} MiB
* +7 d  growth: ~{projected_growth_7d_bytes / 1024 / 1024:.1f} MiB

A NEGATIVE projected growth means the allocator is RELEASING memory
back to the OS faster than it acquires it (typically a transient
post-spike state) — not a leak.

## Decision rule

PASS  ⇔ slope < threshold
        OR  |t-stat| < 1.96 (slope statistically indistinguishable from 0)

FAIL  ⇔ slope ≥ threshold AND |t-stat| ≥ 1.96

INCONCLUSIVE ⇔ slope ≥ threshold AND |t-stat| < 1.96
              (re-run with longer soak or investigate noise sources)
"""


# ─────────────────────────────────────────────────────────────────────
# Bench-format emission for the SLO aggregator
# ─────────────────────────────────────────────────────────────────────

def to_bench_format(
    *,
    beta1: float,
    se_beta1: float,
    used_n: int,
    decision: dict[str, Any],
) -> dict[str, Any]:
    """Emit a **flat** eval-bench-format payload, one entry per soak metric.

    Schema matches what ``scripts/parse_criterion_report.py`` ingests
    via ``--eval-bench-format`` (a flat ``{key: bench_entry}`` mapping
    where ``bench_entry`` carries ``mean_ns``/``median_ns``/``jitter_ns``).
    Wrapping in a ``{"results": …}`` envelope would silently break
    ingestion.

    Keys:
      ``soak::rss_slope_bps`` — observed slope in B/s, scaled ×1e9
                                 so the SLO comparator (``lte``) is
                                 ratio-aware.
      ``soak::sample_count``  — used sample count (post-smoothing).
      ``soak::leak_status``    — 0 = PASS, 1 = INCONCLUSIVE,
                                 2 = FAIL (only useful for triage).
    """
    status_code = {"PASS": 0, "INCONCLUSIVE": 1, "FAIL": 2}.get(
        decision["status"], 2
    )

    def _entry(value: float, jitter: float = 0.0) -> dict[str, float]:
        v = float(value)
        return {
            "mean_ns": v,
            "median_ns": v,
            "std_dev_ns": float(jitter),
            "mad_ns": float(jitter),
            "approx_p95_ns": v + float(jitter),
            "jitter_ns": float(jitter),
        }

    return {
        "soak::rss_slope_bps": _entry(beta1 * 1e9, se_beta1 * 1e9),
        "soak::sample_count": _entry(used_n),
        "soak::leak_status": _entry(status_code),
    }


# ─────────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────────

def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    p.add_argument("--csv", required=True,
                   help="2-col CSV (t_seconds, rss_bytes).")
    p.add_argument("--slope-threshold-bytes-per-sec", type=float, default=870.0,
                   dest="threshold_bps",
                   help="FAIL when slope ≥ this AND statistically significant.")
    p.add_argument("--warmup-seconds", type=float, default=1800.0,
                   help="Drop samples with t < this (default 30 min).")
    p.add_argument("--smooth-window", type=int, default=10,
                   help="Median-of-N smoothing window (1 = disabled).")
    p.add_argument("--significance-t-value", type=float, default=1.96,
                   help="Decision threshold on |t-statistic| (default 1.96).")
    p.add_argument("--report-out", default=None,
                   help="Optional: path to write the markdown report.")
    p.add_argument("--bench-out", default=None,
                   help="Optional: path to write the bench-format JSON.")
    args = p.parse_args(argv)

    # 1. Load CSV
    try:
        t_raw, y_raw = load_csv(args.csv)
    except FileNotFoundError:
        print(f"::error::CSV not found: {args.csv}", file=sys.stderr)
        return 2
    except ValueError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 2

    raw_n = len(t_raw)
    if raw_n < 30:
        print(f"::error::too few samples ({raw_n} < 30)", file=sys.stderr)
        return 2

    duration_total_s = max(t_raw) - min(t_raw)

    # 2. Drop warmup
    t_post, y_post = apply_warmup(t_raw, y_raw, warmup_s=args.warmup_seconds)
    if len(t_post) < 10:
        print(
            f"::error::too few samples post-warmup ({len(t_post)} < 10)",
            file=sys.stderr,
        )
        return 2

    # 3. Smooth
    if args.smooth_window > 1:
        # We must keep the t and y arrays aligned, so smooth them
        # together by collapsing each non-overlapping window to its
        # median.
        t_used: list[float] = []
        y_used: list[float] = []
        for i in range(0, len(t_post) - args.smooth_window + 1, args.smooth_window):
            t_used.append(statistics.median(t_post[i : i + args.smooth_window]))
            y_used.append(statistics.median(y_post[i : i + args.smooth_window]))
    else:
        t_used = t_post
        y_used = y_post

    used_n = len(t_used)
    if used_n < 3:
        print(
            f"::error::too few samples post-smoothing ({used_n} < 3)",
            file=sys.stderr,
        )
        return 2

    # 4. OLS fit
    beta0, beta1, se_beta1, df = ols_slope_and_se(t_used, y_used)

    # 5. Decide
    decision = decide(
        slope_bps=beta1,
        se_bps=se_beta1,
        df=df,
        threshold_bps=args.threshold_bps,
        significance_t_value=args.significance_t_value,
    )

    # 6. Emit report
    proj_24h = beta1 * 24 * 3600
    proj_7d = beta1 * 7 * 24 * 3600
    if args.report_out:
        Path(args.report_out).parent.mkdir(parents=True, exist_ok=True)
        report = render_report(
            raw_n=raw_n, used_n=used_n,
            warmup_s=args.warmup_seconds,
            smooth_window=args.smooth_window,
            beta0=beta0, beta1=beta1, se_beta1=se_beta1, df=df,
            threshold_bps=args.threshold_bps,
            decision=decision,
            duration_total_s=duration_total_s,
            projected_growth_24h_bytes=proj_24h,
            projected_growth_7d_bytes=proj_7d,
        )
        with open(args.report_out, "w", encoding="utf-8") as f:
            f.write(report)

    # 7. Emit bench-format for SLO ingestion
    if args.bench_out:
        import json
        Path(args.bench_out).parent.mkdir(parents=True, exist_ok=True)
        bench = to_bench_format(
            beta1=beta1,
            se_beta1=se_beta1,
            used_n=used_n,
            decision=decision,
        )
        with open(args.bench_out, "w", encoding="utf-8") as f:
            json.dump(bench, f, indent=2)

    # 8. Stdout summary (always, for CI logs)
    print(
        f"soak leak detector: status={decision['status']} "
        f"slope={beta1:.2f} B/s  se={se_beta1:.2f}  "
        f"t={decision.get('t_obs', 0.0):.2f}  threshold={args.threshold_bps:.0f}"
    )
    print(f"reason: {decision['reason']}")

    return {"PASS": 0, "INCONCLUSIVE": 2, "FAIL": 1}.get(decision["status"], 2)


if __name__ == "__main__":
    sys.exit(main())
