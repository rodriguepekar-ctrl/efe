"""Stable ID generators used across the orchestrator.

We use UUID4 everywhere (collision probability negligible at our
scale) but wrap generation so that tests can inject a deterministic
generator via :class:`SequentialIdGen`. This removes UUID flakiness
from fixtures that want a known ordering.

Three abstractions:

* :func:`new_run_id`, :func:`new_worker_id`, :func:`new_hb_id` — the
  public helpers.
* :class:`IdGen` — protocol implemented by system + fake.
* :data:`DEFAULT` — module-level singleton used by the helpers.

Tests call :func:`set_id_gen` to swap it out.
"""

from __future__ import annotations

import uuid
from typing import Protocol


class IdGen(Protocol):
    def uuid4(self) -> str:  # pragma: no cover — protocol
        ...


class SystemIdGen:
    __slots__ = ()

    def uuid4(self) -> str:
        return str(uuid.uuid4())


class SequentialIdGen:
    """Yield deterministic UUID-shaped strings for tests.

    The format is ``00000000-0000-0000-0000-<12-digit-counter>`` —
    still a valid UUID, still 36 chars, but predictable and stable.
    """

    __slots__ = ("_n",)

    def __init__(self, start: int = 0) -> None:
        self._n = int(start)

    def uuid4(self) -> str:
        self._n += 1
        return f"00000000-0000-0000-0000-{self._n:012d}"


_DEFAULT: IdGen = SystemIdGen()


def set_id_gen(gen: IdGen) -> None:
    global _DEFAULT
    _DEFAULT = gen


def reset_id_gen() -> None:
    global _DEFAULT
    _DEFAULT = SystemIdGen()


def new_run_id() -> str:
    return _DEFAULT.uuid4()


def new_worker_id() -> str:
    return _DEFAULT.uuid4()


def new_hb_id() -> str:
    return _DEFAULT.uuid4()


def new_uuid() -> str:
    return _DEFAULT.uuid4()


__all__ = [
    "IdGen",
    "SequentialIdGen",
    "SystemIdGen",
    "new_hb_id",
    "new_run_id",
    "new_uuid",
    "new_worker_id",
    "reset_id_gen",
    "set_id_gen",
]
