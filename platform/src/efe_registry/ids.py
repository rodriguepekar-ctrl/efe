"""Deterministic ID generation — same pattern as :mod:`efe_gate.ids`."""

from __future__ import annotations

import uuid
from typing import Protocol


class IdGen(Protocol):
    def uuid4(self) -> str:  # pragma: no cover
        ...


class SystemIdGen:
    __slots__ = ()

    def uuid4(self) -> str:
        return str(uuid.uuid4())


class SequentialIdGen:
    """Produces stable UUID-looking strings for reproducible tests."""

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


def new_uuid() -> str:
    return _DEFAULT.uuid4()


__all__ = [
    "IdGen",
    "SequentialIdGen",
    "SystemIdGen",
    "new_uuid",
    "reset_id_gen",
    "set_id_gen",
]
