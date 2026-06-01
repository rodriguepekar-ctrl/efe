"""EFE Model Registry & Serving.

Public façade for ``efe_registry``. Contains:

* :class:`RegistrySettings` — top-level configuration bundle.
* :class:`Registry` — thin orchestration façade over core services.
* :class:`RollbackExecutor` — the < 60 s p99 rollback engine.
* :class:`TenantRouter` — the hot-path request dispatcher.

Typical use from ops code::

    from efe_registry import Registry, RegistrySettings

    settings = RegistrySettings.from_env()
    registry = Registry.from_settings(settings)
    receipt = await registry.rollback(
        tenant_id="acme", task_id="summarize",
        actor="ops:jdoe", reason="latency regression",
        trigger_kind="manual",
    )
    assert receipt.latency_ms < 60_000

The package is deliberately small at the surface: heavy lifting lives
in the sub-packages (``signing``, ``router``, ``canary``, ``rollback``,
``serving``, ``contracts``).
"""

from __future__ import annotations

from efe_registry.config import (
    AdmissionSettings,
    CanarySettings,
    DatabaseSettings,
    RegistrySettings,
    RollbackSettings,
    RouterSettings,
    ServingSettings,
    SigningSettings,
)
from efe_registry.core.artifact import Artifact, ArtifactRole, ArtifactState
from efe_registry.core.registry import Registry
from efe_registry.rollback.executor import RollbackExecutor
from efe_registry.rollback.receipt import RollbackReceipt
from efe_registry.router.tenant_router import RouteDecision, TenantRouter
from efe_registry.version import __version__

__all__ = [
    "AdmissionSettings",
    "Artifact",
    "ArtifactRole",
    "ArtifactState",
    "CanarySettings",
    "DatabaseSettings",
    "Registry",
    "RegistrySettings",
    "RollbackExecutor",
    "RollbackReceipt",
    "RollbackSettings",
    "RouteDecision",
    "RouterSettings",
    "ServingSettings",
    "SigningSettings",
    "TenantRouter",
    "__version__",
]
