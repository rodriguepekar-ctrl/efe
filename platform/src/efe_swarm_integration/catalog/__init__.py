"""Anchor catalog bridge : Feature / baseline ↔ ConeRef."""

from __future__ import annotations

from efe_swarm_integration.catalog.bridge import CatalogBridge, CatalogBinding
from efe_swarm_integration.catalog.compiler import CatalogCompiler, EmbedderService
from efe_swarm_integration.catalog.store import (
    ConeBindingStore,
    InMemoryConeBindingStore,
)
from efe_swarm_integration.catalog.validator import (
    CatalogValidationError,
    validate_catalog_yaml,
)
from efe_swarm_integration.catalog.yaml_loader import (
    CatalogYaml,
    LabelYaml,
    load_catalog_yaml,
)

__all__ = [
    "CatalogBinding",
    "CatalogBridge",
    "CatalogCompiler",
    "CatalogValidationError",
    "CatalogYaml",
    "ConeBindingStore",
    "EmbedderService",
    "InMemoryConeBindingStore",
    "LabelYaml",
    "load_catalog_yaml",
    "validate_catalog_yaml",
]
