"""Test-support helpers for Letter D.

Provides :class:`FakeEfeFacade`, a drop-in replacement for
:class:`EfeFacade` that scripts verdicts without hitting gRPC or
loading sentence-transformers. Plus pytest fixtures.

Usage (pytest)::

    from efe_tool.integrations.testing import FakeEfeFacade

    def test_route(client):
        fake = FakeEfeFacade.with_verdict(label="fraud", confidence=0.9)
        # inject via the framework adapter (see per-framework docs)
        ...
"""

from __future__ import annotations

import asyncio
import threading
from collections import deque
from dataclasses import dataclass, field
from typing import Any, Deque, Mapping, Optional

from ._errors import (
    EfeFacadeClosedError,
    EfeFacadeError,
    EfeFacadeRejectedError,
    EfeFacadeUsageError,
)
from ._types import FacadeStats, HealthStatus, Verdict


# ---------------------------------------------------------------------------
#  Verdict factory
# ---------------------------------------------------------------------------


def make_verdict(
    *,
    label: str = "alpha",
    confidence: float = 0.85,
    reject: bool = False,
    rounds: int = 4,
    alignment_delta: float = 0.1,
    catalog: str = "support_triage",
    zone: Optional[str] = None,
    elapsed_ms: int = 42,
    request_id: Optional[str] = None,
) -> Verdict:
    """Build a canonical :class:`Verdict` for tests.

    ``zone`` defaults to ``"trusted"`` for confidence ≥ 0.80,
    ``"warn"`` for 0.50 ≤ confidence < 0.80, else ``"reject"``.
    """
    if zone is None:
        if reject:
            zone = "reject"
        elif confidence >= 0.80:
            zone = "trusted"
        elif confidence >= 0.50:
            zone = "warn"
        else:
            zone = "reject"
    return Verdict(
        label=label, confidence=confidence, reject=reject,
        rounds=rounds, alignment_delta=alignment_delta,
        catalog=catalog, zone=zone, elapsed_ms=elapsed_ms,
        request_id=request_id,
    )


# ---------------------------------------------------------------------------
#  FakeEfeFacade
# ---------------------------------------------------------------------------


@dataclass
class FakeEfeFacade:
    """A drop-in for :class:`EfeFacade` that never hits gRPC.

    Attributes
    ----------
    default_verdict:
        Returned when ``responses`` is empty.
    responses:
        FIFO queues keyed by catalog name. Pop on each ``classify``.
    raise_on_classify:
        When set, every call raises this exception.
    """

    default_verdict: Verdict = field(
        default_factory=lambda: make_verdict(),
    )
    responses: dict[str, Deque[Any]] = field(default_factory=dict)
    raise_on_classify: Optional[BaseException] = None
    is_ready: bool = True
    is_closed: bool = False
    _lock: threading.Lock = field(default_factory=threading.Lock, repr=False)
    _calls: list[dict[str, Any]] = field(default_factory=list, repr=False)
    _stats: FacadeStats = field(default_factory=FacadeStats, repr=False)

    # ------------------------------------------------------------------
    #  Factories
    # ------------------------------------------------------------------

    @classmethod
    def with_verdict(cls, **kwargs: Any) -> "FakeEfeFacade":
        """Build a fake whose :meth:`classify` always returns the given
        verdict (via ``kwargs`` forwarded to :func:`make_verdict`)."""
        return cls(default_verdict=make_verdict(**kwargs))

    @classmethod
    def with_responses(
        cls, responses: Mapping[str, list[Verdict | Exception]],
    ) -> "FakeEfeFacade":
        """Build a fake that pops one response per call per catalog."""
        r: dict[str, Deque[Any]] = {
            k: deque(v) for k, v in responses.items()
        }
        return cls(responses=r)

    # ------------------------------------------------------------------
    #  Lifecycle (no-ops on success path)
    # ------------------------------------------------------------------

    def preload(self) -> None:
        self.is_ready = True
        self._stats.is_ready = True

    def close(self, *, timeout_s: float = 30.0) -> None:
        self.is_closed = True
        self.is_ready = False
        self._stats.is_closed = True
        self._stats.is_ready = False

    def healthcheck(self) -> HealthStatus:
        if self.is_closed:
            return HealthStatus(
                status="NOT_SERVING", latency_ms=0,
                checks={"closed": True}, reason="facade closed",
            )
        return HealthStatus(
            status="SERVING" if self.is_ready else "NOT_READY",
            latency_ms=1,
            checks={"embedder_loaded": True, "grpc_reachable": True,
                    "catalogs_loaded": True, "is_ready": self.is_ready},
        )

    def reload_catalog(self, name: str) -> None:
        # No-op in tests; record for assertion if needed.
        self._calls.append({"op": "reload_catalog", "name": name})

    # ------------------------------------------------------------------
    #  Hot path
    # ------------------------------------------------------------------

    def classify(
        self,
        text: str,
        *,
        catalog: str,
        timeout_s: Optional[float] = None,
        request_id: Optional[str] = None,
        strict: bool = False,
    ) -> Verdict:
        self._check_open()
        if not isinstance(text, str) or not text.strip():
            raise EfeFacadeUsageError("text must be a non-empty string")
        if self.raise_on_classify is not None:
            exc = self.raise_on_classify
            if isinstance(exc, EfeFacadeError):
                raise exc
            raise EfeFacadeError(str(exc), cause=exc) from exc
        with self._lock:
            self._calls.append({
                "op": "classify",
                "text": text,
                "catalog": catalog,
                "timeout_s": timeout_s,
                "request_id": request_id,
                "strict": strict,
            })
            queue = self.responses.get(catalog)
            if queue:
                item = queue.popleft()
                if isinstance(item, BaseException):
                    if isinstance(item, EfeFacadeError):
                        raise item
                    raise EfeFacadeError(str(item), cause=item) from item
                verdict = item if isinstance(item, Verdict) else self.default_verdict
            else:
                verdict = self.default_verdict
            # Override the catalog field so callers can assert on it.
            verdict = Verdict(
                label=verdict.label,
                confidence=verdict.confidence,
                reject=verdict.reject,
                rounds=verdict.rounds,
                alignment_delta=verdict.alignment_delta,
                catalog=catalog,
                zone=verdict.zone,
                elapsed_ms=verdict.elapsed_ms,
                request_id=request_id or verdict.request_id,
            )
            self._stats.classify_total += 1
            if verdict.is_reject:
                self._stats.classify_rejected += 1
        if strict and verdict.is_reject:
            raise EfeFacadeRejectedError(
                f"classification rejected (strict=True): confidence="
                f"{verdict.confidence:.3f}",
                context=verdict.as_dict(),
            )
        return verdict

    async def classify_async(
        self,
        text: str,
        *,
        catalog: str,
        timeout_s: Optional[float] = None,
        request_id: Optional[str] = None,
        strict: bool = False,
    ) -> Verdict:
        await asyncio.sleep(0)
        return self.classify(
            text, catalog=catalog, timeout_s=timeout_s,
            request_id=request_id, strict=strict,
        )

    # ------------------------------------------------------------------
    #  Introspection
    # ------------------------------------------------------------------

    @property
    def catalogs(self) -> list[str]:
        return list(self.responses.keys())

    def stats(self) -> FacadeStats:
        return self._stats

    @property
    def calls(self) -> list[dict[str, Any]]:
        """All recorded method invocations, in order."""
        return list(self._calls)

    @property
    def classify_calls(self) -> list[dict[str, Any]]:
        return [c for c in self._calls if c["op"] == "classify"]

    # ------------------------------------------------------------------
    #  Internals
    # ------------------------------------------------------------------

    def _check_open(self) -> None:
        if self.is_closed:
            raise EfeFacadeClosedError("FakeEfeFacade is closed")


# ---------------------------------------------------------------------------
#  Pytest fixtures (importable from a conftest.py if pytest is present)
# ---------------------------------------------------------------------------


def efe_facade_fixture_factory():  # pragma: no cover - glue
    """Return a pytest fixture lambda producing a fresh FakeEfeFacade.

    Import this in your conftest.py::

        from efe_tool.integrations.testing import efe_facade_fixture_factory
        efe_facade = efe_facade_fixture_factory()
    """
    import pytest

    @pytest.fixture
    def _efe() -> FakeEfeFacade:
        return FakeEfeFacade()
    return _efe


__all__ = [
    "FakeEfeFacade",
    "make_verdict",
    "efe_facade_fixture_factory",
]
