"""Pre-registration : HMAC-sealed immutable copy of the experiment plan.

Invariant **I6** — Pre-registration scellée — demands that every
analysis publishes a hash of the plan frozen *before* any exposure, so
any post-hoc change is traceable.

Two operations live here:

* :func:`seal` — produces a :class:`PreRegistration` from an
  :class:`~efe_rollout.experiment.ExperimentSpec` by computing a
  canonical-JSON dump, hashing it with SHA-256, and signing the hash
  with HMAC-SHA256 under a caller-supplied key.

* :func:`verify` — given a :class:`PreRegistration` and the spec it
  claims to cover, recomputes the hash + HMAC and returns ``True`` iff
  both match. If the spec or the ``plan_json`` was tampered with, the
  check fails.

The HMAC uses the rollout-specific domain tag
``DOMAIN_TAG_PREREG`` so the same raw key cannot be replayed against
exposure signatures or NATS envelopes.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime

from efe_rollout.errors import (
    PreRegistrationSignatureInvalid,
    PreRegistrationSpecHashMismatch,
)
from efe_rollout.experiment import ExperimentSpec
from efe_rollout.hashing import (
    DOMAIN_TAG_PREREG,
    canonical_json_bytes,
    content_hash,
    hmac_sign,
    hmac_verify,
)


@dataclass(frozen=True, slots=True)
class PreRegistration:
    """Sealed pre-registration record.

    ``signature`` covers ``spec_hash || plan_json_bytes`` under the
    pre-reg domain tag.
    """

    experiment_id: str
    spec_hash: bytes
    signature: bytes
    plan_json: str  # canonical-JSON string of the spec
    signing_key_id: str
    created_at: datetime


def _prereg_message(spec_hash: bytes, plan_json: str) -> bytes:
    """What the HMAC is computed over.

    The separator byte ``0x1F`` (ASCII unit-separator) sits between the
    fixed-size spec_hash and the variable-length plan_json so a collision
    attack has to match both fields — trimming ``plan_json`` by itself
    moves the separator into the ``plan_json`` hash input, changing the
    whole digest.
    """
    return spec_hash + b"\x1f" + plan_json.encode("utf-8")


def seal(
    *,
    experiment_id: str,
    spec: ExperimentSpec,
    signing_key: bytes,
    signing_key_id: str,
    now: datetime,
) -> PreRegistration:
    """Produce an HMAC-sealed pre-registration for ``spec``.

    The canonical-JSON of the spec is stored verbatim inside
    :attr:`PreRegistration.plan_json` so an independent auditor can
    recompute the hash without access to the original ``ExperimentSpec``
    object.
    """
    plan_json = canonical_json_bytes(spec.to_canonical_dict()).decode("utf-8")
    spec_hash = content_hash(spec.to_canonical_dict())
    signature = hmac_sign(
        _prereg_message(spec_hash, plan_json),
        signing_key,
        domain_tag=DOMAIN_TAG_PREREG,
    )
    return PreRegistration(
        experiment_id=experiment_id,
        spec_hash=spec_hash,
        signature=signature,
        plan_json=plan_json,
        signing_key_id=signing_key_id,
        created_at=now,
    )


def verify(
    prereg: PreRegistration,
    *,
    signing_key: bytes,
    spec: ExperimentSpec | None = None,
) -> None:
    """Raise if ``prereg`` has been tampered with.

    When ``spec`` is provided, its :meth:`ExperimentSpec.spec_hash`
    must match the sealed ``spec_hash`` — this catches the case where
    someone forges a matching signature over a modified spec.
    """
    # Constant-time verify via :func:`hmac.compare_digest` (wrapped by
    # :func:`hmac_verify`). We do not re-emit the expected bytes in the
    # error path — leaking the expected digest would help an attacker
    # craft a matching forgery.
    if not hmac_verify(
        _prereg_message(prereg.spec_hash, prereg.plan_json),
        signing_key,
        prereg.signature,
        domain_tag=DOMAIN_TAG_PREREG,
    ):
        raise PreRegistrationSignatureInvalid(
            "pre-registration HMAC signature does not verify",
            detail={"experiment_id": prereg.experiment_id},
        )
    # Recompute hash from plan_json and ensure consistency.
    parsed = json.loads(prereg.plan_json)
    recomputed = content_hash(parsed)
    if recomputed != prereg.spec_hash:
        raise PreRegistrationSpecHashMismatch(
            "plan_json does not hash to the sealed spec_hash",
            detail={"experiment_id": prereg.experiment_id},
        )
    if spec is not None and spec.spec_hash() != prereg.spec_hash:
        raise PreRegistrationSpecHashMismatch(
            "live spec_hash disagrees with sealed spec_hash — spec mutated",
            detail={
                "experiment_id": prereg.experiment_id,
                "live_hash": spec.spec_hash().hex(),
                "sealed_hash": prereg.spec_hash.hex(),
            },
        )


__all__ = ["PreRegistration", "seal", "verify"]
