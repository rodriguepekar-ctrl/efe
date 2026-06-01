"""HMAC / canonical JSON primitives — domain-separated.

Every HMAC use in M10 carries an explicit *domain separation tag* so a
payload signed for one scope is not valid for another. The full list of
tags is canonical — adding a new one requires a PR + an ADR update.

Functions
---------

:func:`canonical_json` — stable JSON encoding (sorted keys, UTF-8, compact
separators).

:func:`hmac_seal` — ``HMAC-SHA256(key, tag || canonical_json(payload))``.

:func:`verify_seal` — constant-time comparison.

The default HMAC key is read from the ``EFE_SCHED_HMAC_KEY`` environment
variable at import time. In tests, :func:`set_key` overrides it.
"""

from __future__ import annotations

import hmac
import json
import logging
import os
from hashlib import sha256
from typing import Any, Final

_LOG = logging.getLogger("efe_scheduling.hashing")

__all__ = [
    "DOMAIN_TAG_DATASET_SEAL",
    "DOMAIN_TAG_DATASET_SEED",
    "DOMAIN_TAG_IDEMPOTENCY_TOKEN",
    "DOMAIN_TAG_JOB_SEAL",
    "DOMAIN_TAG_PROMOTION_SEAL",
    "DOMAIN_TAG_SCHED_EVENT_V1",
    "DOMAIN_TAG_SCHED_FIRE",
    "canonical_json",
    "current_key",
    "hmac_seal",
    "is_using_dev_key",
    "reset_key",
    "set_key",
    "verify_seal",
]

# ---------------------------------------------------------------------------
# Domain tags
# ---------------------------------------------------------------------------

DOMAIN_TAG_SCHED_FIRE: Final[bytes] = b"efe-sched/fire/v1\x00"
DOMAIN_TAG_JOB_SEAL: Final[bytes] = b"efe-sched/job-seal/v1\x00"
DOMAIN_TAG_SCHED_EVENT_V1: Final[bytes] = b"efe-sched/event/v1\x00"
DOMAIN_TAG_DATASET_SEED: Final[bytes] = b"efe-sched/dataset-seed/v1\x00"
DOMAIN_TAG_DATASET_SEAL: Final[bytes] = b"efe-sched/dataset-seal/v1\x00"
DOMAIN_TAG_PROMOTION_SEAL: Final[bytes] = b"efe-sched/promotion-seal/v1\x00"
DOMAIN_TAG_IDEMPOTENCY_TOKEN: Final[bytes] = b"efe-sched/idempotency/v1\x00"

_ALL_TAGS: Final[frozenset[bytes]] = frozenset(
    {
        DOMAIN_TAG_SCHED_FIRE,
        DOMAIN_TAG_JOB_SEAL,
        DOMAIN_TAG_SCHED_EVENT_V1,
        DOMAIN_TAG_DATASET_SEED,
        DOMAIN_TAG_DATASET_SEAL,
        DOMAIN_TAG_PROMOTION_SEAL,
        DOMAIN_TAG_IDEMPOTENCY_TOKEN,
    }
)


def _assert_known_tag(tag: bytes) -> None:
    """Fail loudly if an unknown tag is passed to seal/verify.

    This is the cheap runtime guard that backs invariant **I6**
    (domain-tag isolation).
    """
    if tag not in _ALL_TAGS:
        raise ValueError(f"unknown HMAC domain tag: {tag!r}")


# ---------------------------------------------------------------------------
# Key management
# ---------------------------------------------------------------------------

_DEV_DEFAULT_KEY: Final[bytes] = b"efe-sched-dev-key-do-not-use-in-prod\x00" * 2
_USING_DEV_KEY: bool = False


def _load_key_from_env() -> bytes:
    """Resolve the active HMAC key, warning once if the dev default is used."""
    global _USING_DEV_KEY
    raw = os.environ.get("EFE_SCHED_HMAC_KEY", "")
    if raw:
        _USING_DEV_KEY = False
        return raw.encode()
    if not _USING_DEV_KEY:
        # Warn once per process lifetime. Production should set the env var
        # via the secrets manager (Vault, KMS, ...).
        _LOG.warning(
            "hashing.using_dev_default_key env=EFE_SCHED_HMAC_KEY missing ; "
            "dev-default key in use — UNSAFE IN PRODUCTION."
        )
        _USING_DEV_KEY = True
    return _DEV_DEFAULT_KEY


_KEY: bytes = _load_key_from_env()


def set_key(key: bytes) -> None:
    """Override the HMAC key. Useful for tests."""
    global _KEY, _USING_DEV_KEY
    if len(key) < 16:
        raise ValueError("HMAC key must be at least 16 bytes")
    _KEY = bytes(key)
    _USING_DEV_KEY = False


def reset_key() -> None:
    """Restore the key from the environment (or the dev default)."""
    global _KEY
    _KEY = _load_key_from_env()


def current_key() -> bytes:
    """Return a copy of the current key (useful for diagnostics)."""
    return bytes(_KEY)


def is_using_dev_key() -> bool:
    """True iff the current key is the unsafe dev default."""
    return _USING_DEV_KEY


# ---------------------------------------------------------------------------
# Canonical JSON
# ---------------------------------------------------------------------------


def canonical_json(obj: Any) -> bytes:
    """Return a canonical UTF-8 JSON encoding.

    Rules :

    - keys sorted lexicographically at every level,
    - no whitespace between tokens (``separators=(',',':')``),
    - ``ensure_ascii=False`` so unicode is preserved,
    - floats use Python's default ``repr`` via ``allow_nan=False``.

    Raises ``ValueError`` on NaN / Inf (not representable in JSON).
    """
    return json.dumps(
        obj,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")


# ---------------------------------------------------------------------------
# Seal / verify
# ---------------------------------------------------------------------------


def hmac_seal(tag: bytes, payload: Any, *, key: bytes | None = None) -> bytes:
    """Return HMAC-SHA256 of ``tag || canonical_json(payload)``.

    Parameters
    ----------
    tag : bytes
        Must be one of the registered domain tags in this module.
    payload : Any
        JSON-serialisable payload.
    key : bytes | None
        Override the module key. Default is the configured key.

    Returns
    -------
    bytes
        32-byte MAC.
    """
    _assert_known_tag(tag)
    message = tag + canonical_json(payload)
    mac = hmac.new(key if key is not None else _KEY, message, sha256).digest()
    return mac


def verify_seal(
    tag: bytes,
    payload: Any,
    expected: bytes,
    *,
    key: bytes | None = None,
) -> bool:
    """Constant-time verification of ``expected`` against the computed MAC."""
    computed = hmac_seal(tag, payload, key=key)
    return hmac.compare_digest(computed, expected)
