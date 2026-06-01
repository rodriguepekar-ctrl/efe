"""Anchor catalog — YAML-defined collection of classes + pre-computed anchors.

A catalog is the **stable patrimony** of the Mode-1 classifier. It is:

* **Authored** by the product team as a YAML file in ``catalogs/``.
* **Compiled** (anchors encoded) on first load, with a checksum of the
  embedder fingerprint + canonical sentences. The compiled form is
  cached locally next to the YAML.
* **Versioned** via SemVer-like ``metadata.version``.

See ``MODE1_ARCHITECTURE.md`` §7..§9 for the full semantics.
"""

from __future__ import annotations

import hashlib
import json
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Mapping, Sequence

import numpy as np

try:
    import yaml
except ImportError as _e:  # pragma: no cover
    raise ImportError(
        "pyyaml is required for AnchorCatalog; `pip install pyyaml`."
    ) from _e

from .embedder import Embedder
from .errors import CatalogError


# ---------------------------------------------------------------------
# Typed config sub-structures
# ---------------------------------------------------------------------


@dataclass(frozen=True)
class ThresholdPolicy:
    """Decision thresholds on the swarm confidence.

    The catalog declares them; the client applies them to produce a
    ``ClassifyResult.reject`` flag.
    """

    confidence_high: float = 0.85
    confidence_medium: float = 0.65

    def zone(self, confidence: float) -> str:
        """Return ``"trusted"`` / ``"warn"`` / ``"reject"`` for a given confidence."""
        if confidence >= self.confidence_high:
            return "trusted"
        if confidence >= self.confidence_medium:
            return "warn"
        return "reject"


@dataclass(frozen=True)
class ClassDef:
    """One class in the catalog."""

    name: str
    description: str
    canonical_sentences: tuple[str, ...]
    deprecated: bool = False
    color: str | None = None


@dataclass(frozen=True)
class AnchorCatalog:
    """In-memory view of a compiled anchor catalog.

    Instances are **immutable** after construction. The typical usage is
    to keep one instance per catalog alive for the life of the process
    (it is threadsafe and cheap to share).
    """

    name: str
    version: str
    description: str
    embedder_fingerprint: str
    dim: int
    classes: tuple[ClassDef, ...]
    anchors: np.ndarray  # shape (N, D), L2-normalised, f32
    max_projection_distance: float
    thresholds: ThresholdPolicy
    source_path: str | None = None
    extra: Mapping[str, Any] = field(default_factory=dict)

    # -----------------------------------------------------------
    # Construction
    # -----------------------------------------------------------

    @classmethod
    def load(
        cls,
        path: str | Path,
        *,
        embedder: Embedder | None = None,
        cache_dir: str | Path | None = None,
    ) -> "AnchorCatalog":
        """Load a catalog from ``path`` (a YAML file).

        If ``embedder`` is provided, any anchors declared as
        ``canonical_sentences`` are encoded and averaged. If the
        catalog has been compiled before (a sibling ``.compiled.json``
        file exists with matching hashes), the compiled form is
        reused — same rule as Cargo's ``Cargo.lock``.

        Raises:
            CatalogError: on schema/embedder mismatch / empty anchors.
        """
        path = Path(path)
        if not path.exists():
            raise CatalogError(f"catalog file not found: {path}")
        with path.open("r", encoding="utf-8") as f:
            raw: dict[str, Any] = yaml.safe_load(f) or {}
        _validate_schema(raw, path)

        # --- Metadata ---
        meta = raw["metadata"]
        name = str(meta["name"])
        version = str(meta["version"])
        description = str(meta.get("description", "")).strip()

        # --- Embedder declaration ---
        emb_spec = raw["embedder"]
        declared_fp = f"sentence-transformers/{emb_spec['model']}@{int(emb_spec['dim'])}"
        declared_dim = int(emb_spec["dim"])

        # --- Classes ---
        classes_raw: list[dict[str, Any]] = raw["classes"]
        if len(classes_raw) < 2:
            raise CatalogError(
                f"catalog {name}@{version}: need at least 2 classes, got {len(classes_raw)}"
            )
        classes: list[ClassDef] = []
        for c in classes_raw:
            if "canonical_sentences" not in c or not c["canonical_sentences"]:
                raise CatalogError(
                    f"class '{c.get('name')}': canonical_sentences must be non-empty"
                )
            classes.append(
                ClassDef(
                    name=str(c["name"]),
                    description=str(c.get("description", "")).strip(),
                    canonical_sentences=tuple(str(s) for s in c["canonical_sentences"]),
                    deprecated=bool(c.get("deprecated", False)),
                    color=c.get("color"),
                )
            )

        # --- Cone policy ---
        cone_cfg = raw.get("cone_policy", {}) or {}
        max_d = float(cone_cfg.get("max_projection_distance", 0.5))
        if not (max_d > 0 and max_d < 2.0):
            raise CatalogError(f"max_projection_distance must be in (0, 2); got {max_d}")

        # --- Thresholds ---
        th = raw.get("thresholds", {}) or {}
        thresholds = ThresholdPolicy(
            confidence_high=float(th.get("confidence_high", 0.85)),
            confidence_medium=float(th.get("confidence_medium", 0.65)),
        )

        # --- Anchors compilation ---
        cache_dir = Path(cache_dir) if cache_dir else path.parent
        cached = cls._try_load_compiled(
            yaml_path=path,
            version=version,
            declared_fp=declared_fp,
            cache_dir=cache_dir,
            classes=classes,
        )
        if cached is not None:
            anchors = cached
            used_fp = declared_fp
        else:
            if embedder is None:
                raise CatalogError(
                    f"catalog {name}@{version} has no compiled anchors and no "
                    "embedder was passed to AnchorCatalog.load(). "
                    "Either compile once via `efe-admin compile-catalog ...` or "
                    "pass embedder=LocalSTEmbedder(...)."
                )
            if embedder.dim() != declared_dim:
                raise CatalogError(
                    f"embedder dim {embedder.dim()} != catalog dim {declared_dim}"
                )
            if embedder.fingerprint() != declared_fp:
                raise CatalogError(
                    f"embedder fingerprint '{embedder.fingerprint()}' does not match "
                    f"catalog '{declared_fp}'. Refusing to compile against a different model."
                )
            anchors = cls._compile_anchors(embedder, classes)
            used_fp = embedder.fingerprint()
            cls._save_compiled(
                yaml_path=path,
                version=version,
                declared_fp=used_fp,
                cache_dir=cache_dir,
                classes=classes,
                anchors=anchors,
            )

        # --- Validate inter-class separation ---
        cls._validate_anchors(anchors, [c.name for c in classes])

        return cls(
            name=name,
            version=version,
            description=description,
            embedder_fingerprint=used_fp,
            dim=declared_dim,
            classes=tuple(classes),
            anchors=anchors,
            max_projection_distance=max_d,
            thresholds=thresholds,
            source_path=str(path),
            extra={k: v for k, v in raw.items() if k not in {
                "apiVersion", "kind", "metadata", "embedder", "classes",
                "cone_policy", "thresholds",
            }},
        )

    # -----------------------------------------------------------
    # Helpers
    # -----------------------------------------------------------

    @staticmethod
    def _compile_anchors(embedder: Embedder, classes: Sequence[ClassDef]) -> np.ndarray:
        """Encode each class as the L2-normalised mean of its canonicals."""
        n, d = len(classes), embedder.dim()
        anchors = np.zeros((n, d), dtype=np.float32)
        for i, c in enumerate(classes):
            vecs = embedder.encode_batch(c.canonical_sentences)
            mean = vecs.mean(axis=0)
            norm = float(np.linalg.norm(mean))
            if norm < 1e-6:
                raise CatalogError(
                    f"anchor for class '{c.name}' collapses to zero (degenerate sentences)"
                )
            anchors[i, :] = mean / norm
        return anchors

    @staticmethod
    def _validate_anchors(anchors: np.ndarray, names: Sequence[str]) -> None:
        """Check inter-class similarity ≤ 0.95 (hard) and emit a debug warning at ≥ 0.80."""
        n = anchors.shape[0]
        if n == 0:
            raise CatalogError("empty anchor matrix")
        # Cosine sim matrix (already L2-normalised).
        sim = anchors @ anchors.T
        for i in range(n):
            for j in range(i + 1, n):
                s = float(sim[i, j])
                if s > 0.95:
                    raise CatalogError(
                        f"classes '{names[i]}' and '{names[j]}' too similar "
                        f"(cosine={s:.4f} > 0.95). Differentiate the canonical sentences."
                    )

    @staticmethod
    def _compiled_hash(
        version: str, declared_fp: str, classes: Sequence[ClassDef]
    ) -> str:
        """Compute the hash that identifies a compiled cache entry."""
        h = hashlib.sha256()
        h.update(version.encode())
        h.update(b"\0")
        h.update(declared_fp.encode())
        h.update(b"\0")
        for c in classes:
            h.update(c.name.encode())
            h.update(b"\0")
            for s in c.canonical_sentences:
                h.update(s.encode())
                h.update(b"\0")
        return h.hexdigest()

    @classmethod
    def _try_load_compiled(
        cls,
        yaml_path: Path,
        version: str,
        declared_fp: str,
        cache_dir: Path,
        classes: Sequence[ClassDef],
    ) -> np.ndarray | None:
        """Return the compiled anchors if the cache is fresh, else ``None``."""
        compiled_path = cache_dir / f"{yaml_path.stem}.compiled.json"
        if not compiled_path.exists():
            return None
        try:
            with compiled_path.open("r", encoding="utf-8") as f:
                data = json.load(f)
        except Exception:
            return None
        if data.get("hash") != cls._compiled_hash(version, declared_fp, classes):
            return None
        try:
            vec_list = data["anchors"]
            arr = np.asarray(vec_list, dtype=np.float32)
        except Exception:
            return None
        return arr

    @classmethod
    def _save_compiled(
        cls,
        yaml_path: Path,
        version: str,
        declared_fp: str,
        cache_dir: Path,
        classes: Sequence[ClassDef],
        anchors: np.ndarray,
    ) -> None:
        try:
            cache_dir.mkdir(parents=True, exist_ok=True)
            compiled_path = cache_dir / f"{yaml_path.stem}.compiled.json"
            payload = {
                "hash": cls._compiled_hash(version, declared_fp, classes),
                "version": version,
                "embedder_fingerprint": declared_fp,
                "classes": [c.name for c in classes],
                "anchors": anchors.tolist(),
            }
            with compiled_path.open("w", encoding="utf-8") as f:
                json.dump(payload, f)
        except Exception:
            # Compile cache is an optimisation; never let a write failure
            # break the happy path.
            pass

    # -----------------------------------------------------------
    # Accessors
    # -----------------------------------------------------------

    def class_by_index(self, idx: int) -> ClassDef:
        """Look up a class by its zero-based index."""
        if not 0 <= idx < len(self.classes):
            raise CatalogError(f"class index {idx} out of range (0..{len(self.classes)})")
        return self.classes[idx]

    def class_by_name(self, name: str) -> ClassDef:
        """Look up a class by its declared name."""
        for c in self.classes:
            if c.name == name:
                return c
        raise CatalogError(f"no class named '{name}' in catalog {self.name}")

    def class_names(self) -> tuple[str, ...]:
        """Ordered tuple of class names (stable across sessions)."""
        return tuple(c.name for c in self.classes)


# ---------------------------------------------------------------------
# Schema validation
# ---------------------------------------------------------------------


def _validate_schema(raw: Mapping[str, Any], path: Path) -> None:
    required_top = {"apiVersion", "kind", "metadata", "embedder", "classes"}
    missing = required_top - raw.keys()
    if missing:
        raise CatalogError(f"{path}: missing top-level keys {sorted(missing)}")
    if raw["apiVersion"] != "efe.mode1/v1":
        raise CatalogError(
            f"{path}: unsupported apiVersion '{raw['apiVersion']}' (expected 'efe.mode1/v1')"
        )
    if raw["kind"] != "AnchorCatalog":
        raise CatalogError(f"{path}: unsupported kind '{raw['kind']}' (expected 'AnchorCatalog')")
    for k in ("name", "version"):
        if k not in raw["metadata"]:
            raise CatalogError(f"{path}: metadata.{k} is required")
    emb = raw.get("embedder", {})
    for k in ("family", "model", "dim"):
        if k not in emb:
            raise CatalogError(f"{path}: embedder.{k} is required")
