"""Catalog-scoped commands: ``:catalog``, ``:reload``, ``:classes``,
``:embedder``.

These are the workhorses of the tuning workflow (UC-B1 in the
architecture doc): switch catalog, recompile after an edit, inspect
class list, swap embedder.
"""

from __future__ import annotations

import os
import time
from pathlib import Path
from typing import Any

from ...cli.errors import (
    CatalogInvalidError,
    CatalogNotFoundError,
    UsageError,
    from_bridge_error,
)
from . import Command, register


# ---------------------------------------------------------------------------
#  :catalog  [PATH]
# ---------------------------------------------------------------------------


def _cmd_catalog(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return _describe_catalog(state)
    path = Path(os.path.expanduser(args[0]))
    _load_into_state(state, path, log)
    cat = state.catalog
    classes = getattr(cat, "classes", ())
    return (
        f"switched to {cat.name}@{cat.version} "
        f"({len(classes)} classes, fingerprint "
        f"{getattr(cat, 'embedder_fingerprint', '?')})"
    )


def _describe_catalog(state: Any) -> str:
    if state.catalog is None:
        return "(no catalog loaded)"
    cat = state.catalog
    classes = getattr(cat, "classes", ())
    src = getattr(cat, "source_path", None) or "(unknown)"
    return (
        f"{cat.name}@{cat.version}\n"
        f"  source:      {src}\n"
        f"  fingerprint: {getattr(cat, 'embedder_fingerprint', '?')}\n"
        f"  dim:         {getattr(cat, 'dim', '?')}\n"
        f"  classes:     {len(classes)}"
    )


register(Command(
    name="catalog",
    run=_cmd_catalog,
    doc="Without arg: describe the current catalog. With PATH: switch.",
    group="catalog",
))


# ---------------------------------------------------------------------------
#  :reload
# ---------------------------------------------------------------------------


def _cmd_reload(state: Any, args: list[str], log: Any) -> str:
    if state.catalog is None:
        raise UsageError(
            ":reload has nothing to reload (no catalog loaded)",
            hint="use `:catalog PATH` to load one",
        )
    src = getattr(state.catalog, "source_path", None)
    if not src:
        raise CatalogInvalidError(
            "current catalog has no known source path; cannot reload"
        )
    old_fp = getattr(state.catalog, "embedder_fingerprint", "?")
    _load_into_state(state, Path(src), log)
    new_fp = getattr(state.catalog, "embedder_fingerprint", "?")
    cat = state.catalog
    classes = getattr(cat, "classes", ())
    if old_fp == new_fp:
        return (
            f"reloaded {cat.name}@{cat.version} "
            f"(no fingerprint change, {len(classes)} classes)"
        )
    return (
        f"reloaded {cat.name}@{cat.version} "
        f"(fingerprint {_short(old_fp)} → {_short(new_fp)}, "
        f"{len(classes)} classes)"
    )


register(Command(
    name="reload",
    run=_cmd_reload,
    doc="Recompile and reload the current catalog from disk",
    needs_catalog=True,
    group="catalog",
))


# ---------------------------------------------------------------------------
#  :classes
# ---------------------------------------------------------------------------


def _cmd_classes(state: Any, args: list[str], log: Any) -> str:
    from ..render import render_classes
    return render_classes(state.catalog, use_color=not state.no_color)


register(Command(
    name="classes",
    run=_cmd_classes,
    doc="List the classes declared in the current catalog",
    needs_catalog=True,
    group="catalog",
))


# ---------------------------------------------------------------------------
#  :embedder [MODEL]
# ---------------------------------------------------------------------------


def _cmd_embedder(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"embedder: {state.embedder_model}"
    new = " ".join(args).strip()
    if not new:
        raise UsageError(":embedder expects a model name")
    state.embedder_model = new

    # Instantiate the new embedder (lazy — torch weights load on first encode).
    from efe_tool.errors import BridgeError
    try:
        from efe_tool.embedder import LocalSTEmbedder
        state.embedder = LocalSTEmbedder(new)
    except BridgeError as e:
        raise from_bridge_error(e) from e

    # The client holds a direct reference to the OLD embedder, which
    # would cause :classify to still validate against the old fingerprint.
    # Rebuild the client so the swap is actually effective.
    if state.client is not None:
        try:
            state.client.close()
        except Exception as e:  # pragma: no cover
            log.warn("repl_client_close_failed", error=str(e))
        state.client = None

    try:
        from efe_tool.client import EfeClient
        state.client = EfeClient(
            endpoint=state.endpoint,
            embedder=state.embedder,
            jwt=state.jwt,
            tenant=state.tenant,
            timeout_s=state.timeout_s,
            insecure=not state.tls,
        )
    except BridgeError as e:
        raise from_bridge_error(e) from e

    log.info("repl_embedder_switched", model=new)

    # The current catalog's embedder fingerprint may now mismatch. Do
    # NOT auto-reload — the user might have wanted to change embedder
    # and load a different catalog. Just warn.
    warning = ""
    if state.catalog is not None:
        fp = getattr(state.embedder, "fingerprint", lambda: "?")()
        cfp = getattr(state.catalog, "embedder_fingerprint", "?")
        if fp != cfp:
            warning = (
                "\n⚠ current catalog was compiled with a different embedder; "
                "run `:reload` or `:catalog PATH` to fix the mismatch."
            )
    return f"embedder switched to '{new}'{warning}"


register(Command(
    name="embedder",
    run=_cmd_embedder,
    doc="Without arg: show current embedder. With MODEL: switch (requires :reload).",
    group="catalog",
))


# ---------------------------------------------------------------------------
#  Internals — the shared catalog loader
# ---------------------------------------------------------------------------


def _load_into_state(state: Any, yaml_path: Path, log: Any) -> None:
    """Load ``yaml_path`` as a catalog and install it on ``state``.

    This is the single source of truth for catalog swaps — both
    ``:catalog PATH`` and ``:reload`` go through it to keep semantics
    consistent with the Lettre A classify command.
    """
    if not yaml_path.exists():
        raise CatalogNotFoundError(
            f"catalog file not found: {yaml_path}",
            hint="check the path; relative paths resolve against the shell cwd",
        )

    t0 = time.perf_counter()
    from efe_tool.catalog import AnchorCatalog
    from efe_tool.errors import BridgeError, CatalogError

    # Decide whether we can skip the embedder (cache hit).
    compiled = yaml_path.with_name(f"{yaml_path.stem}.compiled.json")
    embedder: Any = None
    if not compiled.exists():
        try:
            from efe_tool.embedder import LocalSTEmbedder
            embedder = (
                state.embedder
                if state.embedder is not None
                else LocalSTEmbedder(state.embedder_model)
            )
        except BridgeError as e:
            raise from_bridge_error(e) from e
    elif state.embedder is None:
        # Cache hit, but we still need an embedder for subsequent
        # classify calls. Try to construct one now, but don't fail
        # the reload if it's unavailable — classify will surface the
        # error later.
        try:
            from efe_tool.embedder import LocalSTEmbedder
            state.embedder = LocalSTEmbedder(state.embedder_model)
        except BridgeError:
            pass

    try:
        state.catalog = AnchorCatalog.load(yaml_path, embedder=embedder)
    except CatalogError as e:
        raise CatalogInvalidError(str(e), cause=e) from e
    except FileNotFoundError as e:
        raise CatalogNotFoundError(str(e), cause=e) from e

    log.info(
        "catalog_loaded",
        name=getattr(state.catalog, "name", "?"),
        version=getattr(state.catalog, "version", "?"),
        fingerprint=getattr(state.catalog, "embedder_fingerprint", ""),
        classes=len(getattr(state.catalog, "classes", ())),
        cache_hit=compiled.exists(),
        elapsed_ms=int((time.perf_counter() - t0) * 1000),
    )


def _short(fp: str, limit: int = 16) -> str:
    if len(fp) <= limit:
        return fp
    # Strip the common prefix `sentence-transformers/` and keep the tail.
    if fp.startswith("sentence-transformers/"):
        fp = fp[len("sentence-transformers/"):]
    if len(fp) <= limit:
        return fp
    return f"{fp[:6]}…{fp[-6:]}"
