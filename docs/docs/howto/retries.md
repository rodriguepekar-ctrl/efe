# How-to — Retries with backoff

> Production retry pattern with exponential backoff + full jitter.
> Reference: `examples/04_production_client.py`.

## When to retry

The `Efe*Error` hierarchy carries a `retryable: bool` class attribute:

| Exception | Retryable | Reason |
|-----------|:---------:|--------|
| `EfeAuthError` | ❌ | refresh token first |
| `EfeRateLimitError` | ✅ | server backpressure |
| `EfeUnavailableError` | ✅ | transient (503 / timeout) |
| `EfeInvalidArgumentError` | ❌ | request shape wrong |
| `EfeNotFoundError` | ❌ | catalog/cone missing |
| `EfeInternalError` | ❌ | server bug — don't hammer |

## Pattern: exponential backoff + full jitter

```python
import random, time, uuid
from efe_tool import EfeRateLimitError, EfeUnavailableError

def classify_with_retries(client, catalog, text, *,
                          max_attempts=3, base_s=0.2, cap_s=5.0):
    request_id = str(uuid.uuid4())
    for attempt in range(1, max_attempts + 1):
        try:
            return client.classify(text, catalog, request_id=request_id)
        except (EfeRateLimitError, EfeUnavailableError):
            if attempt == max_attempts:
                raise
            cap = min(base_s * 2 ** (attempt - 1), cap_s)
            time.sleep(random.uniform(0, cap))   # FULL jitter
```

**Why "full jitter" not "decorrelated jitter"?** Both work, full
jitter is simpler and provably bounds tail latency under herd
conditions. See AWS architecture blog for the math.

## Idempotency keys

The `request_id` argument acts as an idempotency key. The server
caches successful responses for the duration of
`EFE_IDEMPOTENCY_TTL_MS` (default 10 s). Subsequent calls with
the same `request_id` return the cached result without re-running
the swarm.

* **In-flight retries** (same logical request) → reuse the SAME
  `request_id`. The server returns the same cached result.
* **New logical requests** → generate a fresh UUIDv4.

## Hard timeout

Set a global per-call timeout via constructor:

```python
client = EfeClient(endpoint="...", embedder=..., timeout_s=10.0)
```

A timeout on the gRPC call surfaces as `EfeUnavailableError`
(retryable). For per-call overrides, use a context manager around
the channel — see the SDK's `_internal.grpc_channel`.

## See also

* [Authentication](auth.md) — token refresh flow.
* [Observability](observability.md) — track retry counters.
