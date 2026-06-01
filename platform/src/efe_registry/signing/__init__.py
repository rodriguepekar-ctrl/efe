"""Cryptographic signing + verification for artifacts."""

from __future__ import annotations

from efe_registry.signing.cache import InMemorySignatureCache, SignatureCache
from efe_registry.signing.models import (
    SigningMethod,
    VerificationInput,
    VerificationLayer,
    VerificationResult,
)
from efe_registry.signing.verifier import (
    CosignRunner,
    NullCosignRunner,
    ScriptedCosignRunner,
    SignatureVerifier,
)

__all__ = [
    "CosignRunner",
    "InMemorySignatureCache",
    "NullCosignRunner",
    "ScriptedCosignRunner",
    "SignatureCache",
    "SignatureVerifier",
    "SigningMethod",
    "VerificationInput",
    "VerificationLayer",
    "VerificationResult",
]
