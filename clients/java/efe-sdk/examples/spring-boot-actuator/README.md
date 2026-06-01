# `efe-sdk-java` — Spring Boot Actuator example

Demonstrates the `@AutoConfiguration` starter pattern + Prometheus
metrics expose via `/actuator/prometheus`.

## Run

```bash
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
./mvnw spring-boot:run
```

## Endpoints

* `POST /api/classify` — body `{"text": "..."}`. Returns the SDK Verdict.
* `GET /actuator/prometheus` — Prometheus scrape endpoint exposing
  the 5 canonical SDK metrics (`efe_sdk_classify_total`,
  `efe_sdk_classify_latency_seconds`, etc.) + Spring Boot's own metrics.
* `GET /actuator/health` — health check.

## What this demonstrates

* **`@AutoConfiguration`** : the canonical Spring Boot 3.x starter pattern
  with `META-INF/spring/org.springframework.boot.autoconfigure.AutoConfiguration.imports`.
* **`@ConfigurationProperties("efe")`** : type-safe binding for app properties.
* **`@ConditionalOnMissingBean`** : graceful coexistence with consumer-provided beans.
* **Prometheus integration** : the SDK's Prometheus interceptor wires
  the 5 canonical metrics into the consumer's `PrometheusRegistry` ;
  Spring Boot Actuator exposes them at `/actuator/prometheus`.

## Notes

The standalone `efe-sdk-spring-boot-starter` artifact (publishable
separately) is reported D27. This example demonstrates the exact
pattern that starter would use.

## See also

* [SDK README](../../README.md)
* [`spring-boot-mvc` example](../spring-boot-mvc/)
* [`spring-boot-webflux` example](../spring-boot-webflux/)
