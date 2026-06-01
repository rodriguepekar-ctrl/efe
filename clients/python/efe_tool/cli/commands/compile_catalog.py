"""Implementation of ``efe-admin compile-catalog``.

Pre-computes anchor centroids from the canonical sentences declared in
a catalog YAML and writes the compiled form next to the YAML (or at
``--out`` if provided). This is the **only** subcommand that pulls
``sentence-transformers`` unconditionally.

Usage (shell):

    efe-admin compile-catalog catalogs/support_triage.yaml
    efe-admin compile-catalog catalogs/x.yaml --out /tmp/x.compiled.json --force
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

from ..config import ResolvedConfig
from ..errors import (
    CatalogInvalidError,
    CatalogNotFoundError,
    UsageError,
)
from ..logging import Logger
from ..output import build_compile_document, render


def run(cfg: ResolvedConfig, log: Logger) -> int:
    # The ``path`` positional is surfaced via ``cfg.catalog_path`` — the
    # parser aliases both ``path`` and ``--catalog`` into that key.
    yaml_path = _resolve_catalog_path(cfg)

    if not yaml_path.exists():
        raise CatalogNotFoundError(f"catalog file not found: {yaml_path}")

    out_path = cfg.out_path or yaml_path.with_name(f"{yaml_path.stem}.compiled.json")

    if out_path.exists() and not cfg.force:
        log.debug("compile_cache_present", out=str(out_path))

    # Force behaviour: remove the current cache so the loader recompiles.
    if cfg.force and out_path.exists():
        try:
            out_path.unlink()
            log.info("compile_cache_cleared", out=str(out_path))
        except OSError as e:  # pragma: no cover
            log.warn("compile_cache_clear_failed", out=str(out_path),
                     error=str(e))

    log.info("compile_start", catalog=str(yaml_path), out=str(out_path))

    t0 = time.perf_counter()
    from efe_tool.catalog import AnchorCatalog
    from efe_tool.embedder import LocalSTEmbedder
    from efe_tool.errors import CatalogError, EmbedderError

    try:
        embedder = LocalSTEmbedder(cfg.embedder_model)
    except Exception as e:
        raise CatalogInvalidError(
            f"cannot load embedder {cfg.embedder_model!r}: {e}",
            hint="check network access and the model name",
            cause=e,
        ) from e

    try:
        catalog = AnchorCatalog.load(
            yaml_path,
            embedder=embedder,
            cache_dir=out_path.parent,
        )
    except CatalogError as e:
        raise CatalogInvalidError(str(e), cause=e) from e
    except EmbedderError as e:
        raise CatalogInvalidError(f"embedder failure: {e}", cause=e) from e

    # If the resolver ordered a different output path than the default
    # (.compiled.json next to YAML), move/rename the file.
    _maybe_relocate_compiled(yaml_path, out_path, log)

    doc = build_compile_document(catalog, request_id=cfg.request_id)
    render(doc, format=cfg.format, pretty=cfg.pretty)

    log.info(
        "compile_done",
        catalog=getattr(catalog, "name", "?"),
        version=getattr(catalog, "version", "?"),
        classes=len(getattr(catalog, "classes", ())),
        dim=getattr(catalog, "dim", 0),
        out=str(out_path),
        elapsed_ms=int((time.perf_counter() - t0) * 1000),
    )
    return 0


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _resolve_catalog_path(cfg: ResolvedConfig) -> Path:
    """Return the catalog path as a :class:`Path`.

    The resolver already folds the ``PATH`` positional argument into
    ``cfg.catalog_path``, so we only need to surface a clean error
    when neither the flag nor the positional was provided.
    """
    if cfg.catalog_path:
        return Path(cfg.catalog_path)
    raise UsageError(
        "compile-catalog: missing PATH",
        hint="pass a catalog YAML path as the first positional argument",
    )


def _maybe_relocate_compiled(
    yaml_path: Path, desired: Path, log: Logger
) -> None:
    default = yaml_path.with_name(f"{yaml_path.stem}.compiled.json")
    if desired.resolve() == default.resolve():
        return
    if not default.exists():
        return
    try:
        desired.parent.mkdir(parents=True, exist_ok=True)
        content = default.read_bytes()
        desired.write_bytes(content)
        default.unlink()
        log.info("compile_cache_relocated",
                 from_=str(default), to=str(desired))
    except OSError as e:  # pragma: no cover
        log.warn("compile_cache_relocate_failed", error=str(e))


__all__ = ["run"]
