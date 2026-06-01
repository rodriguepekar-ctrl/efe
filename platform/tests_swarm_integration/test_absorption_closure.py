"""Closure tests — verify that every M1-M11 module is reachable from M12.

These tests trigger a concrete import chain for each module that lacks
a public ``__all__`` (``efe_ft``, ``efe_gate``, ``efe_orch``,
``efe_worker``) by going through :mod:`adapters_errors` and exercising
:func:`translate_to_swarm_error`.
"""

from __future__ import annotations

import pytest

from efe_ft.errors import FtError
from efe_gate.errors import ArtifactFetchFailed, DecisionError
from efe_orch.errors import OrchError
from efe_worker.errors import HeartbeatTimeout

from efe_swarm_integration.errors import (
    ArtefactDigestMismatch,
    BackpressureRejected,
    SwarmIntegrationError,
    TimeoutExceeded,
)
from efe_swarm_integration.integration import (
    ABSORBED_ERROR_TAXONOMY,
    translate_to_swarm_error,
)


@pytest.mark.parametrize(
    "exc_factory, expected_cls",
    [
        (lambda: FtError("ft broke"), SwarmIntegrationError),
        (lambda: DecisionError("gate decision"), SwarmIntegrationError),
        (lambda: ArtifactFetchFailed("fetch"), ArtefactDigestMismatch),
        (lambda: OrchError("orchestration"), BackpressureRejected),
        (lambda: HeartbeatTimeout("hb"), TimeoutExceeded),
    ],
)
def test_translate_to_swarm_error(exc_factory, expected_cls) -> None:
    """Every M04/M05/M08/orch exception is translatable."""
    swarm_exc = translate_to_swarm_error(exc_factory())
    assert isinstance(swarm_exc, expected_cls)
    assert "source_class" in (swarm_exc.detail or {})


def test_taxonomy_covers_all_expected_modules() -> None:
    """The translation table names classes from the 4 hidden modules."""
    module_names = {
        cls.__module__.split(".")[0]
        for cls in ABSORBED_ERROR_TAXONOMY.keys()
    }
    assert "efe_ft" in module_names
    assert "efe_gate" in module_names
    assert "efe_orch" in module_names
    assert "efe_worker" in module_names


def test_already_swarm_error_passes_through() -> None:
    src = SwarmIntegrationError("keep me")
    assert translate_to_swarm_error(src) is src
