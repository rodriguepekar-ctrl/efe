"""MinHash + LSH deduplication — scratch implementation, no external dep.

We avoid ``datasketch`` to keep the package lean. The implementation
is a faithful Broder 1997 MinHash with ``num_perm`` permutations and
an LSH banding structure (Broder 2000) for sub-linear query time.

Characteristics
---------------

* Signature size: ``num_perm`` x 8 bytes = ~1 KiB per text for 128 perms.
* Query time: O(bands) per query (bands ≈ 16 for threshold 0.9).
* Insert time: O(bands).
* Memory: O(N_samples x bands) for the index.

The default parameters (``num_perm=128``, ``threshold=0.90``) match the
architecture document. Tune only if your benchmark backs it.
"""

from __future__ import annotations

import hashlib
import struct
import threading
from collections import defaultdict
from typing import Protocol

import numpy as np

from efe_ft.hashing import normalise_text

#: Shingle size — trigrams are the sweet spot between sensitivity to
#: typos and robustness to rewording.
SHINGLE_SIZE: int = 3

_MAX_UINT64: int = (1 << 64) - 1


def _tokenise(text: str) -> set[bytes]:
    """NFKC-normalise, lowercase, k-shingle (trigrams by default)."""
    s = normalise_text(text).lower()
    if len(s) <= SHINGLE_SIZE:
        return {s.encode("utf-8")}
    return {
        s[i : i + SHINGLE_SIZE].encode("utf-8")
        for i in range(len(s) - SHINGLE_SIZE + 1)
    }


def _seed_permutations(num_perm: int, seed: int = 0) -> tuple[np.ndarray, np.ndarray]:
    """Two arrays of 64-bit multipliers/adders for the universal hash."""
    rng = np.random.default_rng(seed)
    a = rng.integers(1, _MAX_UINT64, size=num_perm, dtype=np.uint64)
    b = rng.integers(0, _MAX_UINT64, size=num_perm, dtype=np.uint64)
    return a, b


class Signature:
    """Fixed-width MinHash signature (``num_perm`` x uint64)."""

    __slots__ = ("_values",)

    def __init__(self, values: np.ndarray) -> None:
        if values.dtype != np.uint64:
            raise TypeError("signature values must be uint64")
        self._values = values

    @property
    def values(self) -> np.ndarray:
        return self._values

    def jaccard(self, other: Signature) -> float:
        """Approximate Jaccard similarity in [0, 1]."""
        if self._values.size != other._values.size:
            raise ValueError("signature size mismatch")
        return float((self._values == other._values).mean())


def sign(
    text: str, *, num_perm: int = 128, seed: int = 0
) -> Signature:
    """Compute a MinHash signature for ``text``."""
    tokens = _tokenise(text)
    if not tokens:
        return Signature(np.full(num_perm, _MAX_UINT64, dtype=np.uint64))
    a, b = _seed_permutations(num_perm, seed)
    # Hash each shingle to uint64 using sha256 first 8 bytes.
    base_hashes = np.fromiter(
        (
            struct.unpack_from("<Q", hashlib.sha256(tok).digest())[0]
            for tok in tokens
        ),
        dtype=np.uint64,
        count=len(tokens),
    )
    # (num_perm, n_tokens) universal hash matrix, take min along n_tokens.
    h = ((a[:, None] * base_hashes[None, :]) + b[:, None])
    return Signature(h.min(axis=1))


class Deduper(Protocol):
    """Narrow surface the pipeline needs."""

    def is_duplicate(self, key: str, text: str) -> str | None:
        """Return the canonical key of a prior near-duplicate, else None.

        If ``None``, the caller should proceed and ``key`` is inserted
        into the index.
        """

    def clear(self) -> None: ...


class InMemoryDeduper:
    """Thread-safe LSH-backed deduper held in process memory.

    For a single-pod MVP this is fine. A Redis-backed implementation
    (``RedisDeduper``) is drop-in for multi-pod — out of scope for
    this module.
    """

    def __init__(
        self,
        *,
        num_perm: int = 128,
        threshold: float = 0.90,
        seed: int = 0,
    ) -> None:
        if not 0.5 <= threshold <= 1.0:
            raise ValueError("threshold must be in [0.5, 1.0]")
        if num_perm < 8 or num_perm & (num_perm - 1):
            # Not strictly required but makes banding division exact.
            # Relax to just "num_perm > 0" if needed.
            pass
        self._num_perm = num_perm
        self._threshold = threshold
        self._seed = seed
        # Choose bands/rows so that banding curve crosses ``threshold``.
        self._bands, self._rows = _optimal_banding(num_perm, threshold)
        # band_idx -> (band_tuple -> set[key])
        self._index: list[dict[bytes, set[str]]] = [
            defaultdict(set) for _ in range(self._bands)
        ]
        # key -> signature values, for pairwise verification after LSH hit
        self._sigs: dict[str, np.ndarray] = {}
        self._lock = threading.Lock()

    def is_duplicate(self, key: str, text: str) -> str | None:
        sig = sign(text, num_perm=self._num_perm, seed=self._seed)
        candidate_hit = self._probe_then_insert(key, sig)
        return candidate_hit

    def clear(self) -> None:
        with self._lock:
            self._index = [defaultdict(set) for _ in range(self._bands)]
            self._sigs.clear()

    # ------------------------------------------------------------ internal

    def _probe_then_insert(self, key: str, sig: Signature) -> str | None:
        bands = self._split_bands(sig.values)
        hit: str | None = None
        with self._lock:
            # 1. Probe all bands for candidates; take the first verified hit.
            candidates: set[str] = set()
            for b_idx, band in enumerate(bands):
                candidates.update(self._index[b_idx].get(band, ()))
            candidates.discard(key)
            for cand in candidates:
                if cand not in self._sigs:
                    continue
                sim = float((self._sigs[cand] == sig.values).mean())
                if sim >= self._threshold:
                    hit = cand
                    break
            if hit is not None:
                return hit
            # 2. No duplicate → insert.
            self._sigs[key] = sig.values
            for b_idx, band in enumerate(bands):
                self._index[b_idx][band].add(key)
        return None

    def _split_bands(self, sig_values: np.ndarray) -> list[bytes]:
        """Split the signature into ``bands`` buckets of ``rows`` each."""
        out: list[bytes] = []
        for b in range(self._bands):
            chunk = sig_values[b * self._rows : (b + 1) * self._rows]
            out.append(chunk.tobytes())
        return out


def _optimal_banding(num_perm: int, threshold: float) -> tuple[int, int]:
    """Pick (bands, rows) minimising the deviation of the S-curve knee
    from ``threshold``. Pure enumeration over divisors of num_perm.
    """
    best = (1, num_perm)
    best_err = float("inf")
    for b in range(1, num_perm + 1):
        if num_perm % b != 0:
            continue
        r = num_perm // b
        # S-curve knee at (1/b)^(1/r)
        knee = (1.0 / b) ** (1.0 / r)
        err = abs(knee - threshold)
        if err < best_err:
            best_err = err
            best = (b, r)
    return best


__all__ = ["Deduper", "InMemoryDeduper", "Signature", "sign"]
