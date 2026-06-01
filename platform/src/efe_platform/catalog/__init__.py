"""Catalog of tasks + anchors loaded from YAML.

A *catalog* in the platform sense is a small YAML file describing:

* a list of **tasks** — each task has an identifier, a short
  description, and a reference to an alignment cone (registered
  upstream via :class:`efe_platform.swarm.ConeRegistryClient`),
* a set of **anchors** — inline anchor vectors that may be imported
  into new cones,
* optional **metadata** (owner, created_at, tags).

The catalog is consumed at startup by the HTTP API to advertise
available routes. It is intentionally flat: no hierarchies, no
inheritance. If you need those, compose catalogs at the caller level.
"""

from __future__ import annotations

from efe_platform.catalog.loader import CatalogLoader, load_catalog
from efe_platform.catalog.models import Catalog, CatalogTask, InlineAnchor

__all__ = [
    "Catalog",
    "CatalogLoader",
    "CatalogTask",
    "InlineAnchor",
    "load_catalog",
]
