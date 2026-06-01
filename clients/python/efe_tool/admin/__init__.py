"""EFE Admin operations — multi-tenant SDK surface.

This sub-package wraps the gRPC admin services for catalog and cone
management. Every method requires a JWT carrying the appropriate
scopes (see ``docs/bench/d11_per_tenant_isolation_architecture.md``
§ 7 + ``docs/bench/d12_catalog_admin_versioning_architecture.md`` § 6):

  * ``admin:cone:read``     — ``list_cone_versions``, ``get_cone``.
  * ``admin:cone:write``    — ``delete_cone``.
  * ``admin:catalog:read``  — ``list_catalogs``, ``list_catalog_versions``,
                              ``get_catalog``.
  * ``admin:catalog:write`` — ``register_catalog``, ``evolve_catalog``,
                              ``delete_catalog``.

The server enforces tenant scoping: every list / get / mutate
operation is filtered by the caller's ``tenant_id`` claim. Cross-
tenant access is impossible by construction.

Public API:

    EfeAdminClient        — high-level admin client (sync)
    CatalogRef            — lightweight catalog descriptor
    AdminError            — error type (subclass of EfeError)

    Sprint D12 (v0.1.12):
        EvolveOp              — discriminated tag for `evolve_catalog`
        RegisteredCatalog     — full canonical catalog returned
                                by `register_catalog` / `evolve_catalog`
                                / `get_catalog`
        CatalogAuditMetadata  — audit envelope on every register/evolve
        CompiledClassRef      — server-side compiled anchors metadata

Stable post-v0.1.12. Scoped to admin scenarios — application code
should keep using :class:`efe_tool.EfeClient` for ``classify``.
"""

from __future__ import annotations

from .client import (
    AdminError,
    CatalogRef,
    EfeAdminClient,
)
from .types import (
    CatalogAuditMetadata,
    CompiledClassRef,
    EvolveOp,
    RegisteredCatalog,
)

__all__ = [
    # D11
    "AdminError",
    "CatalogRef",
    "EfeAdminClient",
    # D12 — catalog admin + versioning
    "CatalogAuditMetadata",
    "CompiledClassRef",
    "EvolveOp",
    "RegisteredCatalog",
]
