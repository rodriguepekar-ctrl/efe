"""K8s admission policy rendering + break-glass."""

from __future__ import annotations

import pytest

from efe_registry.admission.override import (
    BreakGlassTracker,
    InMemoryOverrideAudit,
)
from efe_registry.admission.policy import (
    ClusterImagePolicy,
    render_policy_yaml,
)
from efe_registry.clock import FakeClock
from efe_registry.config import AdmissionSettings, SigningSettings


def test_render_policy_yaml_key_based():
    policy = ClusterImagePolicy.from_settings(
        admission=AdmissionSettings(),
        signing=SigningSettings(kms_key_arn="awskms:///alias/t"),
    )
    out = render_policy_yaml(policy)
    assert "apiVersion: policy.sigstore.dev/v1beta1" in out
    assert 'kms: "awskms:///alias/t"' in out
    assert "mode: enforce" in out


def test_render_policy_yaml_with_secondary_key_rotation():
    policy = ClusterImagePolicy.from_settings(
        admission=AdmissionSettings(),
        signing=SigningSettings(
            kms_key_arn="awskms:///alias/t",
            kms_key_arn_secondary="awskms:///alias/t-next",
        ),
    )
    out = render_policy_yaml(policy)
    assert 'kms: "awskms:///alias/t"' in out
    assert 'kms: "awskms:///alias/t-next"' in out


def test_render_policy_yaml_keyless_only():
    policy = ClusterImagePolicy.from_settings(
        admission=AdmissionSettings(mode="warn"),
        signing=SigningSettings(
            keyless_cert_identity="https://github.com/efe/efe/...",
            keyless_issuer="https://token.actions.githubusercontent.com",
        ),
    )
    out = render_policy_yaml(policy)
    assert "keyless:" in out
    assert "mode: warn" in out


@pytest.mark.asyncio
async def test_break_glass_fire_and_expire():
    clock = FakeClock()
    audit = InMemoryOverrideAudit()
    tracker = BreakGlassTracker(
        settings=AdmissionSettings(break_glass_ttl_s=60),
        audit=audit, clock=clock,
    )
    await tracker.fire(
        namespace="efe-prod", actor="jdoe", reason="incident",
    )
    assert len(tracker.active()) == 1
    assert len(audit.events) == 1
    clock.advance(120.0)
    expired = await tracker.expire_stale()
    assert len(expired) == 1
    assert len(tracker.active()) == 0
    assert len(audit.events) == 2  # fire + revoke
