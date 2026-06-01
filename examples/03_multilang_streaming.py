"""Sprint D10 — example 3: multilingual concurrent streaming.

Goal: process a stream of FR + EN messages concurrently with
back-pressure. Demonstrates:

    * AsyncEfeClient (asyncio path) — D10 thread-pool skeleton ;
      v0.2.0 will provide a native grpc.aio backend with the
      same surface.
    * Multilingual anchor design (one catalog, mixed languages)
    * Bounded concurrency via asyncio.Semaphore
    * Per-message timing + structured aggregation

Pre-requisites:
    * efe-server reachable on 127.0.0.1:50051 with the load-test
      env preset ``EFE_IDEMPOTENCY_TTL_MS=0`` (so concurrency
      stress is real, not cache-served — see d9_deep_analysis_supplement.md A2)
    * pip install efe-tool==0.1.10
    * Python 3.10+

Run:
    python examples/03_multilang_streaming.py
"""

from __future__ import annotations

import asyncio
import time
import warnings
from pathlib import Path
from typing import AsyncIterator

# AsyncEfeClient is a thread-pool skeleton in v0.1.x — silence the
# DeprecationWarning emitted at instantiation since we are
# explicitly aware of the staged delivery.
warnings.filterwarnings(
    "ignore",
    message=r"AsyncEfeClient is a thread-pool skeleton.*",
    category=DeprecationWarning,
)

from efe_tool import AnchorCatalog, AsyncEfeClient, LocalSTEmbedder


CATALOG_PATH = (
    Path(__file__).resolve().parent.parent / "catalogs" / "support_triage.yaml"
)

# Synthetic 8-message stream — 4 FR + 4 EN — repeated 3× to simulate
# a sustained inbound flow.
_BASE_STREAM: list[str] = [
    "I'd like to cancel my subscription.",
    "Pourquoi ai-je été facturé deux fois ?",
    "How do I update my payment method?",
    "Je n'arrive pas à me connecter à mon compte.",
    "The delivery is 5 days late, what's going on?",
    "Ma commande est marquée livrée mais je n'ai rien reçu.",
    "Can I extend the return window?",
    "Quelles sont les conditions de remboursement ?",
]


async def fake_stream() -> AsyncIterator[str]:
    """Yield each test message, with a 50 ms gap to simulate paced upstream."""
    for batch in range(3):
        for msg in _BASE_STREAM:
            yield msg
            await asyncio.sleep(0.05)


async def classify_one(
    client: AsyncEfeClient,
    catalog: AnchorCatalog,
    text: str,
    sem: asyncio.Semaphore,
) -> tuple[str, str, float, float]:
    """Classify one message under a concurrency cap; return (text, label, conf, ms)."""
    async with sem:
        t0 = time.monotonic()
        result = await client.classify(text, catalog)
        elapsed_ms = (time.monotonic() - t0) * 1000.0
    return text, result.label, result.confidence, elapsed_ms


async def main() -> None:
    embedder = LocalSTEmbedder.from_env()
    catalog = AnchorCatalog.load(CATALOG_PATH, embedder=embedder)

    sem = asyncio.Semaphore(8)  # ≤ 8 in-flight RPCs (back-pressure)
    tasks: list[asyncio.Task] = []

    async with AsyncEfeClient(
        host="127.0.0.1:50051", insecure=True, max_workers=8,
    ) as client:
        async for text in fake_stream():
            tasks.append(
                asyncio.create_task(classify_one(client, catalog, text, sem))
            )
        results = await asyncio.gather(*tasks, return_exceptions=True)

    # Aggregate stats.
    successes = [r for r in results if not isinstance(r, BaseException)]
    failures = [r for r in results if isinstance(r, BaseException)]

    print(f"\nProcessed {len(results)} messages "
          f"({len(successes)} OK, {len(failures)} failed)")
    if successes:
        latencies = sorted(r[3] for r in successes)
        p50 = latencies[len(latencies) // 2]
        p99 = latencies[max(0, int(0.99 * len(latencies)) - 1)]
        labels = {}
        for _, label, _, _ in successes:
            labels[label] = labels.get(label, 0) + 1
        print(f"  latency p50: {p50:.1f} ms — p99: {p99:.1f} ms")
        print(f"  label distribution: {dict(sorted(labels.items()))}")
    for exc in failures[:3]:
        print(f"  ✗ {type(exc).__name__}: {exc}")


if __name__ == "__main__":
    asyncio.run(main())
