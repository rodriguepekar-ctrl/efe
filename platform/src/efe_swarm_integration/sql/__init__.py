"""Postgres DDL for M12."""

from __future__ import annotations

from efe_swarm_integration.sql.ddl import (
    FULL_DDL,
    SWARM_ARTEFACT_MANIFESTS_DDL,
    SWARM_CALL_LOG_DDL,
    SWARM_CONE_BINDINGS_DDL,
)

__all__ = [
    "FULL_DDL",
    "SWARM_ARTEFACT_MANIFESTS_DDL",
    "SWARM_CALL_LOG_DDL",
    "SWARM_CONE_BINDINGS_DDL",
]
