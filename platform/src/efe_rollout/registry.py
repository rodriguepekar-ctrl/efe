"""Experiment registry — persistence + FSM + pre-registration.

The :class:`ExperimentRegistry` is the **single source of truth** for
what experiments exist, what state they are in, and what plan has
been pre-registered for each. Every state transition goes through a
compare-and-swap on ``state_version`` so concurrent mutations are
serialised.

Responsibilities :

* Create / fetch :class:`Experiment` rows by id / by name.
* Load a full :class:`ExperimentSpec` given an experiment id.
* Transition FSM states under optimistic lock.
* Seal + verify :class:`PreRegistration` via :mod:`efe_rollout.prereg`.
* Track active-layer allocations via :class:`LayerAllocation` rows
  (enforces "≤ 1 running experiment per (tenant, task, layer)").

The storage Protocol is implemented by a :class:`SqlRegistryStore`
(production) and an :class:`InMemoryRegistryStore` (unit tests).
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Protocol

from efe_rollout.clock import Clock, SystemClock
from efe_rollout.errors import (
    DuplicateExperiment,
    ExperimentNotFound,
    LayerSlotBusy,
    PreRegistrationMissing,
    PreRegistrationSealed,
    StateVersionConflict,
)
from efe_rollout.experiment import (
    Experiment,
    ExperimentFsm,
    ExperimentSpec,
    ExperimentState,
    Variant,
)
from efe_rollout.ids import new_uuid
from efe_rollout.prereg import PreRegistration
from efe_rollout.prereg import seal as prereg_seal
from efe_rollout.prereg import verify as prereg_verify

# ---------------------------------------------------------------------------
# Storage Protocol
# ---------------------------------------------------------------------------


class RegistryStore(Protocol):
    async def create(
        self, *, spec: ExperimentSpec, now: datetime,
    ) -> Experiment:  # pragma: no cover
        ...

    async def get(
        self, experiment_id: str,
    ) -> Experiment:  # pragma: no cover
        ...

    async def get_by_name(
        self, *, tenant_id: str, name: str,
    ) -> Experiment:  # pragma: no cover
        ...

    async def list(
        self, *, tenant_id: str | None = None, state: str | None = None,
    ) -> tuple[Experiment, ...]:  # pragma: no cover
        ...

    async def transition(
        self,
        experiment_id: str,
        *,
        from_state: ExperimentState,
        to_state: ExperimentState,
        expected_version: int,
        now: datetime,
    ) -> Experiment:  # pragma: no cover
        ...

    async def set_prereg(
        self, prereg: PreRegistration,
    ) -> None:  # pragma: no cover
        ...

    async def get_prereg(
        self, experiment_id: str,
    ) -> PreRegistration | None:  # pragma: no cover
        ...

    async def register_layer_allocation(
        self,
        *,
        tenant_id: str,
        task_id: str,
        layer: str,
        experiment_id: str,
        now: datetime,
    ) -> None:  # pragma: no cover
        ...

    async def end_layer_allocation(
        self,
        *,
        tenant_id: str,
        task_id: str,
        layer: str,
        experiment_id: str,
        now: datetime,
    ) -> None:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# In-memory implementation (tests + dev)
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _LayerSlot:
    tenant_id: str
    task_id: str
    layer: str
    experiment_id: str
    started_at: datetime
    ended_at: datetime | None = None


class InMemoryRegistryStore:
    """Async-lock-guarded store suitable for unit tests."""

    __slots__ = (
        "_by_name",
        "_experiments",
        "_layer_slots",
        "_lock",
        "_preregs",
    )

    def __init__(self) -> None:
        self._experiments: dict[str, Experiment] = {}
        self._by_name: dict[tuple[str, str], str] = {}
        self._preregs: dict[str, PreRegistration] = {}
        self._layer_slots: list[_LayerSlot] = []
        self._lock = asyncio.Lock()

    async def create(
        self, *, spec: ExperimentSpec, now: datetime,
    ) -> Experiment:
        async with self._lock:
            key = (spec.tenant_id, spec.name)
            if key in self._by_name:
                raise DuplicateExperiment(
                    f"experiment {spec.name!r} already exists for tenant "
                    f"{spec.tenant_id!r}",
                )
            experiment_id = new_uuid()
            variants_with_ids = tuple(
                Variant(
                    name=v.name,
                    allocation_pct=v.allocation_pct,
                    payload=v.payload,
                    is_control=v.is_control,
                    variant_id=v.variant_id or new_uuid(),
                )
                for v in spec.variants
            )
            spec_with_ids = ExperimentSpec(
                name=spec.name,
                tenant_id=spec.tenant_id,
                task_id=spec.task_id,
                layer=spec.layer,
                kind=spec.kind,
                owner=spec.owner,
                hypothesis=spec.hypothesis,
                primary_metric=spec.primary_metric,
                variants=variants_with_ids,
                guardrails=spec.guardrails,
                alpha=spec.alpha,
                power_target=spec.power_target,
                minimum_detectable_effect=spec.minimum_detectable_effect,
                stopping_rule=spec.stopping_rule,
                planned_sample_size=spec.planned_sample_size,
                planned_duration_days=spec.planned_duration_days,
                traffic_allocation_bps=spec.traffic_allocation_bps,
                covariates_for_cuped=spec.covariates_for_cuped,
                notes=spec.notes,
            )
            exp = Experiment(
                experiment_id=experiment_id,
                spec=spec_with_ids,
                state="draft",
                state_version=0,
                created_at=now,
            )
            self._experiments[experiment_id] = exp
            self._by_name[key] = experiment_id
            return exp

    async def get(self, experiment_id: str) -> Experiment:
        exp = self._experiments.get(experiment_id)
        if exp is None:
            raise ExperimentNotFound(
                f"unknown experiment_id {experiment_id!r}",
            )
        return exp

    async def get_by_name(
        self, *, tenant_id: str, name: str,
    ) -> Experiment:
        eid = self._by_name.get((tenant_id, name))
        if eid is None:
            raise ExperimentNotFound(
                f"unknown experiment name {name!r} for tenant {tenant_id!r}",
            )
        return self._experiments[eid]

    async def list(
        self, *, tenant_id: str | None = None, state: str | None = None,
    ) -> tuple[Experiment, ...]:
        rows = tuple(self._experiments.values())
        if tenant_id is not None:
            rows = tuple(e for e in rows if e.spec.tenant_id == tenant_id)
        if state is not None:
            rows = tuple(e for e in rows if e.state == state)
        return rows

    async def transition(
        self,
        experiment_id: str,
        *,
        from_state: ExperimentState,
        to_state: ExperimentState,
        expected_version: int,
        now: datetime,
    ) -> Experiment:
        async with self._lock:
            current = self._experiments.get(experiment_id)
            if current is None:
                raise ExperimentNotFound(
                    f"unknown experiment_id {experiment_id!r}",
                )
            if current.state != from_state:
                raise StateVersionConflict(
                    f"expected state {from_state!r}, got {current.state!r}",
                )
            if current.state_version != expected_version:
                raise StateVersionConflict(
                    f"expected version {expected_version}, "
                    f"got {current.state_version}",
                )
            nxt = current.with_state(to_state, now=now)
            self._experiments[experiment_id] = nxt
            return nxt

    async def set_prereg(self, prereg: PreRegistration) -> None:
        async with self._lock:
            if prereg.experiment_id in self._preregs:
                raise PreRegistrationSealed(
                    "pre-registration already sealed — "
                    "create a new experiment",
                )
            self._preregs[prereg.experiment_id] = prereg

    async def get_prereg(
        self, experiment_id: str,
    ) -> PreRegistration | None:
        return self._preregs.get(experiment_id)

    async def register_layer_allocation(
        self,
        *,
        tenant_id: str,
        task_id: str,
        layer: str,
        experiment_id: str,
        now: datetime,
    ) -> None:
        async with self._lock:
            for slot in self._layer_slots:
                if (
                    slot.tenant_id == tenant_id
                    and slot.task_id == task_id
                    and slot.layer == layer
                    and slot.ended_at is None
                    and slot.experiment_id != experiment_id
                ):
                    raise LayerSlotBusy(
                        f"layer {layer!r} for ({tenant_id}, {task_id}) "
                        f"already occupied by experiment "
                        f"{slot.experiment_id!r}",
                    )
            self._layer_slots.append(
                _LayerSlot(
                    tenant_id=tenant_id,
                    task_id=task_id,
                    layer=layer,
                    experiment_id=experiment_id,
                    started_at=now,
                ),
            )

    async def end_layer_allocation(
        self,
        *,
        tenant_id: str,
        task_id: str,
        layer: str,
        experiment_id: str,
        now: datetime,
    ) -> None:
        async with self._lock:
            for slot in self._layer_slots:
                if (
                    slot.tenant_id == tenant_id
                    and slot.task_id == task_id
                    and slot.layer == layer
                    and slot.experiment_id == experiment_id
                    and slot.ended_at is None
                ):
                    slot.ended_at = now
                    return


# ---------------------------------------------------------------------------
# High-level façade
# ---------------------------------------------------------------------------


class ExperimentRegistry:
    """Service-level glue over :class:`RegistryStore`.

    Consumers of the rollout module talk to this class, not to the
    store directly — it centralises FSM validation, pre-registration
    sealing, and layer-slot orchestration.
    """

    __slots__ = ("_clock", "_signing_key", "_signing_key_id", "_store")

    def __init__(
        self,
        *,
        store: RegistryStore,
        signing_key: bytes,
        signing_key_id: str,
        clock: Clock | None = None,
    ) -> None:
        self._store = store
        self._clock = clock or SystemClock()
        if len(signing_key) < 16:
            raise ValueError("signing_key too short (need 16+ bytes)")
        self._signing_key = bytes(signing_key)
        self._signing_key_id = signing_key_id

    # ---- CRUD -----------------------------------------------------------

    async def create(self, spec: ExperimentSpec) -> Experiment:
        return await self._store.create(spec=spec, now=self._clock.now())

    async def get(self, experiment_id: str) -> Experiment:
        return await self._store.get(experiment_id)

    async def get_by_name(
        self, *, tenant_id: str, name: str,
    ) -> Experiment:
        return await self._store.get_by_name(tenant_id=tenant_id, name=name)

    async def list(
        self, *, tenant_id: str | None = None, state: str | None = None,
    ) -> tuple[Experiment, ...]:
        return await self._store.list(tenant_id=tenant_id, state=state)

    # ---- FSM ------------------------------------------------------------

    async def transition(
        self,
        experiment_id: str,
        *,
        to_state: ExperimentState,
        actor_role: str,
    ) -> Experiment:
        """Move an experiment between FSM states, with atomicity.

        When transitioning **into** ``running`` we must ensure the
        layer slot is free *before* mutating state — otherwise a
        :class:`LayerSlotBusy` reaches the caller with the experiment
        already in ``running``, violating the "≤ 1 active per slot"
        invariant.

        Order :
          1. Validate FSM (pure, no I/O).
          2. If target is ``running``: pre-reserve the layer slot.
          3. Commit the state transition (CAS on ``state_version``).
          4. On success: for terminal states, end the layer slot.
          5. On step-3 failure after step-2 success: compensate by
             calling ``end_layer_allocation`` to release the slot.
        """
        exp = await self._store.get(experiment_id)
        ExperimentFsm.validate(
            exp.state, to_state, actor_role=actor_role,
        )
        now = self._clock.now()
        # Step 2: reserve layer slot BEFORE state change so a LayerSlotBusy
        # doesn't leave us in a corrupt half-transitioned state.
        if to_state == "running":
            await self._store.register_layer_allocation(
                tenant_id=exp.spec.tenant_id,
                task_id=exp.spec.task_id,
                layer=exp.spec.layer,
                experiment_id=experiment_id,
                now=now,
            )
        try:
            updated = await self._store.transition(
                experiment_id,
                from_state=exp.state,
                to_state=to_state,
                expected_version=exp.state_version,
                now=now,
            )
        except Exception:
            # Step 5: compensate — release the slot we just reserved.
            if to_state == "running":
                await self._store.end_layer_allocation(
                    tenant_id=exp.spec.tenant_id,
                    task_id=exp.spec.task_id,
                    layer=exp.spec.layer,
                    experiment_id=experiment_id,
                    now=now,
                )
            raise
        # Step 4: close the layer slot on terminal states.
        if to_state in {"aborted", "shipped", "rollback",
                        "inconclusive", "canceled"}:
            await self._store.end_layer_allocation(
                tenant_id=exp.spec.tenant_id,
                task_id=exp.spec.task_id,
                layer=exp.spec.layer,
                experiment_id=experiment_id,
                now=now,
            )
        return updated

    # ---- pre-registration ----------------------------------------------

    async def pre_register(self, experiment_id: str) -> PreRegistration:
        """Seal the spec + move ``designed → registered``.

        Seal semantics :

        * A sealed prereg is **immutable** — re-calling with the state
          already ``registered`` (or beyond) raises
          :class:`PreRegistrationSealed`.
        * If the experiment is still ``designed`` AND a prereg already
          exists (because a previous call sealed it but the
          ``designed → registered`` transition crashed), we treat the
          call as a resume : verify the stored prereg matches the
          current spec, then drive the pending transition. This is the
          only safe way to unblock an operator stuck at the half-done
          point; the integrity guarantee (spec_hash tamper detection)
          is preserved by :func:`prereg.verify`.
        """
        exp = await self._store.get(experiment_id)
        existing = await self._store.get_prereg(experiment_id)

        if existing is not None and exp.state != "designed":
            # Already fully sealed + transitioned (or terminal). No-op.
            raise PreRegistrationSealed(
                "experiment already pre-registered",
            )

        if existing is not None and exp.state == "designed":
            # Resume path : prior call sealed the prereg but the
            # subsequent state transition did not commit. Verify
            # integrity before driving the transition forward.
            prereg_verify(
                existing,
                signing_key=self._signing_key,
                spec=exp.spec,
            )
            await self._store.transition(
                experiment_id,
                from_state="designed",
                to_state="registered",
                expected_version=exp.state_version,
                now=self._clock.now(),
            )
            return existing

        if exp.state != "designed":
            raise PreRegistrationMissing(
                "pre-registration requires state 'designed', "
                f"got {exp.state!r}",
            )

        prereg = prereg_seal(
            experiment_id=experiment_id,
            spec=exp.spec,
            signing_key=self._signing_key,
            signing_key_id=self._signing_key_id,
            now=self._clock.now(),
        )
        await self._store.set_prereg(prereg)
        # Advance to 'registered' state. If this fails, the operator can
        # re-run pre_register; the resume path above will finish the job.
        await self._store.transition(
            experiment_id,
            from_state="designed",
            to_state="registered",
            expected_version=exp.state_version,
            now=self._clock.now(),
        )
        return prereg

    async def verify_prereg(self, experiment_id: str) -> PreRegistration:
        prereg = await self._store.get_prereg(experiment_id)
        if prereg is None:
            raise PreRegistrationMissing(
                f"no pre-registration for {experiment_id!r}",
            )
        exp = await self._store.get(experiment_id)
        prereg_verify(
            prereg,
            signing_key=self._signing_key,
            spec=exp.spec,
        )
        return prereg


__all__ = [
    "ExperimentRegistry",
    "InMemoryRegistryStore",
    "RegistryStore",
]
