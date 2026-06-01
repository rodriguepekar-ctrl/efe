"""Error-taxonomy absorption for M05 / M08 / orch / M04.

These four modules lack a curated public API (``__all__``) — their
domain logic is internal and triggered via NATS events / REST. For
operational purposes, however, **their error classes must be
translatable** into M12 error shapes so that an incident anywhere in
the platform can be surfaced with a consistent ``code`` / ``http_status``.

This module is the single place that imports each of these error
taxonomies, and exposes :func:`translate_to_swarm_error` which maps a
platform-wide :class:`Exception` into an M12 :class:`SwarmIntegrationError`
preserving the semantic equivalence.
"""

from __future__ import annotations

import logging
from typing import Mapping

# Concrete imports across M04 / M05 / M08 / orch — absorption proof.
from efe_ft.errors import FtError
from efe_gate.errors import ArtifactFetchFailed, DecisionError
from efe_orch.errors import OrchError
from efe_worker.errors import DataError, HeartbeatTimeout

from efe_swarm_integration.errors import (
    ArtefactDigestMismatch,
    BackpressureRejected,
    SwarmIntegrationError,
    SwarmUnavailable,
    TimeoutExceeded,
)

_LOG = logging.getLogger("efe_swarm_integration.integration.errors")

__all__ = [
    "ABSORBED_ERROR_TAXONOMY",
    "translate_to_swarm_error",
]


ABSORBED_ERROR_TAXONOMY: Mapping[type, type] = {
    FtError: SwarmIntegrationError,
    DecisionError: SwarmIntegrationError,
    ArtifactFetchFailed: ArtefactDigestMismatch,
    OrchError: BackpressureRejected,
    DataError: SwarmIntegrationError,
    HeartbeatTimeout: TimeoutExceeded,
}


def translate_to_swarm_error(exc: Exception) -> SwarmIntegrationError:
    """Map a platform exception into the M12 taxonomy.

    Unknown exceptions are wrapped in :class:`SwarmIntegrationError`
    with the original type name in the detail — this keeps the audit
    trail actionable without forcing callers to ``isinstance`` every
    platform class.
    """
    if isinstance(exc, SwarmIntegrationError):
        return exc
    target_cls = None
    for cls, swarm_cls in ABSORBED_ERROR_TAXONOMY.items():
        if isinstance(exc, cls):
            target_cls = swarm_cls
            break
    if target_cls is None:
        target_cls = SwarmIntegrationError
    message = str(exc) or type(exc).__name__
    new_exc = target_cls(
        message,
        detail={"source_class": type(exc).__name__},
    )
    _LOG.debug(
        "m12.errors.translate src=%s -> tgt=%s",
        type(exc).__name__,
        target_cls.__name__,
    )
    return new_exc


# Keep imports visible.
_ = SwarmUnavailable
