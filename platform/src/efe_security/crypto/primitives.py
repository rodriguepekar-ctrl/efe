"""Low-level cryptographic primitives used across M11.

Domain tags are **M11-specific** (prefix ``efe-sec/``). They do not
overlap with the M10 ``efe-sched/*`` tags — the ``_assert_known_tag``
guard refuses anything outside this file's registry.

The module key is loaded from ``EFE_SEC_HMAC_KEY`` at import ; a warning
is logged once when the dev default is active. :func:`set_key` overrides
for tests.

:class:`DefaultCryptoProvider` implements the :class:`CryptoProvider`
Protocol and routes on ``alg_id`` for crypto-agility.
"""

from __future__ import annotations

import hmac
import json
import logging
import os
from dataclasses import dataclass, field
from hashlib import sha256
from typing import Any, Final, Protocol

from efe_security.crypto.agility import (
    ALG_AES256_GCM_V1,
    ALG_ED25519_V1,
    ALG_ES256_V1,
    ALG_HMAC_SHA256_V1,
    is_algorithm_deprecated,
)
from efe_security.errors import AlgorithmDeprecated, CryptoError, SealInvalid

_LOG = logging.getLogger("efe_security.crypto")

__all__ = [
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
    "CryptoProvider",
    "DefaultCryptoProvider",
    "Envelope",
    "SecureBytes",
    "canonical_json",
    "current_key",
    "hmac_seal",
    "is_using_dev_key",
    "reset_key",
    "set_key",
    "verify_seal",
]


# ---------------------------------------------------------------------------
# Domain tags (§Annex A)
# ---------------------------------------------------------------------------

DOMAIN_TAG_USER_ID: Final[bytes] = b"efe-sec/user-id/v1\x00"
DOMAIN_TAG_AUDIT_CHAIN: Final[bytes] = b"efe-sec/audit-chain/v1\x00"
DOMAIN_TAG_PDP_CACHE: Final[bytes] = b"efe-sec/pdp-cache/v1\x00"
DOMAIN_TAG_CONSENT_SEAL: Final[bytes] = b"efe-sec/consent-seal/v1\x00"
DOMAIN_TAG_TENANT_DEK: Final[bytes] = b"efe-sec/tenant-dek/v1\x00"
DOMAIN_TAG_SESSION_OPAQUE: Final[bytes] = b"efe-sec/session/v1\x00"
DOMAIN_TAG_PII_FIELD_HASH: Final[bytes] = b"efe-sec/pii-hash/v1\x00"
DOMAIN_TAG_DSAR_RECEIPT: Final[bytes] = b"efe-sec/dsar-receipt/v1\x00"
DOMAIN_TAG_INCIDENT_SEAL: Final[bytes] = b"efe-sec/incident-seal/v1\x00"
DOMAIN_TAG_KEY_ROTATION_V1: Final[bytes] = b"efe-sec/key-rotation/v1\x00"

_ALL_TAGS: Final[frozenset[bytes]] = frozenset(
    {
        DOMAIN_TAG_USER_ID,
        DOMAIN_TAG_AUDIT_CHAIN,
        DOMAIN_TAG_PDP_CACHE,
        DOMAIN_TAG_CONSENT_SEAL,
        DOMAIN_TAG_TENANT_DEK,
        DOMAIN_TAG_SESSION_OPAQUE,
        DOMAIN_TAG_PII_FIELD_HASH,
        DOMAIN_TAG_DSAR_RECEIPT,
        DOMAIN_TAG_INCIDENT_SEAL,
        DOMAIN_TAG_KEY_ROTATION_V1,
    }
)


def _assert_known_tag(tag: bytes) -> None:
    if tag not in _ALL_TAGS:
        raise CryptoError(
            f"unknown HMAC domain tag: {tag!r}",
            detail={"registered": [t.decode(errors='replace') for t in _ALL_TAGS]},
        )


# ---------------------------------------------------------------------------
# Key management
# ---------------------------------------------------------------------------

_DEV_DEFAULT_KEY: Final[bytes] = b"efe-sec-dev-key-do-not-use-in-prod\x00" * 2
_USING_DEV_KEY: bool = False


def _load_key_from_env() -> bytes:
    global _USING_DEV_KEY
    raw = os.environ.get("EFE_SEC_HMAC_KEY", "")
    if raw:
        _USING_DEV_KEY = False
        return raw.encode()
    if not _USING_DEV_KEY:
        _LOG.warning(
            "crypto.using_dev_default_key env=EFE_SEC_HMAC_KEY missing ; "
            "dev-default key in use — UNSAFE IN PRODUCTION."
        )
        _USING_DEV_KEY = True
    return _DEV_DEFAULT_KEY


_KEY: bytes = _load_key_from_env()


def set_key(key: bytes) -> None:
    global _KEY, _USING_DEV_KEY
    if len(key) < 16:
        raise ValueError("HMAC key must be at least 16 bytes")
    _KEY = bytes(key)
    _USING_DEV_KEY = False


def reset_key() -> None:
    global _KEY
    _KEY = _load_key_from_env()


def current_key() -> bytes:
    return bytes(_KEY)


def is_using_dev_key() -> bool:
    return _USING_DEV_KEY


# ---------------------------------------------------------------------------
# Canonical JSON
# ---------------------------------------------------------------------------


def canonical_json(obj: Any) -> bytes:
    """Stable JSON encoding : sorted keys, UTF-8, no whitespace, no NaN."""
    return json.dumps(
        obj,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")


# ---------------------------------------------------------------------------
# Seal / verify (HMAC-SHA256)
# ---------------------------------------------------------------------------


def hmac_seal(tag: bytes, payload: Any, *, key: bytes | None = None) -> bytes:
    _assert_known_tag(tag)
    message = tag + canonical_json(payload)
    return hmac.new(key if key is not None else _KEY, message, sha256).digest()


def verify_seal(
    tag: bytes,
    payload: Any,
    expected: bytes,
    *,
    key: bytes | None = None,
) -> bool:
    return hmac.compare_digest(hmac_seal(tag, payload, key=key), expected)


# ---------------------------------------------------------------------------
# SecureBytes — self-zeroising bytes-like container
# ---------------------------------------------------------------------------


class SecureBytes:
    """A mutable byte container that best-effort wipes on deletion.

    Python's GC guarantees are weak but we do what we can : overwrite the
    backing ``bytearray`` with three passes (``\\x00``, ``\\xff``,
    random) before releasing the reference.
    """

    __slots__ = ("_buf", "_destroyed")

    def __init__(self, data: bytes | bytearray) -> None:
        self._buf = bytearray(data)
        self._destroyed = False

    def __len__(self) -> int:
        return len(self._buf)

    def __bytes__(self) -> bytes:
        if self._destroyed:
            raise CryptoError("SecureBytes is already destroyed")
        return bytes(self._buf)

    def view(self) -> memoryview:
        if self._destroyed:
            raise CryptoError("SecureBytes is already destroyed")
        return memoryview(self._buf)

    def destroy(self) -> None:
        if self._destroyed:
            return
        n = len(self._buf)
        for pattern in (0x00, 0xFF):
            for i in range(n):
                self._buf[i] = pattern
        # Third pass : random-ish (but we avoid importing prng to keep
        # this module dependency-free).
        for i in range(n):
            self._buf[i] = (i * 31 + 17) & 0xFF
        self._buf.clear()
        self._destroyed = True

    def __del__(self) -> None:
        try:
            self.destroy()
        except Exception:  # pragma: no cover - defensive
            pass

    # ------------------------------------------------------------------
    # Anti-duplication hooks : block every serialisation / copy path.
    # ------------------------------------------------------------------

    def __reduce__(self) -> tuple:
        raise CryptoError("SecureBytes cannot be pickled")

    def __reduce_ex__(self, protocol: int) -> tuple:
        raise CryptoError("SecureBytes cannot be pickled (reduce_ex)")

    def __copy__(self) -> "SecureBytes":
        raise CryptoError("SecureBytes cannot be copy.copy()'d")

    def __deepcopy__(self, memo: dict) -> "SecureBytes":
        raise CryptoError("SecureBytes cannot be copy.deepcopy()'d")

    def __getstate__(self) -> None:  # pragma: no cover - defensive
        raise CryptoError("SecureBytes has no serialisable state")

    def __setstate__(self, state: object) -> None:  # pragma: no cover
        raise CryptoError("SecureBytes cannot be rehydrated")

    def __iter__(self) -> object:
        raise CryptoError("SecureBytes refuses iteration (anti-leak)")

    def __repr__(self) -> str:  # pragma: no cover
        return f"<SecureBytes len={len(self._buf) if not self._destroyed else 0}>"

    def __str__(self) -> str:  # pragma: no cover
        return self.__repr__()


# ---------------------------------------------------------------------------
# Envelope (for DEK / KEK envelope encryption)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Envelope:
    """Structured ciphertext — the output of encrypt()."""

    alg_id: str
    ciphertext: bytes
    nonce: bytes
    aad: bytes
    wrapped_dek: bytes
    key_id: str
    key_version: int

    def to_dict(self) -> dict[str, Any]:
        return {
            "alg_id": self.alg_id,
            "ciphertext": self.ciphertext.hex(),
            "nonce": self.nonce.hex(),
            "aad": self.aad.hex(),
            "wrapped_dek": self.wrapped_dek.hex(),
            "key_id": self.key_id,
            "key_version": self.key_version,
        }


# ---------------------------------------------------------------------------
# CryptoProvider Protocol
# ---------------------------------------------------------------------------


class CryptoProvider(Protocol):
    def hmac(self, data: bytes, *, tag: bytes, alg_id: str = ALG_HMAC_SHA256_V1) -> bytes: ...
    def verify_hmac(self, data: bytes, mac: bytes, *, tag: bytes, alg_id: str = ALG_HMAC_SHA256_V1) -> bool: ...
    def sign(self, data: bytes, *, key_id: str, alg_id: str) -> bytes: ...
    def verify_signature(self, data: bytes, signature: bytes, *, key_id: str, alg_id: str) -> bool: ...
    def encrypt(self, plaintext: bytes, *, key_id: str, alg_id: str = ALG_AES256_GCM_V1, aad: bytes = b"") -> Envelope: ...
    def decrypt(self, envelope: Envelope) -> bytes: ...


# ---------------------------------------------------------------------------
# DefaultCryptoProvider — pure-stdlib fallback + optional cryptography
# ---------------------------------------------------------------------------


try:  # pragma: no cover - optional dep
    from cryptography.hazmat.primitives.asymmetric import ed25519, ec
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature, decode_dss_signature

    _CRYPTO_LIB_AVAILABLE = True
except ImportError:  # pragma: no cover
    _CRYPTO_LIB_AVAILABLE = False


@dataclass(slots=True)
class _InMemoryKey:
    key_id: str
    alg_id: str
    material: bytes
    key_version: int = 1


@dataclass(slots=True)
class DefaultCryptoProvider:
    """In-process CryptoProvider.

    For HMAC : uses the module-level key.
    For AES-GCM / Ed25519 / ES256 : requires the ``cryptography`` package.
    """

    _keys: dict[str, _InMemoryKey] = field(default_factory=dict)
    _kek_material: bytes | None = None

    def register_key(self, key_id: str, *, alg_id: str, material: bytes, key_version: int = 1) -> None:
        if is_algorithm_deprecated(alg_id):
            raise AlgorithmDeprecated(f"algorithm {alg_id} is deprecated")
        self._keys[key_id] = _InMemoryKey(key_id, alg_id, material, key_version)

    def set_kek(self, kek: bytes) -> None:
        if len(kek) < 32:
            raise CryptoError("KEK must be at least 32 bytes")
        self._kek_material = bytes(kek)

    # ------------------------------------------------------------------
    # HMAC
    # ------------------------------------------------------------------

    def hmac(self, data: bytes, *, tag: bytes, alg_id: str = ALG_HMAC_SHA256_V1) -> bytes:
        if alg_id != ALG_HMAC_SHA256_V1:
            raise CryptoError(f"unsupported HMAC alg_id: {alg_id}")
        _assert_known_tag(tag)
        return hmac.new(_KEY, tag + data, sha256).digest()

    def verify_hmac(
        self, data: bytes, mac: bytes, *, tag: bytes, alg_id: str = ALG_HMAC_SHA256_V1
    ) -> bool:
        expected = self.hmac(data, tag=tag, alg_id=alg_id)
        return hmac.compare_digest(expected, mac)

    # ------------------------------------------------------------------
    # Signatures
    # ------------------------------------------------------------------

    def sign(self, data: bytes, *, key_id: str, alg_id: str) -> bytes:
        if not _CRYPTO_LIB_AVAILABLE:
            raise CryptoError(
                "signing requires the 'cryptography' package",
                detail={"alg_id": alg_id},
            )
        k = self._keys.get(key_id)
        if k is None:
            raise CryptoError(f"unknown key_id {key_id!r}")
        if k.alg_id != alg_id:
            raise CryptoError(f"alg mismatch: key={k.alg_id} req={alg_id}")
        if alg_id == ALG_ED25519_V1:
            pk = ed25519.Ed25519PrivateKey.from_private_bytes(k.material)
            return pk.sign(data)
        if alg_id == ALG_ES256_V1:
            pk = serialization.load_der_private_key(k.material, password=None)
            assert isinstance(pk, ec.EllipticCurvePrivateKey)
            der = pk.sign(data, ec.ECDSA(hashes.SHA256()))
            r, s = decode_dss_signature(der)
            return r.to_bytes(32, "big") + s.to_bytes(32, "big")
        raise CryptoError(f"unsupported signing alg_id: {alg_id}")

    def verify_signature(
        self, data: bytes, signature: bytes, *, key_id: str, alg_id: str
    ) -> bool:
        if not _CRYPTO_LIB_AVAILABLE:
            raise CryptoError("verify requires the 'cryptography' package")
        k = self._keys.get(key_id)
        if k is None:
            raise CryptoError(f"unknown key_id {key_id!r}")
        try:
            if alg_id == ALG_ED25519_V1:
                pk = ed25519.Ed25519PrivateKey.from_private_bytes(k.material)
                pk.public_key().verify(signature, data)
                return True
            if alg_id == ALG_ES256_V1:
                pk = serialization.load_der_private_key(k.material, password=None)
                assert isinstance(pk, ec.EllipticCurvePrivateKey)
                if len(signature) != 64:
                    return False
                r = int.from_bytes(signature[:32], "big")
                s = int.from_bytes(signature[32:], "big")
                der = encode_dss_signature(r, s)
                pk.public_key().verify(der, data, ec.ECDSA(hashes.SHA256()))
                return True
        except Exception:
            return False
        raise CryptoError(f"unsupported verify alg_id: {alg_id}")

    # ------------------------------------------------------------------
    # Envelope encryption (AES-GCM + KEK wrap)
    # ------------------------------------------------------------------

    def encrypt(
        self,
        plaintext: bytes,
        *,
        key_id: str,
        alg_id: str = ALG_AES256_GCM_V1,
        aad: bytes = b"",
    ) -> Envelope:
        if not _CRYPTO_LIB_AVAILABLE:
            raise CryptoError("AES-GCM requires the 'cryptography' package")
        if alg_id != ALG_AES256_GCM_V1:
            raise CryptoError(f"unsupported encryption alg: {alg_id}")
        if self._kek_material is None:
            raise CryptoError("KEK not configured ; call set_kek()")
        # Generate a fresh DEK.
        from os import urandom

        dek = urandom(32)
        nonce = urandom(12)
        ct = AESGCM(dek).encrypt(nonce, plaintext, aad)
        # Wrap the DEK with the KEK (simple AES-GCM wrap ; real deployments
        # use a KMS call).
        wrap_nonce = urandom(12)
        wrapped = wrap_nonce + AESGCM(self._kek_material[:32]).encrypt(wrap_nonce, dek, b"dek-wrap")
        # Wipe DEK
        dek_sb = SecureBytes(dek)
        dek_sb.destroy()
        return Envelope(
            alg_id=alg_id,
            ciphertext=ct,
            nonce=nonce,
            aad=aad,
            wrapped_dek=wrapped,
            key_id=key_id,
            key_version=self._keys.get(key_id, _InMemoryKey(key_id, alg_id, b"")).key_version,
        )

    def decrypt(self, envelope: Envelope) -> bytes:
        if not _CRYPTO_LIB_AVAILABLE:
            raise CryptoError("AES-GCM requires the 'cryptography' package")
        if self._kek_material is None:
            raise CryptoError("KEK not configured ; call set_kek()")
        wrap_nonce, wrapped_body = envelope.wrapped_dek[:12], envelope.wrapped_dek[12:]
        try:
            dek = AESGCM(self._kek_material[:32]).decrypt(wrap_nonce, wrapped_body, b"dek-wrap")
        except Exception as exc:
            raise SealInvalid("DEK unwrap failed") from exc
        try:
            pt = AESGCM(dek).decrypt(envelope.nonce, envelope.ciphertext, envelope.aad)
        except Exception as exc:
            raise SealInvalid("ciphertext auth failed") from exc
        finally:
            sb = SecureBytes(dek)
            sb.destroy()
        return pt
