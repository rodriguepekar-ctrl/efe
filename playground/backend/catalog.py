"""
EFE Playground — Catalog management.

A `Catalog` is a tenant-owned set of classes. Each class has a centroid
(mean of L2-normalized example embeddings) and metadata.

Inspired by D12 catalog architecture but simplified for the playground :
  - In-memory (no Postgres backend ; lost on restart)
  - Per-tenant isolation (D11 zero-trust)
  - Versioned : each Register/Evolve creates a new immutable version
  - Confidence threshold per-class (operator-tunable)
  - Quotas enforced per D12 policy::DEFAULT_MAX_* :
      MAX_NAMES_PER_TENANT     = 100
      MAX_VERSIONS_PER_NAME    = 1000
      MAX_CLASSES_PER_CATALOG  = 256
      MAX_SENTENCES_PER_CLASS  = 64

Cf. `docs/bench/d12_catalog_admin_versioning_architecture.md`.

Audit fix M15-A1 (HIGH) — adds Evolve RPC.
Audit fix M15-A2 (HIGH) — enforces D12 catalog quotas.
"""
from __future__ import annotations

import hashlib
import json
import time
import uuid
from dataclasses import dataclass, field
from typing import Optional

import numpy as np


# D12 catalog policy quotas (mirror `crates/efe-server/src/catalog/policy.rs`)
MAX_NAMES_PER_TENANT = 100
MAX_VERSIONS_PER_NAME = 1000
MAX_CLASSES_PER_CATALOG = 256
MAX_SENTENCES_PER_CLASS = 64


class QuotaExceededError(ValueError):
    """Raised when a D12 catalog quota would be exceeded."""
    pass


@dataclass
class CatalogClass:
    name: str
    examples: list[str]                       # raw sentence inputs
    centroid: list[float]                     # 384-dim L2-normalized
    confidence_threshold: float = 0.5
    metadata: dict = field(default_factory=dict)


@dataclass
class CatalogVersion:
    version_id: str                           # uuid4
    tenant_id: str
    name: str                                 # catalog name
    version: int                              # 1, 2, 3...
    classes: list[CatalogClass]
    created_at: float                         # unix epoch
    fingerprint: str                          # sha256 of canonical content
    embedder_info: dict


class CatalogStore:
    """Per-tenant catalog storage with versioning.

    Threading : NOT thread-safe ; the playground Flask app uses
    Werkzeug single-thread mode. Production EFE uses Postgres D14.
    """

    def __init__(self) -> None:
        # tenant_id -> catalog_name -> [CatalogVersion]
        self._store: dict[str, dict[str, list[CatalogVersion]]] = {}

    def register(
        self,
        *,
        tenant_id: str,
        name: str,
        classes_input: list[dict],
        embedder,
    ) -> CatalogVersion:
        """Create a new catalog version.

        `classes_input` : [{ name: str, examples: [str], confidence_threshold?: float }]

        Enforces D12 quotas (audit fix M15-A2) :
          - MAX_NAMES_PER_TENANT
          - MAX_VERSIONS_PER_NAME (per existing catalog)
          - MAX_CLASSES_PER_CATALOG
          - MAX_SENTENCES_PER_CLASS
        """
        # Validate input
        if not classes_input:
            raise ValueError("at least one class required")

        # Audit fix M15-A2 : D12 quotas
        existing_versions = self._store.get(tenant_id, {}).get(name, [])
        existing_names = list(self._store.get(tenant_id, {}).keys())

        # Quota: max names per tenant (counted only when creating NEW catalog)
        if name not in existing_names:
            if len(existing_names) >= MAX_NAMES_PER_TENANT:
                raise QuotaExceededError(
                    f"quota_exceeded: max_names_per_tenant ({MAX_NAMES_PER_TENANT}) "
                    f"reached for tenant {tenant_id!r}"
                )

        # Quota: max versions per name
        if len(existing_versions) >= MAX_VERSIONS_PER_NAME:
            raise QuotaExceededError(
                f"quota_exceeded: max_versions_per_name ({MAX_VERSIONS_PER_NAME}) "
                f"reached for catalog {name!r}"
            )

        # Quota: max classes per catalog
        if len(classes_input) > MAX_CLASSES_PER_CATALOG:
            raise QuotaExceededError(
                f"quota_exceeded: max_classes_per_catalog ({MAX_CLASSES_PER_CATALOG}) "
                f"; got {len(classes_input)} classes"
            )

        # Build CatalogClass objects with centroids
        classes: list[CatalogClass] = []
        for cls in classes_input:
            cls_name = cls.get("name")
            examples = cls.get("examples", [])
            if not cls_name or not examples:
                raise ValueError(
                    f"class must have `name` and ≥ 1 `examples`, got {cls}"
                )

            # Quota: max sentences per class
            if len(examples) > MAX_SENTENCES_PER_CLASS:
                raise QuotaExceededError(
                    f"quota_exceeded: max_sentences_per_class "
                    f"({MAX_SENTENCES_PER_CLASS}) ; class {cls_name!r} "
                    f"has {len(examples)} examples"
                )
            # Compute centroid = mean of L2-normalized embeddings
            embs = embedder.encode_batch(examples)
            centroid = embs.mean(axis=0)
            # Re-normalize centroid (mean of unit vectors is not unit-length)
            norm = np.linalg.norm(centroid)
            if norm > 0:
                centroid = centroid / norm
            classes.append(
                CatalogClass(
                    name=cls_name,
                    examples=examples,
                    centroid=centroid.tolist(),
                    confidence_threshold=float(
                        cls.get("confidence_threshold", 0.5)
                    ),
                    metadata=cls.get("metadata", {}),
                )
            )

        # Determine version number (existing_versions already fetched above for quota check)
        next_version = len(existing_versions) + 1

        # Compute fingerprint
        canon = json.dumps(
            {
                "tenant_id": tenant_id,
                "name": name,
                "version": next_version,
                "classes": [
                    {
                        "name": c.name,
                        "examples": c.examples,
                        "confidence_threshold": c.confidence_threshold,
                    }
                    for c in classes
                ],
            },
            sort_keys=True,
        ).encode("utf-8")
        fingerprint = hashlib.sha256(canon).hexdigest()

        cv = CatalogVersion(
            version_id=str(uuid.uuid4()),
            tenant_id=tenant_id,
            name=name,
            version=next_version,
            classes=classes,
            created_at=time.time(),
            fingerprint=fingerprint,
            embedder_info={
                "backend": embedder.info.backend,
                "model_name": embedder.info.model_name,
                "dim": embedder.info.dim,
            },
        )

        # Persist
        self._store.setdefault(tenant_id, {}).setdefault(name, []).append(cv)
        return cv

    def evolve(
        self,
        *,
        tenant_id: str,
        name: str,
        additional_examples_per_class: dict[str, list[str]],
        embedder,
    ) -> CatalogVersion:
        """Audit fix M15-A1 (HIGH) — Evolve a catalog by adding examples.

        D12 Evolve RPC : take the latest version + append new examples to
        existing classes (or create new classes), recompute centroids,
        register as a new version. Mirrors `crates/efe-server/src/catalog/admin.rs::evolve`.

        Quota check : merged class examples must stay ≤ MAX_SENTENCES_PER_CLASS.
        """
        latest = self.get_latest(tenant_id, name)
        if latest is None:
            raise KeyError(
                f"catalog {name!r} not found for tenant {tenant_id!r} ; "
                f"use Register first"
            )

        # Build merged classes (existing examples + new ones)
        existing_by_name = {c.name: c for c in latest.classes}
        merged_classes: list[dict] = []

        # First, augment existing classes
        for cls in latest.classes:
            new_examples = additional_examples_per_class.get(cls.name, [])
            merged_examples = list(cls.examples) + list(new_examples)
            merged_classes.append({
                "name": cls.name,
                "examples": merged_examples,
                "confidence_threshold": cls.confidence_threshold,
                "metadata": cls.metadata,
            })

        # Second, add brand-new classes (in additional_examples but not in existing)
        for cls_name, examples in additional_examples_per_class.items():
            if cls_name not in existing_by_name:
                merged_classes.append({
                    "name": cls_name,
                    "examples": examples,
                    "confidence_threshold": 0.5,
                    "metadata": {},
                })

        # Delegate to register (which handles quotas + centroids + versioning)
        return self.register(
            tenant_id=tenant_id,
            name=name,
            classes_input=merged_classes,
            embedder=embedder,
        )

    def list_catalogs(self, tenant_id: str) -> list[dict]:
        out = []
        for name, versions in self._store.get(tenant_id, {}).items():
            latest = versions[-1]
            out.append({
                "name": name,
                "latest_version": latest.version,
                "fingerprint": latest.fingerprint,
                "class_count": len(latest.classes),
                "created_at": latest.created_at,
                "all_versions": [
                    {"version": v.version, "fingerprint": v.fingerprint,
                     "created_at": v.created_at}
                    for v in versions
                ],
            })
        return out

    def get_latest(
        self, tenant_id: str, name: str
    ) -> Optional[CatalogVersion]:
        versions = self._store.get(tenant_id, {}).get(name, [])
        return versions[-1] if versions else None

    def get_version(
        self, tenant_id: str, name: str, version: int
    ) -> Optional[CatalogVersion]:
        for v in self._store.get(tenant_id, {}).get(name, []):
            if v.version == version:
                return v
        return None

    def delete(self, tenant_id: str, name: str) -> bool:
        if name in self._store.get(tenant_id, {}):
            del self._store[tenant_id][name]
            return True
        return False

    def classify(
        self,
        *,
        tenant_id: str,
        catalog_name: str,
        text: str,
        embedder,
        top_k: int = 3,
    ) -> dict:
        """Classify a text against the latest version of a catalog.

        Returns : {
            "catalog_name": str,
            "catalog_version": int,
            "top_k": [{class_name, score, above_threshold}, ...],
            "embedding_norm": float,
            "embedder_info": dict,
        }
        """
        cv = self.get_latest(tenant_id, catalog_name)
        if cv is None:
            raise KeyError(f"catalog {catalog_name!r} not found for tenant {tenant_id!r}")

        # Encode input
        emb = embedder.encode(text)
        emb_norm = float(np.linalg.norm(emb))

        # Score against each class centroid (cosine similarity)
        scores = []
        for cls in cv.classes:
            centroid = np.array(cls.centroid, dtype=np.float32)
            score = float(np.dot(emb, centroid))
            scores.append({
                "class_name": cls.name,
                "score": score,
                "above_threshold": score >= cls.confidence_threshold,
                "confidence_threshold": cls.confidence_threshold,
            })
        # Sort by descending score
        scores.sort(key=lambda x: x["score"], reverse=True)

        return {
            "catalog_name": catalog_name,
            "catalog_version": cv.version,
            "fingerprint": cv.fingerprint,
            "top_k": scores[:top_k],
            "embedding_norm": emb_norm,
            "embedder_info": cv.embedder_info,
        }
