"""Contract compatibility — canonical schema hashing + deprecation."""

from __future__ import annotations

from efe_registry.contracts.canonicalizer import canonicalize_schema
from efe_registry.contracts.hasher import contract_hash_from_schemas
from efe_registry.contracts.registry import ContractRegistry
from efe_registry.contracts.sunset import DeprecationSweeper, SweeperAlert

__all__ = [
    "ContractRegistry",
    "DeprecationSweeper",
    "SweeperAlert",
    "canonicalize_schema",
    "contract_hash_from_schemas",
]
