"""Hashing primitives for :mod:`efe_rollout`.

Four layers of hashing coexist in this module, each with a distinct
purpose:

1. :func:`murmur3_hash64` — **fast non-cryptographic** hash used at the
   hot path of :class:`~efe_rollout.assignment.assigner.HashAssigner`.
   Has excellent avalanche properties, which is what we rely on to
   prove multi-layer orthogonality (invariant **I4**).

2. :func:`content_hash` — canonical SHA-256 of a mapping. Used for the
   pre-registration ``spec_hash`` (invariant **I6**).

3. :func:`hmac_sign` / :func:`hmac_verify` — HMAC-SHA256 of an arbitrary
   byte string with a caller-supplied key. Used for exposure rows, for
   the pre-registration signature, and for NATS envelopes. Each usage
   prepends a **domain-separation tag** so a key legitimately reused
   across sites cannot be replayed across them.

4. :func:`bucket` — fold a 64-bit hash into ``[0, K)`` uniformly. This
   is the hand-rolled equivalent of ``hash_u64 % K`` but with a tiny
   shift-and-multiply trick (Lemire 2019) to avoid the 0-bucket bias.

Domain tags for HMAC reuse:

* ``DOMAIN_TAG_NATS`` — for envelope signatures (matches the rollout
  topic family).
* ``DOMAIN_TAG_PREREG`` — for pre-registration plan signatures.
* ``DOMAIN_TAG_EXPOSURE`` — for per-exposure HMAC columns.

Pure-python fallback for murmur3 is provided so the package works in
environments where ``mmh3`` is not installed — both paths yield the
same 64-bit result, verified by a parity test.
"""

from __future__ import annotations

import hashlib
import hmac
import json
from collections.abc import Mapping
from typing import Any

# ---------------------------------------------------------------------------
# Domain-separation tags
# ---------------------------------------------------------------------------

DOMAIN_TAG_NATS: bytes = b"efe-rollout/nats/v1\x00"
DOMAIN_TAG_PREREG: bytes = b"efe-rollout/prereg/v1\x00"
DOMAIN_TAG_EXPOSURE: bytes = b"efe-rollout/exposure/v1\x00"
DOMAIN_TAG_OUTCOME: bytes = b"efe-rollout/outcome/v1\x00"


# ---------------------------------------------------------------------------
# Canonical JSON + SHA-256
# ---------------------------------------------------------------------------


def canonical_json_bytes(payload: Any) -> bytes:
    """Deterministic JSON encoding — sorted keys, no whitespace, UTF-8.

    Values that aren't JSON-native are converted through
    :func:`_default_json` (datetimes become ISO-8601, bytes become hex,
    everything else becomes ``str(o)``).
    """
    return json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        default=_default_json,
    ).encode("utf-8")


def content_hash(payload: Mapping[str, Any]) -> bytes:
    return hashlib.sha256(canonical_json_bytes(payload)).digest()


# ---------------------------------------------------------------------------
# HMAC with explicit domain tag
# ---------------------------------------------------------------------------


def hmac_sign(message: bytes, key: bytes, *, domain_tag: bytes) -> bytes:
    """Compute HMAC-SHA256 of ``domain_tag || message`` under ``key``.

    The domain tag ensures that a key legitimately reused across sites
    (NATS, pre-reg, exposure) cannot be replayed across them.
    """
    if not isinstance(domain_tag, bytes) or not domain_tag:
        raise ValueError("domain_tag must be a non-empty byte string")
    mac = hmac.new(key, domain_tag + message, hashlib.sha256)
    return mac.digest()


def hmac_verify(
    message: bytes, key: bytes, signature: bytes, *, domain_tag: bytes,
) -> bool:
    expected = hmac_sign(message, key, domain_tag=domain_tag)
    return hmac.compare_digest(expected, signature)


# ---------------------------------------------------------------------------
# MurmurHash3 (fast, non-crypto)
# ---------------------------------------------------------------------------


def murmur3_hash64(data: bytes | str, *, seed: int = 0) -> int:
    """Return a non-negative 64-bit hash for ``data``.

    Uses the high-quality ``mmh3.hash64`` when the C extension is
    available, and falls back to a pure-python implementation otherwise
    so the package stays importable in slim environments.

    Parity between the two backends is verified in
    ``tests_rollout/test_hashing_parity.py``.
    """
    payload = data.encode("utf-8") if isinstance(data, str) else bytes(data)

    try:
        import mmh3  # type: ignore[import-not-found]

        # mmh3.hash64 returns (low64, high64) signed ints. We xor-fold
        # them into one unsigned 64-bit value so the fallback can
        # replicate the same 64-bit answer deterministically.
        low, high = mmh3.hash64(payload, seed, signed=False)
        return int((low ^ high) & 0xFFFF_FFFF_FFFF_FFFF)
    except Exception:
        return _pure_murmur3_64(payload, seed)


def _pure_murmur3_64(data: bytes, seed: int) -> int:
    """Self-contained MurmurHash3_x64_128 → folded 64-bit result.

    Reference : Austin Appleby's public-domain MurmurHash3 (2011).
    We fold the 128-bit output into 64 bits by XOR to match the
    behaviour of the C-extension path above.
    """

    c1 = 0x87C3_7B91_1142_53D5
    c2 = 0x4CF5_AD43_2745_937F
    mask = 0xFFFF_FFFF_FFFF_FFFF

    length = len(data)
    nblocks = length // 16

    h1 = seed & mask
    h2 = seed & mask

    # Body
    for i in range(nblocks):
        k1 = int.from_bytes(data[i * 16: i * 16 + 8], "little")
        k2 = int.from_bytes(data[i * 16 + 8: i * 16 + 16], "little")

        k1 = (k1 * c1) & mask
        k1 = _rotl64(k1, 31)
        k1 = (k1 * c2) & mask
        h1 ^= k1

        h1 = _rotl64(h1, 27)
        h1 = (h1 + h2) & mask
        h1 = (h1 * 5 + 0x52DC_E729) & mask

        k2 = (k2 * c2) & mask
        k2 = _rotl64(k2, 33)
        k2 = (k2 * c1) & mask
        h2 ^= k2

        h2 = _rotl64(h2, 31)
        h2 = (h2 + h1) & mask
        h2 = (h2 * 5 + 0x3849_5AB5) & mask

    # Tail
    tail_index = nblocks * 16
    tail_len = length - tail_index
    k1 = 0
    k2 = 0
    if tail_len >= 15:
        k2 ^= data[tail_index + 14] << 48
    if tail_len >= 14:
        k2 ^= data[tail_index + 13] << 40
    if tail_len >= 13:
        k2 ^= data[tail_index + 12] << 32
    if tail_len >= 12:
        k2 ^= data[tail_index + 11] << 24
    if tail_len >= 11:
        k2 ^= data[tail_index + 10] << 16
    if tail_len >= 10:
        k2 ^= data[tail_index + 9] << 8
    if tail_len >= 9:
        k2 ^= data[tail_index + 8]
        k2 = (k2 * c2) & mask
        k2 = _rotl64(k2, 33)
        k2 = (k2 * c1) & mask
        h2 ^= k2
    if tail_len >= 8:
        k1 ^= data[tail_index + 7] << 56
    if tail_len >= 7:
        k1 ^= data[tail_index + 6] << 48
    if tail_len >= 6:
        k1 ^= data[tail_index + 5] << 40
    if tail_len >= 5:
        k1 ^= data[tail_index + 4] << 32
    if tail_len >= 4:
        k1 ^= data[tail_index + 3] << 24
    if tail_len >= 3:
        k1 ^= data[tail_index + 2] << 16
    if tail_len >= 2:
        k1 ^= data[tail_index + 1] << 8
    if tail_len >= 1:
        k1 ^= data[tail_index]
        k1 = (k1 * c1) & mask
        k1 = _rotl64(k1, 31)
        k1 = (k1 * c2) & mask
        h1 ^= k1

    # Finalisation
    h1 ^= length & mask
    h2 ^= length & mask
    h1 = (h1 + h2) & mask
    h2 = (h2 + h1) & mask
    h1 = _fmix64(h1)
    h2 = _fmix64(h2)
    h1 = (h1 + h2) & mask
    h2 = (h2 + h1) & mask

    return (h1 ^ h2) & mask


def _rotl64(x: int, r: int) -> int:
    mask = 0xFFFF_FFFF_FFFF_FFFF
    return ((x << r) | (x >> (64 - r))) & mask


def _fmix64(k: int) -> int:
    mask = 0xFFFF_FFFF_FFFF_FFFF
    k ^= k >> 33
    k = (k * 0xFF51_AFD7_ED55_8CCD) & mask
    k ^= k >> 33
    k = (k * 0xC4CE_B9FE_1A85_EC53) & mask
    k ^= k >> 33
    return k


def bucket(hash_u64: int, *, buckets: int) -> int:
    """Fold ``hash_u64`` uniformly into ``[0, buckets)``.

    Uses Lemire's multiply-and-shift trick which gives a perfectly
    uniform distribution as long as ``buckets ≤ 2**32`` (we always
    stay well below that).
    """
    if buckets <= 0:
        raise ValueError("buckets must be positive")
    return int(((hash_u64 & 0xFFFF_FFFF) * buckets) >> 32)


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _default_json(o: Any) -> Any:
    if hasattr(o, "isoformat"):
        return o.isoformat()
    if hasattr(o, "hex"):
        return o.hex()
    return str(o)


__all__ = [
    "DOMAIN_TAG_EXPOSURE",
    "DOMAIN_TAG_NATS",
    "DOMAIN_TAG_OUTCOME",
    "DOMAIN_TAG_PREREG",
    "bucket",
    "canonical_json_bytes",
    "content_hash",
    "hmac_sign",
    "hmac_verify",
    "murmur3_hash64",
]
