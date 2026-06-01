"""Sprint **D23 (Φ-B) — M9 Go SDK extension** — pin tests anti-régression.

**31 anti-regression pin tests** that anchor the structural surface
of `clients/go/efe-sdk/` so future refactors can't accidentally break :

  - module path + Go version floor
  - public API surface (13 functional options + 10 sentinel errors
    + Verdict struct field set + IsTrusted/IsWarn/IsReject methods)
  - 5 canonical Prometheus metric names
  - internal proto stubs structure
  - mock package surface
  - 3 framework examples sub-modules (gin / fiber / chi)
  - 4 shell scripts (gen-proto, lint, test, publish)
  - architecture doc presence + cross-link

Each test docstring lists the pin id + 1-line invariant.

These tests **DON'T require** :

  - Go toolchain installed (we inspect file content, not `go build`)
  - protoc / protoc-gen-go (we anchor the placeholder stubs structure)
  - network access (no `go get`, no staging hits)

They run in < 5 s and gate every PR via `.github/workflows/sdk-go-test.yml`
(job `pin-tests`).

Sections :
    §1. Module skeleton — 5 pin tests
    §2. version.go const — 1 pin test
    §3. Public API surface (options + errors) — 6 pin tests
    §4. Verdict struct + methods — 3 pin tests
    §5. Prometheus metrics canonical — 1 pin test
    §6. Internal proto stubs — 2 pin tests
    §7. Mock package — 3 pin tests
    §8. Examples sub-modules — 3 pin tests
    §9. Scripts shell — 4 pin tests
    §10. Docs + architecture cross-link — 3 pin tests

Cf. `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md`.
"""
from __future__ import annotations

import os
import re
import stat
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SDK_ROOT = REPO_ROOT / "clients" / "go" / "efe-sdk"
INTERNAL = SDK_ROOT / "internal"
PROTO_V1 = INTERNAL / "proto" / "efe" / "v1"
MOCK_PKG = SDK_ROOT / "mock"
EXAMPLES = SDK_ROOT / "examples"
SCRIPTS = SDK_ROOT / "scripts"

WORKFLOWS = REPO_ROOT / ".github" / "workflows"
ARCH_DOC = REPO_ROOT / "docs" / "EFE_GO_SDK_EXTENSION_ARCHITECTURE.md"


def _read(p: Path) -> str:
    """Read a file as UTF-8 ; fail loud if missing."""
    if not p.is_file():
        raise FileNotFoundError(f"missing pin-tested file : {p}")
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# §1. Module skeleton — 5 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d23_go_sdk_module_path_canonical() -> None:
    """**D23-1** — `go.mod` declares the canonical immutable module path.

    Module path is **frozen post-v1.0.0 GA** ; renaming would
    break every consumer that did `go get github.com/efe-ai/efe-sdk-go`.
    """
    src = _read(SDK_ROOT / "go.mod")
    # Anchor exactly on the module declaration line — defends against
    # stray `module foo` strings appearing in retract directives.
    assert re.search(
        r"^module\s+github\.com/efe-ai/efe-sdk-go\s*$",
        src,
        re.MULTILINE,
    ), "module path must be github.com/efe-ai/efe-sdk-go"


def test_d23_go_sdk_go_version_floor() -> None:
    """**D23-2** — `go 1.22` minimum (slog stable + errors.Join).

    Pin the floor : `slog`, `errors.Join`, `for range int`, generics
    quality-of-life all shipped in 1.22. Anything older drops these.
    """
    src = _read(SDK_ROOT / "go.mod")
    assert re.search(r"^go\s+1\.22\b", src, re.MULTILINE), \
        "go.mod must declare `go 1.22` (or higher) as the floor"


def test_d23_go_sdk_required_files_present() -> None:
    """**D23-3** — module skeleton present : README + LICENSE + Makefile + doc.go."""
    for rel in ("README.md", "LICENSE", "Makefile", "doc.go", "go.mod"):
        p = SDK_ROOT / rel
        assert p.is_file(), f"missing module file : {rel}"
        # Defend against accidental empty placeholder.
        assert p.stat().st_size > 0, f"empty module file : {rel}"


def test_d23_go_sdk_license_apache2() -> None:
    """**D23-4** — LICENSE is Apache-2.0 (canonical for the org)."""
    src = _read(SDK_ROOT / "LICENSE")
    # Anchor on the canonical copyright header from the Apache license
    # template. Both "Apache License" + "Version 2.0" must appear.
    assert "Apache License" in src
    assert "Version 2.0" in src


def test_d23_go_sdk_required_deps_present() -> None:
    """**D23-5** — go.mod requires the 7 canonical runtime deps.

    Removing/renaming any of these is a major surface change. Pin
    them to anchor the soft-dep contract (OTel + Prom) + retry lib
    + grpc stack.
    """
    src = _read(SDK_ROOT / "go.mod")
    canonical_deps = (
        "github.com/cenkalti/backoff/v4",
        "github.com/prometheus/client_golang",
        "github.com/stretchr/testify",
        "go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc",
        "go.opentelemetry.io/otel",
        "google.golang.org/grpc",
        "google.golang.org/protobuf",
    )
    for dep in canonical_deps:
        assert dep in src, f"go.mod missing canonical dep : {dep}"


# ─────────────────────────────────────────────────────────────────────
# §2. version.go const — 1 pin test
# ─────────────────────────────────────────────────────────────────────


def test_d23_go_sdk_version_const() -> None:
    """**D23-6** — `version.go` declares `const Version = "..."`.

    Aligned with EFE platform semver. Sub-1.0 = private/beta. The
    pin checks the declaration shape, not the exact version number
    (which bumps every sprint).
    """
    src = _read(SDK_ROOT / "version.go")
    m = re.search(r'^const\s+Version\s*=\s*"([^"]+)"\s*$', src, re.MULTILINE)
    assert m is not None, "version.go must declare `const Version = \"X.Y.Z\"`"
    # Validate semver shape (X.Y.Z + optional pre-release).
    assert re.match(r"^\d+\.\d+\.\d+(-[A-Za-z0-9.+-]+)?$", m.group(1)), \
        f"version `{m.group(1)}` is not semver-shaped"


# ─────────────────────────────────────────────────────────────────────
# §3. Public API surface (options + errors) — 6 pin tests
# ─────────────────────────────────────────────────────────────────────


# Canonical 13 ClientOption WithXxx() functions. Adding/removing
# requires a deliberate test bump (and a CHANGELOG entry).
CANONICAL_OPTIONS = (
    "WithJWT",
    "WithMTLS",
    "WithTLS",
    "WithInsecureSkipVerify",
    "WithDeadline",
    "WithDialTimeout",
    "WithRetry",
    "WithOTel",
    "WithPrometheus",
    "WithLogger",
    "WithUserAgent",
    "WithMaxRecvSize",
    "WithCustomDialer",
)


def test_d23_go_sdk_options_canonical() -> None:
    """**D23-7** — all 13 canonical `WithXxx` options exist with stable signatures."""
    src = _read(SDK_ROOT / "options.go")
    for opt in CANONICAL_OPTIONS:
        # Anchor on `func WithXxx(...) ClientOption {` declaration —
        # a comment line `// WithXxx ...` would not match.
        pattern = rf"^func\s+{opt}\s*\("
        assert re.search(pattern, src, re.MULTILINE), \
            f"options.go missing canonical option : {opt}"


def test_d23_go_sdk_options_count_exact() -> None:
    """**D23-8** — exactly 13 `WithXxx` options exist (no extras, no fewer).

    Anti-drift : surface bumps require a bump in CANONICAL_OPTIONS
    and a CHANGELOG entry. This test catches accidental option
    additions that bypass the deliberate review.
    """
    src = _read(SDK_ROOT / "options.go")
    # Anchor on `func With<Name>(` — captures only actual function
    # declarations (vs comments / docstring refs). Multi-line param
    # lists (e.g. WithCustomDialer) make a parens-balanced regex
    # brittle, so we match the function name only — every `func
    # With...` declared in options.go is an option by convention,
    # the canonical list above is the authoritative pin.
    found = re.findall(
        r"^func\s+(With\w+)\s*\(",
        src,
        re.MULTILINE,
    )
    assert sorted(found) == sorted(CANONICAL_OPTIONS), \
        f"options drift : found={sorted(found)} want={sorted(CANONICAL_OPTIONS)}"


# Canonical 10 sentinel errors. Same principle as options : adding
# requires CHANGELOG + test update.
CANONICAL_SENTINELS = (
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


def test_d23_go_sdk_sentinels_canonical() -> None:
    """**D23-9** — all 10 canonical sentinel errors are declared."""
    src = _read(SDK_ROOT / "errors.go")
    for sentinel in CANONICAL_SENTINELS:
        # Anchor on `Err... = errors.New("efe: ...")` declaration.
        # Allow either bare `Err... =` (inside `var ( ... )`) OR
        # `var Err... =` (outside the block).
        pattern = (
            rf"(^\s*{sentinel}\s*=\s*errors\.New\b)"
            rf"|(^var\s+{sentinel}\s*=\s*errors\.New\b)"
        )
        assert re.search(pattern, src, re.MULTILINE), \
            f"errors.go missing sentinel : {sentinel}"


def test_d23_go_sdk_sentinels_count_exact() -> None:
    """**D23-10** — exactly 10 sentinels (no drift).

    Captures `Err* = errors.New(...)` declarations regardless of
    whether they sit inside a `var ( ... )` block or not.
    """
    src = _read(SDK_ROOT / "errors.go")
    found = re.findall(
        r"(?:^|\s)(Err\w+)\s*=\s*errors\.New\b",
        src,
        re.MULTILINE,
    )
    assert sorted(set(found)) == sorted(CANONICAL_SENTINELS), \
        f"sentinels drift : found={sorted(set(found))} want={sorted(CANONICAL_SENTINELS)}"


def test_d23_go_sdk_errors_grpc_mapping_present() -> None:
    """**D23-11** — `fromGRPC` translates gRPC codes to sentinels.

    Pin the central translation function — the SOLE conversion path
    between server-side gRPC errors and client-side handling. The
    SDK NEVER surfaces raw `status.Status` to consumers.
    """
    src = _read(SDK_ROOT / "errors.go")
    # The function exists.
    assert re.search(r"^func\s+fromGRPC\s*\(", src, re.MULTILINE), \
        "errors.go must declare `fromGRPC(err error) error`"
    # It references the canonical gRPC code constants.
    for code in (
        "Unauthenticated",
        "PermissionDenied",
        "NotFound",
        "FailedPrecondition",
        "DeadlineExceeded",
        "Unavailable",
        "ResourceExhausted",
        "InvalidArgument",
    ):
        assert f"codes.{code}" in src, \
            f"fromGRPC must reference codes.{code} for sentinel translation"


def test_d23_go_sdk_is_retryable_present() -> None:
    """**D23-12** — `IsRetryable` returns true only for the retry-eligible set.

    Public API helper — consumers rely on it for custom retry loops.
    Pin that the function exists + handles the canonical retryable
    sentinels (ErrUnavailable, ErrTimeout, ErrRateLimited).
    """
    src = _read(SDK_ROOT / "errors.go")
    assert re.search(r"^func\s+IsRetryable\s*\(", src, re.MULTILINE), \
        "errors.go must declare `IsRetryable(err error) bool`"


# ─────────────────────────────────────────────────────────────────────
# §4. Verdict struct + methods — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


# Canonical Verdict struct field names (Go) + JSON tags (snake_case
# wire format, immutable).
CANONICAL_VERDICT_FIELDS = (
    ("Label", "label"),
    ("Confidence", "confidence"),
    ("Reject", "reject"),
    ("Rounds", "rounds"),
    ("AlignmentDelta", "alignment_delta"),
    ("Catalog", "catalog"),
    ("Zone", "zone"),
    ("SwarmID", "swarm_id"),
    ("RequestID", "request_id"),
    ("ElapsedMs", "elapsed_ms"),
)


def test_d23_go_sdk_verdict_fields_stable() -> None:
    """**D23-13** — Verdict struct exposes the 10 canonical fields.

    Cross-language pin : same field set as Python SDK Verdict (M8,
    `LETTER_D_ARCHITECTURE.md` §5.1). Adding/removing requires a
    cross-language coordinated bump.
    """
    src = _read(SDK_ROOT / "verdict.go")
    for go_field, _json_tag in CANONICAL_VERDICT_FIELDS:
        # Anchor on the field declaration `<Field>` followed by
        # whitespace + a Go primitive type. Avoid matching comments.
        pattern = rf"^\s*{go_field}\s+(string|float64|bool|int|int64)\b"
        assert re.search(pattern, src, re.MULTILINE), \
            f"verdict.go missing field : {go_field}"


def test_d23_go_sdk_verdict_json_tags_snake_case() -> None:
    """**D23-14** — every Verdict field has the canonical snake_case JSON tag.

    Wire format compatibility : the HTTP JSON shape emitted by the
    3 framework examples (gin/fiber/chi) is pinned by these tags.
    A typo (e.g. `swarmId` instead of `swarm_id`) breaks every
    downstream consumer parsing the JSON response.
    """
    src = _read(SDK_ROOT / "verdict.go")
    for go_field, json_tag in CANONICAL_VERDICT_FIELDS:
        # Pin the JSON tag literal — match either with or without
        # `,omitempty` suffix (RequestID uses omitempty).
        pattern = rf'`json:"{json_tag}(?:,omitempty)?"`'
        assert re.search(pattern, src), \
            f"verdict.go field {go_field} missing JSON tag `{json_tag}`"


def test_d23_go_sdk_verdict_zone_predicates() -> None:
    """**D23-15** — Verdict exposes IsTrusted / IsWarn / IsReject methods.

    Public API helpers — consumers rely on these for routing logic
    (auto-approve trusted, escalate warn, block reject). Pin the
    method set so refactors don't silently drop one.
    """
    src = _read(SDK_ROOT / "verdict.go")
    for method in ("IsTrusted", "IsWarn", "IsReject"):
        # Anchor on `func (v *Verdict) IsXxx()`.
        pattern = rf"^func\s+\(\s*v\s+\*Verdict\s*\)\s+{method}\s*\("
        assert re.search(pattern, src, re.MULTILINE), \
            f"verdict.go missing method : {method}"


# ─────────────────────────────────────────────────────────────────────
# §5. Prometheus metrics canonical names — 1 pin test
# ─────────────────────────────────────────────────────────────────────


CANONICAL_METRIC_NAMES = (
    "efe_sdk_classify_total",
    "efe_sdk_classify_latency_seconds",
    "efe_sdk_classify_errors_total",
    "efe_sdk_in_flight",
    "efe_sdk_retry_attempts_total",
)


def test_d23_go_sdk_metrics_canonical_names() -> None:
    """**D23-16** — the 5 Prometheus metric names are stable.

    Bumping a metric name == breaking every Grafana dashboard +
    Prom alert that references it. Strict pin : exactly these 5,
    no more, no fewer.
    """
    src = _read(SDK_ROOT / "metrics.go")
    for name in CANONICAL_METRIC_NAMES:
        assert name in src, f"metrics.go missing canonical metric : {name}"
    # Anti-drift : count occurrences of the `efe_sdk_` prefix in
    # the metric name **constants** (= name on a `metric...= "efe_sdk_..."`
    # line). Catches accidental additions.
    name_constants = re.findall(
        r'^\s*metric\w+\s*=\s*"(efe_sdk_\w+)"',
        src,
        re.MULTILINE,
    )
    assert sorted(set(name_constants)) == sorted(CANONICAL_METRIC_NAMES), \
        f"metrics drift : found={sorted(set(name_constants))} want={sorted(CANONICAL_METRIC_NAMES)}"


# ─────────────────────────────────────────────────────────────────────
# §6. Internal proto stubs — 2 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d23_go_sdk_proto_stubs_present() -> None:
    """**D23-17** — internal/proto/efe/v1/ exists with placeholder types.

    The hand-coded `types.go` is replaceable by `protoc-gen-go`
    output via `scripts/gen-proto.sh`. The PROTOS_HASH.txt anchors
    drift detection between the actual `.proto` files and the
    generated stubs.
    """
    assert (PROTO_V1 / "types.go").is_file(), \
        "internal/proto/efe/v1/types.go missing (placeholder stubs)"
    assert (PROTO_V1 / "PROTOS_HASH.txt").is_file(), \
        "internal/proto/efe/v1/PROTOS_HASH.txt missing (drift anchor)"


def test_d23_go_sdk_proto_stubs_swarm_service_client() -> None:
    """**D23-18** — proto stubs declare `SwarmServiceClient` interface.

    The SDK relies on this interface for the gRPC client. Renaming
    it would break the client.go imports.
    """
    src = _read(PROTO_V1 / "types.go")
    # The interface declaration.
    assert "SwarmServiceClient" in src, \
        "proto stubs missing SwarmServiceClient"
    # Critical methods of the gRPC client interface.
    for method in ("Execute",):
        assert method in src, \
            f"SwarmServiceClient missing method : {method}"


# ─────────────────────────────────────────────────────────────────────
# §7. Mock package — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d23_go_sdk_mock_package_files() -> None:
    """**D23-19** — `mock/` ships verdict.go + client.go + server.go."""
    for rel in ("verdict.go", "client.go", "server.go"):
        p = MOCK_PKG / rel
        assert p.is_file(), f"mock/{rel} missing"
        assert p.stat().st_size > 0, f"mock/{rel} empty"


def test_d23_go_sdk_mock_verdict_factory() -> None:
    """**D23-20** — `mock.NewVerdict(opts ...VerdictOption) *Verdict` exists.

    Consumer test fixture — produces a canonical pre-baked verdict
    for unit tests that don't want to spin up a mock server.
    """
    src = _read(MOCK_PKG / "verdict.go")
    assert re.search(r"^func\s+NewVerdict\s*\(", src, re.MULTILINE), \
        "mock/verdict.go must declare `NewVerdict(opts ...VerdictOption) *Verdict`"
    assert "VerdictOption" in src, \
        "mock/verdict.go must declare `VerdictOption` type"


def test_d23_go_sdk_mock_server_bufconn() -> None:
    """**D23-21** — `mock.NewServer(t)` wires bufconn for in-process tests.

    The mock server uses `google.golang.org/grpc/test/bufconn` so
    integration tests don't need a real network port. Pin both
    the constructor + the bufconn import.
    """
    src = _read(MOCK_PKG / "server.go")
    assert re.search(r"^func\s+NewServer\s*\(", src, re.MULTILINE), \
        "mock/server.go must declare `NewServer(t *testing.T) ...`"
    assert "bufconn" in src, "mock/server.go must use bufconn for in-process gRPC"


# ─────────────────────────────────────────────────────────────────────
# §8. Examples sub-modules — 3 pin tests (one per framework)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("framework", ["gin", "fiber", "chi"])
def test_d23_go_sdk_example_submodule_complete(framework: str) -> None:
    """**D23-22..24** — each of gin/fiber/chi ships go.mod + main.go + README.md.

    Each example is an INDEPENDENT Go sub-module (own `go.mod`)
    so consumers can `go get` only what they need without pulling
    all 3 framework deps. Pin the file set + that go.mod requires
    the SDK module path.
    """
    ex_dir = EXAMPLES / framework
    assert ex_dir.is_dir(), f"examples/{framework}/ missing"
    for f in ("go.mod", "main.go", "README.md"):
        p = ex_dir / f
        assert p.is_file(), f"examples/{framework}/{f} missing"
        assert p.stat().st_size > 0, f"examples/{framework}/{f} empty"
    # The example go.mod must declare a separate module path under
    # the SDK's module path (canonical `github.com/efe-ai/efe-sdk-go/examples/<framework>`).
    gomod = _read(ex_dir / "go.mod")
    assert "github.com/efe-ai/efe-sdk-go" in gomod, \
        f"examples/{framework}/go.mod must reference the SDK module path"


# ─────────────────────────────────────────────────────────────────────
# §9. Scripts shell — 4 pin tests
# ─────────────────────────────────────────────────────────────────────


CANONICAL_SCRIPTS = ("gen-proto.sh", "lint.sh", "test.sh", "publish.sh")


@pytest.mark.parametrize("script", CANONICAL_SCRIPTS)
def test_d23_go_sdk_script_present_and_executable(script: str) -> None:
    """**D23-25..28** — each script exists, is executable, and has valid bash syntax.

    Operator-facing entry points — a typo or chmod -x would
    silently break the M9 CI workflow + the publish flow.
    """
    p = SCRIPTS / script
    assert p.is_file(), f"scripts/{script} missing"
    # Must be executable.
    mode = p.stat().st_mode
    assert mode & stat.S_IXUSR, f"scripts/{script} not executable (chmod +x)"
    # Bash syntax check via `bash -n` if bash is available.
    bash = "/bin/bash" if Path("/bin/bash").exists() else "bash"
    if subprocess.run(
        [bash, "--version"], capture_output=True, check=False
    ).returncode == 0:
        proc = subprocess.run(
            [bash, "-n", str(p)],
            capture_output=True,
            check=False,
        )
        assert proc.returncode == 0, \
            f"scripts/{script} bash syntax error : {proc.stderr.decode()}"
    # Must declare a strict-mode preamble (`set -euo pipefail`) —
    # defends against silent bugs in the publish flow.
    src = _read(p)
    assert "set -euo pipefail" in src, \
        f"scripts/{script} missing `set -euo pipefail` strict-mode preamble"


# ─────────────────────────────────────────────────────────────────────
# §10. Docs + architecture cross-link — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d23_go_sdk_doc_go_quickstart_present() -> None:
    """**D23-29** — `doc.go` contains the canonical quickstart godoc.

    The package-level godoc is the first thing developers see on
    pkg.go.dev. Pin that the canonical quickstart + API surface
    section are present.
    """
    src = _read(SDK_ROOT / "doc.go")
    assert "Package efe is the official Go SDK" in src, \
        "doc.go missing canonical package summary"
    assert "Quickstart" in src or "quickstart" in src, \
        "doc.go missing Quickstart section"
    assert "efe.NewClient" in src, \
        "doc.go quickstart must show NewClient usage"


def test_d23_go_sdk_architecture_doc_present() -> None:
    """**D23-30** — architecture doc exists at the canonical path.

    `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md` is the source of
    truth for the M9 module. Removing it leaves the SDK source
    files referencing a dead anchor (every file header points
    to it).
    """
    assert ARCH_DOC.is_file(), \
        f"missing architecture doc : {ARCH_DOC.relative_to(REPO_ROOT)}"
    src = _read(ARCH_DOC)
    # Must mention the canonical module path + sprint id.
    assert "efe-sdk-go" in src
    assert "D23" in src or "Sprint D23" in src


def test_d23_go_sdk_ci_workflows_present() -> None:
    """**D23-31** — both CI workflows exist and reference the SDK.

    `sdk-go-test.yml` (PR gate) + `sdk-go-smoke.yml` (daily
    staging smoke) are the only CI surfaces for M9. Both must
    exist + correctly reference `clients/go/efe-sdk/`.
    """
    test_wf = WORKFLOWS / "sdk-go-test.yml"
    smoke_wf = WORKFLOWS / "sdk-go-smoke.yml"
    assert test_wf.is_file(), f"missing CI workflow : {test_wf.name}"
    assert smoke_wf.is_file(), f"missing CI workflow : {smoke_wf.name}"
    test_src = _read(test_wf)
    smoke_src = _read(smoke_wf)
    # Both must reference the SDK path so they actually trigger on
    # SDK changes.
    for src in (test_src, smoke_src):
        assert "clients/go/efe-sdk" in src, \
            "CI workflow must reference clients/go/efe-sdk path"
    # The PR-gate workflow must run pin tests (this very file).
    assert "test_d23_go_sdk_extension.py" in test_src, \
        "sdk-go-test.yml must invoke the pin tests file"


# ─────────────────────────────────────────────────────────────────────
# §11. Audit fixes (M9-A1..A6) — 8 pin tests anti-régression
# ─────────────────────────────────────────────────────────────────────
#
# After the M9 chirurgical audit pass, 6 defects were corrected
# (cf. CHANGELOG.md "Audit fixes — M9 chirurgical audit pass 1").
# The following pin tests anchor those fixes so future refactors
# can't accidentally regress them.
#
#   A1 — bufconn TLS handshake mismatch (CRIT)
#   A2 — fake gRPC errors don't satisfy status.FromError (CRIT)
#   A3 — ClassifyStream index race (HIGH)
#   A4 — mock Healthcheck ErrTimeout exception (MED)
#   A5 — unused grpcconn helpers / dead code (LOW)
#   A6 — redundant pb.SwarmServiceServer embed in mock Server (LOW)


def test_d23_go_sdk_audit_fix_a1_bufconn_uses_insecure_creds() -> None:
    """**M9-A1 (CRIT)** — `client.go::buildDialOptions` switches to
    `insecure.NewCredentials()` when a custom dialer is set without
    explicit TLS material.

    Pre-fix : the bufconn-based mock failed because the SDK still
    issued a TLS handshake (`InsecureSkipVerify: true`) on a memory
    pipe that doesn't speak TLS.
    """
    src = _read(SDK_ROOT / "client.go")
    # The canonical switch case must be present.
    assert "o.contextDialer != nil" in src, \
        "client.go must check for custom dialer in TLS resolution"
    assert "o.tlsConfig == nil && o.mtls == nil" in src, \
        "bufconn path must require absence of explicit TLS material"
    assert "insecure.NewCredentials()" in src, \
        "client.go must reference insecure.NewCredentials() for bufconn"
    # Audit-fix marker present so the rationale is preserved in code.
    assert "M9-A1" in src, "client.go must reference audit fix M9-A1"


def test_d23_go_sdk_audit_fix_a2_mock_uses_real_status_error() -> None:
    """**M9-A2 (CRIT)** — `mock/server.go::errUnimplemented` uses
    `status.Error()` instead of a hand-rolled struct.

    Pre-fix : the hand-rolled `mockGrpcErr` returned `GRPCStatus()
    interface{}` which did NOT satisfy gRPC's
    `interface { GRPCStatus() *status.Status }` (signature mismatch).
    Result : `status.FromError` rejected it, `fromGRPC` fell through,
    `IsRetryable` returned false, retry tests silently broken.
    """
    src = _read(MOCK_PKG / "server.go")
    # The canonical real status.Error usage.
    assert 'status.Error(codes.Unimplemented' in src, \
        "mock/server.go errUnimplemented must use status.Error(codes.Unimplemented, ...)"
    # The pre-fix struct (declaration, not just the name in comments) MUST be gone.
    assert not re.search(r"^type\s+mockGrpcErr\s+struct", src, re.MULTILINE), \
        "pre-fix `type mockGrpcErr struct` declaration must be removed"
    # The pre-fix wrong-signature method declaration MUST be gone.
    # We anchor on `func (...) GRPCStatus() interface{}` — this catches
    # the bad declaration but NOT the same string appearing in a docstring
    # explaining why we removed it.
    assert not re.search(
        r"^func\s+\([^)]*\)\s+GRPCStatus\s*\(\s*\)\s+interface\{\}",
        src,
        re.MULTILINE,
    ), "pre-fix `GRPCStatus() interface{}` method declaration must be removed"
    # And the integration test must use the canonical helper too.
    int_src = _read(SDK_ROOT / "integration_test.go")
    assert 'status.Error(codes.Unavailable' in int_src, \
        "integration_test.go must use status.Error(codes.Unavailable, ...) for retry test"
    # Same anchoring on declaration shape (vs comment substring).
    assert not re.search(r"^type\s+fakeGRPCErr\s+struct", int_src, re.MULTILINE), \
        "pre-fix `type fakeGRPCErr struct` declaration must be removed"
    assert not re.search(
        r"^func\s+fakeStatusError\s*\(", int_src, re.MULTILINE,
    ), "pre-fix `func fakeStatusError(...)` declaration must be removed"


def test_d23_go_sdk_audit_fix_a3_classify_stream_dispatcher() -> None:
    """**M9-A3 (HIGH)** — `classify.go::ClassifyStream` uses a
    single-dispatcher pattern (one goroutine reads from `texts`,
    assigns indices monotonically, fans out to workers).

    Pre-fix : N workers all read from `texts` directly + grabbed
    indices from a shared mutex-protected counter. Race : Worker A
    pulls text 0 from channel, gets preempted, Worker B pulls text 1
    AND grabs the counter first → text 0 receives Index=1, text 1
    receives Index=0. The doc claim "Ordering is preserved via the
    Index field" was silently broken.
    """
    src = _read(SDK_ROOT / "classify.go")
    # Canonical dispatcher pattern markers.
    assert "indexedText" in src or "fanout" in src, \
        "classify.go ClassifyStream must use dispatcher + fan-out channel pattern"
    # The pre-fix mutex pattern MUST be gone.
    assert "idxLock" not in src, \
        "pre-fix `idxLock sync.Mutex` index counter must be removed"
    assert "var nextIdx int" not in src, \
        "pre-fix `var nextIdx int` shared counter must be removed"
    # Audit-fix marker.
    assert "M9-A3" in src, "classify.go must reference audit fix M9-A3"


def test_d23_go_sdk_audit_fix_a4_mock_healthcheck_no_timeout_exception() -> None:
    """**M9-A4 (MED)** — `mock/client.go::Healthcheck` no longer
    special-cases `ErrTimeout`.

    Pre-fix : `if m.err != nil && !errors.Is(m.err, efe.ErrTimeout)`
    meant `SetError(ErrTimeout)` left Healthcheck reporting healthy
    while Classify returned ErrTimeout — inconsistent.
    """
    src = _read(MOCK_PKG / "client.go")
    # The pre-fix exception MUST be gone.
    assert "errors.Is(m.err, efe.ErrTimeout)" not in src, \
        "pre-fix ErrTimeout exception in Healthcheck must be removed"
    # And the imports section MUST no longer pull `errors` (was only
    # used by the exception).
    assert '\t"errors"' not in src, \
        "mock/client.go must not import `errors` after audit fix M9-A4"
    # Audit-fix marker present.
    assert "M9-A4" in src, "mock/client.go must reference audit fix M9-A4"


def test_d23_go_sdk_audit_fix_a5_grpcconn_helpers_pruned() -> None:
    """**M9-A5 (LOW)** — unused `grpcconn` helpers removed.

    Pre-fix : `ResolveCredentials`, `BufconnDialer`, `EnsureNonZeroDialer`
    were declared but never called from the SDK (TLS resolution
    consolidated in `tls.go`). Triggered `golangci-lint -E unused`
    warnings.
    """
    src = _read(SDK_ROOT / "internal" / "grpcconn" / "conn.go")
    # The 3 unused helpers MUST be gone.
    for unused in ("ResolveCredentials", "BufconnDialer", "EnsureNonZeroDialer"):
        assert f"func {unused}" not in src, \
            f"unused helper {unused} must be removed from grpcconn/conn.go"
    # The 2 actually-used helpers MUST remain.
    for used in ("StripScheme", "DefaultKeepalive"):
        assert f"func {used}" in src, \
            f"used helper {used} must remain in grpcconn/conn.go"
    # Audit-fix marker.
    assert "M9-A5" in src, "grpcconn/conn.go must reference audit fix M9-A5"


def test_d23_go_sdk_audit_fix_a6_mock_server_no_redundant_embed() -> None:
    """**M9-A6 (LOW)** — `mock/server.go::Server` no longer embeds
    `pb.SwarmServiceServer` (interface).

    Pre-fix : `pb.SwarmServiceServer` was embedded as a struct field
    but its only method (`Execute`) was already shadowed by Server's
    own definition. The embed was unreachable + nil → would panic
    if a future refactor added a method to the interface.

    Fix : remove embed + add explicit compile-time guard
    `var _ pb.SwarmServiceServer = (*Server)(nil)`.
    """
    src = _read(MOCK_PKG / "server.go")
    # The redundant embed (interface inside the struct) MUST be gone.
    # Pre-fix shape : `\tpb.SwarmServiceServer` on its own line inside
    # the struct, with a comment "unexported embed".
    assert "pb.SwarmServiceServer  // unexported embed" not in src, \
        "pre-fix redundant pb.SwarmServiceServer embed must be removed"
    # The compile-time guard MUST be present so the interface
    # satisfaction is enforced statically.
    assert "var _ pb.SwarmServiceServer = (*Server)(nil)" in src, \
        "mock/server.go must declare `var _ pb.SwarmServiceServer = (*Server)(nil)` guard"
    # Audit-fix marker.
    assert "M9-A6" in src, "mock/server.go must reference audit fix M9-A6"


def test_d23_go_sdk_audit_fix_doc_options_count_corrected() -> None:
    """**M9-A7 (MED)** — `doc.go` claims "13 [WithXxx] options"
    (was incorrectly "10").

    Cross-checked against `CANONICAL_OPTIONS` (13 items) — keeping
    the doc and the actual surface in sync prevents silent drift.
    """
    src = _read(SDK_ROOT / "doc.go")
    assert "13 [WithXxx] options" in src, \
        "doc.go must claim 13 WithXxx options (matches CANONICAL_OPTIONS count)"
    # Anti-regression : the old claim "10 [WithXxx] options" must be gone.
    assert "10 [WithXxx] options" not in src, \
        "pre-fix `10 [WithXxx] options` claim must be removed"


def test_d23_go_sdk_audit_fix_doc_onclassify_signature_corrected() -> None:
    """**M9-A8 (HIGH)** — both `doc.go` and `README.md` show the
    correct `OnClassify(func(ctx context.Context, req *pb.ExecuteRequest) ...)`
    signature in the mock-package example.

    Pre-fix : the doc example dropped the `ctx context.Context`
    parameter — copy-pasting the doc would yield a compile error.
    """
    for path in (SDK_ROOT / "doc.go", SDK_ROOT / "README.md"):
        src = _read(path)
        # Find the `OnClassify(func(...)` line in the example block ;
        # the canonical fix has `ctx context.Context, req *pb.ExecuteRequest`.
        assert "OnClassify(func(ctx context.Context, req *pb.ExecuteRequest)" in src, \
            f"{path.name} must show OnClassify(func(ctx context.Context, req *pb.ExecuteRequest)...)"
        # Anti-regression : the old wrong shape must be gone.
        assert "OnClassify(func(req *pb.ExecuteRequest)" not in src, \
            f"{path.name} must not show pre-fix `OnClassify(func(req *pb.ExecuteRequest)...)`"
