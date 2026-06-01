"""Dataclasses + enums for the signing subsystem."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path


class SigningMethod(str, Enum):
    KEY_KMS = "key_kms"
    KEYLESS_OIDC = "keyless_oidc"
    ATTESTATION = "attestation"


class VerificationLayer(str, Enum):
    L1_PRE_PUBLISH = "L1"
    L2_ADMISSION = "L2"
    L3_PRE_LOAD = "L3"
    L4_RUNTIME = "L4"


@dataclass(frozen=True, slots=True)
class VerificationInput:
    """What the verifier is asked to verify.

    Exactly one of (``oci_ref``, ``blob_path``) must be supplied.
    """

    artifact_id: str
    layer: VerificationLayer
    oci_ref: str | None = None
    blob_path: Path | None = None
    signature_path: Path | None = None
    certificate_path: Path | None = None
    require_attestations: tuple[str, ...] = field(default_factory=tuple)


@dataclass(frozen=True, slots=True)
class VerificationResult:
    ok: bool
    method: SigningMethod
    key_id: str | None
    rekor_uuid: str | None
    verified_at_ns: int
    layer: VerificationLayer
    details: dict[str, str]


__all__ = [
    "SigningMethod",
    "VerificationInput",
    "VerificationLayer",
    "VerificationResult",
]
