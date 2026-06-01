"""Pre-registration — HMAC sealing + tamper detection (invariant I6)."""

from __future__ import annotations


import pytest

from datetime import datetime, timezone

from efe_rollout.errors import (
    PreRegistrationSignatureInvalid,
    PreRegistrationSpecHashMismatch,
)
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.prereg import PreRegistration, seal, verify


def _spec(name: str = "test") -> ExperimentSpec:
    return ExperimentSpec(
        name=name,
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True),
            Variant(name="treatment", allocation_pct=50),
        ),
    )


def test_seal_and_verify_roundtrip():
    key = b"k" * 32
    s = _spec()
    prereg = seal(
        experiment_id="exp-1",
        spec=s,
        signing_key=key,
        signing_key_id="kms/alias-1",
        now=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )
    verify(prereg, signing_key=key, spec=s)


def test_verify_rejects_wrong_key():
    key = b"k" * 32
    s = _spec()
    prereg = seal(
        experiment_id="exp-1",
        spec=s,
        signing_key=key,
        signing_key_id="kms/alias-1",
        now=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )
    with pytest.raises(PreRegistrationSignatureInvalid):
        verify(prereg, signing_key=b"w" * 32, spec=s)


def test_verify_detects_plan_json_tamper():
    key = b"k" * 32
    s = _spec()
    prereg = seal(
        experiment_id="exp-1", spec=s,
        signing_key=key,
        signing_key_id="kms/alias-1",
        now=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )
    tampered_plan = prereg.plan_json.replace("prompt", "model")
    tampered = PreRegistration(
        experiment_id=prereg.experiment_id,
        spec_hash=prereg.spec_hash,
        signature=prereg.signature,
        plan_json=tampered_plan,
        signing_key_id=prereg.signing_key_id,
        created_at=prereg.created_at,
    )
    with pytest.raises(PreRegistrationSignatureInvalid):
        verify(tampered, signing_key=key)


def test_verify_detects_spec_mutation_after_seal():
    key = b"k" * 32
    s = _spec()
    prereg = seal(
        experiment_id="exp-1", spec=s,
        signing_key=key,
        signing_key_id="kms/alias-1",
        now=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )
    mutated = ExperimentSpec(
        name=s.name,
        tenant_id=s.tenant_id,
        task_id=s.task_id,
        layer=s.layer,
        kind=s.kind,
        owner=s.owner,
        hypothesis="DIFFERENT",   # critical field changed
        primary_metric=s.primary_metric,
        variants=s.variants,
    )
    with pytest.raises(PreRegistrationSpecHashMismatch):
        verify(prereg, signing_key=key, spec=mutated)
