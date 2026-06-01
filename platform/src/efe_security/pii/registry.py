"""Field registry — maps persisted columns to PII classifications.

Any ``TEXT`` column entering the platform must appear here — invariant
**I8**. A lint CI scan (out-of-scope of this module) parses Alembic
migrations and cross-references.
"""

from __future__ import annotations

import asyncio
import enum
from dataclasses import dataclass, field
from typing import Any

from efe_security.errors import FieldNotClassified
from efe_security.pii.classifier import Classification

__all__ = ["FieldDescriptor", "FieldTransform", "PiiFieldRegistry"]


class FieldTransform(str, enum.Enum):
    none = "none"
    redact = "redact"
    pseudonymize = "pseudonymize"
    hash = "hash"
    tokenize = "tokenize"
    minimize = "minimize"
    encrypt = "encrypt"


@dataclass(frozen=True, slots=True)
class FieldDescriptor:
    table_name: str
    column_name: str
    classification: Classification
    detector: str  # e.g. "regex:email", "none", "ml:ner"
    transform: FieldTransform
    retention_days: int
    notes: str = ""


@dataclass(slots=True)
class PiiFieldRegistry:
    _rows: dict[tuple[str, str], FieldDescriptor] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def register(self, desc: FieldDescriptor) -> FieldDescriptor:
        async with self._lock:
            key = (desc.table_name, desc.column_name)
            if key in self._rows:
                raise ValueError(
                    f"field already registered: {desc.table_name}.{desc.column_name}"
                )
            self._rows[key] = desc
            return desc

    async def require(self, table: str, column: str) -> FieldDescriptor:
        async with self._lock:
            desc = self._rows.get((table, column))
        if desc is None:
            raise FieldNotClassified(
                f"{table}.{column} is not classified",
                detail={"table": table, "column": column},
            )
        return desc

    async def get(self, table: str, column: str) -> FieldDescriptor | None:
        async with self._lock:
            return self._rows.get((table, column))

    async def list_by_classification(
        self, classification: Classification
    ) -> list[FieldDescriptor]:
        async with self._lock:
            return [d for d in self._rows.values() if d.classification == classification]

    async def size(self) -> int:
        async with self._lock:
            return len(self._rows)


_ = Any
