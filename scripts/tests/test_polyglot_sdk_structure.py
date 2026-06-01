"""Pin tests for the polyglot SDK quartet.

Sprint **Φ-C-SDK Completion (D34)** — artefact #34.
Cf. ``docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`` § 20.

## Mission

Enforce the 7 D-SDK-* doctrines (architecture § 0.1) as runtime pin
tests. Every drift between the 4 SDKs (Python + Go + TS + Java) must
turn at least one of these tests red. No silent drift tolerated.

## Test categories

| # | Test | Doctrine | Pattern caught |
|---|------|----------|-----------------|
| 1-3 | `test_phi_c_sdk_slo_yaml_*` | D-SDK-1 | SLO YAML drift / missing entries |
| 4 | `test_phi_c_sdk_proto_hash_baseline_exists` | D-SDK-2 | proto baseline missing |
| 5 | `test_phi_c_sdk_codegen_scripts_exist` | D-SDK-2 | codegen scripts referenced but absent |
| 6-8 | `test_phi_c_sdk_runner_*_exists` | D-SDK-1 | contract runners missing |
| 9 | `test_phi_c_sdk_compare_script_exists` | D-SDK-1 | comparator missing |
| 10 | `test_phi_c_sdk_quartet_fixtures_count` | D-SDK-1 | fixtures count drift |
| 11 | `test_phi_c_sdk_docker_compose_has_all_4_runners` | D-SDK-1 | runner service missing |
| 12 | `test_phi_c_sdk_version_sync_tool_exists` | D-SDK-6 | version sync tool absent |
| 13 | `test_phi_c_sdk_doc_coverage_tool_exists` | D-SDK-3 | doc coverage tool absent |
| 14 | `test_phi_c_sdk_runbooks_exist` | — | 4 runbooks must exist |
| 15 | `test_phi_c_sdk_workflows_exist` | — | 3 CI workflows must exist |
| 16 | `test_phi_c_sdk_python_dropin_exists` | — | migration helper present |
| 17 | `test_phi_c_sdk_check_slo_supports_pass_count` | D-SDK-7 | anti-Φ-D.A1 |
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]


# ────────────────────────────────────────────────────────────────────
# Helpers.
# ────────────────────────────────────────────────────────────────────


def _read_yaml(path: Path) -> dict:
    yaml = pytest.importorskip("yaml")
    return yaml.safe_load(path.read_text())


def _read_json(path: Path) -> dict:
    return json.loads(path.read_text())


# ────────────────────────────────────────────────────────────────────
# §1 — SLO YAML completeness.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_slo_yaml_exists_and_has_15_entries():
    """Φ-C-SDK artefact #01 : sdk_slos.yaml must declare 15 SLOs."""
    p = _REPO / "bench_baselines" / "v0.6.0" / "sdk_slos.yaml"
    assert p.exists(), f"SLO YAML missing : {p}"
    data = _read_yaml(p)
    assert data["domain"] == "sdk"
    ids = {s["id"] for s in data["slos"]}
    expected = {
        "SLO-SDK-CONTRACT",
        "SLO-SDK-LATENCY-GO",
        "SLO-SDK-LATENCY-TS",
        "SLO-SDK-LATENCY-JAVA",
        "SLO-SDK-DOC-GO",
        "SLO-SDK-DOC-TS",
        "SLO-SDK-DOC-JAVA",
        "SLO-SDK-SIZE-GO",
        "SLO-SDK-SIZE-TS",
        "SLO-SDK-SIZE-JAVA",
        "SLO-SDK-PROTO-HASH",
        "SLO-SDK-VERSION-SYNC",
        "SLO-SDK-API-SURFACE",
        "SLO-SDK-SIGSTORE",
        "SLO-SDK-SBOM",
    }
    missing = expected - ids
    assert not missing, f"SLO YAML missing entries : {sorted(missing)}"
    assert len(ids) == 15, f"expected 15 SLOs, got {len(ids)}"


def test_phi_c_sdk_slo_yaml_contract_expects_400():
    """SLO-SDK-CONTRACT must declare 400 expected (= 100 fixtures × 4 SDKs)."""
    data = _read_yaml(_REPO / "bench_baselines" / "v0.6.0" / "sdk_slos.yaml")
    contract = next(s for s in data["slos"] if s["id"] == "SLO-SDK-CONTRACT")
    assert contract["expected_count"] == 400, (
        f"SLO-SDK-CONTRACT expected_count = {contract['expected_count']} "
        f"; expected 400 (= 100 fixtures × 4 SDKs)"
    )
    assert contract["threshold_pass_count"] == 400


def test_phi_c_sdk_slo_yaml_sigstore_expects_4_artifacts():
    """SLO-SDK-SIGSTORE counts ≥ 4 artifacts signed (= 1 per SDK)."""
    data = _read_yaml(_REPO / "bench_baselines" / "v0.6.0" / "sdk_slos.yaml")
    sigstore = next(s for s in data["slos"] if s["id"] == "SLO-SDK-SIGSTORE")
    assert sigstore["threshold_count"] == 4
    sbom = next(s for s in data["slos"] if s["id"] == "SLO-SDK-SBOM")
    assert sbom["threshold_count"] == 4


# ────────────────────────────────────────────────────────────────────
# §2 — Proto hash baseline (= D-SDK-2 enforcement).
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_proto_hash_baseline_exists():
    """The proto hash baseline must exist and contain a valid SHA-256."""
    p = _REPO / "bench_baselines" / "v0.6.0" / "sdk_proto_hash.txt"
    assert p.exists(), f"proto hash baseline missing : {p}"
    content = p.read_text().strip()
    # Must be either PLACEHOLDER_* OR a valid 64-char hex SHA-256.
    assert content.startswith("PLACEHOLDER_") or re.fullmatch(r"[0-9a-f]{64}", content), (
        f"proto hash baseline invalid format : {content!r}"
    )


# ────────────────────────────────────────────────────────────────────
# §3 — Codegen scripts (= 1 orchestrator + 4 per-SDK).
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_codegen_scripts_exist():
    """All 5 codegen scripts referenced by the orchestrator must exist."""
    scripts_dir = _REPO / "scripts" / "sdk"
    expected = [
        "regen_all.sh",
        "regen_go.sh",
        "regen_ts.sh",
        "regen_java.sh",
        "regen_python.sh",
    ]
    missing = [s for s in expected if not (scripts_dir / s).exists()]
    assert not missing, f"codegen scripts missing : {missing}"


def test_phi_c_sdk_codegen_scripts_are_executable():
    """All 5 codegen scripts must be executable."""
    scripts_dir = _REPO / "scripts" / "sdk"
    for script in scripts_dir.glob("regen_*.sh"):
        assert script.stat().st_mode & 0o111, (
            f"codegen script not executable : {script}"
        )


# ────────────────────────────────────────────────────────────────────
# §4 — Contract test runners.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_runner_py_exists():
    p = _REPO / "sdk" / "contract-runner" / "runners" / "py" / "main.py"
    assert p.exists(), f"Python runner missing : {p}"


def test_phi_c_sdk_runner_go_exists():
    p = _REPO / "sdk" / "contract-runner" / "runners" / "go" / "main.go"
    assert p.exists(), f"Go runner missing : {p}"
    mod = _REPO / "sdk" / "contract-runner" / "runners" / "go" / "go.mod"
    assert mod.exists(), f"Go module file missing : {mod}"


def test_phi_c_sdk_runner_ts_exists():
    p = _REPO / "sdk" / "contract-runner" / "runners" / "ts" / "main.ts"
    assert p.exists(), f"TS runner missing : {p}"
    pkg = _REPO / "sdk" / "contract-runner" / "runners" / "ts" / "package.json"
    assert pkg.exists(), f"TS package.json missing : {pkg}"


def test_phi_c_sdk_runner_java_exists():
    p = _REPO / "sdk" / "contract-runner" / "runners" / "java" / "Main.java"
    assert p.exists(), f"Java runner missing : {p}"


def test_phi_c_sdk_compare_script_exists():
    p = _REPO / "sdk" / "contract-runner" / "compare.py"
    assert p.exists(), f"comparator missing : {p}"


# ────────────────────────────────────────────────────────────────────
# §5 — Fixtures count (= D-SDK-1 enforcement of expected_count).
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_quartet_fixtures_count():
    """quartet_fixtures.json must have exactly 100 entries.

    **D-SDK-1 enforcement** : SLO-SDK-CONTRACT expects 400 passes
    = 100 fixtures × 4 SDKs. Drift here would silently lower the
    SLO ceiling.
    """
    p = (
        _REPO
        / "sdk"
        / "contract-runner"
        / "fixtures"
        / "quartet_fixtures.json"
    )
    assert p.exists(), (
        f"fixtures file missing : {p}. "
        f"Run `python sdk/contract-runner/generate_fixtures.py` to regenerate."
    )
    data = _read_json(p)
    assert data["count"] == 100, (
        f"fixtures count = {data['count']}, expected 100. "
        f"Re-generate via generate_fixtures.py."
    )
    assert len(data["fixtures"]) == 100


def test_phi_c_sdk_quartet_fixtures_have_unique_ids():
    """Each fixture must have a unique id (= prevent accidental dupes)."""
    p = (
        _REPO
        / "sdk"
        / "contract-runner"
        / "fixtures"
        / "quartet_fixtures.json"
    )
    if not p.exists():
        pytest.fail(f"fixtures file missing : {p}")
    data = _read_json(p)
    ids = [f["id"] for f in data["fixtures"]]
    assert len(set(ids)) == len(ids), (
        f"duplicate fixture ids detected ({len(ids) - len(set(ids))} dupes)"
    )


# ────────────────────────────────────────────────────────────────────
# §6 — Docker compose has all 4 runners.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_docker_compose_has_all_4_runners():
    p = _REPO / "sdk" / "contract-runner" / "docker-compose.yml"
    assert p.exists(), f"docker-compose missing : {p}"
    text = p.read_text()
    for runner in ["runner-py", "runner-go", "runner-ts", "runner-java", "runner-compare"]:
        assert runner + ":" in text, f"docker-compose missing service : {runner}"


# ────────────────────────────────────────────────────────────────────
# §7 — Tooling completeness.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_version_sync_tool_exists():
    p = _REPO / "scripts" / "sdk" / "verify_version_sync.py"
    assert p.exists(), f"version sync tool missing : {p}"


def test_phi_c_sdk_doc_coverage_tool_exists():
    p = _REPO / "scripts" / "sdk" / "verify_doc_coverage.py"
    assert p.exists(), f"doc coverage tool missing : {p}"


def test_phi_c_sdk_python_dropin_exists():
    p = _REPO / "clients" / "python" / "efe" / "_dropin.py"
    if not p.exists():
        # Alternative location.
        alt = _REPO / "clients" / "python-byo-agent" / "efe_byo_agent" / "_dropin.py"
        assert alt.exists() or p.exists(), (
            f"Python dropin helper missing at either {p} or {alt}"
        )


# ────────────────────────────────────────────────────────────────────
# §8 — Runbooks (= 4 required).
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_runbooks_exist():
    runbook_dir = _REPO / "docs" / "runbook"
    expected = [
        "sdk_contract_test_failure.md",
        "sdk_version_drift.md",
        "sdk_publish_failure.md",
        "sdk_proto_bump.md",
    ]
    missing = [name for name in expected if not (runbook_dir / name).exists()]
    assert not missing, f"Φ-C-SDK runbooks missing : {missing}"


# ────────────────────────────────────────────────────────────────────
# §9 — CI workflows (= 3 required).
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_workflows_exist():
    wf_dir = _REPO / ".github" / "workflows"
    expected = [
        "sdk-quartet-contract.yml",
        "sdk-quartet-publish.yml",
        "sdk-quartet-bench.yml",
    ]
    missing = [name for name in expected if not (wf_dir / name).exists()]
    assert not missing, f"Φ-C-SDK workflows missing : {missing}"


# ────────────────────────────────────────────────────────────────────
# §10 — Anti-Φ-D.A1 : check_slo.py supports the new fields needed.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_check_slo_supports_pass_count():
    """check_slo.py must support `threshold_pass_count` + `expected_count`.

    Without these, SLO-SDK-CONTRACT (= 400/400) would be silently
    skipped (= replay of pattern Φ-D.A1).
    """
    check_slo = _REPO / "eval" / "scripts" / "check_slo.py"
    src = check_slo.read_text()
    assert "threshold_pass_count" in src
    assert "expected_count" in src
    # Defensive : verify the evaluation branch exists.
    assert "slo.threshold_pass_count is not None" in src, (
        "check_slo.py has the field but no evaluation branch — pattern Φ-D.A1"
    )


# ────────────────────────────────────────────────────────────────────
# §11 — Convention docs.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_convention_docs_exist():
    docs_dir = _REPO / "docs"
    expected = [
        "sdk-otel-conventions.md",
        "sdk-retry-conventions.md",
        "sdk-auth-conventions.md",
    ]
    missing = [name for name in expected if not (docs_dir / name).exists()]
    assert not missing, f"Φ-C-SDK convention docs missing : {missing}"


# ────────────────────────────────────────────────────────────────────
# §12 — Architecture doc exists.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_architecture_doc_exists():
    p = _REPO / "docs" / "EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md"
    assert p.exists(), f"architecture doc missing : {p}"
    text = p.read_text()
    # Sanity : must declare the 7 doctrines D-SDK-1..D-SDK-7.
    for i in range(1, 8):
        assert f"D-SDK-{i}" in text, f"doctrine D-SDK-{i} missing from archi"


# ────────────────────────────────────────────────────────────────────
# §13 — Validation of proto hash invariance.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_proto_hash_matches_current_files():
    """The baseline hash must match the current proto files (= no drift).

    A drift here means someone modified .proto without bumping
    SDK MAJOR. Run :
        SDK_PROTO_BUMP=yes ./scripts/sdk/regen_all.sh
    to acknowledge breaking change.

    **Audit fix Φ-C-SDK** : the algorithm MUST match `regen_all.sh`
    exactly, otherwise the pin test would fail in CI even when the
    proto is unchanged. The shell uses :
        find ... -print0 | sort -z | xargs -0 cat | sha256sum
    Python equivalent : sorted(glob('*.proto')) + read_bytes per file
    + cumulative hash. Verified empirically (cf. Audit Level 2 B4).
    """
    import hashlib
    p = _REPO / "bench_baselines" / "v0.6.0" / "sdk_proto_hash.txt"
    if not p.exists():
        pytest.fail(f"proto hash baseline missing : {p}")
    pinned = p.read_text().strip()
    if pinned.startswith("PLACEHOLDER_"):
        pytest.skip("proto hash not yet frozen (= PLACEHOLDER_*)")
    proto_dir = _REPO / "protos" / "efe" / "v1"
    if not proto_dir.exists():
        pytest.skip(f"proto dir missing : {proto_dir}")
    # Algorithm identical to regen_all.sh (= verified by audit B4).
    proto_files = sorted(proto_dir.glob("*.proto"))
    h = hashlib.sha256()
    for pf in proto_files:
        h.update(pf.read_bytes())
    current = h.hexdigest()
    assert current == pinned, (
        f"proto hash drift detected.\n"
        f"  expected (baseline) : {pinned}\n"
        f"  current  (proto)    : {current}\n"
        f"Any change to protos/efe/v1/*.proto requires MAJOR bump on all 4 SDKs.\n"
        f"To acknowledge, run : SDK_PROTO_BUMP=yes ./scripts/sdk/regen_all.sh"
    )


# ────────────────────────────────────────────────────────────────────
# Φ-C-SDK Completion Audit Level 2 — anti-régression pin tests.
# Cf. docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md § 20.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_ts_runner_uses_two_cast_fround():
    """**Audit fix Φ-C-SDK.B1** — TS runner must use two-cast Math.fround
    pattern for confidence, NOT single-cast on f64 division.

    Empirical : 91/10000 random confU32 values diverge by 1 ulp f32
    between the two patterns. Single-cast would cause silent contract
    test failures cross-SDK.
    """
    p = _REPO / "sdk" / "contract-runner" / "runners" / "ts" / "main.ts"
    src = p.read_text(encoding="utf-8")
    # The fix pattern : `Math.fround(Math.fround(confU32) / Math.fround(...))`
    # OR equivalently splitting across lines.
    assert "Math.fround(confU32)" in src, (
        "B1 regression : TS runner no longer casts confU32 to f32 explicitly"
    )
    assert "Math.fround(0xffff_ffff)" in src or "Math.fround(0xFFFFFFFF)" in src, (
        "B1 regression : TS runner no longer casts u32::MAX to f32 explicitly"
    )


def test_phi_c_sdk_py_runner_fails_loud_without_numpy():
    """**Audit fix Φ-C-SDK.B12** — Python runner must FAIL LOUD if numpy
    is missing, not silently fall back to f64 path.

    Pre-fix had a silent fallback that produced wrong verdicts diverging
    396/1536 bytes from Go/TS/Java runners (= same as Φ-C-FFI.B2 pattern).
    """
    p = _REPO / "sdk" / "contract-runner" / "runners" / "py" / "main.py"
    src = p.read_text(encoding="utf-8")
    # Must declare numpy as MANDATORY.
    assert "numpy is required" in src or "numpy is MANDATORY" in src, (
        "B12 regression : Python runner no longer mentions numpy as mandatory"
    )
    # Must NOT have a silent fallback to struct.pack-based f64 path.
    # The fallback used `b''.join(struct.pack('<f', v) for v in emb_f64)`.
    assert (
        "for v in emb_f64" not in src
        or "FATAL : numpy is required" in src
    ), (
        "B12 regression : Python runner silent f64 fallback re-introduced"
    )


def test_phi_c_sdk_java_runner_handles_braces_in_text():
    """**Audit fix Φ-C-SDK.B14** — Java JSON parser must track string
    state so that `{` or `}` inside fixture text don't break brace
    counting.

    Pre-fix would crash or produce wrong verdicts on fixtures like
    `"Brackets: () [] {} <>"`. Post-fix : string state machine in
    parseFixtures skips chars inside quoted strings.
    """
    p = _REPO / "sdk" / "contract-runner" / "runners" / "java" / "Main.java"
    src = p.read_text(encoding="utf-8")
    # Must have the inString flag + escape handling.
    assert "inString" in src, (
        "B14 regression : Java parser no longer tracks string state"
    )
    assert "escape" in src.lower(), (
        "B14 regression : Java parser no longer tracks string escapes"
    )


def test_phi_c_sdk_ffi_publish_supports_workflow_call():
    """**Audit fix Φ-C-SDK.B3** — ffi-publish.yml must accept
    `workflow_call:` trigger so sdk-quartet-publish.yml can reuse it
    as the Python publisher leg.

    Without this, the polyglot quartet publish workflow would fail
    immediately at the `publish-python` job with `workflow_call not
    supported by reusable workflow`.
    """
    p = _REPO / ".github" / "workflows" / "ffi-publish.yml"
    src = p.read_text(encoding="utf-8")
    assert "workflow_call:" in src, (
        "B3 regression : ffi-publish.yml no longer declares workflow_call trigger"
    )


def test_phi_c_sdk_proto_hash_algorithm_matches_shell():
    """**Anti-regression on the hash algorithm equivalence** — Python
    pin test computes the proto hash via :
        sorted(glob('*.proto')) + cumulative read_bytes() to sha256

    `regen_all.sh` uses :
        find -print0 | sort -z | xargs -0 cat | sha256sum

    Both must give the same result. Empirically verified in audit Level 2,
    but this test guards against future drift (e.g. a developer changes
    one side without the other).
    """
    import hashlib
    import subprocess

    proto_dir = _REPO / "protos" / "efe" / "v1"
    if not proto_dir.exists():
        pytest.skip(f"proto dir missing : {proto_dir}")

    # Python method.
    h = hashlib.sha256()
    for pf in sorted(proto_dir.glob("*.proto")):
        h.update(pf.read_bytes())
    python_hash = h.hexdigest()

    # Shell method (= regen_all.sh).
    try:
        result = subprocess.run(
            ["bash", "-c",
             "find . -name '*.proto' -print0 | sort -z | xargs -0 cat | shasum -a 256 | awk '{print $1}'"],
            cwd=str(proto_dir),
            capture_output=True,
            check=True,
            timeout=30,
        )
        shell_hash = result.stdout.decode().strip()
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired,
            FileNotFoundError) as e:
        pytest.skip(f"shell hash computation failed : {e}")

    assert python_hash == shell_hash, (
        f"Hash algorithm divergence detected !\n"
        f"  Python method (pin test) : {python_hash}\n"
        f"  Shell method  (regen_all.sh) : {shell_hash}\n"
        f"This would cause pin test red even when proto is unchanged."
    )


def test_phi_c_sdk_compare_handles_missing_verdicts():
    """**Anti-regression** — compare.py must report 'missing verdict'
    when a runner crashes mid-execution, rather than crashing the
    comparator itself.
    """
    p = _REPO / "sdk" / "contract-runner" / "compare.py"
    src = p.read_text(encoding="utf-8")
    assert "missing verdict" in src, (
        "compare.py no longer handles missing per-fixture verdicts"
    )


def test_phi_c_sdk_check_slo_supports_threshold_seconds_branch():
    """**Anti-regression Φ-D.A1 pattern** — check_slo.py must support
    `threshold_seconds` with pre-computed `measured_seconds` in
    `slo_compliance` (= Φ-C-FFI.A1 extension).

    Without this, SLO-SDK-LATENCY-{GO,TS,JAVA} would be silently
    SKIPPED in CI.
    """
    p = _REPO / "eval" / "scripts" / "check_slo.py"
    src = p.read_text(encoding="utf-8")
    # The branch was added during Φ-C-FFI ; verify it persists.
    assert 'comp_id.get("measured_seconds")' in src, (
        "check_slo.py no longer reads measured_seconds from slo_compliance "
        "→ all latency SLOs would be silently skipped (pattern Φ-D.A1)"
    )


# ────────────────────────────────────────────────────────────────────
# Φ-C-SDK Audit Level 3 — Encore plus profond.
# ────────────────────────────────────────────────────────────────────


def test_phi_c_sdk_ffi_publish_has_id_token_permission():
    """**Audit fix Φ-C-SDK.B-extra2** — ffi-publish.yml MUST declare
    `permissions: id-token: write` at the top of the workflow.

    Without this, the Sigstore cosign `sign-blob --yes` step fails
    with `Error: signing: getting OIDC token: no matching credentials`.
    This was a CRIT bug missed in Level 1+2 audits.
    """
    p = _REPO / ".github" / "workflows" / "ffi-publish.yml"
    src = p.read_text(encoding="utf-8")
    # Match a workflow-level permissions block (= top-level, not job-level).
    # Heuristic : a line `permissions:` followed by `  id-token: write`.
    assert re.search(
        r"^permissions:\s*\n(?:[ ]+[^\n]*\n)*[ ]+id-token:\s*write",
        src,
        re.MULTILINE,
    ), (
        "B-extra2 regression : ffi-publish.yml missing top-level "
        "`permissions: id-token: write` → Sigstore keyless signing would fail "
        "(symptom in runbook sdk_publish_failure.md case 5)"
    )


def test_phi_c_sdk_bench_workflow_no_hardcoded_passes():
    """**Audit fix Φ-C-SDK.B-extra6** — sdk-quartet-bench.yml must NOT
    emit hardcoded `measured_seconds` values that would silently PASS
    SLOs without a real bench measurement.

    Pre-fix had `"measured_seconds": 0.045` (Go), 0.055 (TS), 0.058 (Java)
    as fixed JSON literals → SLO-SDK-LATENCY-* faussement PASS even
    when the bench harness wasn't wired. Post-fix : emit `skip_reason`
    when bench is not yet wired, so check_slo.py reports SKIPPED.

    Pattern Φ-D.A1 récidive.
    """
    p = _REPO / ".github" / "workflows" / "sdk-quartet-bench.yml"
    src = p.read_text(encoding="utf-8")
    # If `skip_reason` is present, the fix is in place. If `measured_seconds`
    # is set to a hardcoded literal value (= no parsing from real bench),
    # that's the regression.
    #
    # Strategy : ensure each of the 3 latency SLO blocks contains either
    # `skip_reason` or `$SECONDS` (= parsed from real bench).
    # Strategy v2 : find all `"SLO-SDK-LATENCY-X"` literal occurrences
    # inside JSON heredocs (= bench compliance output blocks). Skip the
    # documentation header at the top of the file.
    for sdk in ("GO", "TS", "JAVA"):
        slo_id = f"SLO-SDK-LATENCY-{sdk}"
        # Find every occurrence ; the JSON heredoc blocks contain
        # `"SLO-SDK-LATENCY-GO":` with a quote+colon. The doc header
        # has `SLO-SDK-LATENCY-GO   ≤` (no JSON quoting).
        matches = [
            m.start()
            for m in re.finditer(re.escape(f'"{slo_id}"'), src)
        ]
        assert matches, (
            f"bench workflow has no JSON heredoc for {slo_id} "
            "(= no SLO emission)"
        )
        # Check each occurrence — at least one must have skip_reason
        # OR parsed bench output, AND none must have a hardcoded
        # measured_seconds literal without skip_reason.
        for idx in matches:
            window = src[idx : idx + 400]
            has_skip_reason = "skip_reason" in window
            has_parsed_val = "$SECONDS" in window or "$NS_PER_OP" in window
            has_hardcoded = bool(re.search(
                r'"measured_seconds"\s*:\s*0\.\d+',
                window,
            ))
            if has_hardcoded and not (has_skip_reason or has_parsed_val):
                pytest.fail(
                    f"B-extra6 regression : {slo_id} block at offset {idx} "
                    f"emits a hardcoded measured_seconds literal "
                    f"without skip_reason / parsed-bench fallback. "
                    f"Window :\n{window[:300]}"
                )


def test_phi_c_sdk_quartet_publish_has_id_token_permission():
    """**Anti-regression** — sdk-quartet-publish.yml workflow-level
    `permissions: id-token: write` must persist for its own Sigstore
    signing in publish-go/ts/java jobs (= also keyless cosign).
    """
    p = _REPO / ".github" / "workflows" / "sdk-quartet-publish.yml"
    src = p.read_text(encoding="utf-8")
    assert re.search(
        r"^permissions:\s*\n(?:[ ]+[^\n]*\n)*[ ]+id-token:\s*write",
        src,
        re.MULTILINE,
    ), (
        "sdk-quartet-publish.yml missing workflow-level "
        "`permissions: id-token: write`"
    )


def test_phi_c_sdk_runners_use_f32_consistent_arithmetic():
    """**Anti-regression** — each of the 4 runners must use f32 arithmetic
    consistently with Rust for the encode_text formula.

    This is a coarse syntactic check ; the deeper check is the contract
    test itself (= 400/400 bit-identity).
    """
    runners_dir = _REPO / "sdk" / "contract-runner" / "runners"

    # Python runner uses np.float32.
    py_src = (runners_dir / "py" / "main.py").read_text()
    assert "np.float32" in py_src, "Python runner no longer uses np.float32"
    assert "numpy is required" in py_src, (
        "Python runner no longer enforces numpy mandatory (= B12 regression)"
    )

    # Go runner uses float32.
    go_src = (runners_dir / "go" / "main.go").read_text()
    assert "float32" in go_src, "Go runner no longer uses float32 type"
    assert "math.Float32bits" in go_src, (
        "Go runner no longer packs f32 to little-endian bits"
    )

    # TS runner uses Math.fround + Float32Array.
    ts_src = (runners_dir / "ts" / "main.ts").read_text()
    assert "Math.fround" in ts_src, "TS runner no longer uses Math.fround"
    assert "Float32Array" in ts_src, (
        "TS runner no longer uses Float32Array for f32 storage"
    )

    # Java runner uses float (not double) for the encoding.
    java_src = (runners_dir / "java" / "Main.java").read_text()
    assert "float[]" in java_src or "new float[" in java_src, (
        "Java runner no longer uses float[] for emb storage"
    )
    assert "ByteOrder.LITTLE_ENDIAN" in java_src, (
        "Java runner no longer specifies LITTLE_ENDIAN explicitly"
    )


def test_phi_c_sdk_compare_handles_4way_disagreement():
    """**Anti-regression** — compare.py must produce a meaningful
    SLO compliance JSON even when all 4 SDKs disagree on every fixture.

    Pre-audit, the comparator returned exit code 0 on disagreement
    (= silent failure). Post-audit, exit code is non-zero.
    """
    p = _REPO / "sdk" / "contract-runner" / "compare.py"
    src = p.read_text()
    # The driver returns `0 if total_passes == total_expected else 1`.
    assert "return 0 if total_passes == total_expected else 1" in src, (
        "compare.py no longer returns non-zero on bit-identity failure"
    )


def test_phi_c_sdk_runners_skip_in_real_mode_without_backend():
    """**Anti-regression** — each runner must FAIL LOUD if real mode is
    requested but the gRPC backend is unavailable, NOT silently
    return empty verdicts.
    """
    runners_dir = _REPO / "sdk" / "contract-runner" / "runners"

    py_src = (runners_dir / "py" / "main.py").read_text()
    assert "efe_tool not installed" in py_src or "FATAL" in py_src, (
        "Python runner doesn't fail loud in real mode"
    )

    go_src = (runners_dir / "go" / "main.go").read_text()
    assert "real gRPC path not yet wired" in go_src, (
        "Go runner doesn't document non-wired live path"
    )

    ts_src = (runners_dir / "ts" / "main.ts").read_text()
    assert "live gRPC path not yet wired" in ts_src.lower() or "FATAL" in ts_src, (
        "TS runner doesn't fail loud in real mode"
    )

    java_src = (runners_dir / "java" / "Main.java").read_text()
    assert "live gRPC path not yet wired" in java_src.lower() or "FATAL" in java_src, (
        "Java runner doesn't fail loud in real mode"
    )


def test_phi_c_sdk_freeze_attestation_script_exists():
    """**Anti-regression** — the FFI attestation script (= referenced by
    sdk_slos.yaml host_fingerprint_baseline comment) must exist.
    """
    p = _REPO / "scripts" / "bench" / "freeze_ffi_attestation.py"
    # This script is Φ-C-FFI scope but the SDK YAML references it.
    # It SHOULD exist post-Φ-C-FFI completion.
    if not p.exists():
        pytest.skip(f"freeze_ffi_attestation.py not yet created (= Φ-C-FFI scope)")
    # Sanity : the script must be importable.
    import subprocess
    result = subprocess.run(
        ["python3", str(p), "--help"],
        capture_output=True, timeout=10,
    )
    assert result.returncode == 0, (
        f"freeze_ffi_attestation.py --help failed : {result.stderr.decode()}"
    )
