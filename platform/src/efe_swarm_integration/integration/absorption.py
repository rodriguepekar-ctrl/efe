"""``wire_up_platform`` — single entry point that fully integrates
M1 → M11 behind a :class:`SwarmClient`.

Production code instantiates the platform like this :

.. code-block:: python

    from efe_security.audit import InMemoryAuditStore
    from efe_security.authz import PolicyDecisionPoint, RbacRegistry
    from efe_security.pii import PiiClassifier
    from efe_swarm_integration.integration import wire_up_platform

    client, admin, dispatcher = wire_up_platform(
        driver=grpc_driver,
        embedder=sentence_transformers_embedder,
        audit_store=InMemoryAuditStore(),
        pdp=PolicyDecisionPoint(rbac=RbacRegistry()),
        pii_classifier=PiiClassifier(),
    )

The returned triple exposes :

* ``client`` — fully-equipped :class:`SwarmClient` (M12) with M11 PDP +
  PII + audit wired in.
* ``admin`` — :class:`AdminClient` (M12) sharing the same driver.
* ``dispatcher`` — :class:`M10JobDispatcher` pre-populated with
  default handlers that delegate to ``client`` / ``admin`` /
  ``catalog``.

Beyond convenience, this function is the **single point where the
actual cross-module imports happen at runtime** — it's the proof of
absorption.
"""

from __future__ import annotations

from dataclasses import dataclass

from efe_security.audit import AuditStore
from efe_security.authz import PolicyDecisionPoint
from efe_security.pii import PiiClassifier

from efe_swarm_integration.admin.client import AdminClient
from efe_swarm_integration.catalog.bridge import CatalogBridge
from efe_swarm_integration.catalog.store import InMemoryConeBindingStore
from efe_swarm_integration.client import EmbedderService, SwarmClient
from efe_swarm_integration.config import DEFAULTS, SwarmIntegrationConfig
from efe_swarm_integration.flow_control.bulkhead import BulkheadRegistry
from efe_swarm_integration.flow_control.circuit_breaker import (
    CircuitBreakerRegistry,
)
from efe_swarm_integration.integration.adapters_m09 import M09DriftAdapter
from efe_swarm_integration.integration.adapters_m10 import (
    M10JobDispatcher,
    build_default_dispatcher,
)
from efe_swarm_integration.integration.adapters_m11 import (
    M11AuditBridge,
    M11PdpAdapter,
    M11PiiAdapter,
)
from efe_swarm_integration.transport.rpc_driver import RpcDriver

__all__ = ["PlatformWiring", "wire_up_platform"]


@dataclass(slots=True)
class PlatformWiring:
    """The bundle of fully-wired services returned by :func:`wire_up_platform`."""

    client: SwarmClient
    admin: AdminClient
    dispatcher: M10JobDispatcher
    drift_adapter: M09DriftAdapter
    catalog: CatalogBridge
    audit_bridge: M11AuditBridge
    pdp_adapter: M11PdpAdapter
    pii_adapter: M11PiiAdapter


def wire_up_platform(
    *,
    driver: RpcDriver,
    embedder: EmbedderService,
    audit_store: AuditStore,
    pdp: PolicyDecisionPoint,
    pii_classifier: PiiClassifier | None = None,
    config: SwarmIntegrationConfig | None = None,
    catalog_store: InMemoryConeBindingStore | None = None,
) -> PlatformWiring:
    """Build a fully-integrated M12 platform wiring.

    Every argument is a real M1-M11 class (not a Protocol stub). The
    function composes :

    * :class:`M11PdpAdapter` (M11 → M12 PDP port).
    * :class:`M11PiiAdapter` (M11 → M12 PII port).
    * :class:`M11AuditBridge` (M11 → M12 audit sink).
    * :class:`CatalogBridge` (M12 internal, optional store override).
    * :class:`SwarmClient` with the three M11 adapters plugged in.
    * :class:`AdminClient` over the same driver.
    * :class:`M09DriftAdapter` so :class:`DriftResult` from M09 can
      trigger cone relaxation through :class:`AdminClient`.
    * :class:`M10JobDispatcher` with the default production handlers.
    """
    cfg = config or DEFAULTS
    catalog = CatalogBridge(store=catalog_store or InMemoryConeBindingStore())

    pdp_adapter = M11PdpAdapter(pdp=pdp)
    pii_adapter = M11PiiAdapter(
        classifier=pii_classifier if pii_classifier is not None else PiiClassifier()
    )
    audit_bridge = M11AuditBridge(store=audit_store)

    bulkheads = BulkheadRegistry(
        capacity=cfg.max_inflight_per_tenant,
        queue_wait_ms=cfg.max_queue_wait_ms,
    )
    breakers = CircuitBreakerRegistry(
        error_rate_threshold=cfg.circuit_breaker.error_rate_threshold,
        window_s=cfg.circuit_breaker.window_s,
        min_samples=cfg.circuit_breaker.min_samples,
        open_cooldown_s=cfg.circuit_breaker.open_cooldown_s,
    )

    client = SwarmClient(
        driver=driver,
        config=cfg,
        embedder=embedder,
        catalog=catalog,
        bulkheads=bulkheads,
        breakers=breakers,
        pdp=pdp_adapter,
        pii_linter=pii_adapter,
    )
    admin = AdminClient(driver=driver)

    drift_adapter = M09DriftAdapter(
        admin=admin,
        catalog=catalog,
        audit=_build_drift_audit(audit_bridge),
    )

    dispatcher = build_default_dispatcher(
        swarm=client, admin=admin, catalog=catalog
    )

    return PlatformWiring(
        client=client,
        admin=admin,
        dispatcher=dispatcher,
        drift_adapter=drift_adapter,
        catalog=catalog,
        audit_bridge=audit_bridge,
        pdp_adapter=pdp_adapter,
        pii_adapter=pii_adapter,
    )


def _build_drift_audit(bridge: M11AuditBridge):
    async def _emit(event, outcome):
        from efe_security.audit import AuditDraft, AuditOutcome, AuditSeverity

        await bridge.store.append(
            AuditDraft(
                occurred_at=event.observed_at,
                tenant_id=event.tenant_id,
                actor_kind="service",
                actor_id="m09.drift.adapter",
                action=f"swarm.drift.{outcome.action}",
                outcome=(
                    AuditOutcome.success
                    if outcome.action != "error"
                    else AuditOutcome.failure
                ),
                severity=(
                    AuditSeverity.warning
                    if event.severity.value in ("major", "critical")
                    else AuditSeverity.info
                ),
                reasons=(f"severity={event.severity.value}",),
                payload={
                    "result_id": event.result_id,
                    "feature": event.feature_name,
                    "method": event.method,
                    "score": event.score,
                    "new_cone_key": outcome.new_cone_key,
                    "error": outcome.error,
                },
            )
        )

    return _emit
