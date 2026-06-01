# `efe-sdk-java` — Spring Boot WebFlux example

Reactive Spring WebFlux + `Mono.fromFuture(client.classifyAsync(...))`
bridge to expose the SDK as non-blocking endpoints.

## Run

```bash
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
./mvnw spring-boot:run
```

## Endpoints

Same as the MVC example — `POST /api/classify` + `GET /api/healthz`.
The difference : controllers return `Mono<ResponseEntity<?>>` so the
JVM thread isn't blocked while the gRPC RPC is in flight (Netty
event-loop friendly).

## See also

* [SDK README](../../README.md)
* [`spring-boot-mvc` example](../spring-boot-mvc/)
* [`spring-boot-actuator` example](../spring-boot-actuator/)
