"""Micro-bench — drive N concurrent bot requests and report stats.

Useful to sanity-check the asyncio plumbing and the ledger arithmetic.
Still runs on the in-memory transport, so numbers are not a real
performance measurement — just a smoke proxy.

Run::

    python examples/batch_bench.py --n 200 --concurrency 32
"""

from __future__ import annotations

import argparse
import asyncio
import statistics
import time

from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.bot.budget import Budget
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint

PROMPT = (
    "plan: swarm.classify "
    "tenant=bench task=benchmark cone_version=1 "
    "problem_embedding=[0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,"
    "0.9,1.0,0.1,0.2,0.3,0.4,0.5,0.6] "
    "dim_latent=16"
)


async def _one(bot: Bot) -> float:
    t0 = time.monotonic()
    reply = await bot.answer(PROMPT, tenant="bench")
    if not reply.ok:
        raise RuntimeError(f"bench call failed: {reply.error}")
    return (time.monotonic() - t0) * 1000


async def run(n: int, concurrency: int) -> None:
    swarm = SwarmClient(
        SwarmEndpoint("in-memory"),
        transport=InMemorySwarmTransport(),
        max_retries=0,
    )
    bot = Bot(
        BotConfig(budget=Budget(max_wall_clock_ms=5_000)),
        swarm=swarm,
    )

    sem = asyncio.Semaphore(concurrency)
    latencies: list[float] = []
    errors = 0

    async def worker() -> None:
        nonlocal errors
        async with sem:
            try:
                latencies.append(await _one(bot))
            except Exception:
                errors += 1

    t0 = time.monotonic()
    await asyncio.gather(*(worker() for _ in range(n)))
    wall = time.monotonic() - t0

    await swarm.close()

    if not latencies:
        print("no successful calls")
        return

    latencies.sort()
    p50 = latencies[len(latencies) // 2]
    p95 = latencies[int(len(latencies) * 0.95)]
    p99 = latencies[int(len(latencies) * 0.99)]
    print(f"total={n} concurrency={concurrency} wall={wall * 1000:.1f}ms errors={errors}")
    print(f"latency (ms): p50={p50:.2f} p95={p95:.2f} p99={p99:.2f}")
    print(f"  mean={statistics.fmean(latencies):.2f} stdev={statistics.pstdev(latencies):.2f}")
    print(f"throughput ≈ {n / wall:.1f} rps")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--n", type=int, default=100)
    parser.add_argument("--concurrency", type=int, default=16)
    args = parser.parse_args()
    asyncio.run(run(args.n, args.concurrency))


if __name__ == "__main__":
    main()
