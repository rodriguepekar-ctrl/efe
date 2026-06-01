"""Sprint **D26 (Φ-B) — M11 Java SDK extension** — pin tests anti-régression.

**32 anti-regression pin tests** that anchor the structural surface
of `clients/java/efe-sdk/` so future refactors can't accidentally break :

  - Maven multi-module skeleton (parent POM + 2 modules)
  - Public API surface (10 sentinels, Verdict 10 fields, Builder, sealed AuthConfig)
  - 5 canonical Prometheus metric names (cross-language Python/Go/TS/Java)
  - Generated proto stubs (placeholder phase) structure
  - 3 Spring Boot examples (MVC + WebFlux + Actuator)
  - 4 shell scripts (gen-proto, lint, publish, e2e-bootstrap)
  - 3 CI workflows (test + smoke + publish)
  - Maven Central pipeline (release profile + GPG plugin + central-publishing)
  - JPMS module-info present + exports canonical

Each test docstring lists the pin id + 1-line invariant.

These tests **DON'T require** :

  - JDK / Maven installed (we inspect file content only)
  - protoc installed (we anchor the placeholder stubs structure)
  - network access

They run in < 5 s and gate every PR via `.github/workflows/sdk-java-test.yml`.

Sections :
    §1. Maven skeleton — 5 pin tests
    §2. Version + cross-lang Verdict — 2 pin tests
    §3. Public API surface (errors + auth + retry) — 6 pin tests
    §4. Verdict + StreamItem records — 2 pin tests
    §5. Prometheus metrics canonical — 1 pin test
    §6. Generated proto stubs — 2 pin tests
    §7. Mock module — 3 pin tests
    §8. Spring Boot examples — 3 pin tests
    §9. Scripts shell — 4 pin tests
    §10. CI workflows + Maven Central pipeline — 4 pin tests

Cf. `docs/EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md`.
"""
from __future__ import annotations

import re
import stat
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SDK_ROOT = REPO_ROOT / "clients" / "java" / "efe-sdk"
PARENT_POM = SDK_ROOT / "pom.xml"
CORE_POM = SDK_ROOT / "efe-sdk-core" / "pom.xml"
MOCK_POM = SDK_ROOT / "efe-sdk-mock" / "pom.xml"
CORE_SRC = SDK_ROOT / "efe-sdk-core" / "src" / "main" / "java" / "ai" / "efe" / "sdk"
MOCK_SRC = SDK_ROOT / "efe-sdk-mock" / "src" / "main" / "java" / "ai" / "efe" / "sdk" / "mock"
GENERATED = SDK_ROOT / "efe-sdk-core" / "src" / "main" / "java" / "efe" / "v1"
TESTS = SDK_ROOT / "efe-sdk-core" / "src" / "test" / "java" / "ai" / "efe" / "sdk"
EXAMPLES = SDK_ROOT / "examples"
SCRIPTS = SDK_ROOT / "scripts"

WORKFLOWS = REPO_ROOT / ".github" / "workflows"
ARCH_DOC = REPO_ROOT / "docs" / "EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md"


def _read(p: Path) -> str:
    if not p.is_file():
        raise FileNotFoundError(f"missing pin-tested file : {p}")
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# §1. Maven skeleton — 5 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d26_java_sdk_parent_pom_canonical() -> None:
    """**D26-1** — parent POM declares the canonical groupId/artifactId/version.

    The groupId+artifactId pair is **frozen post-v1.0.0 GA** ; any
    rename breaks every Maven Central consumer.
    """
    src = _read(PARENT_POM)
    assert re.search(r"<groupId>ai\.efe</groupId>", src), \
        "parent POM groupId must be ai.efe"
    assert re.search(r"<artifactId>efe-sdk-java-parent</artifactId>", src), \
        "parent POM artifactId must be efe-sdk-java-parent"
    assert re.search(r"<packaging>pom</packaging>", src), \
        "parent POM must declare packaging=pom"


def test_d26_java_sdk_java_version_floor() -> None:
    """**D26-2** — Java 17+ release floor (records + sealed interfaces require 17)."""
    src = _read(PARENT_POM)
    assert re.search(r"<maven\.compiler\.release>17</maven\.compiler\.release>", src), \
        "parent POM must declare maven.compiler.release=17"


def test_d26_java_sdk_required_files_present() -> None:
    """**D26-3** — module skeleton present : LICENSE + README + CHANGELOG + .gitignore."""
    for rel in ("README.md", "LICENSE", "CHANGELOG.md", ".gitignore", "pom.xml"):
        p = SDK_ROOT / rel
        assert p.is_file(), f"missing module file : {rel}"
        assert p.stat().st_size > 0, f"empty module file : {rel}"


def test_d26_java_sdk_two_modules_present() -> None:
    """**D26-4** — parent POM declares 2 modules : efe-sdk-core + efe-sdk-mock."""
    src = _read(PARENT_POM)
    assert re.search(r"<module>efe-sdk-core</module>", src), \
        "parent POM must include efe-sdk-core module"
    assert re.search(r"<module>efe-sdk-mock</module>", src), \
        "parent POM must include efe-sdk-mock module"
    assert CORE_POM.is_file(), "efe-sdk-core/pom.xml missing"
    assert MOCK_POM.is_file(), "efe-sdk-mock/pom.xml missing"


def test_d26_java_sdk_grpc_netty_shaded_canonical_dep() -> None:
    """**D26-5** — efe-sdk-core uses io.grpc:grpc-netty-shaded (canonical for prod Java).

    Pre-shaded Netty + boringssl + ALPN — zero conflict with consumer
    Netty versions. Pin-tested to prevent accidental switch to non-shaded.
    """
    src = _read(CORE_POM)
    assert re.search(
        r"<artifactId>grpc-netty-shaded</artifactId>",
        src,
    ), "efe-sdk-core must depend on grpc-netty-shaded"
    # Ensure non-shaded grpc-netty is NOT in deps.
    assert not re.search(
        r"<artifactId>grpc-netty</artifactId>",
        src,
    ), "efe-sdk-core must NOT use non-shaded grpc-netty (would conflict with consumer Netty)"


# ─────────────────────────────────────────────────────────────────────
# §2. Version + cross-lang Verdict — 2 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d26_java_sdk_version_const() -> None:
    """**D26-6** — Version.VERSION is semver-shaped."""
    src = _read(CORE_SRC / "Version.java")
    m = re.search(r'public\s+static\s+final\s+String\s+VERSION\s*=\s*"([^"]+)";', src)
    assert m is not None, "Version.java must declare `public static final String VERSION`"
    assert re.match(r"^\d+\.\d+\.\d+(-[A-Za-z0-9.+-]+)?$", m.group(1)), \
        f"version `{m.group(1)}` is not semver-shaped"


CANONICAL_VERDICT_FIELDS = (
    ("label", "String"),
    ("confidence", "double"),
    ("reject", "boolean"),
    ("rounds", "int"),
    ("alignmentDelta", "double"),
    ("catalog", "String"),
    ("zone", "String"),
    ("swarmId", "String"),
    ("requestId", "String"),
    ("elapsedMs", "long"),
)


def test_d26_java_sdk_verdict_field_set_matches_others() -> None:
    """**D26-7** — Verdict record has the 10 cross-language fields matching
    Python M8 + Go M9 + TS M10 SDKs exactly.

    camelCase Java field names. Adding/removing requires coordinated
    bump across the 4 SDKs + the wire contract proto.
    """
    src = _read(CORE_SRC / "Verdict.java")
    for name, java_type in CANONICAL_VERDICT_FIELDS:
        # Anchor on `<type> <name>,?` inside the record header.
        pattern = rf"\b{java_type}\s+{name}\s*[,)]"
        assert re.search(pattern, src), \
            f"Verdict record missing field {name}: {java_type}"


# ─────────────────────────────────────────────────────────────────────
# §3. Public API surface (errors + auth + retry) — 6 pin tests
# ─────────────────────────────────────────────────────────────────────


CANONICAL_SENTINELS_JAVA = (
    "EfeUnauthenticatedException",
    "EfePermissionDeniedException",
    "EfeCatalogNotFoundException",
    "EfeCatalogInvalidException",
    "EfeTimeoutException",
    "EfeUnavailableException",
    "EfeRateLimitedException",
    "EfeInvalidArgumentException",
    "EfeInternalException",
    "EfeClosedException",
)


def test_d26_java_sdk_sentinels_canonical() -> None:
    """**D26-8** — all 10 sentinel exception classes exist + extend EfeException."""
    for sentinel in CANONICAL_SENTINELS_JAVA:
        path = CORE_SRC / "exception" / f"{sentinel}.java"
        assert path.is_file(), f"exception/{sentinel}.java missing"
        src = _read(path)
        assert re.search(
            rf"public\s+final\s+class\s+{sentinel}\s+extends\s+EfeException",
            src,
        ), f"{sentinel} must extend EfeException"


def test_d26_java_sdk_sentinels_count_exact() -> None:
    """**D26-9** — exactly 10 sentinels in the exception/ folder + 1 base
    EfeException.java. Exclude the base from the sentinel count.
    """
    exception_dir = CORE_SRC / "exception"
    assert exception_dir.is_dir()
    # All Efe*Exception.java EXCLUDING the base class itself.
    files = sorted(
        p.name for p in exception_dir.glob("Efe*Exception.java")
        if p.name != "EfeException.java"
    )
    expected = sorted(f"{s}.java" for s in CANONICAL_SENTINELS_JAVA)
    assert "EfeException.java" in [p.name for p in exception_dir.glob("*.java")], \
        "EfeException base class missing"
    assert files == expected, \
        f"sentinels drift : found={files} want={expected}"


def test_d26_java_sdk_efe_exception_base_extends_runtime() -> None:
    """**D26-10** — EfeException base extends RuntimeException (unchecked) +
    has code() + isRetryable + errorCode static helpers."""
    src = _read(CORE_SRC / "exception" / "EfeException.java")
    assert re.search(
        r"public\s+class\s+EfeException\s+extends\s+RuntimeException",
        src,
    ), "EfeException must extend RuntimeException (unchecked)"
    assert "public String code()" in src, "EfeException must expose code()"
    assert re.search(r"public\s+static\s+boolean\s+isRetryable\s*\(", src), \
        "EfeException must expose static isRetryable(Throwable)"
    assert re.search(r"public\s+static\s+String\s+errorCode\s*\(", src), \
        "EfeException must expose static errorCode(Throwable)"


def test_d26_java_sdk_auth_config_sealed_interface() -> None:
    """**D26-11** — AuthConfig is a sealed interface permitting exactly
    JwtAuth, MtlsAuth, NoAuth (Java 17 sealed types)."""
    src = _read(CORE_SRC / "auth" / "AuthConfig.java")
    assert re.search(
        r"public\s+sealed\s+interface\s+AuthConfig\s+permits\s+JwtAuth\s*,\s*MtlsAuth\s*,\s*NoAuth",
        src,
    ), "AuthConfig must be `sealed interface ... permits JwtAuth, MtlsAuth, NoAuth`"


def test_d26_java_sdk_exponential_backoff() -> None:
    """**D26-12** — ExponentialBackoff class with full jitter + canonical defaults."""
    src = _read(CORE_SRC / "retry" / "ExponentialBackoff.java")
    assert re.search(
        r"public\s+final\s+class\s+ExponentialBackoff\s+implements\s+RetryBackoff",
        src,
    ), "ExponentialBackoff must implement RetryBackoff"
    # Full jitter formula uses `random.nextDouble()` * base.
    assert "random.nextDouble()" in src, \
        "ExponentialBackoff must use random.nextDouble() for full jitter"


def test_d26_java_sdk_retry_config_record() -> None:
    """**D26-13** — RetryConfig is a record with `exponential` + `disabled` factories."""
    src = _read(CORE_SRC / "retry" / "RetryConfig.java")
    assert re.search(r"public\s+record\s+RetryConfig\s*\(", src), \
        "RetryConfig must be a record"
    assert "RetryConfig exponential(" in src, \
        "RetryConfig.exponential factory must exist"
    assert "RetryConfig disabled(" in src, \
        "RetryConfig.disabled factory must exist"


# ─────────────────────────────────────────────────────────────────────
# §4. Verdict + StreamItem records — 2 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d26_java_sdk_verdict_record_predicates() -> None:
    """**D26-14** — Verdict record has isTrusted + isWarn + isReject + deriveZone."""
    src = _read(CORE_SRC / "Verdict.java")
    assert re.search(r"public\s+record\s+Verdict\s*\(", src), \
        "Verdict must be a record"
    for method in ("isTrusted", "isWarn", "isReject"):
        assert re.search(rf"public\s+boolean\s+{method}\s*\(\s*\)", src), \
            f"Verdict must expose {method}()"
    assert re.search(r"public\s+static\s+String\s+deriveZone\s*\(", src), \
        "Verdict must expose static deriveZone(double)"


def test_d26_java_sdk_stream_item_record() -> None:
    """**D26-15** — StreamItem record present + exactly-one-non-null invariant."""
    src = _read(CORE_SRC / "StreamItem.java")
    assert re.search(r"public\s+record\s+StreamItem\s*\(", src), \
        "StreamItem must be a record"
    # Exactly-one-non-null check.
    assert "exactly one of verdict / error" in src, \
        "StreamItem must enforce exactly-one-non-null invariant"


# ─────────────────────────────────────────────────────────────────────
# §5. Prometheus metrics canonical — 1 pin test
# ─────────────────────────────────────────────────────────────────────


CANONICAL_METRIC_NAMES_JAVA = (
    "efe_sdk_classify_total",
    "efe_sdk_classify_latency_seconds",
    "efe_sdk_classify_errors_total",
    "efe_sdk_in_flight",
    "efe_sdk_retry_attempts_total",
)


def test_d26_java_sdk_metric_names_match_other_sdks() -> None:
    """**D26-16** — 5 Prometheus metric names match Python M8 + Go M9 + TS M10
    SDKs exactly. Cross-lang dashboard compatibility."""
    src = _read(CORE_SRC / "prometheus" / "SdkMetrics.java")
    for name in CANONICAL_METRIC_NAMES_JAVA:
        assert name in src, f"SdkMetrics.java missing canonical metric : {name}"


# ─────────────────────────────────────────────────────────────────────
# §6. Generated proto stubs — 2 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d26_java_sdk_proto_stubs_present() -> None:
    """**D26-17** — `efe/v1/{Types,Efe,SwarmServiceGrpc}.java` placeholder
    stubs + PROTOS_HASH.txt all present."""
    for rel in ("Types.java", "Efe.java", "SwarmServiceGrpc.java"):
        p = GENERATED / rel
        assert p.is_file(), f"_generated/efe/v1/{rel} missing"
        assert p.stat().st_size > 0
    hash_file = SDK_ROOT / "efe-sdk-core" / "src" / "main" / "proto" / "PROTOS_HASH.txt"
    assert hash_file.is_file(), "PROTOS_HASH.txt missing (drift anchor)"


def test_d26_java_sdk_swarm_service_grpc_present() -> None:
    """**D26-18** — `SwarmServiceGrpc` declares BlockingStub + FutureStub
    + the canonical `Execute` method."""
    src = _read(GENERATED / "SwarmServiceGrpc.java")
    assert "SwarmServiceBlockingStub" in src
    assert "SwarmServiceFutureStub" in src
    assert "newBlockingStub" in src
    assert "newFutureStub" in src
    assert "METHOD_EXECUTE" in src or "execute" in src.lower()


# ─────────────────────────────────────────────────────────────────────
# §7. Mock module — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d26_java_sdk_mock_module_files() -> None:
    """**D26-19** — efe-sdk-mock ships MockEfeClient + MockEfeServer + VerdictFactory."""
    for rel in ("MockEfeClient.java", "MockEfeServer.java", "VerdictFactory.java"):
        p = MOCK_SRC / rel
        assert p.is_file(), f"mock/{rel} missing"
        assert p.stat().st_size > 0


def test_d26_java_sdk_mock_efe_client_surface() -> None:
    """**D26-20** — MockEfeClient has setVerdict/setError/getCalls/reset/classify/healthcheck/close."""
    src = _read(MOCK_SRC / "MockEfeClient.java")
    for method in ("setVerdict", "setError", "getCalls", "getCallCount", "reset",
                    "classify", "classifyAsync", "healthcheck", "close"):
        assert method in src, f"MockEfeClient missing method : {method}"


def test_d26_java_sdk_mock_server_in_process() -> None:
    """**D26-21** — MockEfeServer uses InProcessServerBuilder + AutoCloseable."""
    src = _read(MOCK_SRC / "MockEfeServer.java")
    assert "InProcessServerBuilder" in src, \
        "MockEfeServer must use InProcessServerBuilder"
    assert "implements AutoCloseable" in src, \
        "MockEfeServer must implement AutoCloseable"
    assert re.search(r"public\s+(static\s+)?MockEfeServer\s+create\s*\(\s*\)", src), \
        "MockEfeServer must expose static create()"


# ─────────────────────────────────────────────────────────────────────
# §8. Spring Boot examples — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("example", ["spring-boot-mvc", "spring-boot-webflux", "spring-boot-actuator"])
def test_d26_java_sdk_example_complete(example: str) -> None:
    """**D26-22..24** — each Spring Boot example has pom.xml + Application.java + README + application.yml."""
    ex_dir = EXAMPLES / example
    assert ex_dir.is_dir(), f"examples/{example}/ missing"

    for f in ("pom.xml", "README.md"):
        p = ex_dir / f
        assert p.is_file(), f"examples/{example}/{f} missing"
        assert p.stat().st_size > 0

    # Application.java present.
    java_files = list((ex_dir / "src" / "main" / "java").rglob("Application.java"))
    assert len(java_files) >= 1, f"examples/{example} must have an Application.java"

    # application.yml present.
    yml = list((ex_dir / "src" / "main" / "resources").glob("application.y*ml"))
    assert len(yml) >= 1, f"examples/{example} must have application.yml"

    # POM references efe-sdk-java.
    pom_src = _read(ex_dir / "pom.xml")
    assert re.search(r"<artifactId>efe-sdk-java</artifactId>", pom_src), \
        f"examples/{example}/pom.xml must depend on efe-sdk-java"


# ─────────────────────────────────────────────────────────────────────
# §9. Scripts shell — 4 pin tests
# ─────────────────────────────────────────────────────────────────────


CANONICAL_SCRIPTS = ("gen-proto.sh", "lint.sh", "publish.sh", "e2e-bootstrap.sh")


@pytest.mark.parametrize("script", CANONICAL_SCRIPTS)
def test_d26_java_sdk_script_present_and_executable(script: str) -> None:
    """**D26-25..28** — each canonical script exists, is executable, has bash syntax + strict mode."""
    p = SCRIPTS / script
    assert p.is_file(), f"scripts/{script} missing"
    mode = p.stat().st_mode
    assert mode & stat.S_IXUSR, f"scripts/{script} not executable"
    bash = "/bin/bash" if Path("/bin/bash").exists() else "bash"
    proc = subprocess.run([bash, "-n", str(p)], capture_output=True, check=False)
    assert proc.returncode == 0, \
        f"scripts/{script} bash syntax error : {proc.stderr.decode()}"
    src = _read(p)
    assert "set -euo pipefail" in src, \
        f"scripts/{script} missing `set -euo pipefail` strict-mode preamble"


# ─────────────────────────────────────────────────────────────────────
# §10. CI workflows + Maven Central pipeline — 4 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d26_java_sdk_ci_workflows_present() -> None:
    """**D26-29** — all 3 CI workflows exist and reference clients/java/efe-sdk."""
    for wf in ("sdk-java-test.yml", "sdk-java-smoke.yml", "sdk-java-publish.yml"):
        p = WORKFLOWS / wf
        assert p.is_file(), f"missing CI workflow : {wf}"
        src = _read(p)
        assert "clients/java/efe-sdk" in src, \
            f"{wf} must reference clients/java/efe-sdk path"
    # The PR-gate workflow must run pin tests.
    test_src = _read(WORKFLOWS / "sdk-java-test.yml")
    assert "test_d26_java_sdk_extension.py" in test_src, \
        "sdk-java-test.yml must invoke the pin tests file"


def test_d26_java_sdk_maven_central_pipeline_canonical() -> None:
    """**D26-30** — parent POM declares the `release` profile with required
    plugins (source + javadoc + GPG + central-publishing-maven-plugin)."""
    src = _read(PARENT_POM)
    # Release profile present.
    assert re.search(r"<id>release</id>", src), \
        "parent POM must declare a `release` profile"
    # Required plugins for Maven Central.
    for required in (
        "maven-source-plugin",
        "maven-javadoc-plugin",
        "maven-gpg-plugin",
        "central-publishing-maven-plugin",
    ):
        assert required in src, \
            f"release profile must declare {required}"


def test_d26_java_sdk_module_info_present() -> None:
    """**D26-31** — JPMS module-info.java present with canonical exports."""
    module_info = SDK_ROOT / "efe-sdk-core" / "src" / "main" / "java" / "module-info.java"
    assert module_info.is_file(), "efe-sdk-core/src/main/java/module-info.java missing"
    src = _read(module_info)
    assert re.search(r"module\s+ai\.efe\.sdk\s*\{", src), \
        "module-info.java must declare module ai.efe.sdk"
    # Canonical exports.
    for pkg in ("ai.efe.sdk", "ai.efe.sdk.auth", "ai.efe.sdk.exception",
                "ai.efe.sdk.retry", "ai.efe.sdk.otel", "ai.efe.sdk.prometheus"):
        assert re.search(rf"exports\s+{re.escape(pkg)}\s*;", src), \
            f"module-info.java must export {pkg}"
    # internal package NOT exported.
    assert not re.search(r"^\s*exports\s+ai\.efe\.sdk\.internal\s*;", src, re.MULTILINE), \
        "module-info.java must NOT export ai.efe.sdk.internal"


def test_d26_java_sdk_architecture_doc_present() -> None:
    """**D26-32** — architecture doc exists at the canonical path."""
    assert ARCH_DOC.is_file(), \
        f"missing architecture doc : {ARCH_DOC.relative_to(REPO_ROOT)}"
    src = _read(ARCH_DOC)
    assert "efe-sdk-java" in src
    assert "D26" in src
    assert "M11" in src


# ─────────────────────────────────────────────────────────────────────
# §11. Audit fixes (M11-A1..A5) — 5 pin tests anti-régression
# ─────────────────────────────────────────────────────────────────────
#
# After the M11 chirurgical audit pass, 5 defects were corrected
# (cf. CHANGELOG.md "Audit fixes — M11 chirurgical audit pass 1") :
#
#   A1 — StreamImpl Java 21 virtual threads → Java 17 fixed pool (CRIT)
#   A2 — Placeholder messages need Serializable for marshaller (CRIT)
#   A3 — Integration tests circular dep core ↔ mock (CRIT)
#   A4 — Unused ProtoUtils import (MED)
#   A5 — ErrorMapper boilerplate simplification (MED)


def test_d26_java_sdk_audit_fix_a1_no_virtual_threads() -> None:
    """**M11-A1 (CRIT)** — StreamImpl uses ThreadPoolExecutor (Java 17
    compat) instead of `Executors.newVirtualThreadPerTaskExecutor()`
    (Java 21+).

    Pre-fix : the project targets Java 17 LTS but used a Java 21 API
    → `mvn compile` would fail with "method newVirtualThreadPerTaskExecutor
    not found".
    """
    src = _read(CORE_SRC / "internal" / "StreamImpl.java")
    # Anchor on the actual call site (`Executors.newVirtualThreadPerTaskExecutor()`)
    # NOT the audit comment that mentions it. Code line starts with `=` (assignment).
    assert not re.search(
        r"=\s*Executors\.newVirtualThreadPerTaskExecutor\(\)",
        src,
    ), "pre-fix `Executors.newVirtualThreadPerTaskExecutor()` call site (Java 21+) must be removed"
    assert "M11-A1" in src, "StreamImpl.java must reference M11-A1 marker"
    assert "setDaemon(true)" in src, \
        "StreamImpl.java must use a daemon thread factory (so JVM can exit)"
    # The replacement uses ThreadPoolExecutor.
    assert "new ThreadPoolExecutor(" in src, \
        "StreamImpl.java must use ThreadPoolExecutor (Java 17 compat)"


def test_d26_java_sdk_audit_fix_a2_placeholders_serializable() -> None:
    """**M11-A2 (CRIT)** — placeholder messages implement Serializable
    so the placeholder marshaller (Java Serialization round-trip)
    works at runtime.

    Pre-fix : `SerializableHolder` wrapper tried to serialize
    non-Serializable inner objects → NotSerializableException at
    runtime when integration tests call client.classify().
    """
    types_src = _read(GENERATED / "Types.java")
    efe_src = _read(GENERATED / "Efe.java")
    grpc_src = _read(GENERATED / "SwarmServiceGrpc.java")

    # ConeRef + DecodedOutput in Types.java implement Serializable.
    assert re.search(
        r"class\s+ConeRef\s+implements\s+Serializable",
        types_src,
    ), "ConeRef must implement Serializable"
    assert re.search(
        r"class\s+DecodedOutput\s+implements\s+Serializable",
        types_src,
    ), "DecodedOutput must implement Serializable"

    # ExecuteRequest + ExecuteResponse implement Serializable.
    assert re.search(
        r"class\s+ExecuteRequest\s+implements\s+Serializable",
        efe_src,
    ), "ExecuteRequest must implement Serializable"
    assert re.search(
        r"class\s+ExecuteResponse\s+implements\s+Serializable",
        efe_src,
    ), "ExecuteResponse must implement Serializable"

    # The dummy SerializableHolder wrapper MUST be removed.
    assert not re.search(
        r"^\s*(?:private\s+)?(?:static\s+)?(?:final\s+)?class\s+SerializableHolder",
        grpc_src,
        re.MULTILINE,
    ), "pre-fix SerializableHolder wrapper class must be removed"

    assert "M11-A2" in types_src or "M11-A2" in efe_src, \
        "placeholder messages must reference M11-A2 marker"


def test_d26_java_sdk_audit_fix_a3_integration_tests_in_mock_module() -> None:
    """**M11-A3 (CRIT)** — integration tests live in efe-sdk-mock
    (NOT efe-sdk-core) to break the circular dep.

    Pre-fix : tests in core imported `ai.efe.sdk.mock.MockEfeServer`
    but core had no test-scope dep on mock (would create a cycle :
    mock depends on core, core's tests depend on mock → Maven reactor
    cannot resolve).

    Fix : move integration tests to efe-sdk-mock/src/test/java/ where
    MockEfeServer is in the same module's main scope.
    """
    mock_integration_dir = SDK_ROOT / "efe-sdk-mock" / "src" / "test" / "java" / "ai" / "efe" / "sdk" / "mock" / "integration"
    assert mock_integration_dir.is_dir(), \
        "integration tests must be in efe-sdk-mock/src/test/java/ai/efe/sdk/mock/integration/"

    for name in ("ClassifyIntegrationTest.java", "HealthcheckIntegrationTest.java", "StreamIntegrationTest.java"):
        p = mock_integration_dir / name
        assert p.is_file(), f"integration test {name} must be in mock module"
        src = _read(p)
        assert re.search(
            r"^package\s+ai\.efe\.sdk\.mock\.integration\s*;",
            src,
            re.MULTILINE,
        ), f"{name} must declare `package ai.efe.sdk.mock.integration;`"

    # And NOT in core.
    core_integration_dir = (
        SDK_ROOT / "efe-sdk-core" / "src" / "test" / "java" / "ai" / "efe" / "sdk" / "integration"
    )
    assert not core_integration_dir.exists(), \
        "pre-fix integration tests directory in core must be removed"


def test_d26_java_sdk_audit_fix_a4_no_unused_proto_utils_import() -> None:
    """**M11-A4 (MED)** — `import io.grpc.protobuf.ProtoUtils` removed
    from SwarmServiceGrpc.java (was unused — placeholder uses Java
    Serialization, not ProtoUtils)."""
    src = _read(GENERATED / "SwarmServiceGrpc.java")
    # Anchor on actual import statement (not the comment that mentions it).
    assert not re.search(
        r"^import\s+io\.grpc\.protobuf\.ProtoUtils\s*;",
        src,
        re.MULTILINE,
    ), "pre-fix `import io.grpc.protobuf.ProtoUtils` must be removed"
    assert "M11-A4" in src, "SwarmServiceGrpc.java must reference M11-A4"


def test_d26_java_sdk_audit_fix_a5_error_mapper_simplified() -> None:
    """**M11-A5 (MED)** — ErrorMapper.java boilerplate simplified.

    Pre-fix : 44 lines of `cause != null ? new X(msg, cause) : new X(msg)`
    ternaries (one per gRPC status code).

    Post-fix : sentinel (msg, cause) constructors accept null cause
    cleanly → switch arms become single expressions.
    """
    src = _read(CORE_SRC / "internal" / "ErrorMapper.java")
    assert not re.search(
        r"cause\s*!=\s*null\s*\?\s*new\s+Efe",
        src,
    ), "pre-fix `cause != null ? new EfeXxx(...) : new EfeXxx(...)` ternary must be removed"
    assert "M11-A5" in src, "ErrorMapper.java must reference M11-A5"
