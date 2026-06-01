# Reference — Environment variables

## Client-side (SDK)

| Variable | Default | Description |
|----------|---------|-------------|
| `EFE_ENDPOINT` | (required by `from_env`) | `host:port` of the gRPC server |
| `EFE_JWT` | unset | Bearer JWT for auth |
| `EFE_TENANT` | unset | Tenant identifier (header `x-tenant-id`) |
| `EFE_TIMEOUT_S` | `5.0` | Per-call timeout in seconds |
| `EFE_INSECURE` | `1` | Use insecure channel (no TLS); `0` for prod |
| `EFE_EMBEDDER_MODEL` | `sentence-transformers/all-MiniLM-L6-v2` | Local embedder |
| `EFE_HOST` | `127.0.0.1:50051` | Used by example 04 (alternative to `EFE_ENDPOINT`) |
| `EFE_TOKEN` | unset | Used by example 04 (alias for `EFE_JWT`) |

## Server-side (running `efe-server`)

| Variable | Default | Description |
|----------|---------|-------------|
| `EFE_LISTEN_ADDR` | `0.0.0.0:50051` | gRPC bind |
| `EFE_METRICS_ADDR` | `0.0.0.0:9090` | Prometheus exporter bind |
| `EFE_AUTH_ENABLED` | `true` | Require JWT (set `false` only for dev) |
| `EFE_FUSION_BETA0` | `0.45` | Mode-1 β₀ (D7 winner) |
| `EFE_FUSION_LAMBDA_BETA` | `0.05` | Mode-1 β decay |
| `EFE_FUSION_ALPHA0` | `0.30` | Mode-1 α₀ |
| `EFE_FUSION_TAU` | `0.85` | Reject threshold |
| `EFE_IDEMPOTENCY_TTL_MS` | `10000` | Server-side LRU TTL ; **set to 0 in load tests** (D9.A2) |
| `EFE_LOG_CENTROID_TRACE` | `0` | Verbose centroid trace logging |
| `RUST_LOG` | `info,efe=info` | tracing crate filter |

## OpenTelemetry

When the `[otel]` extra is installed:

| Variable | Default | Description |
|----------|---------|-------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://localhost:4317` | Collector |
| `OTEL_SERVICE_NAME` | `efe-tool` | Service identifier |
| `OTEL_RESOURCE_ATTRIBUTES` | unset | comma-separated `key=value` |

## See also

* [Operations / Deployment](../../operations/deployment.md) — full
  values.yaml for production.
