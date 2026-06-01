#!/usr/bin/env python3
"""Sprint D13 — Prometheus cardinality probe.

Scrape an `efe-server`'s ``/metrics`` endpoint, parse the
OpenMetrics text format, and report cardinality breakdown by
metric family. Used by the runbook
``docs/runbooks/d13_obs_cardinality_bomb.md`` for triage.

Usage::

    python scripts/observability/cardinality_probe.py \\
        --url http://efe-server:9090/metrics \\
        --top 20 \\
        --panic-budget 50000 \\
        --format json

Exit codes :

* 0 — within budget.
* 1 — over budget (alertable).
* 2 — usage / fetch error.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from collections import defaultdict
from typing import TextIO


# OpenMetrics line :
#   ``efe_grpc_requests_total{method="X",code="OK",tenant="acme"} 42``
# We just need the metric name (before `{`) for cardinality counting.
# Comments + HELP/TYPE lines start with `#`.
_LINE_RE = re.compile(r"^(?P<name>[a-zA-Z_:][a-zA-Z0-9_:]*)(\{[^}]*\})?\s+\S+")


def fetch_metrics(url: str, timeout: float) -> str:
    """Fetch /metrics via HTTP. Bare-minimum stdlib, no requests dep."""
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            return resp.read().decode("utf-8", errors="replace")
    except (urllib.error.URLError, TimeoutError) as exc:
        raise SystemExit(f"FATAL: cannot fetch {url}: {exc}") from exc


def parse_cardinality(metrics_text: str) -> dict[str, int]:
    """Return {family_name: distinct_series_count}.

    Each unique label-set under a family name = 1 series.
    """
    series_per_family: dict[str, set[str]] = defaultdict(set)
    for raw in metrics_text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        m = _LINE_RE.match(line)
        if not m:
            continue
        name = m.group("name")
        # The full identifier is `name{labels}` ; the labels part
        # disambiguates series within a family.
        ident_end = line.find(" ")
        if ident_end == -1:
            continue
        ident = line[:ident_end]
        series_per_family[name].add(ident)
    return {k: len(v) for k, v in series_per_family.items()}


def filter_efe(cardinality: dict[str, int]) -> dict[str, int]:
    return {k: v for k, v in cardinality.items() if k.startswith(("efe_", "efe:"))}


def report_text(
    cardinality: dict[str, int],
    *,
    top: int,
    panic_budget: int,
    out: TextIO,
) -> int:
    total = sum(cardinality.values())
    over_budget = total > panic_budget
    out.write(f"Total efe_* / efe:* series : {total}\n")
    out.write(f"Panic budget               : {panic_budget}\n")
    out.write(f"Status                     : "
              f"{'OVER BUDGET' if over_budget else 'OK'}\n\n")
    out.write(f"Top {top} families :\n")
    sorted_families = sorted(cardinality.items(), key=lambda kv: -kv[1])
    for name, count in sorted_families[:top]:
        bar_width = int(40 * count / max(1, sorted_families[0][1]))
        bar = "█" * bar_width
        out.write(f"  {count:>6}  {bar}  {name}\n")
    if over_budget:
        return 1
    return 0


def report_json(
    cardinality: dict[str, int],
    *,
    panic_budget: int,
    out: TextIO,
) -> int:
    total = sum(cardinality.values())
    over_budget = total > panic_budget
    doc = {
        "schema_version": "1.0",
        "tool": "cardinality_probe",
        "total_series": total,
        "panic_budget": panic_budget,
        "over_budget": over_budget,
        "families": dict(sorted(cardinality.items(),
                                key=lambda kv: -kv[1])),
    }
    out.write(json.dumps(doc, indent=2))
    out.write("\n")
    return 1 if over_budget else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    parser.add_argument(
        "--url",
        default="http://localhost:9090/metrics",
        help="/metrics endpoint URL (default : http://localhost:9090/metrics)",
    )
    parser.add_argument(
        "--input-file",
        help="Read metrics from a file instead of fetching via HTTP "
             "(useful for offline analysis ; mutually exclusive with --url).",
    )
    parser.add_argument(
        "--top", type=int, default=20,
        help="Show top N families by cardinality (default : 20).",
    )
    parser.add_argument(
        "--panic-budget", type=int, default=50_000,
        help="Total cardinality threshold ; non-zero exit if exceeded "
             "(default : 50 000, matches alert D13ObsCardinalityBomb).",
    )
    parser.add_argument(
        "--format", choices=("text", "json"), default="text",
        help="Output format (default : text).",
    )
    parser.add_argument(
        "--timeout", type=float, default=5.0,
        help="HTTP timeout in seconds (default : 5.0).",
    )
    args = parser.parse_args(argv)

    if args.input_file:
        with open(args.input_file, "r", encoding="utf-8") as f:
            metrics_text = f.read()
    else:
        metrics_text = fetch_metrics(args.url, args.timeout)

    cardinality = filter_efe(parse_cardinality(metrics_text))

    if args.format == "json":
        return report_json(
            cardinality,
            panic_budget=args.panic_budget,
            out=sys.stdout,
        )
    return report_text(
        cardinality,
        top=args.top,
        panic_budget=args.panic_budget,
        out=sys.stdout,
    )


if __name__ == "__main__":
    raise SystemExit(main())
