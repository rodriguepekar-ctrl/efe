"""Append-only JSONL audit log.

Every bot reply, every tool call, and every policy violation is written
here as a single JSON line. The audit log is the **source of truth** for
compliance review and post-hoc debugging — it is deliberately cheap to
produce (one line per event) and impossible to "lose" halfway through
(``fsync`` after each write by default).

Schema
------

Every line is a dict containing:

* ``ts`` — ISO-8601 UTC timestamp,
* ``request_id`` — UUIDv4,
* ``tenant`` — organization label,
* ``kind`` — ``"bot_reply" | "tool_invocation" | "policy_violation" | "error"``,
* ``payload`` — kind-specific dict,
* ``platform_version`` — :data:`efe_platform.VERSION`,
* ``schema_version`` — integer, bumped when breaking changes are made.

The module never loses entries on crashes (short of hardware failure):
writes go through a single file handle opened in append mode, and
``fsync`` is called once per append unless explicitly disabled.
"""

from __future__ import annotations

import hashlib
import json
import os
import threading
from collections.abc import Mapping
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from efe_platform.errors import AuditError
from efe_platform.version import VERSION

AUDIT_SCHEMA_VERSION: int = 1


# ---------------------------------------------------------------------------
#  Entry
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class AuditEntry:
    """One audit event."""

    ts: str
    request_id: str
    tenant: str
    kind: str
    payload: Mapping[str, Any] = field(default_factory=dict)
    platform_version: str = VERSION
    schema_version: int = AUDIT_SCHEMA_VERSION

    def to_jsonl(self) -> str:
        """Encode as a one-line JSON string (no trailing newline)."""
        return json.dumps(asdict(self), ensure_ascii=False, sort_keys=True)

    @classmethod
    def now(
        cls,
        *,
        request_id: str,
        tenant: str,
        kind: str,
        payload: Mapping[str, Any],
    ) -> AuditEntry:
        return cls(
            ts=datetime.now(timezone.utc).isoformat(),
            request_id=request_id,
            tenant=tenant,
            kind=kind,
            payload=dict(payload),
        )


# ---------------------------------------------------------------------------
#  Log writer
# ---------------------------------------------------------------------------


class AuditLog:
    """Thread-safe JSONL audit writer."""

    def __init__(
        self,
        path: str | os.PathLike[str],
        *,
        fsync_each: bool = True,
        redact_prompts: bool = False,
    ) -> None:
        self._path = Path(path)
        self._fsync_each = fsync_each
        self._redact_prompts = redact_prompts
        self._lock = threading.Lock()
        self._path.parent.mkdir(parents=True, exist_ok=True)
        try:
            self._fh = self._path.open("a", encoding="utf-8")
        except OSError as exc:
            raise AuditError(f"cannot open audit log: {exc}") from exc

    # ---------------------------------------------------------- Lifecycle

    def close(self) -> None:
        with self._lock:
            try:
                self._fh.flush()
                self._fh.close()
            except Exception:  # pragma: no cover — defensive
                pass

    def __enter__(self) -> AuditLog:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # ----------------------------------------------------------- Write API

    def append(self, entry: AuditEntry) -> None:
        if self._redact_prompts:
            entry = _redact(entry)
        line = entry.to_jsonl() + "\n"
        with self._lock:
            try:
                self._fh.write(line)
                self._fh.flush()
                if self._fsync_each:
                    os.fsync(self._fh.fileno())
            except OSError as exc:
                raise AuditError(f"audit write failed: {exc}") from exc

    def record_bot_reply(
        self,
        *,
        request_id: str,
        tenant: str,
        reply_payload: Mapping[str, Any],
    ) -> None:
        """Convenience helper for the hottest path."""
        self.append(
            AuditEntry.now(
                request_id=request_id,
                tenant=tenant,
                kind="bot_reply",
                payload=reply_payload,
            )
        )

    def record_error(
        self,
        *,
        request_id: str,
        tenant: str,
        error_payload: Mapping[str, Any],
    ) -> None:
        self.append(
            AuditEntry.now(
                request_id=request_id,
                tenant=tenant,
                kind="error",
                payload=error_payload,
            )
        )

    # ----------------------------------------------------------- Read-back

    def replay(self) -> list[dict[str, Any]]:
        """Return the full log parsed — useful for tests and debug tools.

        Do NOT use this on production-sized logs.
        """
        with self._path.open("r", encoding="utf-8") as fh:
            entries: list[dict[str, Any]] = []
            for line_no, raw in enumerate(fh, start=1):
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    entries.append(json.loads(raw))
                except json.JSONDecodeError as exc:
                    raise AuditError(
                        f"malformed audit line {line_no}: {exc.msg}"
                    ) from exc
            return entries


# ---------------------------------------------------------------------------
#  Redaction
# ---------------------------------------------------------------------------


def _redact(entry: AuditEntry) -> AuditEntry:
    """Return a copy of ``entry`` with user-facing text hashed.

    We hash ``final_text`` and any ``raw_prompt`` fields found inside
    the payload. The hash is ``sha256(text)[:16]`` — short enough for
    logs, unique enough for correlation.
    """
    payload = dict(entry.payload)
    if "final_text" in payload and isinstance(payload["final_text"], str):
        payload["final_text"] = _hash_prefix(payload["final_text"])
    if "raw_prompt" in payload and isinstance(payload["raw_prompt"], str):
        payload["raw_prompt"] = _hash_prefix(payload["raw_prompt"])
    return AuditEntry(
        ts=entry.ts,
        request_id=entry.request_id,
        tenant=entry.tenant,
        kind=entry.kind,
        payload=payload,
        platform_version=entry.platform_version,
        schema_version=entry.schema_version,
    )


def _hash_prefix(text: str) -> str:
    h = hashlib.sha256(text.encode("utf-8")).hexdigest()
    return f"sha256:{h[:16]}"


__all__ = ["AUDIT_SCHEMA_VERSION", "AuditEntry", "AuditLog"]
