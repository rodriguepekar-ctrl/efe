"""Baseline seal — canonical hash + HMAC signature.

The seal is what makes a baseline *immutable* (invariant I4). The
workflow :

1. Caller constructs a :class:`~efe_drift.records.BaselineSpec`.
2. :func:`seal_baseline` computes :func:`content_hash` over the
   canonical-JSON of the spec and signs it with HMAC-SHA256 under a
   rollout-specific domain tag.
3. The returned :class:`SealedBaseline` is stored in the DB.
4. :func:`verify_baseline` can be called at any time to prove the
   row has not been tampered with.

Tamper cases detected :

* **signature tamper** — HMAC does not verify under the same key.
* **spec mutation** — the JSON payload was altered (hash mismatch).
* **domain-tag reuse** — a signature legitimate on another M09 artefact
  (e.g. a NATS envelope) does not replay as a baseline signature.

The module mirrors the design of :mod:`efe_rollout.prereg` and uses
the same constant-time ``hmac.compare_digest`` under the hood.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime

from efe_drift.errors import (
    BaselineSignatureInvalid,
    BaselineSpecHashMismatch,
)
from efe_drift.hashing import (
    DOMAIN_TAG_BASELINE,
    canonical_json_bytes,
    content_hash,
    hmac_sign,
    hmac_verify,
)
from efe_drift.records import BaselineSpec


@dataclass(frozen=True, slots=True)
class SealedBaseline:
    """Immutable seal metadata attached to a :class:`BaselineSpec`."""

    spec_hash: bytes
    signature: bytes
    plan_json: str
    signing_key_id: str
    sealed_at: datetime


# ---------------------------------------------------------------------------
# Message composition
# ---------------------------------------------------------------------------


def _seal_message(spec_hash: bytes, plan_json: str) -> bytes:
    """Canonical byte string that the HMAC covers.

    We interleave ``spec_hash || 0x1F || plan_json.utf8`` so that
    trimming or padding ``plan_json`` by itself yields a different
    digest — same trick as :mod:`efe_rollout.prereg`.
    """
    return spec_hash + b"\x1f" + plan_json.encode("utf-8")


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def seal_baseline(
    *,
    spec: BaselineSpec,
    signing_key: bytes,
    signing_key_id: str,
    sealed_at: datetime,
) -> SealedBaseline:
    """Produce an immutable seal for ``spec``."""
    if len(signing_key) < 16:
        raise ValueError(
            f"signing_key too short ({len(signing_key)} bytes, need 16+)",
        )
    plan_json = canonical_json_bytes(spec.to_canonical_dict()).decode("utf-8")
    spec_hash = content_hash(spec.to_canonical_dict())
    signature = hmac_sign(
        _seal_message(spec_hash, plan_json),
        signing_key,
        domain_tag=DOMAIN_TAG_BASELINE,
    )
    return SealedBaseline(
        spec_hash=spec_hash,
        signature=signature,
        plan_json=plan_json,
        signing_key_id=signing_key_id,
        sealed_at=sealed_at,
    )


def verify_baseline(
    sealed: SealedBaseline,
    *,
    signing_key: bytes,
    spec: BaselineSpec | None = None,
) -> None:
    """Raise if ``sealed`` has been tampered with.

    When ``spec`` is provided, its :meth:`spec_hash` must match
    the sealed hash — this catches a forged signature over a
    mutated spec.
    """
    if not hmac_verify(
        _seal_message(sealed.spec_hash, sealed.plan_json),
        signing_key,
        sealed.signature,
        domain_tag=DOMAIN_TAG_BASELINE,
    ):
        raise BaselineSignatureInvalid(
            "baseline HMAC signature does not verify",
        )
    # plan_json must deterministically hash to spec_hash.
    parsed = json.loads(sealed.plan_json)
    recomputed = content_hash(parsed)
    if recomputed != sealed.spec_hash:
        raise BaselineSpecHashMismatch(
            "plan_json does not hash to the sealed spec_hash",
            detail={"expected_hex": sealed.spec_hash.hex()[:16] + "..."},
        )
    if spec is not None:
        live_hash = content_hash(spec.to_canonical_dict())
        if live_hash != sealed.spec_hash:
            raise BaselineSpecHashMismatch(
                "live spec_hash disagrees with sealed spec_hash — "
                "spec mutated after seal",
            )


__all__ = [
    "SealedBaseline",
    "seal_baseline",
    "verify_baseline",
]
