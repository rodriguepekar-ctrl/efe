"""Forced-assignment overrides for QA, founders, debug scenarios.

Each entry pins ``(experiment_id, user_id) → variant_id``. The
:class:`HashAssigner` consults a :class:`ForcedAssignmentStore` at the
top of its routing ladder, so forced assignments take precedence over
holdout, layer allocation, traffic cap, and hash-based variant pick.

The store is intentionally in-memory — production wires a DB-backed
:class:`ForcedAssignmentStore` subclass via Protocol duck-typing.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from typing import Protocol


@dataclass(frozen=True, slots=True)
class ForcedAssignment:
    experiment_id: str
    user_id: str
    variant_id: str
    reason: str
    created_by: str


class ForcedAssignmentStore(Protocol):
    async def lookup(
        self, *, experiment_id: str, user_id: str,
    ) -> ForcedAssignment | None:  # pragma: no cover
        ...

    async def put(
        self, forced: ForcedAssignment,
    ) -> None:  # pragma: no cover
        ...

    async def remove(
        self, *, experiment_id: str, user_id: str,
    ) -> None:  # pragma: no cover
        ...

    async def list(
        self, experiment_id: str,
    ) -> tuple[ForcedAssignment, ...]:  # pragma: no cover
        ...


class InMemoryForcedAssignmentStore:
    """In-memory implementation for tests + single-process deployments."""

    __slots__ = ("_by_exp",)

    def __init__(self, seed: Iterable[ForcedAssignment] = ()) -> None:
        self._by_exp: dict[str, dict[str, ForcedAssignment]] = {}
        for f in seed:
            self._by_exp.setdefault(f.experiment_id, {})[f.user_id] = f

    async def lookup(
        self, *, experiment_id: str, user_id: str,
    ) -> ForcedAssignment | None:
        per_exp = self._by_exp.get(experiment_id)
        if per_exp is None:
            return None
        return per_exp.get(user_id)

    async def put(self, forced: ForcedAssignment) -> None:
        self._by_exp.setdefault(forced.experiment_id, {})[forced.user_id] = forced

    async def remove(
        self, *, experiment_id: str, user_id: str,
    ) -> None:
        per_exp = self._by_exp.get(experiment_id)
        if per_exp is None:
            return
        per_exp.pop(user_id, None)

    async def list(
        self, experiment_id: str,
    ) -> tuple[ForcedAssignment, ...]:
        per_exp = self._by_exp.get(experiment_id, {})
        return tuple(per_exp.values())


__all__ = [
    "ForcedAssignment",
    "ForcedAssignmentStore",
    "InMemoryForcedAssignmentStore",
]
