"""efe_dry_run_deploy + dry_run.compute_diff pin tests (M18 D49)."""
from __future__ import annotations

import pytest

from mcp_server.deploy.dry_run import compute_diff


def _target_manifest(**overrides):
    base = {
        "slug": "x",
        "version": "v2.0.0",
        "manifestSha256": "newsha",
        "framework": "hash",
        "model_name": "fake-model-v2",
        "dim": 384,
        "is_dev_unsigned": False,
    }
    base.update(overrides)
    return base


def _current(**overrides):
    base = {
        "bundle": {
            "slug": "x",
            "version": "v1.0.0",
            "manifest_sha256": "oldsha",
        },
        "manifest": {
            "framework": "hash",
            "model_name": "fake-model-v1",
            "dim": 384,
            "manifestSha256": "oldsha",
        },
    }
    base.update(overrides)
    return base


# ─────────────────────────────────────────────────────────────────────
# compute_diff unit tests
# ─────────────────────────────────────────────────────────────────────


def test_compute_diff_no_current_targets_only():
    """When current is None, manifest_changed is True and current_* are None."""
    diff = compute_diff(
        current=None,
        target_manifest=_target_manifest(),
        target_signature_verified=True,
    )
    assert diff.manifest_changed is True
    assert diff.current_slug is None
    assert diff.target_slug == "x"


def test_compute_diff_same_sha_no_change():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(manifestSha256="oldsha"),
        target_signature_verified=True,
    )
    assert diff.manifest_changed is False
    assert diff.expected_replica_restarts == 0


def test_compute_diff_dim_change_creates_blocker():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(dim=768),
        target_signature_verified=True,
    )
    assert diff.dim_changed is True
    assert any(r.level == "BLOCKER" and r.category == "dim_change" for r in diff.risks)
    assert diff.has_blockers is True


def test_compute_diff_framework_change_high_risk():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(framework="sentence-transformers"),
        target_signature_verified=True,
    )
    assert diff.framework_changed is True
    assert any(r.level == "HIGH" for r in diff.risks)


def test_compute_diff_model_name_change_medium_risk():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(model_name="brand-new-model"),
        target_signature_verified=True,
    )
    assert diff.model_name_changed is True
    assert any(r.category == "model_name_change" for r in diff.risks)


def test_compute_diff_unsigned_target_blocker():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(is_dev_unsigned=True),
        target_signature_verified=True,    # even if "verified", dev_unsigned blocks
    )
    assert any(r.category == "unsigned_bundle" for r in diff.risks)
    assert diff.has_blockers is True


def test_compute_diff_signature_unverified_blocker():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(),
        target_signature_verified=False,
    )
    assert any(r.category == "signature_unverified" for r in diff.risks)
    assert diff.has_blockers is True


def test_compute_diff_signature_unknown_medium_risk():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(),
        target_signature_verified=None,
    )
    assert any(r.category == "signature_unknown" for r in diff.risks)
    assert diff.highest_risk_level in ("MEDIUM", "HIGH", "BLOCKER")


def test_compute_diff_rolling_update_zero_downtime():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(),
        target_signature_verified=True,
        rolling_update=True,
    )
    assert diff.expected_downtime_seconds_p99 == 0


def test_compute_diff_recreate_strategy_estimates_downtime():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(),
        target_signature_verified=True,
        rolling_update=False,
        desired_replicas=4,
    )
    assert diff.expected_downtime_seconds_p99 == 4 * 30


def test_compute_diff_to_dict_shape():
    diff = compute_diff(
        current=_current(),
        target_manifest=_target_manifest(),
        target_signature_verified=True,
    )
    out = diff.to_dict()
    for key in (
        "current", "target", "diff", "risks",
        "blocking_issues", "recommended_actions",
        "highest_risk_level", "has_blockers",
    ):
        assert key in out


# ─────────────────────────────────────────────────────────────────────
# Tool integration
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def deploy_seeded(fake_oci_registry, fake_backend, monkeypatch):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v2.0.0",
        annotations={
            "org.efe.bundle.framework": "hash",
            "org.efe.bundle.model_name": "fake-model-v2",
            "org.efe.bundle.dim": "384",
        },
    )
    fake_backend.set_deployment_state({
        "bundle": {
            "slug": "x", "version": "v1.0.0",
            "manifest_sha256": "oldsha",
        },
        "manifest": {
            "framework": "hash",
            "model_name": "fake-model-v1",
            "dim": 384,
        },
    })


@pytest.mark.asyncio
async def test_dry_run_deploy_local_fallback(
    agent_native_dispatcher, deploy_seeded,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_dry_run_deploy",
        args={"slug": "x", "version": "v2.0.0"},
    )
    assert res.is_error is False, res.content
    assert res.content["source"] == "fallback_local"
    assert res.content["target"]["bundle"]["version"] == "v2.0.0"
    assert "diff" in res.content


@pytest.mark.asyncio
async def test_dry_run_deploy_backend_path(
    agent_native_dispatcher, deploy_seeded, fake_backend,
):
    fake_backend.enable_dry_run_endpoint(True)
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_dry_run_deploy",
        args={"slug": "x", "version": "v2.0.0"},
    )
    assert res.is_error is False
    assert res.content["source"] == "backend_mocked"


@pytest.mark.asyncio
async def test_dry_run_deploy_unknown_bundle_returns_not_found(
    agent_native_dispatcher, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_dry_run_deploy",
        args={"slug": "missing", "version": "v9.9.9"},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_found"


@pytest.mark.asyncio
async def test_dry_run_deploy_invalid_version_pattern_rejected(
    agent_native_dispatcher,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_dry_run_deploy",
        args={"slug": "x", "version": "1.0"},   # missing v prefix + patch
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"
