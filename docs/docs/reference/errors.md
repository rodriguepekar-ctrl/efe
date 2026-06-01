# Reference — Errors

The SDK raises one of two exception families.

## D10 hierarchy (recommended)

Each subclass maps 1:1 to a gRPC status code, with a class-level
`retryable` hint to drive backoff logic.

| Exception | gRPC code | retryable | Action |
|-----------|----------:|:---------:|--------|
| `EfeError` | (base) | varies | catch-all |
| `EfeAuthError` | UNAUTHENTICATED (16) / PERMISSION_DENIED (7) | ❌ | refresh token |
| `EfeRateLimitError` | RESOURCE_EXHAUSTED (8) | ✅ | backoff + retry |
| `EfeInvalidArgumentError` | INVALID_ARGUMENT (3) / FAILED_PRECONDITION (9) / OUT_OF_RANGE (11) | ❌ | fix code |
| `EfeNotFoundError` | NOT_FOUND (5) | ❌ | check catalog/cone key |
| `EfeUnavailableError` | UNAVAILABLE (14) / DEADLINE_EXCEEDED (4) | ✅ | backoff + retry |
| `EfeInternalError` | INTERNAL (13) / UNKNOWN (2) | ❌ | file a bug |

## Translation

Use `raise_from_grpc(rpc_error)` to convert a raw `grpc.RpcError`:

```python
import grpc
from efe_tool import raise_from_grpc

try:
    stub.Execute(req)
except grpc.RpcError as e:
    raise raise_from_grpc(e) from e
```

The original `grpc.RpcError` is preserved as `__cause__`.

## Legacy hierarchy (pre-D10)

Kept for backward compatibility with existing partner code:

| Exception | Notes |
|-----------|-------|
| `BridgeError` | base of legacy + D10 hierarchies |
| `ConfigError` | invalid config (env var, missing file) |
| `CatalogError` | catalog YAML / embedding failure |
| `EmbedderError` | local embedder failure |
| `NetworkError` | gRPC transport failure |
| `LowConfidenceWarning` | non-fatal warning (when `strict=True`) |

`EfeError` is itself a subclass of `BridgeError`, so legacy
`except BridgeError` still catches everything D10 raises.

## Retry decision flow

```
catch EfeError → if .retryable: backoff + retry (≤ N attempts)
                   else: re-raise / surface to caller
```

See [How-to / Retries](../howto/retries.md) for the canonical
backoff implementation.
