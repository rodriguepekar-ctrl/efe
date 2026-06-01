"""Snapshot materialiser — query + partition + hash + persist."""

from __future__ import annotations

import uuid
from collections import Counter
from dataclasses import dataclass
from typing import Literal

import numpy as np
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import AuditChain
from efe_ft.db.models import Snapshot, SnapshotMember, TrainingSample
from efe_ft.errors import SnapshotError
from efe_ft.hashing import content_hash_iter


@dataclass(frozen=True, slots=True)
class Partitioning:
    """How to split the materialised IDs into train/val/holdout."""

    train: int
    val: int
    holdout: int
    seed: int = 0

    def total(self) -> int:
        return self.train + self.val + self.holdout


@dataclass(frozen=True, slots=True)
class MaterialisationSpec:
    tenant_id: str
    task_id: str
    status_filter: Literal["labelled", "pending", "any"] = "labelled"
    partitioning: Partitioning | None = None
    min_samples: int = 10
    actor_id: str = "system"


async def materialise(
    session: AsyncSession,
    spec: MaterialisationSpec,
) -> Snapshot:
    """Materialise a new snapshot.

    Idempotence: if an existing snapshot has the same
    ``content_hash``, it is returned unchanged.
    """
    stmt = select(TrainingSample.id, TrainingSample.label).where(
        TrainingSample.tenant_id == spec.tenant_id,
        TrainingSample.task_id == spec.task_id,
        TrainingSample.status != "archived",
    )
    if spec.status_filter == "labelled":
        stmt = stmt.where(TrainingSample.label.is_not(None))
    elif spec.status_filter == "pending":
        stmt = stmt.where(TrainingSample.label.is_(None))

    result = await session.execute(stmt)
    rows = list(result.all())
    if len(rows) < spec.min_samples:
        raise SnapshotError(
            f"not enough samples for snapshot: {len(rows)} < {spec.min_samples}",
        )

    ids = [r[0] for r in rows]
    labels = [r[1] for r in rows]
    c_hash = content_hash_iter(ids)

    # Idempotence: look up existing snapshot.
    existing_stmt = select(Snapshot).where(
        Snapshot.tenant_id == spec.tenant_id,
        Snapshot.task_id == spec.task_id,
        Snapshot.content_hash == c_hash,
    )
    existing = (await session.execute(existing_stmt)).scalar_one_or_none()
    if existing is not None:
        return existing

    # Partition: deterministic seeded shuffle.
    partitioning = spec.partitioning
    if partitioning is not None:
        if partitioning.total() > len(ids):
            raise SnapshotError(
                f"partitioning totals {partitioning.total()} but only "
                f"{len(ids)} samples available",
            )
        rng = np.random.default_rng(partitioning.seed)
        perm = rng.permutation(len(ids))
        # Use stable slicing
        assignments: dict[str, str] = {}
        for _i, idx in enumerate(perm[: partitioning.train]):
            assignments[ids[idx]] = "train"
        for _i, idx in enumerate(perm[partitioning.train : partitioning.train + partitioning.val]):
            assignments[ids[idx]] = "val"
        tail_start = partitioning.train + partitioning.val
        for _i, idx in enumerate(perm[tail_start : tail_start + partitioning.holdout]):
            assignments[ids[idx]] = "holdout"
    else:
        assignments = {sid: "train" for sid in ids}

    holdout_hash: bytes | None = None
    if any(v == "holdout" for v in assignments.values()):
        holdout_ids = [sid for sid, part in assignments.items() if part == "holdout"]
        holdout_hash = content_hash_iter(holdout_ids)

    label_balance = dict(Counter(labels))
    # Drop None keys (pending status snapshots)
    label_balance = {k: v for k, v in label_balance.items() if k is not None}

    snap = Snapshot(
        id=str(uuid.uuid4()),
        tenant_id=spec.tenant_id,
        task_id=spec.task_id,
        sample_ids_hash=c_hash,
        content_hash=c_hash,
        n_samples=len(ids),
        label_balance=label_balance,
        holdout_ids_hash=holdout_hash,
        created_by=spec.actor_id,
    )
    session.add(snap)
    await session.flush()

    # Members
    members = [
        SnapshotMember(
            snapshot_id=snap.id,
            sample_id=sid,
            partition=assignments.get(sid, "train"),
        )
        for sid in ids
    ]
    session.add_all(members)

    # Audit
    chain = AuditChain(session, spec.tenant_id)
    await chain.record(
        kind="snapshot_materialised",
        actor_id=spec.actor_id,
        subject_id=snap.id,
        payload={
            "task_id": spec.task_id,
            "n_samples": len(ids),
            "label_balance": label_balance,
            "partitioning": (
                None if partitioning is None else {
                    "train": partitioning.train,
                    "val": partitioning.val,
                    "holdout": partitioning.holdout,
                }
            ),
        },
    )
    await session.flush()
    return snap


__all__ = ["MaterialisationSpec", "Partitioning", "materialise"]
