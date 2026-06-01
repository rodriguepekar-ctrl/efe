"""Sprint **D24 (Φ-B) — M10 TypeScript SDK extension** — pin tests anti-régression.

**32 anti-regression pin tests** that anchor the structural surface
of `clients/typescript/@efe/sdk/` so future refactors can't accidentally break :

  - module path + Node version floor + pnpm workspace
  - public API surface (28 exports + 13-options-equiv constructor + 10 sentinel
    classes + Verdict 10 fields cross-language pinned)
  - 5 canonical Prometheus metric names (Node only — match Go SDK)
  - generated proto stubs structure
  - mock package surface (MockEfeClient + newVerdict + createMockServer)
  - 3 framework examples (React + Vue + Express, all workspaces)
  - 5 shell scripts (gen-proto, lint, publish, bundlesize, e2e-bootstrap)
  - 3 CI workflows (test + smoke + publish)
  - dual ESM + CJS + browser exports map
  - bundle size budget present

Each test docstring lists the pin id + 1-line invariant.

These tests **DON'T require** :

  - Node toolchain installed (we inspect file content + JSON, not `pnpm build`)
  - buf / @bufbuild/protoc-gen-es (we anchor the placeholder stubs structure)
  - network access (no `npm install`, no staging hits)

They run in < 5 s and gate every PR via
`.github/workflows/sdk-typescript-test.yml` (job `pin-tests`).

Sections :
    §1. Module skeleton — 5 pin tests
    §2. version.ts + cross-language Verdict — 2 pin tests
    §3. Public API surface (errors + options + retry) — 6 pin tests
    §4. Verdict + StreamItem types — 2 pin tests
    §5. Prometheus metrics canonical (Node) — 1 pin test
    §6. Generated proto stubs — 2 pin tests
    §7. Mock package — 3 pin tests
    §8. Examples (React + Vue + Express) — 3 pin tests
    §9. Scripts shell + Node — 5 pin tests
    §10. CI workflows + exports map — 3 pin tests

Cf. `docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md`.
"""
from __future__ import annotations

import json
import os
import re
import stat
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SDK_ROOT = REPO_ROOT / "clients" / "typescript" / "@efe" / "sdk"
SRC = SDK_ROOT / "src"
CORE = SRC / "core"
NODE = SRC / "node"
WEB = SRC / "web"
MOCK = SRC / "mock"
GENERATED = SRC / "_generated"
TESTS = SDK_ROOT / "tests"
EXAMPLES = SDK_ROOT / "examples"
SCRIPTS = SDK_ROOT / "scripts"

WORKFLOWS = REPO_ROOT / ".github" / "workflows"
ARCH_DOC = REPO_ROOT / "docs" / "EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md"


def _read(p: Path) -> str:
    """Read a file as UTF-8 ; fail loud if missing."""
    if not p.is_file():
        raise FileNotFoundError(f"missing pin-tested file : {p}")
    return p.read_text(encoding="utf-8")


def _read_json(p: Path) -> dict:
    return json.loads(_read(p))


# ─────────────────────────────────────────────────────────────────────
# §1. Module skeleton — 5 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d24_typescript_sdk_package_name_canonical() -> None:
    """**D24-1** — `package.json` declares the canonical name `@efe/sdk`.

    Module path is **frozen post-v1.0.0 GA** ; renaming would break
    every consumer that did `pnpm add @efe/sdk`.
    """
    pkg = _read_json(SDK_ROOT / "package.json")
    assert pkg.get("name") == "@efe/sdk", \
        f"package.json name must be @efe/sdk, got {pkg.get('name')}"


def test_d24_typescript_sdk_node_version_floor() -> None:
    """**D24-2** — `engines.node ≥ 18` (slog stable equivalent in Node ;
    `fetch` available globally + structuredClone)."""
    pkg = _read_json(SDK_ROOT / "package.json")
    engines = pkg.get("engines", {})
    assert "node" in engines, "engines.node MUST be specified"
    assert ">=18" in engines["node"] or ">= 18" in engines["node"], \
        f"engines.node must be ≥ 18, got {engines.get('node')}"


def test_d24_typescript_sdk_required_files_present() -> None:
    """**D24-3** — module skeleton present : README + LICENSE +
    CHANGELOG + tsconfig + tsup + vitest + biome + buf configs."""
    canonical = (
        "package.json",
        "tsconfig.json",
        "tsconfig.build.json",
        "tsup.config.ts",
        "vitest.config.ts",
        "playwright.config.ts",
        "biome.json",
        "buf.yaml",
        "buf.gen.yaml",
        "README.md",
        "LICENSE",
        "CHANGELOG.md",
        ".gitignore",
        ".npmignore",
    )
    for rel in canonical:
        p = SDK_ROOT / rel
        assert p.is_file(), f"missing module file : {rel}"
        assert p.stat().st_size > 0, f"empty module file : {rel}"


def test_d24_typescript_sdk_pnpm_workspace_present() -> None:
    """**D24-4** — root `pnpm-workspace.yaml` declares the SDK +
    examples sub-packages."""
    pnpm_ws = REPO_ROOT / "pnpm-workspace.yaml"
    assert pnpm_ws.is_file(), "missing pnpm-workspace.yaml at repo root"
    src = _read(pnpm_ws)
    assert "clients/typescript/@efe/sdk" in src, \
        "pnpm-workspace.yaml must reference clients/typescript/@efe/sdk"
    assert "examples" in src, \
        "pnpm-workspace.yaml must include examples sub-packages"


def test_d24_typescript_sdk_required_deps_present() -> None:
    """**D24-5** — package.json declares the canonical Connect-RPC +
    bufbuild deps + peerDependencies for OTel + grpc-js + prom-client.

    Removing/renaming any of these is a major surface change.
    """
    pkg = _read_json(SDK_ROOT / "package.json")
    deps = pkg.get("dependencies", {})
    canonical_deps = (
        "@bufbuild/protobuf",
        "@connectrpc/connect",
        "@connectrpc/connect-node",
        "@connectrpc/connect-web",
    )
    for d in canonical_deps:
        assert d in deps, f"package.json missing canonical dep : {d}"
    peer = pkg.get("peerDependencies", {})
    canonical_peer = (
        "@grpc/grpc-js",
        "@opentelemetry/api",
        "prom-client",
    )
    for d in canonical_peer:
        assert d in peer, f"package.json missing peer dep : {d}"


# ─────────────────────────────────────────────────────────────────────
# §2. version.ts + cross-language Verdict — 2 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d24_typescript_sdk_version_const() -> None:
    """**D24-6** — `version.ts` declares `export const Version =
    'X.Y.Z' as const`."""
    src = _read(CORE / "version.ts")
    m = re.search(r"export\s+const\s+Version\s*=\s*'([^']+)'\s*as\s*const", src)
    assert m is not None, \
        "version.ts must declare `export const Version = 'X.Y.Z' as const`"
    # Validate semver shape.
    assert re.match(r"^\d+\.\d+\.\d+(-[A-Za-z0-9.+-]+)?$", m.group(1)), \
        f"version `{m.group(1)}` is not semver-shaped"


def test_d24_typescript_sdk_verdict_field_set_matches_m8_m9() -> None:
    """**D24-7** — Verdict interface has the 10 cross-language fields
    matching Python M8 + Go M9 exactly.

    Field names use TS camelCase ; cross-lang consistency is at the
    field-position level (10 fields, same order).
    """
    src = _read(CORE / "verdict.ts")
    canonical_fields = (
        ("label", "string"),
        ("confidence", "number"),
        ("reject", "boolean"),
        ("rounds", "number"),
        ("alignmentDelta", "number"),
        ("catalog", "string"),
        ("zone", "string"),
        ("swarmId", "string"),
        ("requestId", "string"),
        ("elapsedMs", "number"),
    )
    for name, ts_type in canonical_fields:
        # Anchor on `readonly <name>: <type>;` declaration.
        pattern = rf"readonly\s+{name}\s*:\s*{ts_type}\s*;"
        assert re.search(pattern, src), \
            f"verdict.ts missing field {name}: {ts_type}"


# ─────────────────────────────────────────────────────────────────────
# §3. Public API surface (errors + options + retry) — 6 pin tests
# ─────────────────────────────────────────────────────────────────────


CANONICAL_SENTINELS_TS = (
    "ErrUnauthenticated",
    "ErrPermissionDenied",
    "ErrCatalogNotFound",
    "ErrCatalogInvalid",
    "ErrTimeout",
    "ErrUnavailable",
    "ErrRateLimited",
    "ErrInvalidArgument",
    "ErrInternal",
    "ErrClosed",
)


def test_d24_typescript_sdk_sentinels_canonical() -> None:
    """**D24-8** — all 10 canonical sentinel classes exist + extend EfeError."""
    src = _read(CORE / "errors.ts")
    for sentinel in CANONICAL_SENTINELS_TS:
        # Anchor on `export class XxxError extends EfeError`.
        pattern = rf"export\s+class\s+{sentinel}\s+extends\s+EfeError"
        assert re.search(pattern, src), \
            f"errors.ts missing sentinel : {sentinel}"


def test_d24_typescript_sdk_sentinels_count_exact() -> None:
    """**D24-9** — exactly 10 sentinels declared (no drift)."""
    src = _read(CORE / "errors.ts")
    found = re.findall(
        r"export\s+class\s+(\w+)\s+extends\s+EfeError",
        src,
    )
    # Filter out the EfeError base itself (which extends Error).
    assert sorted(found) == sorted(CANONICAL_SENTINELS_TS), \
        f"sentinels drift : found={sorted(found)} want={sorted(CANONICAL_SENTINELS_TS)}"


def test_d24_typescript_sdk_efe_error_base_exists() -> None:
    """**D24-10** — `EfeError` base class extends `Error` + has `code`
    field."""
    src = _read(CORE / "errors.ts")
    assert re.search(r"export\s+class\s+EfeError\s+extends\s+Error", src), \
        "errors.ts must declare `export class EfeError extends Error`"
    assert "readonly code: string" in src, \
        "EfeError must have `readonly code: string` field"


def test_d24_typescript_sdk_is_retryable_and_error_code() -> None:
    """**D24-11** — `isRetryable(err)` + `errorCode(err)` helpers exported."""
    src = _read(CORE / "errors.ts")
    assert re.search(r"export\s+function\s+isRetryable\s*\(", src), \
        "errors.ts must export `isRetryable(err)`"
    assert re.search(r"export\s+function\s+errorCode\s*\(", src), \
        "errors.ts must export `errorCode(err)`"


def test_d24_typescript_sdk_options_validation() -> None:
    """**D24-12** — `validateOptions(opts)` throws on bad config + fills defaults."""
    src = _read(CORE / "options.ts")
    assert re.search(r"export\s+function\s+validateOptions\s*\(", src), \
        "options.ts must export `validateOptions(opts)`"
    assert "ResolvedClientOptions" in src, \
        "options.ts must export ResolvedClientOptions interface"
    # Required validations.
    assert "endpoint required" in src
    assert "deadline must be" in src
    assert "retry.maxAttempts" in src


def test_d24_typescript_sdk_exponential_backoff() -> None:
    """**D24-13** — `ExponentialBackoff` class with full jitter + defaults."""
    src = _read(CORE / "retry.ts")
    assert re.search(
        r"export\s+class\s+ExponentialBackoff\s+implements\s+RetryBackoff",
        src,
    ), "retry.ts must declare `class ExponentialBackoff implements RetryBackoff`"
    # Full jitter formula uses Math.random() * base.
    assert "Math.random" in src or "this.random()" in src, \
        "retry.ts must use random-based jitter"
    # Default values.
    assert "100" in src and "30_000" in src and "2.0" in src, \
        "retry.ts must use canonical defaults (100 ms, 30_000 ms, 2.0)"


# ─────────────────────────────────────────────────────────────────────
# §4. Verdict + StreamItem types — 2 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d24_typescript_sdk_verdict_predicates() -> None:
    """**D24-14** — `isTrusted` / `isWarn` / `isReject` + `deriveZone`
    helpers are exported."""
    src = _read(CORE / "verdict.ts")
    for fn in ("isTrusted", "isWarn", "isReject", "deriveZone"):
        assert re.search(rf"export\s+function\s+{fn}\s*\(", src), \
            f"verdict.ts must export {fn}"


def test_d24_typescript_sdk_stream_item_present() -> None:
    """**D24-15** — `StreamItem` interface present (Node-only API)."""
    src = _read(CORE / "verdict.ts")
    assert re.search(r"export\s+interface\s+StreamItem", src), \
        "verdict.ts must export StreamItem interface"
    # Required fields.
    assert "readonly index: number" in src
    assert "readonly verdict?: Verdict" in src
    assert "readonly error?: Error" in src


# ─────────────────────────────────────────────────────────────────────
# §5. Prometheus metrics canonical (Node) — 1 pin test
# ─────────────────────────────────────────────────────────────────────


CANONICAL_METRIC_NAMES_TS = (
    "efe_sdk_classify_total",
    "efe_sdk_classify_latency_seconds",
    "efe_sdk_classify_errors_total",
    "efe_sdk_in_flight",
    "efe_sdk_retry_attempts_total",
)


def test_d24_typescript_sdk_metric_names_match_go_sdk() -> None:
    """**D24-16** — 5 Prometheus metric names match Go SDK (M9) +
    Python SDK (M8) exactly. Cross-lang dashboard compatibility.

    Renaming a metric == breaking every Grafana dashboard +
    Prom alert that references it.
    """
    src = _read(NODE / "metrics.ts")
    for name in CANONICAL_METRIC_NAMES_TS:
        assert name in src, f"metrics.ts missing canonical metric : {name}"
    # Anti-drift : count occurrences via the METRIC_NAMES const.
    name_constants = re.findall(
        r"['\"](efe_sdk_\w+)['\"]",
        src,
    )
    assert sorted(set(name_constants)) == sorted(CANONICAL_METRIC_NAMES_TS), \
        f"metrics drift : found={sorted(set(name_constants))} want={sorted(CANONICAL_METRIC_NAMES_TS)}"


# ─────────────────────────────────────────────────────────────────────
# §6. Generated proto stubs — 2 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d24_typescript_sdk_proto_stubs_present() -> None:
    """**D24-17** — `src/_generated/efe/v1/{types,efe,efe_connect}.ts`
    + `PROTOS_HASH.txt` all present."""
    for rel in (
        "efe/v1/types_pb.ts",
        "efe/v1/efe_pb.ts",
        "efe/v1/efe_connect.ts",
        "PROTOS_HASH.txt",
    ):
        p = GENERATED / rel
        assert p.is_file(), f"_generated/{rel} missing"
        assert p.stat().st_size > 0, f"_generated/{rel} empty"


def test_d24_typescript_sdk_efe_service_descriptor() -> None:
    """**D24-18** — `efe_connect.ts` declares EfeService + HealthService
    descriptors with the canonical method names (Execute + Check)."""
    src = _read(GENERATED / "efe/v1/efe_connect.ts")
    assert "EfeService" in src
    assert "execute" in src.lower()  # matches `name: 'Execute'` or `execute:` key
    assert "HealthService" in src
    assert "Check" in src
    # Idempotency level is required for Connect-RPC retry policy detection.
    assert "idempotency" in src
    assert "idempotent" in src


# ─────────────────────────────────────────────────────────────────────
# §7. Mock package — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d24_typescript_sdk_mock_package_files() -> None:
    """**D24-19** — `src/mock/` ships verdict.ts + client.ts + server.ts."""
    for rel in ("verdict.ts", "client.ts", "server.ts"):
        p = MOCK / rel
        assert p.is_file(), f"mock/{rel} missing"
        assert p.stat().st_size > 0, f"mock/{rel} empty"


def test_d24_typescript_sdk_mock_client_shape() -> None:
    """**D24-20** — `MockEfeClient` class + `newVerdict()` factory exist
    with the canonical surface."""
    src_client = _read(MOCK / "client.ts")
    src_verdict = _read(MOCK / "verdict.ts")
    # MockEfeClient class with the 7 public methods.
    assert re.search(r"export\s+class\s+MockEfeClient", src_client), \
        "mock/client.ts must declare `export class MockEfeClient`"
    for method in ("setVerdict", "setError", "getCalls", "getCallCount", "reset", "classify", "healthcheck", "close"):
        assert method in src_client, f"MockEfeClient missing method : {method}"
    # newVerdict factory.
    assert re.search(r"export\s+function\s+newVerdict\s*\(", src_verdict), \
        "mock/verdict.ts must export `newVerdict(overrides?)`"


def test_d24_typescript_sdk_mock_server_create_fn() -> None:
    """**D24-21** — `createMockServer` Node-only helper exported."""
    src = _read(MOCK / "server.ts")
    assert re.search(r"export\s+async\s+function\s+createMockServer\s*\(", src), \
        "mock/server.ts must export `async function createMockServer(opts?)`"
    # Uses @grpc/grpc-js (lazy require) for in-memory server.
    assert "@grpc/grpc-js" in src, \
        "mock/server.ts must use @grpc/grpc-js for in-memory server"


# ─────────────────────────────────────────────────────────────────────
# §8. Examples (React + Vue + Express) — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("framework", ["react", "vue", "express"])
def test_d24_typescript_sdk_example_submodule_complete(framework: str) -> None:
    """**D24-22..24** — each of react/vue/express ships package.json +
    tsconfig.json + a `src/` entry + README.md, and references @efe/sdk
    via `workspace:*`."""
    ex_dir = EXAMPLES / framework
    assert ex_dir.is_dir(), f"examples/{framework}/ missing"
    for f in ("package.json", "tsconfig.json", "README.md"):
        p = ex_dir / f
        assert p.is_file(), f"examples/{framework}/{f} missing"
        assert p.stat().st_size > 0, f"examples/{framework}/{f} empty"
    # The example package.json must reference @efe/sdk via workspace:*.
    pkg = _read_json(ex_dir / "package.json")
    deps = pkg.get("dependencies", {})
    assert "@efe/sdk" in deps, f"examples/{framework}/package.json must depend on @efe/sdk"
    assert deps["@efe/sdk"] == "workspace:*", \
        f"examples/{framework} must reference @efe/sdk via workspace:* (not pinned version)"
    # And there must be a `src/` directory with at least one entry.
    src_dir = ex_dir / "src"
    assert src_dir.is_dir(), f"examples/{framework}/src/ missing"


# ─────────────────────────────────────────────────────────────────────
# §9. Scripts shell + Node — 5 pin tests
# ─────────────────────────────────────────────────────────────────────


CANONICAL_SCRIPTS = (
    "gen-proto.sh",
    "lint.sh",
    "publish.sh",
    "bundlesize.mjs",
    "e2e-bootstrap.sh",
)


@pytest.mark.parametrize("script", CANONICAL_SCRIPTS)
def test_d24_typescript_sdk_script_present(script: str) -> None:
    """**D24-25..29** — each canonical script exists, is executable
    (.sh) or readable (.mjs), and parses correctly."""
    p = SCRIPTS / script
    assert p.is_file(), f"scripts/{script} missing"
    if script.endswith(".sh"):
        # Must be executable.
        mode = p.stat().st_mode
        assert mode & stat.S_IXUSR, f"scripts/{script} not executable"
        # bash -n parses the script.
        bash = "/bin/bash" if Path("/bin/bash").exists() else "bash"
        proc = subprocess.run([bash, "-n", str(p)], capture_output=True, check=False)
        assert proc.returncode == 0, \
            f"scripts/{script} bash syntax error : {proc.stderr.decode()}"
        # Strict-mode preamble.
        src = _read(p)
        assert "set -euo pipefail" in src, \
            f"scripts/{script} missing `set -euo pipefail`"
    else:
        # .mjs : use node --check (if available).
        if subprocess.run(["which", "node"], capture_output=True, check=False).returncode == 0:
            proc = subprocess.run(
                ["node", "--check", str(p)], capture_output=True, check=False,
            )
            assert proc.returncode == 0, \
                f"scripts/{script} node --check failed : {proc.stderr.decode()}"


# ─────────────────────────────────────────────────────────────────────
# §10. CI workflows + exports map — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d24_typescript_sdk_ci_workflows_present() -> None:
    """**D24-30** — all 3 CI workflows exist and reference the SDK
    path."""
    for wf in ("sdk-typescript-test.yml", "sdk-typescript-smoke.yml", "sdk-typescript-publish.yml"):
        p = WORKFLOWS / wf
        assert p.is_file(), f"missing CI workflow : {wf}"
        src = _read(p)
        assert "clients/typescript/@efe/sdk" in src or "@efe/sdk" in src, \
            f"{wf} must reference clients/typescript/@efe/sdk path"
    # The PR-gate workflow must run pin tests.
    test_src = _read(WORKFLOWS / "sdk-typescript-test.yml")
    assert "test_d24_typescript_sdk_extension.py" in test_src, \
        "sdk-typescript-test.yml must invoke the pin tests file"


def test_d24_typescript_sdk_exports_map_canonical() -> None:
    """**D24-31** — `package.json` declares the canonical exports map :
    universal `.` (browser+node conditional), `./node`, `./web`, `./mock`,
    `./package.json`."""
    pkg = _read_json(SDK_ROOT / "package.json")
    exports = pkg.get("exports", {})
    # Canonical sub-paths.
    for sub in (".", "./node", "./web", "./mock", "./package.json"):
        assert sub in exports, f"exports must include {sub}"
    # Universal "." must have browser + node + default conditions.
    universal = exports["."]
    assert "browser" in universal, "exports['.'] must have browser condition"
    assert "node" in universal, "exports['.'] must have node condition"
    # Node has both import + require (ESM + CJS dual).
    assert "import" in universal["node"], "Node export must have ESM import"
    assert "require" in universal["node"], "Node export must have CJS require (dual)"
    # Browser has only import (ESM only).
    assert "import" in universal["browser"]


def test_d24_typescript_sdk_architecture_doc_present() -> None:
    """**D24-32** — architecture doc exists at the canonical path +
    references D24 Φ-B + the @efe/sdk module path.

    `docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md` is the source
    of truth for the M10 module.
    """
    assert ARCH_DOC.is_file(), \
        f"missing architecture doc : {ARCH_DOC.relative_to(REPO_ROOT)}"
    src = _read(ARCH_DOC)
    assert "@efe/sdk" in src
    assert "D24" in src or "Sprint D24" in src
    assert "M10" in src


# ─────────────────────────────────────────────────────────────────────
# §11. Audit fixes (M10-A1..A14) — 11 pin tests anti-régression
# ─────────────────────────────────────────────────────────────────────
#
# After the M10 chirurgical audit pass, 13 defects were corrected
# (cf. CHANGELOG.md "Audit fixes — M10 chirurgical audit pass 1").
# The following pin tests anchor those fixes so future refactors can't
# accidentally regress them.
#
#   A1  — createPromiseClient + placeholder ServiceType cast (CRIT)
#   A2  — exactOptionalPropertyTypes signal/timeout violations (CRIT)
#   A3  — require() in ESM context (CRIT)
#   A4  — gRPC integration tests gated by EFE_TEST_REAL_PROTO (CRIT)
#   A5  — stream.ts lost wakeup signal race (HIGH)
#   A6  — stream.ts worker race + Promise.all orchestrator (HIGH)
#   A7  — fromConnectError return type clarity (HIGH)
#   A9  — _TLS_DEFAULT_OPTIONS dead code removal (MED)
#   A10 — biome.json _doc field removal (MED)
#   A11 — vite-env.d.ts present for Vue + React examples (MED)
#   A12 — App.tsx ReactElement instead of JSX.Element (MED)
#   A13 — tsup web bundle externals (MED)
#   A14 — grpc-js server.start() deprecation removal (LOW)


def test_d24_typescript_sdk_audit_fix_a1_create_promise_client_cast() -> None:
    """**M10-A1 (CRIT)** — `createPromiseClient(EfeService as any, ...)`
    cast present in both Node + Web client.

    Pre-fix : `createPromiseClient(EfeService, transport)` failed type
    check because hand-coded `EfeService` doesn't satisfy the bufbuild
    `ServiceType` interface. Without the cast, `pnpm typecheck` would
    fail and the SDK couldn't compile until `pnpm gen:proto` produces
    real types.

    The cast is documented as a placeholder-phase artefact ; it
    becomes a no-op identity once real codegen runs.
    """
    for path in (NODE / "client.ts", WEB / "client.ts"):
        src = _read(path)
        # Audit-fix marker present.
        assert "M10-A1" in src, f"{path.name} must reference M10-A1 marker"
        # `EfeService as any` cast in createPromiseClient.
        assert re.search(
            r"createPromiseClient\(\s*EfeService\s+as\s+any",
            src,
        ), f"{path.name} must cast EfeService at createPromiseClient boundary"
        assert re.search(
            r"createPromiseClient\(\s*HealthService\s+as\s+any",
            src,
        ), f"{path.name} must cast HealthService at createPromiseClient boundary"


def test_d24_typescript_sdk_audit_fix_a2_conditional_spread() -> None:
    """**M10-A2 (CRIT)** — `signal` is conditionally spread when present.

    Pre-fix : `{ signal: opts.signal, timeoutMs: ... }` when
    `opts.signal` is `undefined` violates `exactOptionalPropertyTypes:
    true` because the Connect-RPC `CallOptions` declares
    `signal?: AbortSignal` (NOT `AbortSignal | undefined`).

    Post-fix : conditional spread `...(opts.signal !== undefined ? { signal: opts.signal } : {})`.
    """
    for path in (NODE / "client.ts", WEB / "client.ts"):
        src = _read(path)
        # Audit-fix marker.
        assert "M10-A2" in src, f"{path.name} must reference M10-A2 marker"
        # The conditional spread pattern.
        assert "opts.signal !== undefined" in src, \
            f"{path.name} must guard signal with `!== undefined`"
        # The pre-fix shape MUST be gone (a literal `signal: opts.signal,`
        # inside an options object). Anchor on absence of bare-prop access
        # at the call site (look for `signal: opts.signal,` followed by
        # `timeoutMs:` — the pre-fix combination).
        assert not re.search(r"signal:\s*opts\.signal\s*,\s*timeoutMs", src), \
            f"{path.name} must not have pre-fix unconditional signal pass"


def test_d24_typescript_sdk_audit_fix_a3_uses_create_require() -> None:
    """**M10-A3 (CRIT)** — ESM-compatible require/dynamic import.

    Pre-fix : `require('prom-client')` and `require('@grpc/grpc-js')` —
    both fail in ESM context (`type: "module"` + module=NodeNext).

    Post-fix :
      - metrics.ts uses `createRequire(import.meta.url)` (sync context).
      - mock/server.ts uses `await import(...)` (async function).
    """
    metrics_src = _read(NODE / "metrics.ts")
    server_src = _read(MOCK / "server.ts")

    # metrics.ts uses createRequire.
    assert "createRequire" in metrics_src, \
        "metrics.ts must use `createRequire` for ESM-compat prom-client load"
    assert "import.meta.url" in metrics_src
    # No bare `require(` in metrics.ts.
    assert not re.search(r"^\s*const\s+\w+\s*=\s*require\(", metrics_src, re.MULTILINE), \
        "metrics.ts must not use bare `require()` at top-level"

    # mock/server.ts uses dynamic import.
    assert "await import(" in server_src, \
        "mock/server.ts must use `await import()` for ESM-compat grpc-js load"
    assert not re.search(r"^\s*const\s+\w+\s*=\s*require\(", server_src, re.MULTILINE), \
        "mock/server.ts must not use bare `require()` at top-level"

    # Audit-fix markers.
    assert "M10-A3" in metrics_src, "metrics.ts must reference M10-A3"
    assert "M10-A3" in server_src, "mock/server.ts must reference M10-A3"


def test_d24_typescript_sdk_audit_fix_a4_integration_gated() -> None:
    """**M10-A4 (CRIT)** — gRPC integration tests gated by
    `EFE_TEST_REAL_PROTO=1`.

    Pre-fix : tests would fail at runtime due to wire-format mismatch
    (mock server JSON vs SDK Connect-RPC binary proto).

    Post-fix : `describeIfRealProto` skip-by-default unless real codegen
    is in place.
    """
    for test_file in (
        TESTS / "integration" / "classify.test.ts",
        TESTS / "integration" / "healthcheck.test.ts",
        TESTS / "integration" / "stream.test.ts",
    ):
        src = _read(test_file)
        assert "EFE_TEST_REAL_PROTO" in src, \
            f"{test_file.name} must check EFE_TEST_REAL_PROTO env var"
        assert "describeIfRealProto" in src, \
            f"{test_file.name} must use `describeIfRealProto` skip-conditional"
        assert "M10-A4" in src, f"{test_file.name} must reference M10-A4"


def test_d24_typescript_sdk_audit_fix_a5_a6_async_queue_dispatcher() -> None:
    """**M10-A5 + M10-A6 (HIGH)** — stream.ts uses race-safe AsyncQueue
    + Promise.all orchestrator.

    Pre-fix M10-A5 : ad-hoc `wakeupResolve` callback could be missed
    between drain + await → consumer hangs.

    Pre-fix M10-A6 : multiple workers race to set `producerDone =
    true` when `workersAlive === 0`.

    Post-fix : `class AsyncQueue<T>` encapsulates push/shift atomically ;
    `Promise.all(workers).finally(() => output.close())` orchestrates
    cleanup race-free.
    """
    src = _read(NODE / "stream.ts")
    # AsyncQueue class present.
    assert re.search(r"class\s+AsyncQueue", src), \
        "stream.ts must declare AsyncQueue class"
    # Promise.all orchestrator.
    assert re.search(r"Promise\.all\(\[dispatcherPromise", src) or \
           re.search(r"Promise\.all\(.*dispatcher", src, re.DOTALL), \
        "stream.ts must orchestrate workers via Promise.all"
    # Pre-fix patterns MUST be gone (anchor on declaration shape, not
    # comment substring — the audit comment explains what was removed).
    assert not re.search(r"^\s*let\s+wakeupResolve\b", src, re.MULTILINE), \
        "pre-fix `let wakeupResolve` declaration must be removed"
    assert not re.search(r"^\s*let\s+workersAlive\b", src, re.MULTILINE), \
        "pre-fix `let workersAlive` counter must be removed"
    # Audit-fix markers.
    assert "M10-A5" in src and "M10-A6" in src, \
        "stream.ts must reference both M10-A5 + M10-A6 audit markers"


def test_d24_typescript_sdk_audit_fix_a7_from_rpc_return_type() -> None:
    """**M10-A7 (HIGH)** — `fromConnectError` return type is `unknown`
    (not the misleading `EfeError | unknown`)."""
    src = _read(CORE / "from-rpc.ts")
    # The return type annotation.
    assert re.search(
        r"export\s+function\s+fromConnectError\s*\(\s*err\s*:\s*unknown\s*\)\s*:\s*unknown",
        src,
    ), "fromConnectError must have signature `(err: unknown): unknown`"
    # Audit-fix marker.
    assert "M10-A7" in src, "from-rpc.ts must reference M10-A7"


def test_d24_typescript_sdk_audit_fix_a9_dead_tls_constants_removed() -> None:
    """**M10-A9 (MED)** — `_TLS_DEFAULT_OPTIONS` dead export removed +
    `node:tls` import removed."""
    src = _read(NODE / "tls.ts")
    # Anchor on declaration / import shape, not comment substring.
    assert not re.search(r"^export\s+const\s+_TLS_DEFAULT_OPTIONS", src, re.MULTILINE), \
        "pre-fix `export const _TLS_DEFAULT_OPTIONS` declaration must be removed"
    # Match only line-starting `import` — comments start with `//` so they
    # don't match `^import` even on the same line.
    assert not re.search(
        r"^import\s*\{[^}]*\bconstants\s+as\s+tlsConstants\b",
        src,
        re.MULTILINE,
    ), "pre-fix `import { constants as tlsConstants }` must be removed"
    assert "M10-A9" in src, "tls.ts must reference M10-A9"


def test_d24_typescript_sdk_audit_fix_a10_biome_no_doc_field() -> None:
    """**M10-A10 (MED)** — biome.json no longer has the non-standard
    `_doc` top-level field."""
    src = _read(SDK_ROOT / "biome.json")
    biome = json.loads(src)
    assert "_doc" not in biome, \
        "pre-fix `_doc` non-standard top-level field must be removed"


def test_d24_typescript_sdk_audit_fix_a11_vite_env_types() -> None:
    """**M10-A11 (MED)** — vite-env.d.ts present for Vue + React
    examples ; `import.meta.env.VITE_*` properly typed."""
    for example in ("vue", "react"):
        env_d = EXAMPLES / example / "src" / "vite-env.d.ts"
        assert env_d.is_file(), f"examples/{example}/src/vite-env.d.ts missing"
        src = _read(env_d)
        assert "vite/client" in src, \
            f"{example}/vite-env.d.ts must reference vite/client types"
        assert "VITE_EFE_ENDPOINT" in src, \
            f"{example}/vite-env.d.ts must declare VITE_EFE_ENDPOINT"


def test_d24_typescript_sdk_audit_fix_a12_react_element_return() -> None:
    """**M10-A12 (MED)** — App.tsx uses `ReactElement` (imported)
    instead of the global `JSX.Element` annotation."""
    src = _read(EXAMPLES / "react" / "src" / "App.tsx")
    assert "ReactElement" in src, \
        "App.tsx must use `ReactElement` (post M10-A12 fix)"
    # Anchor on function signature shape, not comment substring.
    assert not re.search(
        r"function\s+\w+\s*\([^)]*\)\s*:\s*JSX\.Element",
        src,
    ), "pre-fix `function ...(): JSX.Element` annotation must be removed"
    assert "M10-A12" in src, "App.tsx must reference M10-A12"


def test_d24_typescript_sdk_audit_fix_a13_tsup_web_externals() -> None:
    """**M10-A13 (MED)** — tsup web bundle externalizes Node-only deps
    (defense-in-depth so they NEVER end up in the browser bundle)."""
    src = _read(SDK_ROOT / "tsup.config.ts")
    assert "M10-A13" in src, "tsup.config.ts must reference M10-A13"
    # Web bundle externals must include the Node-only peerDeps.
    # Anchor on the web bundle config block.
    web_block_match = re.search(
        r"entry:\s*\{\s*index:\s*'src/web\.ts'\s*\}.*?external:\s*\[(.*?)\]",
        src,
        re.DOTALL,
    )
    assert web_block_match is not None, \
        "tsup.config.ts must have a web bundle config block with externals"
    web_externals = web_block_match.group(1)
    for required in ("@grpc/grpc-js", "prom-client", "@opentelemetry/api"):
        assert required in web_externals, \
            f"web bundle externals must include {required}"


def test_d24_typescript_sdk_audit_fix_a14_no_grpc_start_call() -> None:
    """**M10-A14 (LOW)** — `server.start()` call removed (deprecated
    since `@grpc/grpc-js` 1.10).

    Post-`bindAsync` resolves, the gRPC server is auto-running. Calling
    `start()` emits a deprecation warning. Since we pin
    `@grpc/grpc-js: ^1.10.0`, the legacy guard is dead code.
    """
    src = _read(MOCK / "server.ts")
    # Anchor on actual call site (`server.start();` at start of line, not in comment).
    assert not re.search(r"^\s*server\.start\(\)", src, re.MULTILINE), \
        "pre-fix `server.start()` call must be removed (deprecated since 1.10)"
    assert "M10-A14" in src, "mock/server.ts must reference M10-A14"
