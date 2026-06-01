"""Algorithm registry — the only place that knows which algos exist.

Each ``ALG_*`` constant is a stable string shipped in persisted data.
New algorithms get a new constant and a new entry in the registry ;
deprecated ones keep the constant (lectures still work) but are flagged
so new writes refuse them.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Final

__all__ = [
    "ALG_AES256_GCM_V1",
    "ALG_ED25519_V1",
    "ALG_ES256_V1",
    "ALG_HMAC_SHA256_V1",
    "AlgorithmRegistry",
    "default_registry",
    "is_algorithm_deprecated",
    "mark_deprecated",
]


# ---------------------------------------------------------------------------
# Stable IDs
# ---------------------------------------------------------------------------

ALG_HMAC_SHA256_V1: Final[str] = "HMAC-SHA256-v1"
ALG_AES256_GCM_V1: Final[str] = "AES256-GCM-v1"
ALG_ED25519_V1: Final[str] = "Ed25519-v1"
ALG_ES256_V1: Final[str] = "ES256-v1"

# Reserved for post-quantum migration (declared but unimplemented).
ALG_KYBER_V1_RESERVED: Final[str] = "Kyber-v1"
ALG_DILITHIUM_V1_RESERVED: Final[str] = "Dilithium-v1"


# ---------------------------------------------------------------------------
# Registry
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _Entry:
    alg_id: str
    category: str  # "hmac" | "aead" | "signature_eddsa" | "signature_ecdsa"
    description: str
    deprecated_at: datetime | None = None


@dataclass(slots=True)
class AlgorithmRegistry:
    _entries: dict[str, _Entry] = field(default_factory=dict)

    def register(self, alg_id: str, *, category: str, description: str) -> None:
        if alg_id in self._entries:
            raise ValueError(f"alg_id already registered: {alg_id}")
        self._entries[alg_id] = _Entry(alg_id=alg_id, category=category, description=description)

    def mark_deprecated(self, alg_id: str, *, at: datetime | None = None) -> None:
        entry = self._entries.get(alg_id)
        if entry is None:
            raise KeyError(f"unknown alg_id: {alg_id}")
        entry.deprecated_at = at or datetime.now(tz=timezone.utc)

    def is_deprecated(self, alg_id: str) -> bool:
        entry = self._entries.get(alg_id)
        if entry is None:
            # Unknown algorithms are treated as deprecated : fail-closed.
            return True
        return entry.deprecated_at is not None

    def get_category(self, alg_id: str) -> str | None:
        entry = self._entries.get(alg_id)
        return entry.category if entry is not None else None

    def list_active(self) -> list[str]:
        return sorted(
            alg_id
            for alg_id, entry in self._entries.items()
            if entry.deprecated_at is None
        )


# Default registry populated at import time.
default_registry: AlgorithmRegistry = AlgorithmRegistry()
default_registry.register(
    ALG_HMAC_SHA256_V1, category="hmac", description="HMAC-SHA256 (RFC 2104)"
)
default_registry.register(
    ALG_AES256_GCM_V1, category="aead", description="AES-256-GCM (NIST SP 800-38D)"
)
default_registry.register(
    ALG_ED25519_V1, category="signature_eddsa", description="Ed25519 (RFC 8032)"
)
default_registry.register(
    ALG_ES256_V1, category="signature_ecdsa", description="ECDSA P-256 + SHA-256"
)


# ---------------------------------------------------------------------------
# Module-level helpers (operate on default_registry)
# ---------------------------------------------------------------------------


def is_algorithm_deprecated(alg_id: str) -> bool:
    return default_registry.is_deprecated(alg_id)


def mark_deprecated(alg_id: str, *, at: datetime | None = None) -> None:
    default_registry.mark_deprecated(alg_id, at=at)
