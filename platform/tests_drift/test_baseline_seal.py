"""Baseline seal — signature + tamper detection."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_drift.baseline import SealedBaseline, seal_baseline, verify_baseline
from efe_drift.errors import (
    BaselineSignatureInvalid,
    BaselineSpecHashMismatch,
)
from efe_drift.records import BaselineSpec


def _spec() -> BaselineSpec:
    return BaselineSpec(
        tenant_id="acme",
        task_id="summarize",
        feature_name="prompt_length",
        feature_kind="numeric",
        window_start=datetime(2026, 4, 1, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 8, tzinfo=timezone.utc),
        sample_count=12_345,
        bins=(0.0, 10.0, 20.0, 30.0),
        stats={"mean": 15.0, "std": 5.0, "n": 12345.0},
    )


def test_seal_verify_roundtrip():
    key = b"k" * 32
    sealed = seal_baseline(
        spec=_spec(),
        signing_key=key,
        signing_key_id="kms/alias-1",
        sealed_at=datetime(2026, 4, 8, tzinfo=timezone.utc),
    )
    verify_baseline(sealed, signing_key=key, spec=_spec())


def test_seal_rejects_short_signing_key():
    with pytest.raises(ValueError):
        seal_baseline(
            spec=_spec(),
            signing_key=b"short",
            signing_key_id="kms/alias-1",
            sealed_at=datetime.now(tz=timezone.utc),
        )


def test_verify_rejects_wrong_key():
    key = b"k" * 32
    sealed = seal_baseline(
        spec=_spec(),
        signing_key=key,
        signing_key_id="kms/alias-1",
        sealed_at=datetime(2026, 4, 8, tzinfo=timezone.utc),
    )
    with pytest.raises(BaselineSignatureInvalid):
        verify_baseline(sealed, signing_key=b"w" * 32)


def test_verify_rejects_tampered_plan_json():
    key = b"k" * 32
    sealed = seal_baseline(
        spec=_spec(),
        signing_key=key,
        signing_key_id="kms/alias-1",
        sealed_at=datetime(2026, 4, 8, tzinfo=timezone.utc),
    )
    tampered = SealedBaseline(
        spec_hash=sealed.spec_hash,
        signature=sealed.signature,
        plan_json=sealed.plan_json.replace("summarize", "hijacked"),
        signing_key_id=sealed.signing_key_id,
        sealed_at=sealed.sealed_at,
    )
    with pytest.raises(BaselineSignatureInvalid):
        verify_baseline(tampered, signing_key=key)


def test_verify_detects_spec_mutation_after_seal():
    key = b"k" * 32
    sealed = seal_baseline(
        spec=_spec(),
        signing_key=key,
        signing_key_id="kms/alias-1",
        sealed_at=datetime(2026, 4, 8, tzinfo=timezone.utc),
    )
    mutated = BaselineSpec(
        tenant_id="acme",
        task_id="summarize",
        feature_name="prompt_length",
        feature_kind="numeric",
        window_start=datetime(2026, 4, 1, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 8, tzinfo=timezone.utc),
        sample_count=99_999,     # changed
        bins=(0.0, 10.0, 20.0, 30.0),
        stats={"mean": 15.0, "std": 5.0, "n": 12345.0},
    )
    with pytest.raises(BaselineSpecHashMismatch):
        verify_baseline(sealed, signing_key=key, spec=mutated)
