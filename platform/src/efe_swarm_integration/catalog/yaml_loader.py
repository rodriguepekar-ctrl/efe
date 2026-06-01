"""Load and parse anchor catalog YAML.

Schema (see M12 architecture Annex D) :

    catalog_id: str
    version: int >= 1
    tenant: str
    task: str
    embedder:
      family: str
      model: str
      version: str
    dim: int >= 1
    locale: str
    defaults:
      max_projection_distance: float > 0
    labels:
      - label: str
        description: str
        anchors: list[str]
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from efe_swarm_integration.errors import CatalogCompilationFailed

__all__ = [
    "CatalogYaml",
    "EmbedderYaml",
    "LabelYaml",
    "load_catalog_yaml",
]


@dataclass(frozen=True, slots=True)
class EmbedderYaml:
    family: str
    model: str
    version: str


@dataclass(frozen=True, slots=True)
class LabelYaml:
    label: str
    anchors: tuple[str, ...]
    description: str = ""


@dataclass(frozen=True, slots=True)
class CatalogYaml:
    catalog_id: str
    version: int
    tenant: str
    task: str
    embedder: EmbedderYaml
    dim: int
    labels: tuple[LabelYaml, ...]
    max_projection_distance: float = 0.35
    locale: str = "en-US"

    def __post_init__(self) -> None:
        if not self.catalog_id:
            raise ValueError("catalog_id is required")
        if self.version < 1:
            raise ValueError("version must be >= 1")
        if not self.tenant or not self.task:
            raise ValueError("tenant and task are required")
        if self.dim < 1:
            raise ValueError("dim must be >= 1")
        if self.max_projection_distance <= 0:
            raise ValueError("max_projection_distance must be > 0")
        if not self.labels:
            raise ValueError("catalog requires at least one label")
        # Labels must be distinct.
        seen: set[str] = set()
        for lb in self.labels:
            if lb.label in seen:
                raise ValueError(f"duplicate label: {lb.label}")
            seen.add(lb.label)
            if not lb.anchors:
                raise ValueError(f"label {lb.label} needs ≥ 1 anchor")


def load_catalog_yaml(path: str | Path) -> CatalogYaml:
    """Read the YAML at ``path`` and build a validated :class:`CatalogYaml`."""
    try:
        import yaml  # type: ignore[import-untyped]
    except ImportError as exc:  # pragma: no cover
        raise CatalogCompilationFailed(
            "PyYAML required to load catalog YAML"
        ) from exc

    p = Path(path)
    if not p.exists():
        raise CatalogCompilationFailed(f"catalog YAML not found: {p}")
    try:
        data = yaml.safe_load(p.read_text())
    except yaml.YAMLError as exc:
        raise CatalogCompilationFailed(f"YAML parse error: {exc}") from exc
    if not isinstance(data, dict):
        raise CatalogCompilationFailed("catalog YAML must be a mapping")

    embedder_raw = data.get("embedder") or {}
    emb = EmbedderYaml(
        family=str(embedder_raw.get("family", "")),
        model=str(embedder_raw.get("model", "")),
        version=str(embedder_raw.get("version", "")),
    )
    defaults = data.get("defaults") or {}
    labels_raw = data.get("labels") or []
    labels = tuple(
        LabelYaml(
            label=str(lb["label"]),
            anchors=tuple(str(a) for a in (lb.get("anchors") or [])),
            description=str(lb.get("description", "")),
        )
        for lb in labels_raw
    )
    try:
        return CatalogYaml(
            catalog_id=str(data["catalog_id"]),
            version=int(data.get("version", 1)),
            tenant=str(data["tenant"]),
            task=str(data["task"]),
            embedder=emb,
            dim=int(data["dim"]),
            labels=labels,
            max_projection_distance=float(
                defaults.get("max_projection_distance", 0.35)
            ),
            locale=str(data.get("locale", "en-US")),
        )
    except (KeyError, TypeError, ValueError) as exc:
        raise CatalogCompilationFailed(f"invalid catalog: {exc}") from exc
