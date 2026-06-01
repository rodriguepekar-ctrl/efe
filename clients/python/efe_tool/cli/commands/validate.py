"""Implementation of ``efe-admin validate``.

Lightweight catalog lint — **no embedder**, no network. Validates the
YAML shape against the documented schema and surfaces a list of issues
and warnings.

Exit codes:

    0  — valid
    4  — invalid (at least one *issue*; warnings alone are non-fatal)

The stdout document lists both ``issues`` (fatal) and ``warnings``
(non-fatal) with a stable schema — see :func:`output.build_validate_document`.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ..config import ResolvedConfig
from ..errors import (
    CatalogInvalidError,
    CatalogNotFoundError,
    UsageError,
)
from ..logging import Logger
from ..output import build_validate_document, render


_REQUIRED_TOP: tuple[str, ...] = (
    "apiVersion", "kind", "metadata", "embedder", "classes",
)


def run(cfg: ResolvedConfig, log: Logger) -> int:
    yaml_path = _resolve_path(cfg)
    if not yaml_path.exists():
        raise CatalogNotFoundError(
            f"catalog file not found: {yaml_path}",
            hint="check the path; relative paths resolve against the current working directory",
        )

    log.info("validate_start", catalog=str(yaml_path),
             strict_thresholds=cfg.strict_thresholds)

    try:
        import yaml  # local import — cheap
    except ImportError as e:  # pragma: no cover
        raise CatalogInvalidError(
            "pyyaml is not installed", cause=e,
        ) from e

    try:
        raw = yaml.safe_load(yaml_path.read_text(encoding="utf-8"))
    except yaml.YAMLError as e:
        raise CatalogInvalidError(
            f"YAML parse error: {e}",
            hint="check indentation and quoting",
            cause=e,
        ) from e
    except OSError as e:
        raise CatalogInvalidError(f"cannot read catalog: {e}", cause=e) from e

    issues: list[dict[str, Any]] = []
    warnings: list[dict[str, Any]] = []

    # Hard guard: every subsequent check assumes a mapping at the root.
    # An empty file, a list, or a scalar would crash the ``dict.get`` calls
    # inside the check helpers.
    if raw is None:
        issues.append({
            "code": "EMPTY_DOCUMENT",
            "detail": "catalog file is empty or contains only YAML comments",
        })
    elif not isinstance(raw, dict):
        issues.append({
            "code": "NOT_A_MAPPING",
            "detail": (
                "catalog top-level must be a YAML mapping; "
                f"got {type(raw).__name__}"
            ),
        })
    else:
        _check_top_level(raw, issues)
        _check_api_version(raw, issues)
        _check_classes(raw, issues)
        _check_thresholds(raw, issues, warnings, strict=cfg.strict_thresholds)
        _check_cone_policy(raw, issues)

    ok = len(issues) == 0
    doc = build_validate_document(
        catalog_path=str(yaml_path),
        ok=ok,
        issues=issues,
        warnings=warnings,
        request_id=cfg.request_id,
    )
    render(doc, format=cfg.format, pretty=cfg.pretty)

    log.info(
        "validate_done",
        catalog=str(yaml_path),
        valid=ok,
        issue_count=len(issues),
        warning_count=len(warnings),
    )
    if not ok:
        # Surface the first issue as the structured error so stderr
        # contains a readable summary.
        raise CatalogInvalidError(
            f"{len(issues)} issue(s) found in {yaml_path}",
            context={"issues": issues},
        )
    return 0


# ---------------------------------------------------------------------------
#  Individual checks
# ---------------------------------------------------------------------------


def _check_top_level(raw: dict, issues: list) -> None:
    missing = [k for k in _REQUIRED_TOP if k not in raw]
    if missing:
        issues.append({
            "code": "MISSING_TOP_LEVEL",
            "keys": missing,
            "detail": f"required top-level keys missing: {missing}",
        })


def _check_api_version(raw: dict, issues: list) -> None:
    av = raw.get("apiVersion")
    kind = raw.get("kind")
    if av is not None and av != "efe.mode1/v1":
        issues.append({
            "code": "UNSUPPORTED_API_VERSION",
            "got": av,
            "expected": "efe.mode1/v1",
        })
    if kind is not None and kind != "AnchorCatalog":
        issues.append({
            "code": "UNSUPPORTED_KIND",
            "got": kind,
            "expected": "AnchorCatalog",
        })


def _check_classes(raw: dict, issues: list) -> None:
    classes = raw.get("classes", []) or []
    if not isinstance(classes, list):
        issues.append({"code": "CLASSES_NOT_LIST"})
        return
    if len(classes) < 2:
        issues.append({
            "code": "TOO_FEW_CLASSES",
            "got": len(classes),
            "minimum": 2,
        })
        return
    seen: set[str] = set()
    for i, c in enumerate(classes):
        if not isinstance(c, dict):
            issues.append({"code": "CLASS_NOT_DICT", "index": i})
            continue
        name = c.get("name")
        if not name or not isinstance(name, str):
            issues.append({"code": "CLASS_MISSING_NAME", "index": i})
            continue
        if name in seen:
            issues.append({"code": "DUPLICATE_CLASS", "name": name})
        seen.add(name)
        sentences = c.get("canonical_sentences", []) or []
        if not isinstance(sentences, list) or len(sentences) == 0:
            issues.append({
                "code": "CLASS_EMPTY_SENTENCES",
                "class": name,
            })


def _check_thresholds(
    raw: dict, issues: list, warnings: list, *, strict: bool
) -> None:
    th = raw.get("thresholds", {}) or {}
    high = th.get("confidence_high")
    medium = th.get("confidence_medium")
    if high is None or medium is None:
        # thresholds are optional; defaults will be applied by AnchorCatalog
        return
    try:
        high_f = float(high)
        medium_f = float(medium)
    except (TypeError, ValueError):
        issues.append({"code": "THRESHOLDS_NOT_NUMERIC",
                       "high": high, "medium": medium})
        return
    if not (0.0 < medium_f <= high_f < 1.0):
        issues.append({
            "code": "THRESHOLDS_OUT_OF_ORDER",
            "detail": "expected 0 < confidence_medium <= confidence_high < 1",
            "high": high_f, "medium": medium_f,
        })
        return
    if strict and (high_f - medium_f) < 0.05:
        warnings.append({
            "code": "THRESHOLD_GAP_TIGHT",
            "gap": round(high_f - medium_f, 4),
            "detail": "confidence_high - confidence_medium < 0.05",
        })


def _check_cone_policy(raw: dict, issues: list) -> None:
    cone = raw.get("cone_policy", {}) or {}
    if "max_projection_distance" in cone:
        try:
            v = float(cone["max_projection_distance"])
        except (TypeError, ValueError):
            issues.append({"code": "CONE_MAX_PROJ_NOT_NUMERIC"})
            return
        if not (0.0 < v < 2.0):
            issues.append({
                "code": "CONE_MAX_PROJ_OUT_OF_RANGE",
                "got": v,
                "detail": "must be in (0, 2)",
            })


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _resolve_path(cfg: ResolvedConfig) -> Path:
    """Return the catalog path to validate.

    The resolver already folds the positional ``PATH`` into
    ``cfg.catalog_path``.
    """
    if cfg.catalog_path:
        return Path(cfg.catalog_path)
    raise UsageError(
        "validate: missing PATH",
        hint="pass a catalog YAML path as the first positional argument",
    )


__all__ = ["run"]
