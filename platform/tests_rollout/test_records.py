"""Exposure HMAC signing + verification."""

from __future__ import annotations

from datetime import datetime, timezone


from efe_rollout.records import Exposure, Outcome


def _mk_exp(**ov) -> Exposure:
    base = dict(
        exposure_id="exp-1",
        experiment_id="e-1",
        tenant_id="acme",
        user_id="u-1",
        assigned_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        layer="prompt",
        forced=False,
        holdout=False,
        variant_id="v-1",
    )
    base.update(ov)
    return Exposure(**base)


def test_sign_then_verify():
    key = b"k" * 32
    e = _mk_exp()
    signed = e.sign(key)
    assert signed.hmac_sig
    assert signed.verify(key)


def test_verify_rejects_tampered_variant():
    key = b"k" * 32
    signed = _mk_exp().sign(key)
    # Caller forged the variant id but kept the signature.
    tampered = Exposure(
        exposure_id=signed.exposure_id,
        experiment_id=signed.experiment_id,
        tenant_id=signed.tenant_id,
        user_id=signed.user_id,
        assigned_at=signed.assigned_at,
        layer=signed.layer,
        forced=signed.forced,
        holdout=signed.holdout,
        variant_id="v-TAMPERED",
        hmac_sig=signed.hmac_sig,
    )
    assert not tampered.verify(key)


def test_verify_unsigned_exposure_fails():
    e = _mk_exp()
    assert not e.verify(b"k" * 32)


def test_outcome_carries_covariate():
    o = Outcome(
        outcome_id="o-1",
        experiment_id="e-1",
        tenant_id="acme",
        user_id="u-1",
        variant_id="v-1",
        metric_name="ctr",
        metric_value=0.5,
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        covariate={"country": "FR", "device": "mobile"},
    )
    assert o.covariate["country"] == "FR"
