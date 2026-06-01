"""Hash-based stable variant assignment — the hot path.

Invariants enforced here :

* **I1** Randomisation stable :
    ``assign(u, e)`` is deterministic given a fixed ``(Experiment, salt)``.
    Restarts, crashes, and DB migrations do not change any assignment.

* **I4** Layer orthogonality :
    The hash input includes the layer salt so two experiments on
    different layers produce statistically independent assignments.

* **I8** Permanent holdout :
    Holdout check runs *first* so no holdout user is ever assigned to
    any experiment.

Routing ladder (first match wins) :

  ┌──────────────────────────────────────────────┐
  │ 1. cache hit                                 │
  │ 2. forced-assignment override (QA / debug)   │
  │ 3. global 1 % holdout                        │
  │ 4. traffic-allocation cap (experiment-level) │
  │ 5. hash-based variant pick (cumulative %)    │
  └──────────────────────────────────────────────┘

Every decision is **written back to the cache** after the hash path,
so a recurring user costs only one cache read on subsequent requests.

Latency budget : p99 < 2 ms (no DB round-trip, no network).
"""

from __future__ import annotations

from dataclasses import dataclass

from efe_rollout.assignment.cache import AssignmentCache, InMemoryAssignmentCache
from efe_rollout.assignment.forced import (
    ForcedAssignmentStore,
    InMemoryForcedAssignmentStore,
)
from efe_rollout.assignment.holdout import HoldoutFilter
from efe_rollout.errors import AllocationInvariantBroken
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.hashing import bucket, murmur3_hash64
from efe_rollout.layers import LayerRegistry


@dataclass(frozen=True, slots=True)
class AssignmentDecision:
    """Rich result from :meth:`HashAssigner.assign`.

    Tests can inspect every routing-ladder decision via the ``reason``
    field. The :class:`~efe_rollout.metrics.ExposureRecorder` reads
    ``forced`` / ``holdout`` to label the Exposure row.
    """

    variant: Variant | None
    forced: bool
    holdout: bool
    from_cache: bool
    outside_traffic: bool
    reason: str


class HashAssigner:
    """Stateless assignment engine.

    Depends on :class:`LayerRegistry` for salt resolution, on a
    :class:`HoldoutFilter` for the global holdout, on an optional
    :class:`ForcedAssignmentStore`, and on an :class:`AssignmentCache`.

    The experiment spec itself is passed per-call — the assigner does
    not talk to a DB. Callers (the service layer) load the spec once
    and reuse it for the remaining requests.
    """

    __slots__ = ("_cache", "_forced", "_holdout", "_layers")

    def __init__(
        self,
        *,
        layers: LayerRegistry,
        holdout: HoldoutFilter,
        forced: ForcedAssignmentStore | None = None,
        cache: AssignmentCache | None = None,
    ) -> None:
        self._layers = layers
        self._holdout = holdout
        self._forced = forced or InMemoryForcedAssignmentStore()
        self._cache = cache or InMemoryAssignmentCache()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def assign(
        self,
        *,
        user_id: str,
        experiment_id: str,
        spec: ExperimentSpec,
    ) -> AssignmentDecision:
        # 1. Cache hit?
        cached = await self._cache.get(
            experiment_id=experiment_id, user_id=user_id,
        )
        if cached is not None:
            variant = self._variant_by_id(spec, cached)
            if variant is not None:
                return AssignmentDecision(
                    variant=variant,
                    forced=False,
                    holdout=False,
                    from_cache=True,
                    outside_traffic=False,
                    reason="cache",
                )
            # Stale cache entry — ``cached`` holds a variant_id that is
            # no longer part of the spec (spec re-created with different
            # variants, or variant dropped). We actively delete the
            # stale entry so the subsequent ``has_record`` probe does
            # NOT fabricate a cached-holdout and we recompute the fresh
            # decision cleanly.
            await self._cache.delete(
                experiment_id=experiment_id, user_id=user_id,
            )
            cached = None
        has_record = await self._has_cached(experiment_id, user_id)

        # 2. Forced override?
        forced = await self._forced.lookup(
            experiment_id=experiment_id, user_id=user_id,
        )
        if forced is not None:
            variant = self._variant_by_id(spec, forced.variant_id)
            await self._cache.put(
                experiment_id=experiment_id,
                user_id=user_id,
                variant_id=forced.variant_id,
            )
            return AssignmentDecision(
                variant=variant,
                forced=True,
                holdout=False,
                from_cache=False,
                outside_traffic=False,
                reason="forced",
            )

        # 3. Cached holdout marker?
        if has_record and cached is None:
            return AssignmentDecision(
                variant=None,
                forced=False,
                holdout=True,
                from_cache=True,
                outside_traffic=False,
                reason="holdout_cache",
            )

        # 4. Global holdout?
        if self._holdout.is_holdout(user_id):
            await self._cache.put(
                experiment_id=experiment_id,
                user_id=user_id,
                variant_id=None,
            )
            return AssignmentDecision(
                variant=None,
                forced=False,
                holdout=True,
                from_cache=False,
                outside_traffic=False,
                reason="holdout",
            )

        # 5. Layer-level traffic allocation (cap).
        salt = self._layers.salt(spec.layer)
        layer_hash = murmur3_hash64(
            f"layer:{spec.layer}:{salt}:{spec.tenant_id}:{user_id}",
        )
        layer_bps = bucket(layer_hash, buckets=10_000)
        if layer_bps >= spec.traffic_allocation_bps:
            await self._cache.put(
                experiment_id=experiment_id,
                user_id=user_id,
                variant_id=None,
            )
            return AssignmentDecision(
                variant=None,
                forced=False,
                holdout=False,
                from_cache=False,
                outside_traffic=True,
                reason="outside_traffic_cap",
            )

        # 6. Hash-based variant pick.
        variant = self._pick_variant(spec, user_id, salt)
        await self._cache.put(
            experiment_id=experiment_id,
            user_id=user_id,
            variant_id=variant.variant_id,
        )
        return AssignmentDecision(
            variant=variant,
            forced=False,
            holdout=False,
            from_cache=False,
            outside_traffic=False,
            reason="hash",
        )

    # ------------------------------------------------------------------
    # helpers
    # ------------------------------------------------------------------

    async def _has_cached(
        self, experiment_id: str, user_id: str,
    ) -> bool:
        has_fn = getattr(self._cache, "has", None)
        if callable(has_fn):
            result = await has_fn(
                experiment_id=experiment_id, user_id=user_id,
            )
            return bool(result)
        return False

    def _variant_by_id(
        self, spec: ExperimentSpec, variant_id: str,
    ) -> Variant | None:
        for v in spec.variants:
            if v.variant_id == variant_id:
                return v
        return None

    def _pick_variant(
        self, spec: ExperimentSpec, user_id: str, salt: str,
    ) -> Variant:
        """Cumulative-percentage pick over ``variants_sorted()``.

        ``variants_sorted()`` returns name-ordered variants so the
        cumulative boundaries don't flip between calls even if the DB
        round-trips the variants in a different order. The experiment
        *name* is mixed into the hash input so two experiments on the
        same layer (same salt, same tenant) produce independent
        variant streams — this is what the "no cross-experiment
        contamination" risk mitigation in M08 §21 calls for.
        """
        variants = spec.variants_sorted()
        variant_hash = murmur3_hash64(
            f"variant:{spec.tenant_id}:{salt}:{spec.name}:{user_id}",
        )
        bps = bucket(variant_hash, buckets=10_000)
        cumulative = 0
        for v in variants:
            cumulative += v.allocation_pct * 100  # % → bps
            if bps < cumulative:
                return v
        # Defence in depth: sum(alloc)==100 is validated at spec
        # construction, so the loop must return. If not, the spec is
        # corrupted.
        raise AllocationInvariantBroken(
            "variant pick fell off the end — allocations do not cover [0, 10000]",
            detail={"bps": bps, "cumulative": cumulative},
        )


__all__ = ["AssignmentDecision", "HashAssigner"]
