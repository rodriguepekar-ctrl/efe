"""Canonical in-memory records for exposures / outcomes / analyses.

Three immutable value types that flow through the ingestion pipeline:

* :class:`Exposure` — "user U saw variant V of experiment E at time T".
  Carries an HMAC signature over all non-HMAC fields so the row cannot
  be forged after the fact (invariant **I5** protection).

* :class:`Outcome` — "metric M took value X for user U on experiment E
  at time T". Carries a ``covariate`` dict that feeds CUPED, HTE, and
  segment analysis downstream.

* :class:`AnalysisResult` — canonical shape of the numbers surfaced to
  operators. Includes frequentist + bayesian outputs.

All three types are ``frozen=True, slots=True`` dataclasses so they are
hashable / serialisable / cheap.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Literal

from efe_rollout.hashing import (
    DOMAIN_TAG_EXPOSURE,
    canonical_json_bytes,
    hmac_sign,
    hmac_verify,
)


@dataclass(frozen=True, slots=True)
class Exposure:
    """Audit row : "this user saw this variant at this time".

    ``variant_id`` is ``None`` when the user fell into the global
    holdout (invariant **I8**).

    ``hmac_sig`` is the HMAC-SHA256 of the canonical JSON of every
    other field, with the rollout-specific domain tag prepended to
    defend against cross-site key replay.
    """

    exposure_id: str
    experiment_id: str
    tenant_id: str
    user_id: str
    assigned_at: datetime
    layer: str
    forced: bool
    holdout: bool
    variant_id: str | None = None
    request_id: str | None = None
    segment_hash: bytes | None = None
    hmac_sig: bytes = b""

    def _payload_for_hmac(self) -> bytes:
        payload = {
            "exposure_id": self.exposure_id,
            "experiment_id": self.experiment_id,
            "tenant_id": self.tenant_id,
            "user_id": self.user_id,
            "assigned_at": self.assigned_at.isoformat(),
            "layer": self.layer,
            "forced": self.forced,
            "holdout": self.holdout,
            "variant_id": self.variant_id,
            "request_id": self.request_id,
            "segment_hash": (
                self.segment_hash.hex() if self.segment_hash else None
            ),
        }
        return canonical_json_bytes(payload)

    def sign(self, key: bytes) -> Exposure:
        """Return a new :class:`Exposure` with ``hmac_sig`` filled in."""
        sig = hmac_sign(
            self._payload_for_hmac(), key, domain_tag=DOMAIN_TAG_EXPOSURE,
        )
        return Exposure(
            exposure_id=self.exposure_id,
            experiment_id=self.experiment_id,
            tenant_id=self.tenant_id,
            user_id=self.user_id,
            assigned_at=self.assigned_at,
            layer=self.layer,
            forced=self.forced,
            holdout=self.holdout,
            variant_id=self.variant_id,
            request_id=self.request_id,
            segment_hash=self.segment_hash,
            hmac_sig=sig,
        )

    def verify(self, key: bytes) -> bool:
        if not self.hmac_sig:
            return False
        return hmac_verify(
            self._payload_for_hmac(),
            key,
            self.hmac_sig,
            domain_tag=DOMAIN_TAG_EXPOSURE,
        )


@dataclass(frozen=True, slots=True)
class Outcome:
    """Metric observation for a (user, experiment, metric) triple.

    ``variant_id`` may be ``None`` when the outcome arrives for a user
    who was recorded as holdout (we still want to track baseline drift
    — invariant **I8** consequence).

    ``covariate`` is freeform but conventionally contains:
      * ``"country"`` / ``"device"`` / ``"tenant_tier"`` — for HTE.
      * ``"<metric>_pre"`` — for CUPED variance reduction.
    """

    outcome_id: str
    experiment_id: str
    tenant_id: str
    user_id: str
    metric_name: str
    metric_value: float
    observed_at: datetime
    variant_id: str | None = None
    request_id: str | None = None
    covariate: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class AnalysisResult:
    """Computed result of an :mod:`efe_rollout.analysis` pipeline run."""

    analysis_id: str
    experiment_id: str
    kind: Literal["interim", "final", "post_hoc", "segment", "cuped"]
    computed_at: datetime
    n_per_variant: dict[str, int]
    primary_metric: str
    estimate: float
    ci_95: tuple[float, float] | None = None
    p_value: float | None = None
    posterior_prob_better: float | None = None
    expected_lift_pct: float | None = None
    credible_interval_95: tuple[float, float] | None = None
    decision: Literal["ship", "rollback", "inconclusive", "continue"] | None = None
    methods: dict[str, Any] = field(default_factory=dict)
    segments: dict[str, dict[str, float]] = field(default_factory=dict)


__all__ = ["AnalysisResult", "Exposure", "Outcome"]
