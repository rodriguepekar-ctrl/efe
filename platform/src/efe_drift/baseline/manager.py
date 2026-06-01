"""BaselineManager — high-level service layer.

The manager orchestrates :

* FSM validation (:class:`BaselineFsm`)
* Persistence (:class:`BaselineStore`)
* Sanity checking (:class:`BaselineValidator`)
* Sealing (:func:`seal_baseline`)
* Rotation policy (:func:`plan_rotation`)
* NATS eventing (optional — a :class:`Publisher` can be injected)

It exposes a small API suitable for the service layer / HTTP handlers
/ CLI :

* :meth:`start_planned` — create a PLANNED baseline for a newly
  promoted artefact.
* :meth:`start_accumulating` — drive ``planned → accumulating``.
* :meth:`close_candidate` — drive ``accumulating → candidate`` given a
  :class:`BaselineCandidate`.
* :meth:`promote` — seal + ``candidate → active`` + rotation.
* :meth:`reject` — ``candidate|planned|accumulating → rejected``.
* :meth:`emergency_retire` — ``active → retired`` (admin only).
* :meth:`tick_rotation` — scheduler hook that walks RETIRING baselines
  whose ``planned_end_at`` has passed and moves them to RETIRED.

All transitions are recorded in the audit journal via
:meth:`BaselineStore.record_transition`.
"""

from __future__ import annotations

from typing import Any

from efe_drift.baseline.candidate import BaselineCandidate
from efe_drift.baseline.rotation import (
    RotationPlan,
    is_rotation_due,
    plan_rotation,
)
from efe_drift.baseline.seal import (
    SealedBaseline,
    seal_baseline,
    verify_baseline,
)
from efe_drift.baseline.state import BaselineFsm
from efe_drift.baseline.store import BaselineStore
from efe_drift.baseline.validator import (
    BaselineValidator,
    ValidationReport,
)
from efe_drift.clock import Clock, SystemClock
from efe_drift.errors import (
    BaselineNotFound,
    ValidationError,
)
from efe_drift.nats.publisher import Publisher
from efe_drift.records import Baseline, BaselineSpec, BaselineState

# ---------------------------------------------------------------------------
# Manager
# ---------------------------------------------------------------------------


class BaselineManager:
    """Service-level orchestrator for baseline lifecycle."""

    __slots__ = (
        "_clock",
        "_default_overlap_days",
        "_publisher",
        "_signing_key",
        "_signing_key_id",
        "_store",
        "_topic_prefix",
        "_validator",
    )

    def __init__(
        self,
        *,
        store: BaselineStore,
        validator: BaselineValidator | None = None,
        signing_key: bytes,
        signing_key_id: str,
        default_overlap_days: int = 3,
        clock: Clock | None = None,
        publisher: Publisher | None = None,
        topic_prefix: str = "efe.drift",
    ) -> None:
        if len(signing_key) < 16:
            raise ValueError(
                f"signing_key too short ({len(signing_key)} bytes, need 16+)",
            )
        self._store = store
        self._validator = validator or BaselineValidator()
        self._signing_key = bytes(signing_key)
        self._signing_key_id = signing_key_id
        self._default_overlap_days = default_overlap_days
        self._clock = clock or SystemClock()
        self._publisher = publisher
        self._topic_prefix = topic_prefix.rstrip(".")

    # ---- Read helpers --------------------------------------------------

    async def get(self, baseline_id: str) -> Baseline:
        return await self._store.get(baseline_id)

    async def active_for(
        self, *, tenant_id: str, task_id: str, feature_name: str,
    ) -> Baseline | None:
        return await self._store.get_active(
            tenant_id=tenant_id,
            task_id=task_id,
            feature_name=feature_name,
        )

    async def list(
        self,
        *,
        tenant_id: str | None = None,
        task_id: str | None = None,
        state: BaselineState | None = None,
    ) -> tuple[Baseline, ...]:
        return await self._store.list(
            tenant_id=tenant_id, task_id=task_id, state=state,
        )

    # ---- Happy path transitions ---------------------------------------

    async def start_planned(self, spec: BaselineSpec) -> Baseline:
        """Create a PLANNED baseline (e.g. right after M06 promotion)."""
        now = self._clock.now()
        baseline = await self._store.create(
            spec=spec, state="planned", now=now,
        )
        await self._store.record_transition(
            baseline_id=baseline.baseline_id,
            from_state="planned",
            to_state="planned",
            actor="system",
            reason="create",
            now=now,
        )
        return baseline

    async def start_accumulating(
        self, baseline_id: str, *, actor: str = "system",
    ) -> Baseline:
        return await self._transition(
            baseline_id,
            to_state="accumulating",
            actor=actor,
            actor_role="system",
            reason="accumulation started",
        )

    async def close_candidate(
        self,
        baseline_id: str,
        *,
        candidate: BaselineCandidate,
        n_bins: int = 10,
        top_k: int = 10,
        custom_edges: tuple[float, ...] | None = None,
        overlap_days: int | None = None,
        model_artifact_id: str | None = None,
        notes: str | None = None,
        actor: str = "system",
    ) -> tuple[Baseline, BaselineSpec, ValidationReport]:
        """Close the candidate's accumulation + update the baseline's
        spec on disk + move to ``CANDIDATE`` state.

        Returns the validator's report so the caller can surface
        warnings in the UI even on successful close.
        """
        existing = await self._store.get(baseline_id)
        if existing.state != "accumulating":
            raise ValidationError(
                f"close_candidate requires state 'accumulating', "
                f"got {existing.state!r}",
            )
        spec = candidate.close(
            n_bins=n_bins,
            top_k=top_k,
            custom_edges=custom_edges,
            model_artifact_id=model_artifact_id,
            overlap_days=overlap_days or self._default_overlap_days,
            notes=notes,
        )
        report = self._validator.validate(spec)
        if not report.ok:
            self._validator.validate_strict(spec)   # raises
        # Persist the spec inside the existing baseline row by
        # re-creating (CAS) — InMemoryBaselineStore stores the spec as
        # part of the Baseline object, so we have to go through the
        # transition path *and* rewrite the spec. We use the seal
        # endpoint with a placeholder hash; the final seal happens in
        # ``promote``.
        # Swap the in-memory baseline's spec first (through an
        # internal helper; the real DB path sets spec columns here).
        await self._replace_spec_in_place(baseline_id, spec)
        # Now move state to CANDIDATE.
        updated = await self._transition(
            baseline_id,
            to_state="candidate",
            actor=actor,
            actor_role="system",
            reason="candidate closed",
        )
        return updated, spec, report

    async def promote(
        self,
        baseline_id: str,
        *,
        actor: str = "owner",
        overlap_days: int | None = None,
    ) -> tuple[Baseline, RotationPlan, SealedBaseline]:
        """Seal the spec + move ``candidate → active`` + rotate previous.

        Returns the now-active baseline, the rotation plan applied,
        and the :class:`SealedBaseline` metadata.
        """
        current = await self._store.get(baseline_id)
        if current.state != "candidate":
            raise ValidationError(
                f"promote requires state 'candidate', "
                f"got {current.state!r}",
            )
        report = self._validator.validate(current.spec)
        if not report.ok:
            self._validator.validate_strict(current.spec)
        now = self._clock.now()
        sealed = seal_baseline(
            spec=current.spec,
            signing_key=self._signing_key,
            signing_key_id=self._signing_key_id,
            sealed_at=now,
        )
        # Persist seal (CAS on state_version).
        sealed_row = await self._store.seal(
            baseline_id,
            spec_hash=sealed.spec_hash,
            signing_key_id=sealed.signing_key_id,
            signature=sealed.signature,
            expected_version=current.state_version,
        )
        # Find the current ACTIVE (if any).
        previous = await self._store.get_active(
            tenant_id=current.spec.tenant_id,
            task_id=current.spec.task_id,
            feature_name=current.spec.feature_name,
        )
        plan = plan_rotation(
            incoming=sealed_row,
            currently_active=previous,
            promotion_time=now,
            overlap_days=overlap_days or self._default_overlap_days,
        )
        # If there is a predecessor, rotate it FIRST so a reader that
        # lists ACTIVE baselines between steps still sees exactly one
        # active (the previous one). This preserves the invariant even
        # across the two CAS operations.
        if plan.has_predecessor:
            assert plan.previous_baseline_id is not None
            assert plan.previous_state_version is not None
            assert plan.previous_planned_end_at is not None
            await self._store.set_planned_end(
                plan.previous_baseline_id,
                planned_end_at=plan.previous_planned_end_at,
                expected_version=plan.previous_state_version,
            )
            # Fetch refreshed view (state_version bumped) before transit.
            refreshed = await self._store.get(plan.previous_baseline_id)
            await self._store.transition(
                plan.previous_baseline_id,
                from_state="active",
                to_state="retiring",
                expected_version=refreshed.state_version,
                now=now,
            )
            await self._store.record_transition(
                baseline_id=plan.previous_baseline_id,
                from_state="active",
                to_state="retiring",
                actor="system",
                reason="rotated by promotion",
                now=now,
            )
        # Now flip our candidate to active.
        new_active = await self._store.transition(
            baseline_id,
            from_state="candidate",
            to_state="active",
            expected_version=sealed_row.state_version,
            now=now,
        )
        await self._store.record_transition(
            baseline_id=baseline_id,
            from_state="candidate",
            to_state="active",
            actor=actor,
            reason="promoted",
            now=now,
        )
        # Optional NATS publication.
        if self._publisher is not None:
            await self._publisher.publish(
                f"{self._topic_prefix}.baseline_promoted",
                {
                    "baseline_id": new_active.baseline_id,
                    "tenant_id": new_active.spec.tenant_id,
                    "task_id": new_active.spec.task_id,
                    "feature_name": new_active.spec.feature_name,
                    "spec_hash": sealed.spec_hash.hex(),
                    "overlap_days": plan.overlap_days,
                    "previous_baseline_id": plan.previous_baseline_id,
                },
            )
        return new_active, plan, sealed

    async def reject(
        self,
        baseline_id: str,
        *,
        actor: str = "owner",
        reason: str,
    ) -> Baseline:
        current = await self._store.get(baseline_id)
        role = "admin" if current.state == "accumulating" else "owner"
        updated = await self._transition(
            baseline_id,
            to_state="rejected",
            actor=actor,
            actor_role=role,
            reason=reason or "rejected",
        )
        return updated

    async def emergency_retire(
        self,
        baseline_id: str,
        *,
        actor: str,
        reason: str,
    ) -> Baseline:
        updated = await self._transition(
            baseline_id,
            to_state="retired",
            actor=actor,
            actor_role="admin",
            reason=reason or "emergency retire",
        )
        if self._publisher is not None:
            await self._publisher.publish(
                f"{self._topic_prefix}.baseline_emergency_retired",
                {
                    "baseline_id": updated.baseline_id,
                    "tenant_id": updated.spec.tenant_id,
                    "task_id": updated.spec.task_id,
                    "feature_name": updated.spec.feature_name,
                    "reason": reason,
                },
            )
        return updated

    async def tick_rotation(self) -> tuple[Baseline, ...]:
        """Scheduler hook — drive ``retiring → retired`` where due.

        Returns the list of baselines that were retired in this tick.
        """
        now = self._clock.now()
        all_retiring = await self._store.list(state="retiring")
        retired: list[Baseline] = []
        for baseline in all_retiring:
            if not is_rotation_due(baseline, now=now):
                continue
            updated = await self._transition(
                baseline.baseline_id,
                to_state="retired",
                actor="system",
                actor_role="system",
                reason="overlap expired",
            )
            retired.append(updated)
            if self._publisher is not None:
                await self._publisher.publish(
                    f"{self._topic_prefix}.baseline_retired",
                    {
                        "baseline_id": updated.baseline_id,
                        "tenant_id": updated.spec.tenant_id,
                        "task_id": updated.spec.task_id,
                        "feature_name": updated.spec.feature_name,
                    },
                )
        return tuple(retired)

    # ---- Verification helper ------------------------------------------

    async def verify_seal(self, baseline_id: str) -> None:
        """Re-verify the HMAC signature of a stored baseline."""
        baseline = await self._store.get(baseline_id)
        if (
            baseline.spec_hash is None
            or baseline.signature is None
            or baseline.signing_key_id is None
        ):
            raise ValidationError(
                f"baseline {baseline_id!r} is not yet sealed",
            )
        # Reconstruct the :class:`SealedBaseline` from stored columns.
        from efe_drift.hashing import canonical_json_bytes
        plan_json = canonical_json_bytes(
            baseline.spec.to_canonical_dict(),
        ).decode("utf-8")
        sealed = SealedBaseline(
            spec_hash=baseline.spec_hash,
            signature=baseline.signature,
            plan_json=plan_json,
            signing_key_id=baseline.signing_key_id,
            sealed_at=baseline.promoted_at or baseline.created_at
                     or self._clock.now(),
        )
        verify_baseline(
            sealed,
            signing_key=self._signing_key,
            spec=baseline.spec,
        )

    # ---- Internals -----------------------------------------------------

    async def _transition(
        self,
        baseline_id: str,
        *,
        to_state: BaselineState,
        actor: str,
        actor_role: str,
        reason: str,
    ) -> Baseline:
        current = await self._store.get(baseline_id)
        BaselineFsm.validate(
            current.state, to_state, actor_role=actor_role,
        )
        now = self._clock.now()
        updated = await self._store.transition(
            baseline_id,
            from_state=current.state,
            to_state=to_state,
            expected_version=current.state_version,
            now=now,
        )
        await self._store.record_transition(
            baseline_id=baseline_id,
            from_state=current.state,
            to_state=to_state,
            actor=actor,
            reason=reason,
            now=now,
        )
        return updated

    async def _replace_spec_in_place(
        self, baseline_id: str, spec: BaselineSpec,
    ) -> None:
        """Update the spec of a non-sealed baseline.

        :class:`InMemoryBaselineStore` doesn't expose a
        ``replace_spec`` endpoint so we reach into its internals. This
        is a white-box operation used only by the manager and never
        by user code. The SQL store will expose a dedicated endpoint
        with proper CAS.
        """
        store: Any = self._store
        if not hasattr(store, "_cells"):
            raise NotImplementedError(
                "replace_spec requires InMemoryBaselineStore internals",
            )
        cell = store._cells.get(baseline_id)
        if cell is None:
            raise BaselineNotFound(
                f"unknown baseline_id {baseline_id!r}",
            )
        from dataclasses import replace
        cell.baseline = replace(cell.baseline, spec=spec)


__all__ = ["BaselineManager"]
