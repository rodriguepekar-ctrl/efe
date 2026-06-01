"""Sprint D12 — admin client types for catalog operations.

Public types exported via :mod:`efe_tool.admin`:

  * :class:`RegisteredCatalog` — full canonical catalog returned
    by ``RegisterCatalog`` / ``EvolveCatalog`` / ``GetCatalog``.
  * :class:`CatalogAuditMetadata` — register/evolve audit envelope.
  * :class:`CompiledClassRef` — server-side compiled anchors metadata.
  * :class:`EvolveOp` — discriminated enum of the 5 evolution operations.

All dataclasses are ``frozen=True`` so instances are safe to share
across threads.
"""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any


class EvolveOp(enum.Enum):
    """Discriminated union tag for :class:`EfeAdminClient.evolve_catalog`.

    Each variant maps 1:1 to a oneof variant in
    ``CatalogEvolutionOp`` (cf. ``protos/efe/v1/catalog.proto`` §
    8.1).
    """

    APPEND_CLASS = "append_class"
    REMOVE_CLASS = "remove_class"
    UPDATE_THRESHOLDS = "update_thresholds"
    REPLACE_CLASS_ANCHORS = "replace_class_anchors"
    RECOMPILE = "recompile"


@dataclass(frozen=True)
class CompiledClassRef:
    """Lightweight reference to a server-side compiled class.

    Instances do NOT carry the actual anchor floats — call
    :meth:`EfeAdminClient.get_catalog` with ``include_compiled=True``
    if the bytes are needed (typically only the server itself does
    this for in-memory cache rehydration).
    """

    class_name: str
    num_anchors: int
    embedder_fingerprint: str


@dataclass(frozen=True)
class CatalogAuditMetadata:
    """Audit envelope stamped at register/evolve time.

    Attributes:
        created_at: server-side timestamp of the mutation.
        created_by: ``sub`` claim of the JWT that triggered the mutation.
        description: free-text operator-supplied note.
        parent_key: canonical key of the previous version
            (empty for v1).
        evolution_op: ``"register"`` for v1 ; ``"append_class"`` /
            ``"remove_class"`` / etc. for evolutions.
        labels: free-form key-value pairs.
        content_sha256: hex digest of the canonical content —
            forensic witness against mid-flight tampering.
    """

    created_at: datetime
    created_by: str
    description: str
    parent_key: str
    evolution_op: str
    labels: dict[str, str] = field(default_factory=dict)
    content_sha256: str = ""


@dataclass(frozen=True)
class RegisteredCatalog:
    """Server-side catalog version + audit metadata.

    Returned by:

      * :meth:`EfeAdminClient.register_catalog`
      * :meth:`EfeAdminClient.evolve_catalog`
      * :meth:`EfeAdminClient.get_catalog`

    The ``content`` field carries the YAML-parseable shape (matches
    :class:`efe_tool.AnchorCatalog`'s YAML schema). The ``compiled``
    field is populated only when ``include_compiled=True`` was set on
    the get call.

    Attributes:
        key: canonical key ``"tenant/name@vN"``.
        tenant: owning tenant.
        name: catalog name.
        version: integer version (monotonic per ``(tenant, name)``).
        content: full YAML-parseable catalog body
            (``embedder``, ``classes``, ``cone_policy``, ``thresholds``).
        metadata: register/evolve audit metadata.
        compiled: optional per-class compiled anchors metadata.
    """

    key: str
    tenant: str
    name: str
    version: int
    content: dict[str, Any]
    metadata: CatalogAuditMetadata
    compiled: list[CompiledClassRef] | None = None
