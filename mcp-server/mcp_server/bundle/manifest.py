"""OCI manifest parser with EFE bundle annotations (M18 D47).

EFE bundles ship as OCI Artifacts v1.1 with `org.efe.bundle.*` annotations
that carry the bundle's logical metadata (slug, version, framework,
manifestSha256, etc.). This module extracts that metadata into a typed
dataclass.

Reference : M13 / D16 §3.2 bundle ID format.
"""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any


# Annotation namespace — kept short for OCI manifest size budget.
_NS = "org.efe.bundle"


class ManifestParseError(ValueError):
    """Raised when an OCI manifest doesn't carry the expected EFE shape."""


@dataclass(frozen=True)
class EFEBundleManifest:
    """Parsed EFE bundle manifest.

    Fields with `None` are absent from the manifest annotations and the
    caller may treat them as "unknown" (e.g. an old bundle predating
    the M18 schema, or a 3rd-party bundle).
    """

    # OCI-level fields
    schema_version: int                      # OCI schemaVersion (typically 2)
    media_type: str                          # OCI mediaType
    config_digest: str                       # config object digest
    layer_count: int
    total_layer_size: int                    # M18-D47-A2 fix : sum of layer.size

    # EFE-level fields (from annotations)
    slug: str | None
    version: str | None
    framework: str | None                    # "sentence-transformers", "hash", …
    model_name: str | None
    dim: int | None
    is_dev_unsigned: bool
    bundle_schema_version: str | None        # "1.0", "1.1", …
    produced_at: str | None                  # ISO8601
    produced_by: str | None                  # builder URL
    min_efe_server_version: str | None
    max_efe_server_version: str | None
    required_features: tuple[str, ...]
    sbom_digest: str | None                  # if SBOM is in-manifest layer
    manifest_sha256: str                     # computed from raw manifest bytes

    # Raw annotations dict for forward-compat
    raw_annotations: dict[str, str] = field(default_factory=dict)

    def is_efe_bundle(self) -> bool:
        return self.slug is not None and self.version is not None


def _annot(annotations: dict[str, str], key: str) -> str | None:
    return annotations.get(f"{_NS}.{key}")


def _bool_annot(annotations: dict[str, str], key: str, default: bool = False) -> bool:
    v = _annot(annotations, key)
    if v is None:
        return default
    return v.lower() in ("true", "1", "yes")


def _int_annot(annotations: dict[str, str], key: str) -> int | None:
    v = _annot(annotations, key)
    if v is None:
        return None
    try:
        return int(v)
    except ValueError:
        return None


def parse_manifest(*, raw_bytes: bytes, manifest_sha256: str) -> EFEBundleManifest:
    """Parse a raw OCI manifest into an EFEBundleManifest.

    `manifest_sha256` is supplied separately because it must be computed
    over the raw bytes the registry served — never over a re-serialized
    JSON (whitespace differences would invalidate the signature).
    """
    try:
        data: Any = json.loads(raw_bytes)
    except json.JSONDecodeError as exc:
        raise ManifestParseError(f"manifest is not valid JSON: {exc}") from exc

    if not isinstance(data, dict):
        raise ManifestParseError(
            f"manifest root must be a JSON object, got {type(data).__name__}"
        )

    schema_version = data.get("schemaVersion")
    if not isinstance(schema_version, int):
        raise ManifestParseError(
            f"manifest schemaVersion must be int, got {type(schema_version).__name__}"
        )
    if schema_version != 2:
        raise ManifestParseError(
            f"unsupported schemaVersion {schema_version!r} (expected 2)"
        )

    media_type = data.get("mediaType", "")
    config = data.get("config") or {}
    config_digest = config.get("digest", "")
    layers = data.get("layers") or []
    if not isinstance(layers, list):
        layers = []
    # M18-D47-A2 fix : compute real total layer size from OCI manifest.
    total_layer_size = 0
    for layer in layers:
        if not isinstance(layer, dict):
            continue
        size = layer.get("size")
        if isinstance(size, int) and size >= 0:
            total_layer_size += size

    annotations = data.get("annotations") or {}
    if not isinstance(annotations, dict):
        annotations = {}

    required_features_str = _annot(annotations, "required_features") or ""
    required_features = tuple(
        f.strip() for f in required_features_str.split(",") if f.strip()
    )

    return EFEBundleManifest(
        schema_version=schema_version,
        media_type=media_type,
        config_digest=config_digest,
        layer_count=len(layers),
        total_layer_size=total_layer_size,
        slug=_annot(annotations, "slug"),
        version=_annot(annotations, "version"),
        framework=_annot(annotations, "framework"),
        model_name=_annot(annotations, "model_name"),
        dim=_int_annot(annotations, "dim"),
        is_dev_unsigned=_bool_annot(annotations, "is_dev_unsigned", default=False),
        bundle_schema_version=_annot(annotations, "schema_version"),
        produced_at=_annot(annotations, "produced_at"),
        produced_by=_annot(annotations, "produced_by"),
        min_efe_server_version=_annot(annotations, "min_efe_server_version"),
        max_efe_server_version=_annot(annotations, "max_efe_server_version"),
        required_features=required_features,
        sbom_digest=_annot(annotations, "sbom_digest"),
        manifest_sha256=manifest_sha256,
        raw_annotations={k: v for k, v in annotations.items() if isinstance(k, str)},
    )
