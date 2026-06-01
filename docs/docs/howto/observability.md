# How-to — Observability

EFE supports the three pillars of observability out of the box:

* **Metrics** — Prometheus on port 9090
* **Tracing** — OpenTelemetry (OTLP gRPC) when wired
* **Structured logs** — JSON-per-line, log aggregator-friendly

## Metrics

The chart's `ServiceMonitor` makes the server discoverable by
prometheus-operator. Sample queries:

```promql
# Request rate per replica
sum by (pod) (rate(grpc_server_handled_total{job="efe"}[5m]))

# p99 latency
histogram_quantile(0.99,
  sum by (le) (rate(grpc_server_handling_seconds_bucket{job="efe"}[5m])))

# Memory growth (soak signal)
deriv(process_resident_memory_bytes{job="efe"}[1h])
```

## Distributed tracing

Install with the `[otel]` extra:

```bash
pip install 'efe-tool[otel]==0.1.10'
```

Configure the OTLP collector via env:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://collector.observability:4317
export OTEL_SERVICE_NAME=my-app
```

Example wiring (cf. `examples/04_production_client.py`):

```python
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter

trace.set_tracer_provider(TracerProvider())
trace.get_tracer_provider().add_span_processor(
    BatchSpanProcessor(OTLPSpanExporter())
)
```

EFE's gRPC server propagates the trace context from the request
metadata, so server-side spans are children of your client spans.

## Structured logs

The server logs JSON to stdout; aggregate via Loki / Cloud Logging
/ Datadog by labelling on `app.kubernetes.io/name=efe`.

Sample log line:

```json
{"ts":"2026-04-27T14:20:01Z","level":"INFO","msg":"classified",
 "label":"fraud","confidence":0.92,"rounds":7,
 "request_id":"abc-123","tenant":"production-pilot"}
```

Sensitive fields (raw user prompts) are NEVER logged — the audit
logger redacts them per the D8 PII pipeline. See
[Operations / Deployment](../../operations/deployment.md#5-observability).

## Client-side logging

For the SDK itself, use Python's standard `logging`:

```python
import logging
logging.basicConfig(level=logging.INFO)
logging.getLogger("efe_tool").setLevel(logging.DEBUG)
```

The SDK never logs request payloads — only structural events
(connect, retry, close).
