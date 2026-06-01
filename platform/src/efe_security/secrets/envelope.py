"""Envelope encryption facade.

Wraps a :class:`KmsClient` + a :class:`KeyRegistry` to produce a
straightforward ``encrypt(plaintext) → Envelope`` / ``decrypt(Envelope) →
bytes`` API for the rest of the platform.

Invariant : for every :class:`Envelope` the ``alg_id`` matches the
registry entry, and the registry entry is in state ``active`` at the
time of encryption.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from efe_security.crypto import ALG_AES256_GCM_V1, csprng_bytes
from efe_security.crypto.primitives import Envelope
from efe_security.errors import CryptoError, KeyExpired, SealInvalid

if TYPE_CHECKING:  # pragma: no cover
    from efe_security.secrets.key_registry import KeyRegistry
    from efe_security.secrets.kms_client import KmsClient


try:  # pragma: no cover - optional dep
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    _CRYPTO_LIB_AVAILABLE = True
except ImportError:  # pragma: no cover
    _CRYPTO_LIB_AVAILABLE = False


__all__ = ["EnvelopeManager"]


@dataclass(slots=True)
class EnvelopeManager:
    kms: "KmsClient"
    registry: "KeyRegistry"
    default_alg: str = ALG_AES256_GCM_V1
    _nonce_size: int = 12

    async def encrypt(
        self,
        plaintext: bytes,
        *,
        kek_key_id: str,
        aad: bytes = b"",
        alg_id: str | None = None,
    ) -> Envelope:
        if not _CRYPTO_LIB_AVAILABLE:
            raise CryptoError("AES-GCM requires the 'cryptography' package")
        algo = alg_id or self.default_alg
        if algo != ALG_AES256_GCM_V1:
            raise CryptoError(f"unsupported envelope alg: {algo}")
        entry = await self.registry.get(kek_key_id)
        if entry.state.value != "active":
            raise KeyExpired(f"KEK {kek_key_id} is not active (state={entry.state})")
        # Generate DEK via KMS (in prod) or CSPRNG (tests).
        dek = await self.kms.generate_dek(key_id=kek_key_id, nbytes=32)
        nonce = csprng_bytes(self._nonce_size)
        ct = AESGCM(dek).encrypt(nonce, plaintext, aad)
        wrapped = await self.kms.wrap(key_id=kek_key_id, plaintext=dek)
        # Best-effort wipe of the DEK reference.
        from efe_security.crypto.primitives import SecureBytes

        sb = SecureBytes(dek)
        sb.destroy()
        key_version = await self.kms.get_key_version(key_id=kek_key_id)
        return Envelope(
            alg_id=algo,
            ciphertext=ct,
            nonce=nonce,
            aad=aad,
            wrapped_dek=wrapped,
            key_id=kek_key_id,
            key_version=key_version,
        )

    async def decrypt(self, envelope: Envelope) -> bytes:
        if not _CRYPTO_LIB_AVAILABLE:
            raise CryptoError("AES-GCM requires the 'cryptography' package")
        entry = await self.registry.get(envelope.key_id)
        # Accept `retiring` keys on decrypt — grace window — but not `retired`.
        if entry.state.value == "retired":
            raise KeyExpired(f"KEK {envelope.key_id} is retired")
        try:
            dek = await self.kms.unwrap(
                key_id=envelope.key_id, ciphertext=envelope.wrapped_dek
            )
        except Exception as exc:
            raise SealInvalid("DEK unwrap failed") from exc
        try:
            pt = AESGCM(dek).decrypt(envelope.nonce, envelope.ciphertext, envelope.aad)
        except Exception as exc:
            raise SealInvalid("AEAD decrypt failed") from exc
        finally:
            from efe_security.crypto.primitives import SecureBytes

            sb = SecureBytes(dek)
            sb.destroy()
        return pt
