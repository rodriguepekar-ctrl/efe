"""Top-level Registry façade.

Orchestrates the small number of public operations used by the rest
of the platform. Heavy logic lives in dedicated modules; this class
is mostly a thin coordinator.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Literal

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_registry.clock import Clock, SystemClock
from efe_registry.config import RegistrySettings
from efe_registry.core.artifact import Artifact, ArtifactState
from efe_registry.core.promoter import ArtifactPromoter, PromoteResult
from efe_registry.core.state_machine import ArtifactTransitionValidator
from efe_registry.db.models import Artifact as ArtifactRow
from efe_registry.errors import (
    ArtifactNotFound,
    DuplicateArtifact,
    TenantMismatch,
    ValidationError,
)
from efe_registry.rollback.executor import RollbackExecutor
from efe_registry.rollback.receipt import RollbackReceipt
from efe_registry.signing.verifier import validate_oci_ref


@dataclass(frozen=True, slots=True)
class PublishRequest:
    tenant_id: str
    task_id: str
    semver: str
    content_hash: bytes
    oci_ref: str
    s3_url: str
    parent_run_id: str
    framework: Literal["vllm", "peft", "full", "other"]
    size_bytes: int
    parent_decision_id: str | None = None


class Registry:
    """Public façade over the registry sub-systems.

    Usage::

        registry = Registry.from_session(
            session_factory, settings,
            promoter=..., rollback=...,
        )
        artifact = await registry.publish(request)

    The class does not own lifecycle of its collaborators — it holds
    references. This makes dependency wiring explicit and testable.
    """

    def __init__(
        self,
        *,
        session_factory: async_sessionmaker[AsyncSession],
        settings: RegistrySettings,
        promoter: ArtifactPromoter,
        rollback_executor: RollbackExecutor,
        clock: Clock | None = None,
    ) -> None:
        self._session_factory = session_factory
        self._settings = settings
        self._promoter = promoter
        self._rollback = rollback_executor
        self._clock: Clock = clock or SystemClock()

    # -------------------- publish --------------------

    async def publish(self, req: PublishRequest) -> Artifact:
        _validate_publish(req)
        async with self._session_factory() as session:
            existing = await session.execute(
                select(ArtifactRow).where(
                    ArtifactRow.tenant_id == req.tenant_id,
                    ArtifactRow.task_id == req.task_id,
                    ArtifactRow.semver == req.semver,
                ),
            )
            row = existing.scalar_one_or_none()
            if row is not None:
                raise DuplicateArtifact(
                    f"artifact ({req.tenant_id}, {req.task_id}, "
                    f"{req.semver}) already exists",
                )
            # Check content_hash uniqueness too (enforced by unique constraint
            # but raise a nicer error).
            clash = await session.execute(
                select(ArtifactRow).where(
                    ArtifactRow.content_hash == req.content_hash,
                ),
            )
            if clash.scalar_one_or_none() is not None:
                raise DuplicateArtifact(
                    f"content_hash {req.content_hash.hex()[:16]}… "
                    f"already recorded",
                )

            now = self._clock.now()
            row = ArtifactRow(
                artifact_id=str(uuid.uuid4()),
                tenant_id=req.tenant_id,
                task_id=req.task_id,
                semver=req.semver,
                content_hash=req.content_hash,
                oci_ref=req.oci_ref,
                s3_url=req.s3_url,
                parent_run_id=req.parent_run_id,
                parent_decision_id=req.parent_decision_id,
                size_bytes=req.size_bytes,
                framework=req.framework,
                state=ArtifactState.UPLOADED.value,
                state_version=0,
                created_at=now,
                uploaded_at=now,
            )
            session.add(row)
            await session.flush()
            await session.commit()
            return Artifact.from_row(row)

    # -------------------- get --------------------

    async def get(self, artifact_id: str) -> Artifact:
        async with self._session_factory() as session:
            row = await session.get(ArtifactRow, artifact_id)
            if row is None:
                raise ArtifactNotFound(
                    f"artifact {artifact_id!r} not found",
                )
            return Artifact.from_row(row)

    async def get_for_tenant(
        self, artifact_id: str, tenant_id: str,
    ) -> Artifact:
        a = await self.get(artifact_id)
        if a.tenant_id != tenant_id:
            raise TenantMismatch(
                f"artifact {artifact_id!r} belongs to tenant "
                f"{a.tenant_id!r}, not {tenant_id!r}",
            )
        return a

    # -------------------- transition (internal) --------------------

    async def transition(
        self,
        *,
        artifact_id: str,
        from_state: ArtifactState,
        to_state: ArtifactState,
        expected_version: int,
        actor: str,
        reason: str | None = None,
    ) -> Artifact:
        async with self._session_factory() as session:
            validator = ArtifactTransitionValidator(session)
            await validator.apply(
                artifact_id=artifact_id,
                expected_state=from_state,
                new_state=to_state,
                expected_version=expected_version,
                actor=actor,
                reason=reason,
            )
            await session.commit()
            row = await session.get(ArtifactRow, artifact_id)
            assert row is not None
            return Artifact.from_row(row)

    # -------------------- promote --------------------

    async def promote(
        self, *,
        decision_id: str, artifact_id: str, actor: str, reason: str,
    ) -> PromoteResult:
        return await self._promoter.promote(
            decision_id=decision_id,
            artifact_id=artifact_id,
            actor=actor,
            reason=reason,
        )

    # -------------------- rollback --------------------

    async def rollback(
        self, *,
        tenant_id: str, task_id: str,
        actor: str, reason: str,
        trigger_kind: Literal[
            "manual", "auto_slo", "auto_regression", "admin",
        ] = "manual",
        idempotency_key: bytes | None = None,
    ) -> RollbackReceipt:
        return await self._rollback.execute(
            tenant_id=tenant_id, task_id=task_id,
            actor=actor, reason=reason,
            trigger_kind=trigger_kind,
            idempotency_key=idempotency_key,
        )


def _validate_publish(req: PublishRequest) -> None:
    if not req.tenant_id or not req.tenant_id.strip():
        raise ValidationError("tenant_id is required")
    if not req.task_id or not req.task_id.strip():
        raise ValidationError("task_id is required")
    if not req.semver or not req.semver.strip():
        raise ValidationError("semver is required")
    if len(req.content_hash) != 32:
        raise ValidationError(
            f"content_hash must be 32 bytes (SHA-256), got "
            f"{len(req.content_hash)}",
        )
    if req.size_bytes <= 0:
        raise ValidationError("size_bytes must be positive")
    if req.framework not in ("vllm", "peft", "full", "other"):
        raise ValidationError(
            f"framework must be vllm|peft|full|other, got {req.framework!r}",
        )
    # Block OCI refs that would be parsable as cosign flags (C2).
    validate_oci_ref(req.oci_ref)


__all__ = ["PublishRequest", "Registry"]
