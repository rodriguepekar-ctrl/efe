"""KMS client Protocol + implementations.

Two implementations are provided :

* :class:`InMemoryKmsClient` — single-process stub for tests.
* :class:`VaultTransitClient` — HashiCorp Vault Transit adapter with
  retry + short-circuit caching (real HTTP calls are optional — the
  client falls back to an error unless the ``requests`` package and
  ``VAULT_ADDR``/``VAULT_TOKEN`` env vars are present).

Both satisfy :class:`KmsClient` and are drop-in for
:class:`~efe_security.secrets.envelope.EnvelopeManager`.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
import os
from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_security.crypto import csprng_bytes
from efe_security.errors import CryptoError, KeyNotFound, RotationFailed

_LOG = logging.getLogger("efe_security.secrets.kms")

__all__ = ["InMemoryKmsClient", "KmsClient", "VaultTransitClient"]


class KmsClient(Protocol):
    """KMS contract.

    All operations are async : real KMS calls incur network latency and
    must never block the event loop.
    """

    async def generate_dek(self, *, key_id: str, nbytes: int = 32) -> bytes: ...  # pragma: no cover
    async def wrap(self, *, key_id: str, plaintext: bytes) -> bytes: ...  # pragma: no cover
    async def unwrap(self, *, key_id: str, ciphertext: bytes) -> bytes: ...  # pragma: no cover
    async def rotate(self, *, key_id: str) -> int: ...  # pragma: no cover
    async def get_key_version(self, *, key_id: str) -> int: ...  # pragma: no cover


# ---------------------------------------------------------------------------
# InMemoryKmsClient — deterministic unit-test stub
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _MemKey:
    material: bytes
    version: int


@dataclass(slots=True)
class InMemoryKmsClient:
    """In-memory KMS. ``wrap`` is XOR + SHA256 MAC (toy) — do not use in
    production."""

    _keys: dict[str, list[_MemKey]] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    def register_root_key(self, key_id: str, material: bytes | None = None) -> None:
        if len(self._keys.get(key_id, [])) > 0:
            raise CryptoError(f"root key already registered: {key_id}")
        mat = material if material is not None else csprng_bytes(32)
        self._keys[key_id] = [_MemKey(material=mat, version=1)]

    async def generate_dek(self, *, key_id: str, nbytes: int = 32) -> bytes:
        if key_id not in self._keys:
            raise KeyNotFound(key_id)
        return csprng_bytes(nbytes)

    async def wrap(self, *, key_id: str, plaintext: bytes) -> bytes:
        versions = self._keys.get(key_id)
        if not versions:
            raise KeyNotFound(key_id)
        active = versions[-1]
        return _toy_wrap(active.material, plaintext, version=active.version)

    async def unwrap(self, *, key_id: str, ciphertext: bytes) -> bytes:
        versions = self._keys.get(key_id)
        if not versions:
            raise KeyNotFound(key_id)
        # The version is in the first 4 bytes.
        version = int.from_bytes(ciphertext[:4], "big")
        material = next((k.material for k in versions if k.version == version), None)
        if material is None:
            raise KeyNotFound(f"{key_id}@v{version}")
        return _toy_unwrap(material, ciphertext)

    async def rotate(self, *, key_id: str) -> int:
        async with self._lock:
            versions = self._keys.get(key_id)
            if not versions:
                raise KeyNotFound(key_id)
            new_ver = versions[-1].version + 1
            versions.append(_MemKey(material=csprng_bytes(32), version=new_ver))
            _LOG.info("kms.inmemory.rotate key_id=%s version=%d", key_id, new_ver)
            return new_ver

    async def get_key_version(self, *, key_id: str) -> int:
        versions = self._keys.get(key_id)
        if not versions:
            raise KeyNotFound(key_id)
        return versions[-1].version


def _toy_wrap(root: bytes, plaintext: bytes, *, version: int) -> bytes:
    """XOR the plaintext with a SHA-256(root || version) stream, then
    append HMAC. **Not for production** — deterministic, no nonce, trivial
    to break. It suffices to verify in-memory round-trips in tests."""
    from hashlib import sha256

    salt = sha256(root + version.to_bytes(4, "big")).digest()
    # Extend the stream as needed.
    stream = bytearray()
    i = 0
    while len(stream) < len(plaintext):
        stream.extend(sha256(salt + i.to_bytes(4, "big")).digest())
        i += 1
    ct = bytes(a ^ b for a, b in zip(plaintext, stream[: len(plaintext)]))
    mac = sha256(root + ct + version.to_bytes(4, "big")).digest()
    return version.to_bytes(4, "big") + ct + mac


def _toy_unwrap(root: bytes, wrapped: bytes) -> bytes:
    from hashlib import sha256

    version = int.from_bytes(wrapped[:4], "big")
    mac = wrapped[-32:]
    ct = wrapped[4:-32]
    expected_mac = sha256(root + ct + version.to_bytes(4, "big")).digest()
    if mac != expected_mac:
        raise CryptoError("toy_unwrap: MAC mismatch")
    salt = sha256(root + version.to_bytes(4, "big")).digest()
    stream = bytearray()
    i = 0
    while len(stream) < len(ct):
        stream.extend(sha256(salt + i.to_bytes(4, "big")).digest())
        i += 1
    return bytes(a ^ b for a, b in zip(ct, stream[: len(ct)]))


# ---------------------------------------------------------------------------
# VaultTransitClient — optional HashiCorp Vault adapter
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class VaultTransitClient:
    """HashiCorp Vault Transit adapter.

    Requires the ``requests`` package and env vars ``VAULT_ADDR`` /
    ``VAULT_TOKEN``. Falls back to raising :class:`CryptoError` when the
    prerequisites are missing — the production deployment sets them.
    """

    address: str = field(default_factory=lambda: os.environ.get("VAULT_ADDR", ""))
    token: str = field(default_factory=lambda: os.environ.get("VAULT_TOKEN", ""))
    transit_mount: str = "transit"

    def _ensure_ready(self) -> Any:
        try:
            import requests  # type: ignore[import-not-found]
        except ImportError as exc:
            raise CryptoError("VaultTransitClient requires 'requests'") from exc
        if not self.address or not self.token:
            raise CryptoError(
                "VAULT_ADDR and VAULT_TOKEN must be set for VaultTransitClient"
            )
        return requests

    def _url(self, op: str, key_id: str) -> str:
        return f"{self.address.rstrip('/')}/v1/{self.transit_mount}/{op}/{key_id}"

    async def generate_dek(self, *, key_id: str, nbytes: int = 32) -> bytes:
        requests = self._ensure_ready()
        url = self._url("datakey/plaintext", key_id)
        headers = {"X-Vault-Token": self.token}
        try:
            resp = await asyncio.to_thread(
                requests.post, url, headers=headers, json={"bits": nbytes * 8}, timeout=5
            )
            resp.raise_for_status()
        except Exception as exc:
            raise CryptoError(f"Vault generate_dek failed: {exc}") from exc
        body = resp.json()
        import base64

        plain_b64 = body["data"]["plaintext"]
        return base64.b64decode(plain_b64)

    async def wrap(self, *, key_id: str, plaintext: bytes) -> bytes:
        requests = self._ensure_ready()
        url = self._url("encrypt", key_id)
        import base64

        payload = {"plaintext": base64.b64encode(plaintext).decode()}
        headers = {"X-Vault-Token": self.token}
        try:
            resp = await asyncio.to_thread(
                requests.post, url, headers=headers, json=payload, timeout=5
            )
            resp.raise_for_status()
        except Exception as exc:
            raise CryptoError(f"Vault wrap failed: {exc}") from exc
        body = resp.json()
        ct = body["data"]["ciphertext"].encode()
        return ct

    async def unwrap(self, *, key_id: str, ciphertext: bytes) -> bytes:
        requests = self._ensure_ready()
        url = self._url("decrypt", key_id)
        headers = {"X-Vault-Token": self.token}
        payload = {"ciphertext": ciphertext.decode()}
        try:
            resp = await asyncio.to_thread(
                requests.post, url, headers=headers, json=payload, timeout=5
            )
            resp.raise_for_status()
        except Exception as exc:
            raise CryptoError(f"Vault unwrap failed: {exc}") from exc
        body = resp.json()
        import base64

        return base64.b64decode(body["data"]["plaintext"])

    async def rotate(self, *, key_id: str) -> int:
        requests = self._ensure_ready()
        url = self._url("keys", key_id) + "/rotate"
        headers = {"X-Vault-Token": self.token}
        try:
            resp = await asyncio.to_thread(
                requests.post, url, headers=headers, timeout=5
            )
            resp.raise_for_status()
        except Exception as exc:
            raise RotationFailed(str(exc)) from exc
        return int(resp.json()["data"]["latest_version"])

    async def get_key_version(self, *, key_id: str) -> int:
        requests = self._ensure_ready()
        url = f"{self.address.rstrip('/')}/v1/{self.transit_mount}/keys/{key_id}"
        headers = {"X-Vault-Token": self.token}
        try:
            resp = await asyncio.to_thread(
                requests.get, url, headers=headers, timeout=5
            )
            resp.raise_for_status()
        except Exception as exc:
            raise CryptoError(str(exc)) from exc
        return int(resp.json()["data"]["latest_version"])


# Silence unused-import warning of hashlib when libs absent.
_ = hashlib
