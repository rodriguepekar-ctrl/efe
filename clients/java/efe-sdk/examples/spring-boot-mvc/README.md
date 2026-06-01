# `efe-sdk-java` — Spring Boot MVC example

Demonstrates the canonical EFE SDK integration in a Spring Boot MVC
(synchronous) REST controller.

## Run

```bash
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
./mvnw spring-boot:run
```

## Endpoints

* `POST /api/classify` — body `{"text": "..."}`. Returns the SDK
  Verdict as JSON, status 200 on success or 4xx/5xx mapped from SDK
  sentinels.
* `GET /api/healthz` — probes the EFE server's gRPC Health endpoint.

## Test

```bash
curl -X POST http://localhost:8080/api/classify \
    -H 'Content-Type: application/json' \
    -d '{"text": "I want a refund"}'

curl http://localhost:8080/api/healthz
```

## See also

* [SDK README](../../README.md)
* [`spring-boot-webflux` example](../spring-boot-webflux/) — reactive
* [`spring-boot-actuator` example](../spring-boot-actuator/) — auto-config
