"""Audit export — JSONL stream + optional Ed25519 signature."""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Iterable

from efe_security.audit.records import AuditRecord

__all__ = ["JsonlExporter"]


@dataclass(slots=True)
class JsonlExporter:
    """Stream AuditRecord[] to newline-delimited JSON bytes."""

    def export(self, records: Iterable[AuditRecord]) -> bytes:
        buf = bytearray()
        for r in records:
            buf.extend(
                json.dumps(r.to_dict(), sort_keys=True, separators=(",", ":")).encode()
            )
            buf.extend(b"\n")
        return bytes(buf)
