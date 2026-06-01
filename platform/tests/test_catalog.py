"""Tests for the YAML catalog loader."""

from __future__ import annotations

from pathlib import Path

import pytest

from efe_platform.catalog.loader import load_catalog
from efe_platform.errors import ConfigError


def test_valid_catalog(catalog_yaml: Path) -> None:
    cat = load_catalog(catalog_yaml)
    assert cat.version == 1
    assert len(cat.tasks) == 1
    assert cat.tasks[0].id == "routing"
    assert cat.anchors[0].id == "a1"
    assert cat.metadata.owner == "test"


def test_get_task(catalog_yaml: Path) -> None:
    cat = load_catalog(catalog_yaml)
    t = cat.get_task("routing")
    assert t.cone_key == "acme/routing@v1"
    with pytest.raises(KeyError):
        cat.get_task("ghost")


def test_by_tenant(catalog_yaml: Path) -> None:
    cat = load_catalog(catalog_yaml)
    assert len(cat.by_tenant("acme")) == 1
    assert cat.by_tenant("other") == ()


def test_unknown_top_key(tmp_path: Path) -> None:
    p = tmp_path / "c.yaml"
    p.write_text("version: 1\nbogus: 42\n", encoding="utf-8")
    with pytest.raises(ConfigError, match="unknown catalog keys"):
        load_catalog(p)


def test_missing_version(tmp_path: Path) -> None:
    p = tmp_path / "c.yaml"
    p.write_text("tasks: []\n", encoding="utf-8")
    with pytest.raises(ConfigError, match="missing required"):
        load_catalog(p)


def test_bad_cone_key_prefix(tmp_path: Path) -> None:
    p = tmp_path / "c.yaml"
    p.write_text(
        """\
version: 1
tasks:
  - id: x
    description: d
    tenant: acme
    cone_key: "other/x@v1"
    dim_latent: 8
""",
        encoding="utf-8",
    )
    with pytest.raises(ConfigError):
        load_catalog(p)


def test_duplicate_task_id(tmp_path: Path) -> None:
    p = tmp_path / "c.yaml"
    p.write_text(
        """\
version: 1
tasks:
  - id: dup
    description: a
    tenant: t
    cone_key: "t/dup@v1"
    dim_latent: 8
  - id: dup
    description: b
    tenant: t
    cone_key: "t/dup@v2"
    dim_latent: 8
""",
        encoding="utf-8",
    )
    with pytest.raises(ConfigError):
        load_catalog(p)


def test_file_not_found(tmp_path: Path) -> None:
    with pytest.raises(ConfigError, match="not found"):
        load_catalog(tmp_path / "nonexistent.yaml")


def test_to_dict_serializable(catalog_yaml: Path) -> None:
    import json

    cat = load_catalog(catalog_yaml)
    # Must round-trip through json without error.
    blob = json.dumps(cat.to_dict())
    assert "routing" in blob
