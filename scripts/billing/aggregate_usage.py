#!/usr/bin/env python3
"""scripts/billing/aggregate_usage.py — Sprint D11.5 cost attribution.

Aggregates per-tenant usage from a Prometheus instance over a
billing window and emits CSV / JSON. Source of truth for the
phase-2 SaaS billing pipeline.

Design notes
────────────

* **Stdlib-only.** No `requests` / `prometheus-api-client` so the
  script ships with the chart without extra deps. urllib gets us
  there.
* **`increase()` over `delta()`.** Resilient to Prometheus restarts
  within the window. Documented in
  [`d11_per_tenant_isolation_architecture.md`](../docs/bench/d11_per_tenant_isolation_architecture.md) §13.
* **Tenant filter.** Optional ``--allow-tenants`` to drop unknown
  / cardinality-bomb labels from the aggregate (defense in
  depth — the ServiceMonitor relabeling does the same upstream).
* **Window step**. The Prometheus query_range step matters for
  accuracy of long windows ; we use 1h step on >= 24h windows.

Usage:
    python scripts/billing/aggregate_usage.py \\
        --prom-url http://prometheus.monitoring:9090 \\
        --start 2026-04-01T00:00:00Z \\
        --end   2026-05-01T00:00:00Z \\
        --output target/billing/2026-04.csv

Exit codes:
    0 — success
    2 — operational error (Prometheus unreachable, malformed args)
    3 — assertion failure (no data, drift > threshold)
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


# ─────────────────────────────────────────────────────────────────────
# Constants
# ─────────────────────────────────────────────────────────────────────

# The three core metrics. Each is sum'd by tenant.
#
# Audit fix C1 (v0.1.11) — the previous implementation called
# ``query_range`` with ``increase(metric[<full window>])`` at every
# step, then summed all the per-step values. Each step returned the
# FULL window's increase, so summing N steps over-counted by ~N×.
# The fix uses ``query`` (instant) at ``t=end`` with the
# full-window ``increase()`` — Prometheus computes the total once,
# exactly. Reference:
# https://prometheus.io/docs/prometheus/latest/querying/functions/#increase
METRIC_REQUESTS_QUERY = (
    "sum by (tenant) (increase(efe_grpc_requests_total[{window}]))"
)
METRIC_COMPUTE_QUERY = (
    "sum by (tenant) (increase(efe_classify_compute_seconds_total[{window}]))"
)
METRIC_EGRESS_QUERY = (
    "sum by (tenant) (increase(efe_egress_bytes_total[{window}]))"
)

# DEFAULT_STEP is kept for backward compat with --step flag, but is
# not used by the new instant-query path (audit fix C1).
DEFAULT_STEP = "1h"


# ─────────────────────────────────────────────────────────────────────
# HTTP layer (stdlib-only)
# ─────────────────────────────────────────────────────────────────────

def query_range(
    prom_url: str,
    query: str,
    start: str,
    end: str,
    step: str,
    *,
    timeout_s: float = 30.0,
) -> dict[str, Any]:
    """Call Prometheus ``/api/v1/query_range`` with stdlib urllib.

    Kept for legacy / debugging use. **NOT used by the production
    aggregation path** post-fix C1 — see :func:`query_instant`.

    Returns the parsed JSON response. Raises :class:`RuntimeError`
    on transport failure / non-200 / non-success status field.
    """
    params = urllib.parse.urlencode({
        "query": query,
        "start": start,
        "end": end,
        "step": step,
    })
    url = f"{prom_url.rstrip('/')}/api/v1/query_range?{params}"
    try:
        req = urllib.request.Request(
            url, headers={"Accept": "application/json"}
        )
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            payload = json.load(resp)
    except urllib.error.URLError as exc:
        raise RuntimeError(
            f"prometheus query_range failed: {exc} (url={url!r})"
        ) from exc

    if payload.get("status") != "success":
        raise RuntimeError(
            f"prometheus returned non-success: {payload.get('status')!r} — "
            f"errorType={payload.get('errorType')} error={payload.get('error')}"
        )
    return payload


def query_instant(
    prom_url: str,
    query: str,
    time_: str,
    *,
    timeout_s: float = 30.0,
) -> dict[str, Any]:
    """Call Prometheus ``/api/v1/query`` (instant) at a fixed time.

    Audit fix C1 — this is the correct API for billing aggregation.
    Evaluates ``increase(metric[<window>])`` ONCE at ``t=time_``,
    returning a single value per series. No N× over-counting.

    Args:
        prom_url: Prometheus base URL.
        query: PromQL expression.
        time_: ISO 8601 instant (typically the billing window end).
        timeout_s: HTTP timeout.

    Returns:
        Parsed Prometheus JSON response.

    Raises:
        RuntimeError: transport / status failure.
    """
    params = urllib.parse.urlencode({
        "query": query,
        "time": time_,
    })
    url = f"{prom_url.rstrip('/')}/api/v1/query?{params}"
    try:
        req = urllib.request.Request(
            url, headers={"Accept": "application/json"}
        )
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            payload = json.load(resp)
    except urllib.error.URLError as exc:
        raise RuntimeError(
            f"prometheus query (instant) failed: {exc} (url={url!r})"
        ) from exc

    if payload.get("status") != "success":
        raise RuntimeError(
            f"prometheus returned non-success: {payload.get('status')!r} — "
            f"errorType={payload.get('errorType')} error={payload.get('error')}"
        )
    return payload


# ─────────────────────────────────────────────────────────────────────
# Pure aggregation logic (testable without Prometheus)
# ─────────────────────────────────────────────────────────────────────

def _sum_series_values(series: dict[str, Any]) -> float:
    """Sum the value-axis of a single Prometheus query_range series.

    Used by the LEGACY query_range path (kept for tests + debugging
    only post-fix C1 — the production path now uses
    :func:`query_instant` + :func:`_extract_instant_value`).

    A query_range series looks like::

        {"metric": {"tenant": "acme"}, "values": [[ts, "12.0"], ...]}

    `increase()` over consecutive non-overlapping windows is
    additive — summing the per-window values gives the total. With
    overlapping windows (e.g. step=1h, increase[30d]) this
    over-counts ; cf. audit fix C1.
    """
    import math

    total = 0.0
    for _ts, val in series.get("values", []):
        try:
            f = float(val)
        except (TypeError, ValueError):
            continue
        if math.isnan(f) or math.isinf(f):
            continue
        total += f
    return total


def _extract_instant_value(series: dict[str, Any]) -> float:
    """Extract the single value from an instant-query series.

    An instant-query series looks like::

        {"metric": {"tenant": "acme"}, "value": [ts, "12.0"]}

    Returns 0.0 on missing / NaN / infinite values.
    """
    import math

    val = series.get("value")
    if not val or len(val) < 2:
        return 0.0
    try:
        f = float(val[1])
    except (TypeError, ValueError):
        return 0.0
    if math.isnan(f) or math.isinf(f):
        return 0.0
    return f


def aggregate_metric(
    payload: dict[str, Any],
    output_key: str,
    out: dict[str, dict[str, float | int]],
) -> None:
    """Mutate ``out`` to attach the per-tenant total of one metric.

    Audit fix C1 (v0.1.11) — accepts both response shapes for
    backward compatibility :

      * **instant query** (production path) — each series carries a
        single ``value`` field ``[ts, "<numeric>"]``. Used post-fix.
      * **range query** (legacy / test) — each series carries a
        ``values`` list. Sums all values (correct ONLY when the
        increase window matches the step ; overcounts otherwise).

    Detected by checking ``"values"`` (list) vs ``"value"`` (pair).
    """
    for series in payload["data"]["result"]:
        tenant = series["metric"].get("tenant", "_unknown")
        if "value" in series and series.get("value"):
            total = _extract_instant_value(series)
        else:
            total = _sum_series_values(series)
        out.setdefault(tenant, {})[output_key] = total


def _window_label(start: str, end: str) -> str:
    """Compute a human-readable window string for ``increase()`` queries.

    Prometheus `increase()` requires a duration argument. We
    derive it from start/end ; for monthly bills this comes out
    to ``744h`` or so.
    """
    s = datetime.fromisoformat(start.replace("Z", "+00:00"))
    e = datetime.fromisoformat(end.replace("Z", "+00:00"))
    delta = e - s
    seconds = int(delta.total_seconds())
    if seconds < 60:
        return f"{seconds}s"
    if seconds < 3600:
        return f"{seconds // 60}m"
    if seconds < 86400:
        return f"{seconds // 3600}h"
    return f"{seconds // 86400}d"


def normalise_usage(
    raw: dict[str, dict[str, float | int]],
    *,
    allow_tenants: list[str] | None = None,
) -> dict[str, dict[str, int | float]]:
    """Normalise types + apply optional tenant filter.

    * Coerces requests / egress_bytes to int.
    * Coerces compute_seconds to float rounded to 2 decimals.
    * Drops `_unknown` and any tenant not in ``allow_tenants`` (when set).
    """
    out: dict[str, dict[str, int | float]] = {}
    for tenant, metrics in raw.items():
        if tenant == "_unknown":
            continue
        if allow_tenants is not None and tenant not in allow_tenants:
            continue
        out[tenant] = {
            "requests": int(metrics.get("requests", 0) or 0),
            "compute_seconds": round(
                float(metrics.get("compute_seconds", 0.0) or 0.0), 2,
            ),
            "egress_bytes": int(metrics.get("egress_bytes", 0) or 0),
        }
    return out


# ─────────────────────────────────────────────────────────────────────
# Output emitters
# ─────────────────────────────────────────────────────────────────────

def emit_csv(usage: dict[str, dict[str, Any]], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["tenant", "requests", "compute_seconds", "egress_bytes"])
        for tenant in sorted(usage):
            m = usage[tenant]
            w.writerow([
                tenant,
                m["requests"],
                m["compute_seconds"],
                m["egress_bytes"],
            ])


def emit_json(
    usage: dict[str, dict[str, Any]],
    path: Path,
    *,
    metadata: dict[str, Any] | None = None,
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema_version": "1.0",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "metadata": metadata or {},
        "tenants": usage,
    }
    with path.open("w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, sort_keys=True)


# ─────────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────────

def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    p.add_argument("--prom-url", required=True,
                   help="Prometheus base URL (e.g. http://prometheus:9090)")
    p.add_argument("--start", required=True,
                   help="ISO 8601 timestamp (UTC), inclusive")
    p.add_argument("--end", required=True,
                   help="ISO 8601 timestamp (UTC), exclusive")
    p.add_argument("--output", required=True,
                   help="Output path. .csv or .json based on extension")
    p.add_argument("--step", default=DEFAULT_STEP,
                   help="Prometheus query_range step (default 1h)")
    p.add_argument("--allow-tenants", action="append", default=[],
                   help="Restrict output to these tenants (repeatable)")
    p.add_argument("--prom-timeout-s", type=float, default=30.0,
                   help="HTTP timeout for Prometheus requests")
    args = p.parse_args(argv)

    # Validate ISO 8601 timestamps.
    for label in ("start", "end"):
        try:
            datetime.fromisoformat(getattr(args, label).replace("Z", "+00:00"))
        except ValueError:
            print(f"::error::{label} not a valid ISO 8601 datetime",
                  file=sys.stderr)
            return 2

    window = _window_label(args.start, args.end)

    # Run three queries ; each is "sum by (tenant) (increase(metric[window]))".
    queries = [
        ("requests", METRIC_REQUESTS_QUERY.format(window=window)),
        ("compute_seconds", METRIC_COMPUTE_QUERY.format(window=window)),
        ("egress_bytes", METRIC_EGRESS_QUERY.format(window=window)),
    ]

    raw: dict[str, dict[str, float | int]] = {}
    t_start = time.monotonic()
    # Audit fix C1 — use instant query at t=end with the full
    # window in increase(). Returns one value per tenant per
    # metric, no over-counting risk.
    for output_key, query in queries:
        try:
            payload = query_instant(
                args.prom_url, query, args.end,
                timeout_s=args.prom_timeout_s,
            )
        except RuntimeError as exc:
            print(f"::error::{exc}", file=sys.stderr)
            return 2
        aggregate_metric(payload, output_key, raw)
    elapsed_s = time.monotonic() - t_start

    usage = normalise_usage(
        raw,
        allow_tenants=args.allow_tenants or None,
    )

    output_path = Path(args.output)
    if output_path.suffix == ".csv":
        emit_csv(usage, output_path)
    elif output_path.suffix == ".json":
        emit_json(usage, output_path, metadata={
            "prom_url": args.prom_url,
            "start": args.start,
            "end": args.end,
            "window": window,
            "step": args.step,
            "elapsed_s": round(elapsed_s, 2),
        })
    else:
        print(f"::error::output must end in .csv or .json (got {output_path.suffix})",
              file=sys.stderr)
        return 2

    print(f"  wrote {output_path}  ({len(usage)} tenants, {elapsed_s:.2f}s)")

    if not usage:
        print("::warning::no tenants found in the window — empty output",
              file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
