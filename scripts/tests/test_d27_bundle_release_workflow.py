"""Sprint **M13 D27** — Pin tests : `.github/workflows/bundle-release.yml`
+ M13 build/sign/verify scripts structural invariants.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §6 + §14 + §16.4.

12 tests covering :
  1. Workflow file present + parseable YAML
  2. Triggers on tags `bundle-v*.*.*` + workflow_dispatch
  3. Runner pinned to `ubuntu-22.04` (T-M13-3 reproducibility)
  4. Has reproducibility check (build twice, compare)
  5. Uses ORAS for OCI push
  6. Has post-publish verify-pull job
  7. Pinned ORAS / cosign / cyclonedx-cli versions
  8. Refuses publish without signature
  9. concurrency cancel-in-progress=false
  10. permissions least-privilege (id-token: write only for cosign)
  11. Loads privkey from GH Secret + masks immediately (T-M13-6)
  12. Signing job uses `environment: production` to gate secrets
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

REPO = Path(__file__).resolve().parents[2]
WORKFLOW = REPO / ".github" / "workflows" / "bundle-release.yml"
SCRIPTS = REPO / "scripts" / "release"


def _load_workflow() -> dict:
    text = WORKFLOW.read_text(encoding="utf-8")
    return yaml.safe_load(text)


def _workflow_text() -> str:
    return WORKFLOW.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# Test 1 : workflow file present
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_file_present() -> None:
    assert WORKFLOW.is_file(), f"M13 D27 workflow missing : {WORKFLOW}"
    # YAML must parse (raises YAMLError on failure)
    doc = _load_workflow()
    assert "jobs" in doc
    assert "name" in doc and doc["name"] == "bundle-release"


# ─────────────────────────────────────────────────────────────────────
# Test 2 : triggers on bundle-v* tag + workflow_dispatch
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_triggers_on_bundle_tag_and_dispatch() -> None:
    doc = _load_workflow()
    # PyYAML parses `on:` as the boolean key True. Handle both.
    triggers = doc.get("on") or doc.get(True)
    assert isinstance(triggers, dict)
    push = triggers.get("push", {})
    assert isinstance(push, dict)
    tags = push.get("tags", [])
    assert any("bundle-v" in t for t in tags), (
        f"D27 workflow must trigger on bundle-v* tag, got : {tags}"
    )
    assert "workflow_dispatch" in triggers


# ─────────────────────────────────────────────────────────────────────
# Test 3 : runner pinned to ubuntu-22.04 (no `ubuntu-latest`)
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_pins_runner_to_ubuntu_22_04() -> None:
    """Threat T-M13-3 : `ubuntu-latest` rolls forward and breaks repro."""
    text = _workflow_text()
    assert "ubuntu-22.04" in text
    assert "ubuntu-latest" not in text, (
        "T-M13-3 violation : `ubuntu-latest` rolls forward, breaks reproducibility"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 4 : reproducibility check (build twice + verify-repro job)
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_has_reproducibility_check_step() -> None:
    doc = _load_workflow()
    jobs = doc["jobs"]
    assert "build-1" in jobs and "build-2" in jobs, (
        "D27 must have build-1 + build-2 parallel jobs for repro check"
    )
    assert "verify-repro" in jobs, (
        "D27 must have verify-repro job that compares manifest SHA"
    )
    text = _workflow_text()
    assert "REPRODUCIBILITY_FAIL" in text or "REPRODUCIBILITY_OK" in text


# ─────────────────────────────────────────────────────────────────────
# Test 5 : uses ORAS for OCI push
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_uses_oras_push() -> None:
    text = _workflow_text()
    assert "oras push" in text, (
        "D27 must use `oras push` for OCI Artifacts publication"
    )
    assert "application/vnd.efe.bundle.v1+json" in text, (
        "D27 must declare canonical EFE bundle artifact-type"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 6 : post-publish verify
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_post_publish_verify_step() -> None:
    doc = _load_workflow()
    assert "verify-pull" in doc["jobs"], (
        "D27 must have verify-pull job (post-publish integrity check)"
    )
    text = _workflow_text()
    assert "verify_pulled_bundle.py" in text


# ─────────────────────────────────────────────────────────────────────
# Test 7 : pinned tool versions (no `:latest` / `@main` / unspecified)
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_uses_pinned_tool_versions() -> None:
    """Threat T-M13-3 + supply chain : never use `latest` action versions."""
    text = _workflow_text()
    # action versions must be pinned (e.g., @v4 not @main)
    action_refs = re.findall(r"uses:\s*[^\s@]+@(\S+)", text)
    for ref in action_refs:
        assert ref != "main" and ref != "master", (
            f"GH Action ref {ref!r} must be pinned to a tag/SHA, not branch"
        )
    # ORAS / cosign / cyclonedx versions must be set in env
    assert "ORAS_VERSION" in text
    assert "COSIGN_VERSION" in text
    assert "CYCLONEDX_CLI_VERSION" in text


# ─────────────────────────────────────────────────────────────────────
# Test 8 : refuses publish without signature
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_no_unsigned_publish() -> None:
    """The publish job MUST come AFTER the sign + license-gate jobs."""
    doc = _load_workflow()
    jobs = doc["jobs"]
    publish = jobs.get("publish", {})
    needs = publish.get("needs", [])
    if isinstance(needs, str):
        needs = [needs]
    # publish must depend transitively on sign (via license-gate→sbom→sign)
    upstream = set(needs)
    # Walk job dep graph
    queue = list(upstream)
    while queue:
        j = queue.pop()
        if j in jobs:
            j_needs = jobs[j].get("needs", [])
            if isinstance(j_needs, str):
                j_needs = [j_needs]
            for n in j_needs:
                if n not in upstream:
                    upstream.add(n)
                    queue.append(n)
    assert "sign" in upstream, (
        "D27 publish job must transitively depend on sign job"
    )
    assert "license-gate" in upstream, (
        "D27 publish job must transitively depend on license-gate"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 9 : concurrency cancel-in-progress=false
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_concurrency_does_not_cancel_in_progress() -> None:
    """Threat T-M13-4 : never cancel an in-flight bundle release
    (would leave OCI inconsistent)."""
    doc = _load_workflow()
    conc = doc.get("concurrency", {})
    assert conc.get("cancel-in-progress") is False, (
        "D27 must declare cancel-in-progress: false"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 10 : permissions least-privilege
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_permissions_least_privilege() -> None:
    """Workflow-level permissions must declare only what's needed."""
    doc = _load_workflow()
    perms = doc.get("permissions", {})
    # contents only `read` (no write at workflow level — just for checkout)
    assert perms.get("contents") == "read"
    # packages write needed for oras push to GHCR
    assert perms.get("packages") == "write"
    # id-token write needed for OIDC (cosign keyless future)
    assert perms.get("id-token") == "write"


# ─────────────────────────────────────────────────────────────────────
# Test 11 : privkey loaded from secret + masked (T-M13-6)
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_loads_privkey_from_secret_and_masks() -> None:
    """T-M13-6 anti-regression : privkey MUST be masked in logs."""
    text = _workflow_text()
    assert "secrets.EFE_BUNDLE_PRIVKEY_PEM" in text, (
        "D27 must load privkey from secrets.EFE_BUNDLE_PRIVKEY_PEM"
    )
    assert "::add-mask::${PRIVKEY_PEM}" in text, (
        "T-M13-6 violation : privkey must be masked in logs immediately after load"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 12 : sign job uses environment: production
# ─────────────────────────────────────────────────────────────────────


def test_d27_workflow_sign_job_uses_production_environment() -> None:
    """Audit fix M13-A1 : sign job must be gated by GH environment so
    secrets are scoped + auditable."""
    doc = _load_workflow()
    sign_job = doc["jobs"].get("sign", {})
    assert sign_job.get("environment") == "production", (
        "D27 sign job must declare `environment: production` to gate "
        "secret access (M13-A1 mitigation)"
    )
