"""API context — holds references to the collaborators the routes need."""

from __future__ import annotations

from dataclasses import dataclass

from efe_registry.config import RegistrySettings
from efe_registry.contracts.registry import ContractRegistry
from efe_registry.core.registry import Registry


@dataclass
class ApiContext:
    settings: RegistrySettings
    registry: Registry
    contracts: ContractRegistry


__all__ = ["ApiContext"]
