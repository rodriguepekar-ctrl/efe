"""Global 1 % holdout — invariant **I8**.

The *holdout* is a fixed fraction of users who are **never** assigned
to any experiment. They stay on the champion model permanently. This
gives the platform three long-term benefits:

1. **Drift detection** — if the holdout group's metrics move without a
   deploy, something external changed (model aging, traffic mix shift).
2. **Contamination baseline** — the "cumulative effect of all live
   experiments" can be estimated by diffing population-wide vs holdout
   means.
3. **Causal reset** — a permanent untouched cohort to reset inference
   pipelines against.

Design notes :

* The holdout check runs **before** the per-experiment hash so the same
  set of users are excluded from every experiment.
* The hash input is ``"holdout:" + user_id`` with no tenant salt —
  holdout is a cross-tenant concept by design.
* We fold the hash into 10 000 buckets (bps) so ``holdout_basis_points
  = 100`` means 1 %.
"""

from __future__ import annotations

from efe_rollout.hashing import bucket, murmur3_hash64


class HoldoutFilter:
    """Pure hash-based holdout predicate — no state, no I/O."""

    __slots__ = ("_bps",)

    def __init__(self, basis_points: int) -> None:
        if not 0 <= basis_points <= 10_000:
            raise ValueError(
                f"holdout basis_points must be in [0, 10000], got {basis_points}",
            )
        self._bps = int(basis_points)

    @property
    def basis_points(self) -> int:
        return self._bps

    def is_holdout(self, user_id: str) -> bool:
        if self._bps <= 0:
            return False
        h = murmur3_hash64(f"holdout:{user_id}")
        return bucket(h, buckets=10_000) < self._bps


__all__ = ["HoldoutFilter"]
