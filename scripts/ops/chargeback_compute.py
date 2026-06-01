#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D98) — Per-tenant chargeback computer.

Aggregates the runtime cost of one tenant across :

    1. **Compute**   — Mode-1 GPU seconds + Mode-2/3 LLM tokens.
    2. **Storage**   — Postgres + Redis + S3 audit per-GB-month.
    3. **Egress**    — cross-region + internet egress GB.
    4. **AL loop**   — annotation oracle USD + retune CPU minutes.
    5. **Overhead**  — flat per-tenant platform overhead.

Emits a per-tenant rollup JSON suitable for the trust center, the
master Grafana dashboard, and Stripe invoice line items.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 12.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import sys
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Dict, Iterable, List, Optional


# ---------------------------------------------------------------------
# Price book — kept centrally; bumped via PR (so audit chain catches).
# ---------------------------------------------------------------------

PRICES_USD = {
    "gpu_second_l4":           0.000180,  # = USD/s × L4 hour 0.65
    "gpu_second_a10":          0.000280,
    "gpu_second_a100":         0.001200,
    "llm_token_routed":        0.00000015,
    "llm_token_premium":       0.00000300,
    "pg_gb_month":             0.20,
    "redis_gb_month":          0.30,
    "s3_audit_gb_month":       0.024,
    "egress_gb_internet":      0.085,
    "egress_gb_cross_region":  0.020,
    "annotation_oracle_call":  0.003,
    "retune_cpu_minute":       0.0035,
    "platform_overhead_month": 25.0,
}


@dataclass
class UsageRecord:
    tenant: str
    gpu_seconds_l4:      float = 0.0
    gpu_seconds_a10:     float = 0.0
    gpu_seconds_a100:    float = 0.0
    llm_tokens_routed:   float = 0.0
    llm_tokens_premium:  float = 0.0
    pg_gb_month:         float = 0.0
    redis_gb_month:      float = 0.0
    s3_audit_gb_month:   float = 0.0
    egress_gb_internet:  float = 0.0
    egress_gb_xregion:   float = 0.0
    annotation_calls:    float = 0.0
    retune_cpu_minutes:  float = 0.0


@dataclass
class CostBreakdown:
    tenant:         str
    compute_usd:    float = 0.0
    storage_usd:    float = 0.0
    egress_usd:     float = 0.0
    al_loop_usd:    float = 0.0
    overhead_usd:   float = 0.0
    total_usd:      float = 0.0
    notes:          List[str] = field(default_factory=list)


# ---------------------------------------------------------------------
# Pricing logic
# ---------------------------------------------------------------------


def compute(record: UsageRecord) -> CostBreakdown:
    out = CostBreakdown(tenant=record.tenant)

    # Compute
    out.compute_usd = (
        record.gpu_seconds_l4   * PRICES_USD["gpu_second_l4"]
        + record.gpu_seconds_a10  * PRICES_USD["gpu_second_a10"]
        + record.gpu_seconds_a100 * PRICES_USD["gpu_second_a100"]
        + record.llm_tokens_routed  * PRICES_USD["llm_token_routed"]
        + record.llm_tokens_premium * PRICES_USD["llm_token_premium"]
    )

    # Storage
    out.storage_usd = (
        record.pg_gb_month       * PRICES_USD["pg_gb_month"]
        + record.redis_gb_month    * PRICES_USD["redis_gb_month"]
        + record.s3_audit_gb_month * PRICES_USD["s3_audit_gb_month"]
    )

    # Egress
    out.egress_usd = (
        record.egress_gb_internet * PRICES_USD["egress_gb_internet"]
        + record.egress_gb_xregion  * PRICES_USD["egress_gb_cross_region"]
    )

    # Active learning loop
    out.al_loop_usd = (
        record.annotation_calls   * PRICES_USD["annotation_oracle_call"]
        + record.retune_cpu_minutes * PRICES_USD["retune_cpu_minute"]
    )

    # Platform overhead (= flat / month)
    out.overhead_usd = PRICES_USD["platform_overhead_month"]

    out.total_usd = (
        out.compute_usd
        + out.storage_usd
        + out.egress_usd
        + out.al_loop_usd
        + out.overhead_usd
    )

    # Quality notes
    if record.llm_tokens_premium > record.llm_tokens_routed * 0.5:
        out.notes.append("⚠ premium LLM usage > 50% of routed — review routing config")
    if record.egress_gb_internet > 100:
        out.notes.append("⚠ internet egress > 100 GB/mo — consider CloudFront / Cloud CDN")
    return out


# ---------------------------------------------------------------------
# I/O
# ---------------------------------------------------------------------


def _read_csv(path: Path) -> List[UsageRecord]:
    rows: List[UsageRecord] = []
    with path.open("r", encoding="utf-8") as fh:
        reader = csv.DictReader(fh)
        for r in reader:
            rows.append(UsageRecord(
                tenant=r["tenant"],
                gpu_seconds_l4=float(r.get("gpu_seconds_l4", 0) or 0),
                gpu_seconds_a10=float(r.get("gpu_seconds_a10", 0) or 0),
                gpu_seconds_a100=float(r.get("gpu_seconds_a100", 0) or 0),
                llm_tokens_routed=float(r.get("llm_tokens_routed", 0) or 0),
                llm_tokens_premium=float(r.get("llm_tokens_premium", 0) or 0),
                pg_gb_month=float(r.get("pg_gb_month", 0) or 0),
                redis_gb_month=float(r.get("redis_gb_month", 0) or 0),
                s3_audit_gb_month=float(r.get("s3_audit_gb_month", 0) or 0),
                egress_gb_internet=float(r.get("egress_gb_internet", 0) or 0),
                egress_gb_xregion=float(r.get("egress_gb_xregion", 0) or 0),
                annotation_calls=float(r.get("annotation_calls", 0) or 0),
                retune_cpu_minutes=float(r.get("retune_cpu_minutes", 0) or 0),
            ))
    return rows


def _emit(records: Iterable[CostBreakdown], out: Path, fmt: str) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    listed = list(records)
    if fmt == "json":
        out.write_text(
            json.dumps(
                {
                    "apiVersion": "efe.io/v1",
                    "kind": "TenantChargeback",
                    "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
                    "rows": [asdict(r) for r in listed],
                    "summary": {
                        "tenant_count": len(listed),
                        "total_usd": sum(r.total_usd for r in listed),
                    },
                },
                indent=2,
            ),
            encoding="utf-8",
        )
    elif fmt == "csv":
        if not listed:
            return
        with out.open("w", encoding="utf-8", newline="") as fh:
            writer = csv.DictWriter(fh, fieldnames=list(asdict(listed[0]).keys()))
            writer.writeheader()
            for r in listed:
                row = asdict(r)
                row["notes"] = "|".join(row["notes"])
                writer.writerow(row)
    else:
        raise ValueError(f"unknown format: {fmt}")


# ---------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description="Per-tenant chargeback computer.")
    p.add_argument("--usage", type=Path,
                   help="CSV with one row per tenant. Header must match "
                        "UsageRecord fields.")
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/chargeback.json"))
    p.add_argument("--format", choices=["json", "csv"], default="json")
    p.add_argument("--demo", action="store_true",
                   help="Use a deterministic 3-tenant demo dataset.")
    args = p.parse_args(argv)

    if args.demo:
        records = [
            UsageRecord(
                tenant="acme",
                gpu_seconds_l4=72_000.0,
                llm_tokens_routed=18_000_000.0,
                pg_gb_month=45.0, redis_gb_month=8.0, s3_audit_gb_month=120.0,
                egress_gb_internet=42.0, annotation_calls=1200.0,
            ),
            UsageRecord(
                tenant="globex",
                gpu_seconds_a10=12_000.0,
                llm_tokens_routed=2_000_000.0, llm_tokens_premium=200_000.0,
                pg_gb_month=12.0, redis_gb_month=3.0, s3_audit_gb_month=18.0,
                egress_gb_internet=5.0, annotation_calls=80.0,
            ),
            UsageRecord(
                tenant="initech",
                gpu_seconds_a100=4_500.0,
                llm_tokens_routed=900_000.0, llm_tokens_premium=900_000.0,
                pg_gb_month=8.0, redis_gb_month=2.0, s3_audit_gb_month=12.0,
                egress_gb_internet=140.0,
            ),
        ]
    else:
        if not args.usage or not args.usage.exists():
            print("--usage CSV required (or --demo)", file=sys.stderr)
            return 64
        records = _read_csv(args.usage)

    rollups = [compute(r) for r in records]
    _emit(rollups, args.out, args.format)
    total = sum(r.total_usd for r in rollups)
    print(f"chargeback → {args.out}")
    print(f"  tenants={len(rollups)}  total=USD {total:,.2f}")
    for r in rollups:
        print(f"  • {r.tenant:<10s} USD {r.total_usd:>9.2f} "
              f"(c={r.compute_usd:.2f}  s={r.storage_usd:.2f}  "
              f"e={r.egress_usd:.2f}  al={r.al_loop_usd:.2f}  "
              f"ov={r.overhead_usd:.2f})")
        for note in r.notes:
            print(f"      {note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
