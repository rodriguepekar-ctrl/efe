"""Audited CSPRNG wrapper.

Every call to :func:`csprng_bytes` goes through this module so we can
(a) inject determinism in tests, (b) optionally audit large draws for
abuse monitoring.
"""

from __future__ import annotations

import logging
import os

_LOG = logging.getLogger("efe_security.prng")

__all__ = ["audit_prng_call", "csprng_bytes", "set_prng_backend"]


# ---------------------------------------------------------------------------
# Backend override for tests
# ---------------------------------------------------------------------------

_backend = os.urandom  # default


def set_prng_backend(fn) -> None:  # type: ignore[no-untyped-def]
    """Override the CSPRNG backend. Primarily for deterministic tests."""
    global _backend
    _backend = fn


# ---------------------------------------------------------------------------
# Audit hook (optional)
# ---------------------------------------------------------------------------

_audit_threshold_bytes = 1024
_draw_counter = 0


def audit_prng_call(n: int) -> None:
    """Record a large draw. Real implementations ship to the audit sink."""
    global _draw_counter
    _draw_counter += n
    _LOG.debug("prng.large_draw bytes=%d total=%d", n, _draw_counter)


def csprng_bytes(n: int) -> bytes:
    """Return *n* cryptographically-strong bytes."""
    if n < 0:
        raise ValueError("csprng_bytes requires n >= 0")
    if n > 4096:  # guard against absurd draws (e.g. accidental n_bytes=1<<30)
        raise ValueError(f"csprng_bytes refuses n > 4096 (got {n})")
    data = _backend(n)
    if n >= _audit_threshold_bytes:
        audit_prng_call(n)
    return bytes(data)
