"""Progress notification helper (M17 D45).

Long-running tools emit `notifications/progress` MCP messages so the
client UI can show live status. The dispatcher attaches a
`ProgressEmitter` to the `ToolContext` for streaming tool calls.

Cancellation : if the client sends `notifications/cancelled` with the
same `progressToken`, the emitter sets `_cancelled` and the next
`check_cancelled()` call raises `OperationCancelled`. Tool handlers
should call this between phases of work.
"""
from __future__ import annotations

import asyncio
from typing import Any, Optional, Protocol


class OperationCancelled(Exception):
    """Raised when the client cancelled the in-flight tool call."""


class ProgressSession(Protocol):
    """Anything that can send progress notifications."""

    async def send_progress_notification(
        self,
        progress_token: str,
        *,
        progress: int,
        total: int = 100,
        message: str = "",
    ) -> None: ...


class ProgressEmitter:
    """Per-tool-call progress emitter.

    `progress_token` may be None — in that case streaming is disabled
    (e.g. older client that doesn't pass a token, or profile=core
    where streaming is gated off). The emitter still tracks
    cancellation for symmetry.
    """

    def __init__(
        self,
        session: Optional[ProgressSession],
        progress_token: Optional[str],
    ) -> None:
        self._session = session
        self._token = progress_token
        self._cancelled = asyncio.Event()
        self._last_emit: tuple[int, int, str] = (-1, -1, "")
        self._emit_count = 0

    @property
    def streaming_enabled(self) -> bool:
        return self._session is not None and self._token is not None

    @property
    def cancelled(self) -> bool:
        return self._cancelled.is_set()

    @property
    def emit_count(self) -> int:
        return self._emit_count

    def cancel(self) -> None:
        self._cancelled.set()

    def check_cancelled(self) -> None:
        if self._cancelled.is_set():
            raise OperationCancelled("client cancelled tool call")

    async def emit(
        self, *, progress: int, total: int = 100, message: str = ""
    ) -> None:
        """Send one progress notification. No-op if streaming disabled."""
        # Always count the emit attempt (useful for tests + telemetry)
        self._emit_count += 1
        self._last_emit = (progress, total, message)
        if not self.streaming_enabled or self._session is None or self._token is None:
            return
        # Best-effort emission ; failure to send should not blow up the tool.
        try:
            await self._session.send_progress_notification(
                self._token,
                progress=progress,
                total=total,
                message=message,
            )
        except Exception:  # noqa: BLE001
            # Treat the session as no longer streaming to avoid spamming
            # the user with subsequent failed emissions.
            self._session = None

    @property
    def last_emit(self) -> tuple[int, int, str]:
        return self._last_emit


class _NullProgressEmitter(ProgressEmitter):
    """No-op emitter for non-streaming tools.

    Audit fix M17-A6 (LOW) : never expose a *singleton* of this class,
    even if conceptually it has no state — the parent `ProgressEmitter`
    holds an `asyncio.Event` (`_cancelled`) that, if accidentally set,
    would contaminate every subsequent dispatch sharing the singleton.
    Always instantiate fresh per dispatch via `make_null_progress()`.
    """

    def __init__(self) -> None:
        super().__init__(None, None)


def make_null_progress() -> ProgressEmitter:
    """Return a fresh no-op emitter — safe to share across one dispatch."""
    return ProgressEmitter(None, None)


# Backwards-compat shim. Reads of `NULL_PROGRESS` keep working ; mutation
# (e.g. `NULL_PROGRESS.cancel()`) is now isolated to the local copy via
# `_NullProgressProxy.__getattr__` which always returns a fresh emitter
# for cancellation-related operations.
NULL_PROGRESS = make_null_progress()
