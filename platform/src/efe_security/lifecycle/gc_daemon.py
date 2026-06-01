"""Garbage-collection daemon for retention enforcement."""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any, Callable

from efe_security.lifecycle.retention import Category, RetentionPolicy

_LOG = logging.getLogger("efe_security.lifecycle.gc")

__all__ = ["GcDaemon", "GcOutcome"]


@dataclass(frozen=True, slots=True)
class GcOutcome:
    """Summary of one GC run.

    * ``purged`` — categories that completed successfully, with count.
    * ``skipped_holds`` — categories whose purge raised an exception
      (legal hold, storage unavailable, etc.) — count is the number of
      *eligible* rows that were not purged.
    * ``errors`` — one entry per failing category, with the underlying
      exception class name ; useful for alerting.
    """

    purged: dict[Category, int] = field(default_factory=dict)
    skipped_holds: dict[Category, int] = field(default_factory=dict)
    errors: dict[Category, str] = field(default_factory=dict)


ScannerFn = Callable[[Category, datetime], int]
PurgeFn = Callable[[Category, datetime], int]


@dataclass(slots=True)
class GcDaemon:
    """Invokes per-category purge callbacks for records older than their
    retention window.

    Callers provide two callbacks :

    * ``scanner(category, threshold)`` — count of eligible rows.
    * ``purger(category, threshold)`` — execute ; returns the count of
      effectively purged rows.

    This split keeps the daemon storage-agnostic.

    Any exception raised by ``purger`` is **logged at ERROR level** and
    recorded in :class:`GcOutcome.errors`. The exception is NOT
    re-raised so one failing category does not block the rest.
    """

    policies: dict[Category, RetentionPolicy]
    scanner: ScannerFn
    purger: PurgeFn

    async def run_once(self, *, now: datetime) -> GcOutcome:
        purged: dict[Category, int] = {}
        skipped: dict[Category, int] = {}
        errors: dict[Category, str] = {}
        for cat, policy in self.policies.items():
            threshold = now - timedelta(days=policy.retention_days)
            if policy.retention_days == 0:
                # prompts_blob → purge immediately.
                threshold = now
            try:
                count = self.scanner(cat, threshold)
            except Exception as exc:  # noqa: BLE001 - never let a scanner kill the daemon
                _LOG.exception(
                    "gc.scanner_failed category=%s err=%s",
                    cat.value,
                    exc,
                )
                errors[cat] = type(exc).__name__
                continue
            if count <= 0:
                continue
            try:
                purged[cat] = self.purger(cat, threshold)
                _LOG.info(
                    "gc.purged category=%s count=%d threshold=%s",
                    cat.value,
                    purged[cat],
                    threshold.isoformat(),
                )
            except Exception as exc:  # noqa: BLE001
                _LOG.exception(
                    "gc.purge_failed category=%s eligible=%d err=%s",
                    cat.value,
                    count,
                    exc,
                )
                skipped[cat] = count
                errors[cat] = type(exc).__name__
        return GcOutcome(purged=purged, skipped_holds=skipped, errors=errors)


_ = Any  # reserved for future typed hooks
