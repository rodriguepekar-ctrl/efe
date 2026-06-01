# SDK Retry + Circuit Breaker Conventions (Φ-C-SDK Polyglot)

> Sprint **Φ-C-SDK Completion (D31)** — artefact #19.
> Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 15.2.

## Mission

Standardiser le comportement de retry + circuit breaker cross-SDK.
Tous les 4 SDKs doivent **par défaut** se comporter identiquement
sur les erreurs transitoires (= UNAVAILABLE, DEADLINE_EXCEEDED).

## Retry policy canonique

| Paramètre | Valeur par défaut | Note |
|-----------|------------------|------|
| max_attempts | 3 | initial call + 2 retries |
| initial_backoff_ms | 100 | premier retry après 100 ms |
| max_backoff_ms | 5000 | cap = 5 s |
| backoff_multiplier | 2.0 | exponential |
| jitter_ratio | 0.2 | ±20 % randomization |
| retryable_codes | `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `RESOURCE_EXHAUSTED` | gRPC status codes |

Les utilisateurs peuvent override via API (= section idiomes ci-dessous).

## Circuit breaker policy canonique

| État | Trigger | Behavior |
|------|---------|----------|
| closed | initial | All requests pass through |
| open | 5 consecutive failures | Reject for 30 s with `CircuitOpen` |
| half-open | after 30 s timeout | Allow 1 probe request |
| → closed | probe succeeds | Reset state |
| → open | probe fails | Reset 30 s timer |

## Idiomes per SDK

### Python

```python
import efe

# Defaults
client = efe.EfeClient(host="efe:443")

# Override
client = efe.EfeClient(
    host="efe:443",
    retry_config=efe.RetryConfig(
        max_attempts=5,
        initial_backoff_ms=200,
        backoff_multiplier=1.5,
    ),
    circuit_breaker=efe.CircuitBreakerConfig(
        failure_threshold=10,
        reset_timeout_ms=60_000,
    ),
)

# Per-call override
verdict = client.classify(text="hi", retry_attempts=1)  # one-shot
```

### Go

```go
import "github.com/efe-ai/efe-go"

// Defaults — functional options pattern.
client, err := efe.NewClient("efe:443")

// Override.
client, err := efe.NewClient("efe:443",
    efe.WithRetry(efe.RetryConfig{
        MaxAttempts:       5,
        InitialBackoff:    200 * time.Millisecond,
        BackoffMultiplier: 1.5,
    }),
    efe.WithCircuitBreaker(efe.CircuitBreakerConfig{
        FailureThreshold: 10,
        ResetTimeout:     60 * time.Second,
    }),
)

// Per-call : use context with timeout.
ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
defer cancel()
verdict, err := client.Classify(ctx, &efe.ClassifyRequest{...})
```

### TypeScript

```typescript
import { EfeClient, RetryConfig } from "@efe-ai/sdk";

// Defaults.
const client = new EfeClient({ host: "efe:443" });

// Override.
const client = new EfeClient({
  host: "efe:443",
  retry: {
    maxAttempts: 5,
    initialBackoffMs: 200,
    backoffMultiplier: 1.5,
  },
  circuitBreaker: {
    failureThreshold: 10,
    resetTimeoutMs: 60_000,
  },
});

// Per-call : AbortController for cancellation.
const controller = new AbortController();
setTimeout(() => controller.abort(), 5_000);
const verdict = await client.classify(
  { text: "hi", catalogId: "router-v1" },
  { signal: controller.signal },
);
```

### Java

```java
import ai.efe.EfeClient;
import ai.efe.RetryConfig;
import ai.efe.CircuitBreakerConfig;

// Defaults — fluent builder.
EfeClient client = EfeClient.builder()
    .host("efe:443")
    .build();

// Override.
EfeClient client = EfeClient.builder()
    .host("efe:443")
    .retry(RetryConfig.builder()
        .maxAttempts(5)
        .initialBackoff(Duration.ofMillis(200))
        .backoffMultiplier(1.5)
        .build())
    .circuitBreaker(CircuitBreakerConfig.builder()
        .failureThreshold(10)
        .resetTimeout(Duration.ofSeconds(60))
        .build())
    .build();

// Per-call : Deadline via CallOptions or CompletableFuture.orTimeout.
Verdict v = client.classifyAsync("hi", "router-v1")
    .orTimeout(5, TimeUnit.SECONDS)
    .join();
```

## Observability hooks

Chaque retry attempt et chaque circuit state transition doit incrémenter :

```
efe_sdk_retry_total{sdk, reason}
efe_sdk_circuit_breaker_state{sdk} ← gauge
```

Cf. `docs/sdk-otel-conventions.md`.

## Non-retryable codes

Les codes gRPC suivants **ne sont JAMAIS retried** :

* `INVALID_ARGUMENT` (= bug user, retry inutile)
* `NOT_FOUND` (= catalog manquant)
* `PERMISSION_DENIED` (= auth invalide)
* `UNAUTHENTICATED` (= token expiré)
* `FAILED_PRECONDITION` (= bundle non chargé)
* `OUT_OF_RANGE` (= seed invalide)

## Anti-patterns

| Pattern | Pourquoi mauvais |
|---|---|
| Retry sur `INVALID_ARGUMENT` | Amplifie le bug, masque la cause root |
| No jitter | Thundering herd lors d'un fail backend |
| Backoff linear (non-exponentiel) | Aggregation des retries en pic |
| Circuit breaker per-host (= pas per-method) | Une méthode lente bloque les rapides |

## Pin test

`test_phi_c_sdk_convention_docs_exist` vérifie ce doc existe.
