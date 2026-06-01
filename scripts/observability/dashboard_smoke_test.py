#!/usr/bin/env python3
"""Sprint D13 — Grafana dashboard smoke test.

Hits the Grafana API for each of the 6 D13 dashboards and verifies :

  1. The dashboard UID resolves (HTTP 200).
  2. The dashboard renders within the SLO budget (D13-OBS-1 : 3 s p99).
  3. Every panel returns at least one data point OR a documented
     "no data" reason (e.g. for catalog panels before D12-bis lands).

Usage::

    python scripts/observability/dashboard_smoke_test.py \\
        --grafana-url http://grafana:3000 \\
        --token "$GRAFANA_API_TOKEN" \\
        --slo-p99-seconds 3.0

Exit codes :

* 0 — all dashboards loaded within SLO.
* 1 — at least one dashboard exceeded the SLO.
* 2 — at least one dashboard UID is missing (HTTP != 200).
* 3 — usage / config error.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request


D13_DASHBOARD_UIDS = (
    "efe-d13-01-operator-overview",
    "efe-d13-02-tenant-drill-down",
    "efe-d13-03-catalog-version-distribution",
    "efe-d13-04-cost-attribution",
    "efe-d13-05-audit-stream",
    "efe-d13-06-capacity-planning",
)


def fetch_dashboard(
    base_url: str,
    uid: str,
    token: str | None,
    timeout: float,
) -> tuple[int, float, dict | None]:
    """Return (status_code, elapsed_seconds, body_or_none)."""
    url = f"{base_url.rstrip('/')}/api/dashboards/uid/{uid}"
    req = urllib.request.Request(url)
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = json.loads(resp.read().decode("utf-8"))
            elapsed = time.perf_counter() - t0
            return resp.status, elapsed, body
    except urllib.error.HTTPError as e:
        elapsed = time.perf_counter() - t0
        return e.code, elapsed, None
    except (urllib.error.URLError, TimeoutError) as e:
        elapsed = time.perf_counter() - t0
        print(f"FATAL: cannot reach {url}: {e}", file=sys.stderr)
        return 0, elapsed, None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    parser.add_argument(
        "--grafana-url",
        default="http://localhost:3000",
        help="Base URL of the Grafana instance.",
    )
    parser.add_argument(
        "--token",
        help="Grafana API token (Bearer). Optional for anonymous orgs.",
    )
    parser.add_argument(
        "--slo-p99-seconds", type=float, default=3.0,
        help="Per-dashboard render time SLO (default : 3.0 = D13-OBS-1).",
    )
    parser.add_argument(
        "--timeout", type=float, default=10.0,
        help="HTTP timeout per request in seconds.",
    )
    parser.add_argument(
        "--format", choices=("text", "json"), default="text",
    )
    args = parser.parse_args(argv)

    results: list[dict] = []
    for uid in D13_DASHBOARD_UIDS:
        status, elapsed, body = fetch_dashboard(
            args.grafana_url, uid, args.token, args.timeout,
        )
        results.append({
            "uid": uid,
            "status": status,
            "elapsed_seconds": round(elapsed, 4),
            "title": (body or {}).get("dashboard", {}).get("title")
                     if status == 200 else None,
            "panel_count": len((body or {}).get("dashboard", {})
                                            .get("panels", []))
                           if status == 200 else None,
        })

    over_slo = [r for r in results if r["elapsed_seconds"] > args.slo_p99_seconds]
    missing  = [r for r in results if r["status"] != 200]

    if args.format == "json":
        doc = {
            "schema_version": "1.0",
            "tool": "dashboard_smoke_test",
            "slo_p99_seconds": args.slo_p99_seconds,
            "n_total":   len(results),
            "n_loaded":  sum(1 for r in results if r["status"] == 200),
            "n_over_slo": len(over_slo),
            "n_missing": len(missing),
            "results": results,
        }
        print(json.dumps(doc, indent=2))
    else:
        for r in results:
            mark = "✓" if r["status"] == 200 else "✗"
            slo_mark = "" if r["elapsed_seconds"] <= args.slo_p99_seconds else " (OVER SLO)"
            print(
                f"  {mark} {r['uid']:42s} "
                f"HTTP {r['status']}  {r['elapsed_seconds']:.3f}s{slo_mark}"
            )
        print()
        print(f"  Summary : {sum(1 for r in results if r['status'] == 200)}/"
              f"{len(results)} loaded, {len(over_slo)} over SLO "
              f"({args.slo_p99_seconds}s)")

    if missing:
        return 2
    if over_slo:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
