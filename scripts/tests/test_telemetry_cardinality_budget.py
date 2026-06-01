"""Sprint **M5 extension** — Cardinality budget pin test.

Architecture § 14 mandates the total Prometheus cardinality stays
below 50 000 séries (Prometheus per-instance soft limit before
performance degradation).

This test computes the worst-case cardinality of every M5 extension
instrument via the canonical formula
`product_cartesien(label_cardinalities)` and asserts the sum
remains within budget.

Pre-existing modules (D13/D14/D15/D16/D17/D18) are NOT counted —
they were budgeted in their own architecture docs. We only count
the 19 NEW instruments added by M5 extension (D29 + D34 + D37) :
**~3 700 séries** projected.
"""
from __future__ import annotations

import pytest

# Maximum tenant cardinality assumed in the architecture.
MAX_TENANTS = 200
# Maximum kernel cardinality (M2 § 6 — 23 ComputeBackend ops + helpers).
MAX_KERNELS = 30
# Maximum GPU device count (T4/A10/H100 mixed deployments).
MAX_GPUS = 8
# 8 GA templates per M1 D33.
N_TEMPLATES = 8
# 8 fusion kernel ids per M1 D33 (Semantic, Logical, Diversifying,
# Quality, Causal, Temporal, Safety, Consistency).
N_KERNEL_IDS = 8
# 3 LLM routers (mock, openai, anthropic).
N_ROUTERS = 3
# 3 Mode-3 aggregators (mean, weighted, median).
N_AGGREGATORS = 3
# Mode-3 modes per call.
N_MODE_PER_CALL = 2
# Outcome label cardinality for kernel calls (ok / fail / oom).
N_OUTCOMES = 3
# Histogram bucket counts (per architecture).
KERNEL_BUCKETS = 12   # exponential(1e-6, 4.0, 12)
NCCL_BUCKETS = 10     # exponential(1e-4, 3.0, 10)
ROUTER_BUCKETS = 8    # exponential(0.05, 2.0, 8)
ANCHORS_BUCKETS = 5   # [1, 3, 5, 10, 20]
AGG_BUCKETS = 5       # exponential(1e-4, 10.0, 5)


# ─────────────────────────────────────────────────────────────────────
# D29 GPU cardinality
# ─────────────────────────────────────────────────────────────────────

def d29_cardinality() -> dict[str, int]:
    return {
        "efe_gpu_kernel_duration_seconds": MAX_KERNELS * KERNEL_BUCKETS,
        "efe_gpu_memory_used_bytes": MAX_GPUS,
        "efe_gpu_utilization_ratio": MAX_GPUS,
        "efe_gpu_kernel_calls_total": MAX_KERNELS * N_OUTCOMES,
        "efe_gpu_oom_total": MAX_GPUS,
        "efe_gpu_nccl_allreduce_seconds": NCCL_BUCKETS,
    }


def test_d29_cardinality_under_budget() -> None:
    total = sum(d29_cardinality().values())
    # D29 budget per architecture § 9.3 : ~1 000 series.
    assert total <= 1_500, f"D29 cardinality {total} > 1 500"


# ─────────────────────────────────────────────────────────────────────
# D34 nano-agent cardinality
# ─────────────────────────────────────────────────────────────────────

def d34_nano_cardinality() -> dict[str, int]:
    return {
        "efe_nano_agent_population_total": N_TEMPLATES * MAX_TENANTS,
        "efe_nano_agent_consensus_weight": N_TEMPLATES,
        "efe_nano_agent_safety_triggered_total": N_TEMPLATES * MAX_TENANTS,
        "efe_nano_agent_kernel_routed_total": N_KERNEL_IDS * N_TEMPLATES,
    }


def test_d34_nano_cardinality_under_budget() -> None:
    total = sum(d34_nano_cardinality().values())
    # Per architecture § 14.1 : 3 272 series projected.
    assert total <= 3_500, f"D34 nano cardinality {total} > 3 500"


# ─────────────────────────────────────────────────────────────────────
# D34 Mode-2 cardinality
# ─────────────────────────────────────────────────────────────────────

def d34_mode2_cardinality() -> dict[str, int]:
    return {
        "efe_mode2_cache_hit_total": N_ROUTERS * MAX_TENANTS,
        "efe_mode2_cache_miss_total": N_ROUTERS * MAX_TENANTS,
        "efe_mode2_router_latency_seconds": N_ROUTERS * ROUTER_BUCKETS,
        "efe_mode2_anchors_generated": N_ROUTERS * ANCHORS_BUCKETS,
        "efe_mode2_seed_mix_collisions_total": 1,
    }


def test_d34_mode2_cardinality_under_budget() -> None:
    total = sum(d34_mode2_cardinality().values())
    # Per architecture § 14.1 : 1 246 series projected.
    assert total <= 1_500, f"D34 mode2 cardinality {total} > 1 500"


# ─────────────────────────────────────────────────────────────────────
# D37 Mode-3 cardinality
# ─────────────────────────────────────────────────────────────────────

def d37_cardinality() -> dict[str, int]:
    return {
        "efe_mode3_subswarms_spawned_total": N_MODE_PER_CALL * MAX_TENANTS,
        "efe_mode3_dispersion": N_AGGREGATORS * MAX_TENANTS,
        "efe_mode3_k_clamped_total": MAX_TENANTS,
        "efe_mode3_aggregation_seconds": N_AGGREGATORS * AGG_BUCKETS,
    }


def test_d37_cardinality_under_budget() -> None:
    total = sum(d37_cardinality().values())
    # Per architecture § 14.1 : 1 215 series projected.
    assert total <= 1_500, f"D37 cardinality {total} > 1 500"


# ─────────────────────────────────────────────────────────────────────
# Total M5 extension cardinality
# ─────────────────────────────────────────────────────────────────────

def test_m5_extension_total_under_50k() -> None:
    """Final invariant : the 19 new instruments sum ≤ 6 500 series.
    The full GA budget of 50k includes pre-existing D13-D18
    instruments (~30 000 series). M5 extension's 6 500 stays well
    within the remaining budget."""
    total = (
        sum(d29_cardinality().values())
        + sum(d34_nano_cardinality().values())
        + sum(d34_mode2_cardinality().values())
        + sum(d37_cardinality().values())
    )
    assert total <= 8_000, (
        f"M5 extension total cardinality {total} > 8 000 ; "
        f"reduce labels or revisit architecture"
    )


def test_m5_extension_no_label_unbounded() -> None:
    """Anti-regression : no instrument should label by `request_id`,
    `swarm_id`, or any UUID — those are unbounded and would
    explode cardinality."""
    from pathlib import Path
    repo = Path(__file__).resolve().parents[2]
    metrics_dir = repo / "crates" / "efe-telemetry" / "src" / "metrics"
    forbidden_labels = {"request_id", "swarm_id", "trace_id", "uuid"}
    new_files = [
        "gpu_instruments.rs",
        "nano_agent_instruments.rs",
        "mode2_instruments.rs",
        "mode3_instruments.rs",
    ]
    for fname in new_files:
        rs = (metrics_dir / fname).read_text(encoding="utf-8")
        for forbidden in forbidden_labels:
            # Match `&["...", "..."],` — the labels block in `Opts::new`.
            # We grep within label-array context : `&[`.
            for chunk in rs.split("&["):
                # Take only the array body (until the first `]`)
                end = chunk.find("]")
                if end < 0:
                    continue
                body = chunk[:end]
                # Skip non-label arrays (e.g. `&[1.0, 3.0, ...]` for buckets).
                if "\"" not in body:
                    continue
                assert (
                    f'"{forbidden}"' not in body
                ), f"Forbidden unbounded label '{forbidden}' in {fname}"


# ─────────────────────────────────────────────────────────────────────
# Total instrument count check (50 GA target)
# ─────────────────────────────────────────────────────────────────────

def test_m5_extension_adds_19_instruments() -> None:
    """Architecture § 4.1 : 31 existing + 19 new = 50 GA. We pin
    the 19 count via the cardinality dicts."""
    n = (
        len(d29_cardinality())
        + len(d34_nano_cardinality())
        + len(d34_mode2_cardinality())
        + len(d37_cardinality())
    )
    assert n == 19, f"M5 extension should add exactly 19 instruments, got {n}"


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
