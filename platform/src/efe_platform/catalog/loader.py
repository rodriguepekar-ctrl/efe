"""YAML loader for the platform catalog.

Accepted YAML shape::

    version: 1
    metadata:
      owner: "platform-team"
      created_at: 2026-04-19
      tags: ["mvp"]
    anchors:
      - id: billing_q1
        values: [0.01, 0.02, 0.03, ...]
    tasks:
      - id: ticket_routing
        description: "Route incoming support tickets to the right queue."
        tenant: acme
        cone_key: "acme/ticket_routing@v2"
        dim_latent: 384
        anchors: ["billing_q1"]
        tags: ["prod"]

Unknown fields are rejected (strict schema) to avoid silent typos.
"""

from __future__ import annotations

from collections.abc import Mapping
from datetime import date, datetime
from pathlib import Path
from typing import Any

import yaml

from efe_platform.catalog.models import (
    Catalog,
    CatalogMetadata,
    CatalogTask,
    InlineAnchor,
)
from efe_platform.errors import ConfigError

_ALLOWED_TOP_KEYS: frozenset[str] = frozenset({"version", "metadata", "anchors", "tasks"})
_ALLOWED_TASK_KEYS: frozenset[str] = frozenset(
    {
        "id",
        "description",
        "tenant",
        "cone_key",
        "dim_latent",
        "anchors",
        "tags",
    }
)
_ALLOWED_METADATA_KEYS: frozenset[str] = frozenset({"owner", "created_at", "tags"})
_ALLOWED_ANCHOR_KEYS: frozenset[str] = frozenset({"id", "values"})


def load_catalog(path: str | Path) -> Catalog:
    """Parse the YAML at ``path`` into a validated :class:`Catalog`."""
    return CatalogLoader().load(path)


class CatalogLoader:
    """Reusable loader — carries no state, but makes DI in tests easier."""

    def load(self, path: str | Path) -> Catalog:
        p = Path(path)
        if not p.is_file():
            raise ConfigError(f"catalog not found: {p}")
        try:
            with p.open("r", encoding="utf-8") as fh:
                raw = yaml.safe_load(fh) or {}
        except yaml.YAMLError as exc:
            raise ConfigError(f"invalid YAML in {p}: {exc}") from exc
        if not isinstance(raw, Mapping):
            raise ConfigError(f"catalog root must be a mapping, got {type(raw).__name__}")
        self._reject_unknown(raw, _ALLOWED_TOP_KEYS, path="")
        version = _need_int(raw, "version")
        meta_block = raw.get("metadata") or {}
        if not isinstance(meta_block, Mapping):
            raise ConfigError("metadata must be a mapping")
        self._reject_unknown(meta_block, _ALLOWED_METADATA_KEYS, path="metadata")

        metadata = CatalogMetadata(
            owner=str(meta_block.get("owner", "")),
            created_at=_coerce_date(meta_block.get("created_at")),
            tags=tuple(_list_of_str(meta_block.get("tags", []), "metadata.tags")),
        )
        anchors = tuple(self._parse_anchors(raw.get("anchors") or []))
        tasks = tuple(self._parse_tasks(raw.get("tasks") or []))
        try:
            return Catalog(
                version=version, tasks=tasks, anchors=anchors, metadata=metadata
            )
        except ValueError as exc:
            raise ConfigError(f"invalid catalog: {exc}") from exc

    # ---------------------------------------------------- anchors

    def _parse_anchors(self, items: Any) -> list[InlineAnchor]:
        if not isinstance(items, list):
            raise ConfigError("anchors must be a list")
        out: list[InlineAnchor] = []
        for i, raw in enumerate(items):
            if not isinstance(raw, Mapping):
                raise ConfigError(f"anchors[{i}] must be a mapping")
            self._reject_unknown(raw, _ALLOWED_ANCHOR_KEYS, path=f"anchors[{i}]")
            aid = str(raw.get("id", ""))
            vals_raw = raw.get("values") or []
            if not isinstance(vals_raw, list) or not all(
                isinstance(v, (int, float)) for v in vals_raw
            ):
                raise ConfigError(f"anchors[{i}].values must be a list of numbers")
            out.append(InlineAnchor(id=aid, values=tuple(float(v) for v in vals_raw)))
        return out

    # ----------------------------------------------------- tasks

    def _parse_tasks(self, items: Any) -> list[CatalogTask]:
        if not isinstance(items, list):
            raise ConfigError("tasks must be a list")
        out: list[CatalogTask] = []
        for i, raw in enumerate(items):
            if not isinstance(raw, Mapping):
                raise ConfigError(f"tasks[{i}] must be a mapping")
            self._reject_unknown(raw, _ALLOWED_TASK_KEYS, path=f"tasks[{i}]")
            try:
                out.append(
                    CatalogTask(
                        id=str(raw.get("id", "")),
                        description=str(raw.get("description", "")),
                        tenant=str(raw.get("tenant", "")),
                        cone_key=str(raw.get("cone_key", "")),
                        dim_latent=_need_int(raw, "dim_latent"),
                        anchors=tuple(
                            _list_of_str(raw.get("anchors", []), f"tasks[{i}].anchors")
                        ),
                        tags=tuple(_list_of_str(raw.get("tags", []), f"tasks[{i}].tags")),
                    )
                )
            except ValueError as exc:
                raise ConfigError(f"tasks[{i}]: {exc}") from exc
        return out

    # --------------------------------------------------- helpers

    @staticmethod
    def _reject_unknown(
        block: Mapping[str, Any], allowed: frozenset[str], *, path: str
    ) -> None:
        unknown = set(block.keys()) - allowed
        if unknown:
            raise ConfigError(
                f"unknown catalog keys at {path or 'root'}: {sorted(unknown)}"
            )


# ---------------------------------------------------------------------------
#  Primitive helpers
# ---------------------------------------------------------------------------


def _need_int(block: Mapping[str, Any], key: str) -> int:
    if key not in block:
        raise ConfigError(f"missing required key {key!r}")
    v = block[key]
    if not isinstance(v, int) or isinstance(v, bool):
        raise ConfigError(f"{key!r} must be an integer")
    return int(v)


def _list_of_str(v: Any, path: str) -> list[str]:
    if not isinstance(v, list):
        raise ConfigError(f"{path} must be a list of strings")
    if not all(isinstance(x, str) for x in v):
        raise ConfigError(f"{path} items must be strings")
    return list(v)


def _coerce_date(v: Any) -> date | None:
    if v is None:
        return None
    if isinstance(v, date):
        return v
    if isinstance(v, datetime):
        return v.date()
    if isinstance(v, str):
        try:
            return date.fromisoformat(v)
        except ValueError as exc:
            raise ConfigError(f"invalid date: {v!r}") from exc
    raise ConfigError(f"metadata.created_at must be ISO date, got {type(v).__name__}")


__all__ = ["CatalogLoader", "load_catalog"]
