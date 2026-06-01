# How-to — Streaming

> Process a stream of inputs with bounded concurrency.
> Reference: `examples/03_multilang_streaming.py`.

The `AsyncEfeClient` lets you classify many messages in parallel
under an `asyncio.Semaphore` for back-pressure.

```python
import asyncio
from efe_tool import AsyncEfeClient, AnchorCatalog, LocalSTEmbedder

async def main():
    embedder = LocalSTEmbedder.from_env()
    catalog = AnchorCatalog.load("catalogs/your.yaml", embedder=embedder)
    sem = asyncio.Semaphore(8)   # ≤ 8 concurrent RPCs

    async with AsyncEfeClient(host="127.0.0.1:50051", insecure=True) as client:
        async def one(text):
            async with sem:
                return await client.classify(text, catalog)

        tasks = [asyncio.create_task(one(t)) for t in inbox()]
        results = await asyncio.gather(*tasks)

asyncio.run(main())
```

## Performance notes

* **v0.1.x — thread-pool skeleton**: under the hood,
  `AsyncEfeClient` uses a `ThreadPoolExecutor` running sync gRPC
  calls. Throughput is bounded by `max_workers` (default 8).
* **v0.2.x — native grpc.aio**: same surface, native async. Expect
  ~2× throughput for the same `max_workers` budget.
* **Idempotency cache** — when running concurrent loads against
  the SAME texts, set `EFE_IDEMPOTENCY_TTL_MS=0` server-side to
  measure real compute (cf. D9.A2).

## See also

* [Retries + backoff](retries.md) — production retry pattern.
* [Observability](observability.md) — metrics for tail latency.
