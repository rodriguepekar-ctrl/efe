"""Catalog YAML validator — sanity checks + PII linting."""

from __future__ import annotations

from typing import Callable, Sequence

from efe_swarm_integration.catalog.yaml_loader import CatalogYaml
from efe_swarm_integration.errors import (
    CatalogCompilationFailed,
    PiiDetectedInPrompt,
)

__all__ = ["CatalogValidationError", "validate_catalog_yaml"]


# Re-export for user convenience.
CatalogValidationError = CatalogCompilationFailed


def validate_catalog_yaml(
    catalog: CatalogYaml,
    *,
    pii_linter: Callable[[str], Sequence[str]] | None = None,
    max_anchors_per_label: int = 500,
    max_total_anchors: int = 10_000,
) -> None:
    """Run sanity + PII checks on anchor texts.

    ``pii_linter`` is a callable that returns a list of detected PII
    kinds per text ; when hooked to :class:`efe_security.PiiClassifier`
    it automatically enforces M11 policy.
    """
    total = 0
    for lb in catalog.labels:
        if len(lb.anchors) > max_anchors_per_label:
            raise CatalogCompilationFailed(
                f"label {lb.label}: too many anchors ({len(lb.anchors)})"
            )
        total += len(lb.anchors)
        for idx, anchor in enumerate(lb.anchors):
            if not anchor.strip():
                raise CatalogCompilationFailed(
                    f"label {lb.label}: empty anchor at idx={idx}"
                )
            if pii_linter is not None:
                hits = list(pii_linter(anchor))
                if hits:
                    raise PiiDetectedInPrompt(
                        f"PII detected in anchor (label={lb.label}, idx={idx})",
                        detail={"kinds": hits},
                    )
    if total > max_total_anchors:
        raise CatalogCompilationFailed(
            f"total anchors exceeds {max_total_anchors}: {total}"
        )
