# SDK OpenTelemetry Conventions (Φ-C-SDK Polyglot)

> Sprint **Φ-C-SDK Completion (D31)** — artefact #18.
> Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 9.

## Mission

Standardiser les noms d'instruments OpenTelemetry **cross-SDK**. Un
opérateur qui regarde un dashboard Grafana doit voir les **mêmes
courbes** pour Python, Go, TS, Java — différentiées uniquement par le
label `sdk`.

## Instruments canoniques (= 5)

| # | Nom | Type | Labels | Description |
|---|-----|------|--------|-------------|
| 1 | `efe_sdk_classify_total` | Counter | `sdk`, `outcome` | Total classify calls |
| 2 | `efe_sdk_classify_seconds` | Histogram | `sdk` | End-to-end latency |
| 3 | `efe_sdk_grpc_seconds` | Histogram | `sdk`, `method` | Pure gRPC latency (excl. marshal) |
| 4 | `efe_sdk_retry_total` | Counter | `sdk`, `reason` | Retry attempts |
| 5 | `efe_sdk_circuit_breaker_state` | Gauge | `sdk` | 0=closed, 1=open, 2=half-open |

### Labels canoniques

* `sdk` ∈ {`py`, `go`, `ts`, `java`}
* `outcome` ∈ {`ok`, `err`}
* `method` ∈ {`Classify`, `BatchClassify`, `Healthcheck`, ...} (= proto RPCs)
* `reason` ∈ {`deadline_exceeded`, `unavailable`, `transient`, ...}

## Implementation per SDK

### Python (PyO3 in-process + gRPC)

```python
import efe
import opentelemetry.metrics as metrics_api

# Auto-instrumentation via the gRPC interceptor.
from opentelemetry.instrumentation.grpc import GrpcAioInstrumentorClient
GrpcAioInstrumentorClient().instrument()

# Manual instrumentation for in-process PyO3 path.
meter = metrics_api.get_meter("efe.sdk.python")
classify_counter = meter.create_counter(
    "efe_sdk_classify_total",
    description="Total classify calls",
)
classify_histogram = meter.create_histogram(
    "efe_sdk_classify_seconds",
    description="End-to-end classify latency",
)

# Record in user code or wrapper :
classify_counter.add(1, {"sdk": "py", "outcome": "ok"})
classify_histogram.record(0.045, {"sdk": "py"})
```

### Go

```go
import (
    "go.opentelemetry.io/otel"
    "go.opentelemetry.io/otel/metric"
    "go.opentelemetry.io/otel/attribute"
    "go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc"
    "google.golang.org/grpc"
)

// Auto-instrumentation via gRPC interceptor.
conn, _ := grpc.Dial(host,
    grpc.WithStatsHandler(otelgrpc.NewClientHandler()))

// Manual instruments.
meter := otel.Meter("efe.sdk.go")
classifyCounter, _ := meter.Int64Counter("efe_sdk_classify_total",
    metric.WithDescription("Total classify calls"))
classifyHistogram, _ := meter.Float64Histogram("efe_sdk_classify_seconds",
    metric.WithDescription("End-to-end classify latency"))

// In Classify() :
classifyCounter.Add(ctx, 1, metric.WithAttributes(
    attribute.String("sdk", "go"),
    attribute.String("outcome", "ok"),
))
classifyHistogram.Record(ctx, 0.045, metric.WithAttributes(
    attribute.String("sdk", "go"),
))
```

### TypeScript

```typescript
import { metrics } from "@opentelemetry/api";
import { GrpcInstrumentation } from "@opentelemetry/instrumentation-grpc";
import { registerInstrumentations } from "@opentelemetry/instrumentation";

// Auto-instrumentation.
registerInstrumentations({
  instrumentations: [new GrpcInstrumentation()],
});

// Manual instruments.
const meter = metrics.getMeter("efe.sdk.ts");
const classifyCounter = meter.createCounter("efe_sdk_classify_total", {
  description: "Total classify calls",
});
const classifyHistogram = meter.createHistogram("efe_sdk_classify_seconds", {
  description: "End-to-end classify latency",
});

// In classify() :
classifyCounter.add(1, { sdk: "ts", outcome: "ok" });
classifyHistogram.record(0.045, { sdk: "ts" });
```

### Java

```java
import io.opentelemetry.api.OpenTelemetry;
import io.opentelemetry.api.metrics.Meter;
import io.opentelemetry.api.metrics.LongCounter;
import io.opentelemetry.api.metrics.DoubleHistogram;
import io.opentelemetry.api.common.Attributes;
import io.opentelemetry.instrumentation.grpc.v1_6.GrpcTelemetry;

// Auto-instrumentation via grpc-1.6 agent.
var telemetry = GrpcTelemetry.builder(openTelemetry).build();
var channel = ManagedChannelBuilder.forTarget(host)
    .intercept(telemetry.newClientInterceptor())
    .build();

// Manual instruments.
Meter meter = openTelemetry.getMeter("efe.sdk.java");
LongCounter classifyCounter = meter.counterBuilder("efe_sdk_classify_total")
    .setDescription("Total classify calls")
    .build();
DoubleHistogram classifyHistogram = meter.histogramBuilder("efe_sdk_classify_seconds")
    .setDescription("End-to-end classify latency")
    .build();

// In classify() :
classifyCounter.add(1, Attributes.of(
    AttributeKey.stringKey("sdk"), "java",
    AttributeKey.stringKey("outcome"), "ok"));
classifyHistogram.record(0.045, Attributes.of(
    AttributeKey.stringKey("sdk"), "java"));
```

## Configuration boilerplate

### OTLP endpoint (all SDKs)

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT="http://collector:4317"
export OTEL_SERVICE_NAME="my-app"
export OTEL_TRACES_EXPORTER="otlp"
export OTEL_METRICS_EXPORTER="otlp"
```

### Auto-config + manual hybrid

| SDK | Auto-config | Manual setup |
|-----|-------------|---------------|
| Python | `opentelemetry-distro` + `opentelemetry-bootstrap -a install` | `MeterProvider`, `TracerProvider` |
| Go | `otelconfgrpc` package | `metric.NewMeterProvider(...)` |
| TS | `@opentelemetry/auto-instrumentations-node` | `NodeSDK` |
| Java | OTEL agent `-javaagent:opentelemetry-javaagent.jar` | `SdkMeterProvider` builder |

## Grafana dashboard

`deploy/observability/grafana/efe-sdks.json` (= artefact #40) consume
ces instruments avec un panel `sdk` filter pour comparer les 4 SDKs.

## Pin test

`scripts/tests/test_polyglot_sdk_structure.py::test_phi_c_sdk_convention_docs_exist`
vérifie que ce doc existe.
