"""SignatureVerifier — scripted cosign runner."""

from __future__ import annotations

from pathlib import Path

import pytest

from efe_registry.config import SigningSettings
from efe_registry.errors import NoSigningMethodConfigured
from efe_registry.signing.cache import InMemorySignatureCache
from efe_registry.signing.models import (
    SigningMethod,
    VerificationInput,
    VerificationLayer,
)
from efe_registry.signing.verifier import (
    ScriptedCosignRunner,
    SignatureVerifier,
)


@pytest.mark.asyncio
async def test_verify_image_with_kms_ok():
    runner = ScriptedCosignRunner(
        responses={
            "verify --output json --key awskms:///alias/t "
            "efe-registry.io/foo:v1": (0, '{"ok":true}', ""),
        },
    )
    verifier = SignatureVerifier(
        settings=SigningSettings(kms_key_arn="awskms:///alias/t"),
        runner=runner,
        cache=InMemorySignatureCache(),
    )
    res = await verifier.verify(VerificationInput(
        artifact_id="a-1",
        layer=VerificationLayer.L1_PRE_PUBLISH,
        oci_ref="efe-registry.io/foo:v1",
    ))
    assert res.ok
    assert res.method == SigningMethod.KEY_KMS


@pytest.mark.asyncio
async def test_verify_image_returns_ok_false_on_bad_signature():
    runner = ScriptedCosignRunner(
        responses={},
        default=(1, "", "cosign: signature does not match"),
    )
    verifier = SignatureVerifier(
        settings=SigningSettings(kms_key_arn="awskms:///alias/t"),
        runner=runner,
        cache=InMemorySignatureCache(),
    )
    res = await verifier.verify(VerificationInput(
        artifact_id="a-2",
        layer=VerificationLayer.L2_ADMISSION,
        oci_ref="efe-registry.io/foo:v2",
    ))
    assert not res.ok


@pytest.mark.asyncio
async def test_verify_blob_missing_signature_raises():
    verifier = SignatureVerifier(
        settings=SigningSettings(kms_key_arn="awskms:///alias/t"),
        runner=ScriptedCosignRunner(responses={}),
    )
    with pytest.raises(Exception):
        await verifier.verify(VerificationInput(
            artifact_id="a-3",
            layer=VerificationLayer.L3_PRE_LOAD,
            blob_path=Path("/tmp/foo"),
            signature_path=None,
        ))


@pytest.mark.asyncio
async def test_no_method_configured_raises():
    verifier = SignatureVerifier(
        settings=SigningSettings(),  # neither kms nor keyless
        runner=ScriptedCosignRunner(responses={}),
    )
    with pytest.raises(NoSigningMethodConfigured):
        await verifier.verify(VerificationInput(
            artifact_id="a-4",
            layer=VerificationLayer.L1_PRE_PUBLISH,
            oci_ref="efe-registry.io/foo:v4",
        ))


@pytest.mark.asyncio
async def test_cache_hit_short_circuits_cosign():
    runner = ScriptedCosignRunner(
        responses={
            "verify --output json --key awskms:///alias/t "
            "efe-registry.io/foo:v5": (0, '{"ok":true}', ""),
        },
        calls=[],
    )
    verifier = SignatureVerifier(
        settings=SigningSettings(
            kms_key_arn="awskms:///alias/t", verify_cache_ttl_s=60,
        ),
        runner=runner,
    )
    inp = VerificationInput(
        artifact_id="a-5",
        layer=VerificationLayer.L1_PRE_PUBLISH,
        oci_ref="efe-registry.io/foo:v5",
    )
    r1 = await verifier.verify(inp)
    r2 = await verifier.verify(inp)
    assert r1.ok and r2.ok
    # runner called once only
    assert len(runner.calls or []) == 1
