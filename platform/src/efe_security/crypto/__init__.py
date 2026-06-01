"""Cryptographic primitives with algorithm agility.

Exposes :

* Algorithm IDs (stable strings) and a :class:`CryptoProvider` Protocol.
* :class:`DefaultCryptoProvider` — the production implementation.
* :func:`canonical_json`, :func:`hmac_seal`, :func:`verify_seal` —
  re-exports with M11 domain tags.
* :func:`csprng_bytes` — audited CSPRNG.
"""

from __future__ import annotations

from efe_security.crypto.agility import (
    ALG_AES256_GCM_V1,
    ALG_ED25519_V1,
    ALG_ES256_V1,
    ALG_HMAC_SHA256_V1,
    AlgorithmRegistry,
    default_registry,
    is_algorithm_deprecated,
    mark_deprecated,
)
from efe_security.crypto.primitives import (
    DOMAIN_TAG_AUDIT_CHAIN,
    DOMAIN_TAG_CONSENT_SEAL,
    DOMAIN_TAG_DSAR_RECEIPT,
    DOMAIN_TAG_INCIDENT_SEAL,
    DOMAIN_TAG_KEY_ROTATION_V1,
    DOMAIN_TAG_PDP_CACHE,
    DOMAIN_TAG_PII_FIELD_HASH,
    DOMAIN_TAG_SESSION_OPAQUE,
    DOMAIN_TAG_TENANT_DEK,
    DOMAIN_TAG_USER_ID,
    CryptoProvider,
    DefaultCryptoProvider,
    Envelope,
    SecureBytes,
    canonical_json,
    current_key,
    hmac_seal,
    reset_key,
    set_key,
    verify_seal,
)
from efe_security.crypto.prng import audit_prng_call, csprng_bytes

__all__ = [
    "ALG_AES256_GCM_V1",
    "ALG_ED25519_V1",
    "ALG_ES256_V1",
    "ALG_HMAC_SHA256_V1",
    "AlgorithmRegistry",
    "CryptoProvider",
    "DOMAIN_TAG_AUDIT_CHAIN",
    "DOMAIN_TAG_CONSENT_SEAL",
    "DOMAIN_TAG_DSAR_RECEIPT",
    "DOMAIN_TAG_INCIDENT_SEAL",
    "DOMAIN_TAG_KEY_ROTATION_V1",
    "DOMAIN_TAG_PDP_CACHE",
    "DOMAIN_TAG_PII_FIELD_HASH",
    "DOMAIN_TAG_SESSION_OPAQUE",
    "DOMAIN_TAG_TENANT_DEK",
    "DOMAIN_TAG_USER_ID",
    "DefaultCryptoProvider",
    "Envelope",
    "SecureBytes",
    "audit_prng_call",
    "canonical_json",
    "csprng_bytes",
    "current_key",
    "default_registry",
    "hmac_seal",
    "is_algorithm_deprecated",
    "mark_deprecated",
    "reset_key",
    "set_key",
    "verify_seal",
]
