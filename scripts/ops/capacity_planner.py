#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D98) — Capacity planner.

Reads a historical traffic CSV (= timestamp × RPS) and projects
N-week-out capacity needs assuming :

   * Compound growth `g` per week (default 1.15 = +15 %/wk).
   * Headroom factor `h` over forecast peak (default 1.30 = +30 %).
   * Per-pod throughput envelope `pod_rps` (default 80 RPS).

Outputs a sized resource plan (pods, GPU count, PG IOPS, Redis nodes)
and a forecast curve consumable by Grafana.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 12.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import math
import statistics
import sys
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import List, Optional


@dataclass
class HistoryPoint:
    timestamp: str
    rps: float


@dataclass
class ForecastWeek:
    week_index: int
    iso_date: str
    forecast_peak_rps: float
    pods_required: int
    gpu_count_min: int
    pg_iops_required: int
    redis_node_count: int


def _read_history(path: Path) -> List[HistoryPoint]:
    points: List[HistoryPoint] = []
    with path.open("r", encoding="utf-8") as fh:
        reader = csv.DictReader(fh)
        for r in reader:
            points.append(HistoryPoint(
                timestamp=r["timestamp"],
                rps=float(r["rps"]),
            ))
    return points


def _peak(history: List[HistoryPoint]) -> float:
    return max(p.rps for p in history)


def _forecast(history: List[HistoryPoint],
              weeks_out: int,
              growth_per_week: float,
              headroom_factor: float,
              pod_rps: float) -> List[ForecastWeek]:
    base_peak = _peak(history)
    week_0 = dt.datetime.now(dt.timezone.utc).date()
    rows: List[ForecastWeek] = []
    for i in range(1, weeks_out + 1):
        forecast_peak = base_peak * (growth_per_week ** i)
        provisioned_peak = forecast_peak * headroom_factor
        pods = max(1, math.ceil(provisioned_peak / pod_rps))
        gpus_min = max(1, math.ceil(pods * 0.6))   # ≈ 60 % GPU-bound
        pg_iops = max(1000, math.ceil(provisioned_peak * 12))   # ~12 IOPS/RPS
        redis_nodes = max(1, math.ceil(provisioned_peak / 1500))
        rows.append(ForecastWeek(
            week_index=i,
            iso_date=(week_0 + dt.timedelta(weeks=i)).isoformat(),
            forecast_peak_rps=round(forecast_peak, 1),
            pods_required=pods,
            gpu_count_min=gpus_min,
            pg_iops_required=pg_iops,
            redis_node_count=redis_nodes,
        ))
    return rows


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description="Capacity planner")
    p.add_argument("--history", type=Path,
                   help="CSV with columns timestamp,rps (one per hour).")
    p.add_argument("--weeks", type=int, default=12)
    p.add_argument("--growth-per-week", type=float, default=1.15)
    p.add_argument("--headroom-factor", type=float, default=1.30)
    p.add_argument("--pod-rps", type=float, default=80.0)
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/capacity_forecast.json"))
    p.add_argument("--demo", action="store_true")
    args = p.parse_args(argv)

    if args.demo:
        # Synthetic 168-hour week with a noon Tue peak at 220 RPS.
        history = []
        base = dt.datetime.now(dt.timezone.utc) - dt.timedelta(hours=168)
        for h in range(168):
            ts = base + dt.timedelta(hours=h)
            hour = ts.hour
            weekday = ts.weekday()
            day_factor = 1.0 if 1 <= weekday <= 4 else 0.6
            hour_factor = 0.4 + 0.6 * math.sin((hour - 6) / 24 * math.pi)
            rps = max(20, 200 * day_factor * hour_factor)
            history.append(HistoryPoint(timestamp=ts.isoformat(), rps=rps))
    elif args.history and args.history.exists():
        history = _read_history(args.history)
    else:
        print("--history CSV required (or --demo)", file=sys.stderr)
        return 64

    forecast = _forecast(
        history,
        weeks_out=args.weeks,
        growth_per_week=args.growth_per_week,
        headroom_factor=args.headroom_factor,
        pod_rps=args.pod_rps,
    )

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps({
        "apiVersion": "efe.io/v1",
        "kind": "CapacityForecast",
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "params": {
            "growth_per_week": args.growth_per_week,
            "headroom_factor": args.headroom_factor,
            "pod_rps": args.pod_rps,
            "weeks_out": args.weeks,
        },
        "current_peak_rps": _peak(history),
        "forecast": [asdict(r) for r in forecast],
    }, indent=2), encoding="utf-8")

    print(f"capacity forecast → {args.out}")
    print(f"  current peak  = {_peak(history):.0f} RPS")
    print(f"  week +1 peak  = {forecast[0].forecast_peak_rps:.0f} RPS "
          f"({forecast[0].pods_required} pods)")
    print(f"  week +{args.weeks} peak  = {forecast[-1].forecast_peak_rps:.0f} RPS "
          f"({forecast[-1].pods_required} pods, {forecast[-1].gpu_count_min} GPUs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
