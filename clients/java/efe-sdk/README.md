# `efe-sdk-java` — official Java SDK for EFE

> **Sprint D26 (Φ-B)** — official Java/JVM SDK for the EFE (Essaim
> Fluide Éphémère) inference platform.
>
> **Status** : Beta (`v0.4.0` private during Φ-B). GA target `v1.0.0`
> post-D40 — published on Maven Central.
> **Java version** : 17 LTS minimum
> **License** : Apache-2.0
> **Wire contract** : `efe.v1` proto family — same `.proto` as Python
> M8, Go M9, TypeScript M10 SDKs.

---

## Install

### Private (during Φ-B) — GitHub Packages

`~/.m2/settings.xml` :

```xml
<servers>
  <server>
    <id>github-efe-ai</id>
    <username>${env.GITHUB_USERNAME}</username>
    <password>${env.GITHUB_TOKEN}</password>
  </server>
</servers>
```

`pom.xml` :

```xml
<repositories>
  <repository>
    <id>github-efe-ai</id>
    <url>https://maven.pkg.github.com/efe-ai/efe</url>
  </repository>
</repositories>

<dependency>
  <groupId>ai.efe</groupId>
  <artifactId>efe-sdk-java</artifactId>
  <version>0.4.0</version>
</dependency>
```

### Public (post-GA) — Maven Central

```xml
<dependency>
  <groupId>ai.efe</groupId>
  <artifactId>efe-sdk-java</artifactId>
  <version>1.0.0</version>
</dependency>
```

---

## Quickstart

```java
import ai.efe.sdk.EfeClient;
import ai.efe.sdk.Verdict;
import ai.efe.sdk.auth.JwtAuth;
import java.time.Duration;

public class Demo {
    public static void main(String[] args) {
        try (EfeClient client = EfeClient.builder()
                .endpoint(System.getenv("EFE_ENDPOINT"))
                .auth(JwtAuth.of(System.getenv("EFE_JWT_TOKEN")))
                .deadline(Duration.ofSeconds(10))
                .build()) {

            Verdict verdict = client.classify("I want a refund");
            System.out.printf("label=%s confidence=%.2f zone=%s%n",
                verdict.label(), verdict.confidence(), verdict.zone());
        }
    }
}
```

`try-with-resources` closes the underlying gRPC channel automatically
(idiomatic Java). Subsequent calls after `close()` throw `EfeClosedException`.

---

## Public API surface

### Classes

| Class | Purpose |
|---|---|
| `EfeClient` | Main entry point. Builder pattern + AutoCloseable. |
| `EfeClient.Builder` | Static nested builder for configuration. |
| `MockEfeClient` (mock module) | Drop-in replacement for unit tests. |
| `MockEfeServer` (mock module) | In-process gRPC server for integration tests. |
| `ExponentialBackoff` | Retry backoff impl. |

### Records (immutable)

| Record | Description |
|---|---|
| `Verdict` | classification verdict (10 fields, immutable) |
| `StreamItem` | one item in `classifyStream` |
| `RetryConfig` | retry policy |
| `JwtAuth` / `MtlsAuth` | auth strategies |

### Sealed interface

`AuthConfig permits JwtAuth, MtlsAuth, NoAuth` — exhaustive pattern
matching at construction time.

### Exceptions (10 sentinels)

All extend `EfeException extends RuntimeException` (unchecked) :

| Class | gRPC code | Retryable |
|---|---|:-:|
| `EfeUnauthenticatedException` | UNAUTHENTICATED (16) | no |
| `EfePermissionDeniedException` | PERMISSION_DENIED (7) | no |
| `EfeCatalogNotFoundException` | NOT_FOUND (5) | no |
| `EfeCatalogInvalidException` | FAILED_PRECONDITION (9) | no |
| `EfeTimeoutException` | DEADLINE_EXCEEDED (4) | yes |
| `EfeUnavailableException` | UNAVAILABLE (14) | yes |
| `EfeRateLimitedException` | RESOURCE_EXHAUSTED (8) | yes |
| `EfeInvalidArgumentException` | INVALID_ARGUMENT (3) | no |
| `EfeInternalException` | INTERNAL (13) / UNKNOWN (2) | no |
| `EfeClosedException` | (local) | no |

```java
import ai.efe.sdk.exception.*;

try {
    Verdict v = client.classify(text);
} catch (EfeUnauthenticatedException e) {
    // 401 — refresh token
} catch (EfeRateLimitedException e) {
    // 429 — backoff
} catch (EfeException e) {
    if (EfeException.isRetryable(e)) {
        // generic retryable handler
    }
}
```

### Static helpers

| Method | Purpose |
|---|---|
| `EfeException.isRetryable(Throwable)` | Returns true iff err ∈ retryable subset. |
| `EfeException.errorCode(Throwable)` | Returns canonical short label (for metrics). |

---

## Three Spring Boot examples

```
clients/java/efe-sdk/examples/
├── spring-boot-mvc/         # Spring MVC + RestController (synchronous)
├── spring-boot-webflux/     # WebFlux + Mono.fromFuture (reactive)
└── spring-boot-actuator/    # Auto-config + Prometheus expose
```

Run any of them :

```bash
cd examples/spring-boot-mvc
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
./mvnw spring-boot:run
```

---

## Mock for unit tests

```java
import ai.efe.sdk.mock.MockEfeClient;
import ai.efe.sdk.mock.VerdictFactory;

class MyHandlerTest {
    @Test
    void handlerProcessesVerdict() {
        var mock = new MockEfeClient();
        mock.setVerdict(VerdictFactory.newVerdict()
            .withLabel("fraud")
            .withConfidence(0.92));

        myHandler.process(mock);

        assertThat(mock.getCalls()).containsExactly("expected input");
    }
}
```

For tests that need the FULL gRPC stack (interceptors, retry, OTel),
use `MockEfeServer` :

```java
try (var server = MockEfeServer.create();
     var client = EfeClient.builder().endpoint(server.getEndpoint()).build()) {

    server.onClassify(req -> ExecuteResponse.newBuilder()
        .setOutput(DecodedOutput.newBuilder().setText("ok"))
        .setConfidence(0.9f)
        .setVerdict(efe.v1.Types.Verdict.VERDICT_CONVERGED)
        .build());

    Verdict v = client.classify("test");
}
```

---

## Development

```bash
cd clients/java/efe-sdk

# 1. Generate proto stubs (one-time + on proto changes).
./scripts/gen-proto.sh

# 2. Build + test + JaCoCo coverage report.
./mvnw clean verify

# 3. Type-check / lint (Checkstyle + SpotBugs + Javadoc).
./scripts/lint.sh

# 4. Build all 3 examples (separate Maven projects).
for ex in examples/*/; do (cd "$ex" && ./mvnw package -DskipTests); done
```

---

## SLOs

| ID | Target |
|:--:|---|
| `SLO-JAVA-1` | `EfeClient.builder().build()` cold start < 100 ms (post JIT) |
| `SLO-JAVA-2` | `classify` p99 ≤ server p99 + 50 ms overhead |
| `SLO-JAVA-3` | Core JAR ≤ 500 KB ; with `grpc-netty-shaded` ≤ 5 MB |
| `SLO-JAVA-4` | Java 17+ supported |
| `SLO-JAVA-5` | Coverage ≥ 80% lines (JaCoCo) |
| `SLO-JAVA-6` | Maven Central publishable (signed JARs + canonical POM) |
| `SLO-JAVA-7` | Javadoc clean (0 warning) |
| `SLO-JAVA-8` | JPMS module-info clean |

---

## Cross-language consistency

| Aspect | Python (M8) | Go (M9) | TypeScript (M10) | Java (M11) |
|---|---|---|---|---|
| Module path | `clients/python/efe_tool/` | `clients/go/efe-sdk/` | `clients/typescript/@efe/sdk/` | `clients/java/efe-sdk/` |
| Public type | `Verdict @dataclass(frozen=True)` | `type Verdict struct {...}` | `interface Verdict {...}` | `record Verdict(...)` |
| Sentinel base | `EfeError(Exception)` | `var Err* = errors.New(...)` | `class EfeError extends Error` | `class EfeException extends RuntimeException` |
| 10 sentinels | ✓ | ✓ | ✓ | ✓ |
| 5 Prom metrics | ✓ | ✓ | ✓ Node only | ✓ |
| OTel auto | ✓ | ✓ | ✓ universal | ✓ |
| Retry policy | ✓ | ✓ | ✓ Node only | ✓ |
| mTLS support | ✓ | ✓ | ✓ Node only | ✓ |

Pin test `test_d26_java_sdk_verdict_field_set_matches_others` anchors
that the 10-field Verdict matches across the 4 SDKs.

---

## References

* [`docs/EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md`](../../../docs/EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md)
  — sprint D26 architecture (~3 027 lignes).
* [`protos/efe/v1/`](../../../protos/efe/v1/) — wire contract.
* Sister SDK docs : Python M8, Go M9, TypeScript M10.
