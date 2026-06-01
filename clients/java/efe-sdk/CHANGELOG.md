# Changelog — efe-sdk-java

All notable changes documented per [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) +
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Mapping** — `ai.efe:efe-sdk-java:vX.Y.Z` ↔ EFE platform sprint
> Dx (cf. repo `CHANGELOG.md`). Until `v1.0.0` GA, this artifact ships
> on private GitHub Packages (`https://maven.pkg.github.com/efe-ai/efe`).
> From `v1.0.0` onwards, it goes on Maven Central via Sonatype Central
> Portal.

## [0.4.0] — 2026-04-30 — Sprint D26 (Φ-B) — Initial release

### Added

* `EfeClient` (final class, AutoCloseable) — main entry point.
  Builder pattern : `EfeClient.builder().endpoint("...").auth(...).build()`.
  Sync (`classify`) + async (`classifyAsync`) + stream (`classifyStream`)
  + healthcheck APIs.
* `Verdict` record — 10 fields cross-language pinned (matches Python
  M8 + Go M9 + TS M10).
* `StreamItem` record (Node-only-equivalent : Java has full streaming).
* 10 sentinel exception classes (`EfeException` base extends
  `RuntimeException` ; 10 final subclasses : `EfeUnauthenticated`,
  `EfePermissionDenied`, `EfeCatalogNotFound`, `EfeCatalogInvalid`,
  `EfeTimeout`, `EfeUnavailable`, `EfeRateLimited`,
  `EfeInvalidArgument`, `EfeInternal`, `EfeClosed`Exceptions).
* `EfeException.isRetryable(Throwable)` + `EfeException.errorCode(Throwable)`
  static helpers.
* `ExponentialBackoff` with full jitter (universal — same algorithm
  as Go/TS SDKs).
* `JwtAuth` + `MtlsAuth` + `NoAuth` records implementing `sealed
  interface AuthConfig` (Java 17 sealed types — exhaustive pattern
  matching at compile time).
* `JwtCallCredentials` (gRPC `CallCredentials` impl) for dynamic JWT
  injection per RPC.
* `MtlsBuilder.build(MtlsAuth)` returns `SslContext` via
  `GrpcSslContexts.forClient()` with TLS 1.3 enforced.
* `OtelClientInterceptor` (universal soft dep via `provided` scope).
* `SdkMetrics` + 5 Prometheus collectors (canonical names match
  Go/TS/Python SDKs).
* `MockEfeClient` (pure Java fixture, no gRPC) + `MockEfeServer`
  (`InProcessServerBuilder` wrapper) + `VerdictFactory` (test
  helpers in `efe-sdk-java-mock` module).
* 3 Spring Boot examples : MVC (`@RestController` + sync) + WebFlux
  (`Mono.fromFuture(client.classifyAsync)`) + Actuator
  (`@AutoConfiguration` + Prometheus expose).
* JPMS `module-info.java` — explicit exports for `ai.efe.sdk`,
  `ai.efe.sdk.auth`, `ai.efe.sdk.exception`, `ai.efe.sdk.retry`.
  `internal/` package NOT exported.
* Maven Central pipeline : `release` profile signs JARs (GPG) +
  attaches sources/javadoc + stages via `central-publishing-maven-plugin`.

### Limitations

* GraalVM native image config NOT shipped — deferred D28+.
* Spring Boot starter (`@EnableEfeClient` auto-config) NOT publishable
  — D26 demonstrates the pattern via the `spring-boot-actuator` example.
  Standalone starter artifact reported D27.
* Android target NOT supported (uses `grpc-netty-shaded` which is
  server-side ; Android needs `grpc-okhttp`).
* Java < 17 NOT supported (uses records + sealed interfaces).
