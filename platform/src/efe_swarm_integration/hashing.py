"""HMAC domain tags + seal helpers for M12.

Tags use the dedicated ``efe-m12/`` prefix to remain orthogonal to
M09/M10/M11 tag namespaces (audit : 0 collision across modules).
"""

from __future__ import annotations

import hmac
import json
import logging
import os
from hashlib import sha256
from typing import Any, Final

_LOG = logging.getLogger("efe_swarm_integration.hashing")

__all__ = [
    "DOMAIN_TAG_ARTEFACT_SYNC",
    "DOMAIN_TAG_CONE_BINDING",
    "DOMAIN_TAG_SWARM_IDEMPOTENCY",
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

DOMAIN_TAG_SWARM_IDEMPOTENCY: Final[bytes] = b"efe-m12/idempotency/v1\x00"
DOMAIN_TAG_ARTEFACT_SYNC: Final[bytes] = b"efe-m12/artefact-sync/v1\x00"
DOMAIN_TAG_CONE_BINDING: Final[bytes] = b"efe-m12/cone-binding/v1\x00"

_ALL_TAGS: Final[frozenset[bytes]] = frozenset(
    {
        DOMAIN_TAG_SWARM_IDEMPOTENCY,
        DOMAIN_TAG_ARTEFACT_SYNC,
        DOMAIN_TAG_CONE_BINDING,
    }
)


def _assert_known_tag(tag: bytes) -> None:
    if tag not in _ALL_TAGS:
        raise ValueError(f"unknown HMAC domain tag: {tag!r}")


# ---------------------------------------------------------------------------
# Key management
# ---------------------------------------------------------------------------

_DEV_DEFAULT_KEY: Final[bytes] = b"efe-m12-dev-key-do-not-use-in-prod\x00" * 2
_USING_DEV_KEY: bool = False


def _load_key_from_env() -> bytes:
    global _USING_DEV_KEY
    raw = os.environ.get("EFE_M12_HMAC_KEY", "")
    if raw:
        _USING_DEV_KEY = False
        return raw.encode()
    if not _USING_DEV_KEY:
        _LOG.warning(
            "m12.hashing.using_dev_default_key env=EFE_M12_HMAC_KEY missing ; "
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
# Canonical JSON + seal
# ---------------------------------------------------------------------------


def canonical_json(obj: Any) -> bytes:
    return json.dumps(
        obj,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")


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
