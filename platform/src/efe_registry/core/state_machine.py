"""Artifact FSM validator.

Mirror of :mod:`efe_gate.state_machine.transitions` for artifact
lifecycle (architecture doc §5.1). The code enforces **14 allowed
transitions** — the architecture document's 13-row table in §5.1 is
missing the ``DEPRECATED -> SERVING`` entry used by the rollback
revival path; the code of record is this whitelist.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Final

from sqlalchemy import update
from sqlalchemy.ext.asyncio import AsyncSession

from efe_registry.core.artifact import ArtifactState
from efe_registry.db.models import (
    Artifact as ArtifactRow,
)
from efe_registry.db.models import (
    ArtifactStateTransition,
)
from efe_registry.errors import (
    ArtifactImmutable,
    ArtifactNotFound,
    InvalidArtifactTransition,
)

# (from_state, to_state) — whitelist of allowed transitions.
ARTIFACT_TRANSITIONS: Final[frozenset[tuple[ArtifactState, ArtifactState]]] = (
    frozenset({
        (ArtifactState.DRAFT, ArtifactState.UPLOADED),
        (ArtifactState.UPLOADED, ArtifactState.SIGNED),
        (ArtifactState.UPLOADED, ArtifactState.QUARANTINED),
        (ArtifactState.SIGNED, ArtifactState.ADMITTED),
        (ArtifactState.SIGNED, ArtifactState.REJECTED),
        (ArtifactState.ADMITTED, ArtifactState.SERVING),
        (ArtifactState.SERVING, ArtifactState.CHAMPION),
        (ArtifactState.CHAMPION, ArtifactState.SERVING),  # rollback
        (ArtifactState.CHAMPION, ArtifactState.DEPRECATED),
        (ArtifactState.SERVING, ArtifactState.DEPRECATED),
        (ArtifactState.DEPRECATED, ArtifactState.SERVING),  # rollback revival
        (ArtifactState.DEPRECATED, ArtifactState.PURGED),
        (ArtifactState.QUARANTINED, ArtifactState.DEPRECATED),
        (ArtifactState.REJECTED, ArtifactState.DEPRECATED),
    })
)


def is_valid_transition(frm: ArtifactState, to: ArtifactState) -> bool:
    return (frm, to) in ARTIFACT_TRANSITIONS


@dataclass(frozen=True, slots=True)
class TransitionResult:
    artifact_id: str
    from_state: ArtifactState
    to_state: ArtifactState
    new_version: int


class ArtifactTransitionValidator:
    """CAS-driven transition helper.

    Uses optimistic locking on ``state_version`` so two concurrent
    promoters cannot both advance the same artifact past a gate
    without one of them observing a version conflict.
    """

    def __init__(self, session: AsyncSession) -> None:
        self._s = session

    async def apply(
        self,
        *,
        artifact_id: str,
        expected_state: ArtifactState,
        new_state: ArtifactState,
        expected_version: int,
        actor: str,
        reason: str | None = None,
        nats_msg_id: str | None = None,
    ) -> TransitionResult:
        if not is_valid_transition(expected_state, new_state):
            raise InvalidArtifactTransition(
                f"{expected_state.value} -> {new_state.value} "
                f"is not in the ARTIFACT_TRANSITIONS whitelist",
            )
        new_version = expected_version + 1

        # CAS update — only advances if state + version match.
        stmt = (
            update(ArtifactRow)
            .where(
                ArtifactRow.artifact_id == artifact_id,
                ArtifactRow.state == expected_state.value,
                ArtifactRow.state_version == expected_version,
            )
            .values(state=new_state.value, state_version=new_version)
        )
        result = await self._s.execute(stmt)
        # SQLAlchemy Result exposes rowcount; type stubs are loose on it.
        if getattr(result, "rowcount", 0) == 0:
            # Either not found, or stale state/version — disambiguate.
            row = await self._s.get(ArtifactRow, artifact_id)
            if row is None:
                raise ArtifactNotFound(
                    f"artifact {artifact_id!r} does not exist",
                )
            if row.state != expected_state.value:
                raise InvalidArtifactTransition(
                    f"expected state {expected_state.value!r}, "
                    f"found {row.state!r}",
                )
            raise ArtifactImmutable(
                f"state version conflict on artifact {artifact_id!r} "
                f"(expected {expected_version}, found {row.state_version})",
            )

        # Journal the transition.
        self._s.add(ArtifactStateTransition(
            transition_id=str(uuid.uuid4()),
            artifact_id=artifact_id,
            from_state=expected_state.value,
            to_state=new_state.value,
            old_version=expected_version,
            new_version=new_version,
            actor=actor,
            reason=reason,
            nats_msg_id=nats_msg_id,
        ))
        await self._s.flush()
        return TransitionResult(
            artifact_id=artifact_id,
            from_state=expected_state,
            to_state=new_state,
            new_version=new_version,
        )


__all__ = [
    "ARTIFACT_TRANSITIONS",
    "ArtifactTransitionValidator",
    "TransitionResult",
    "is_valid_transition",
]
