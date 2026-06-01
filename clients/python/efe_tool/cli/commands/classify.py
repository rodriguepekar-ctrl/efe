"""Implementation of ``efe-admin classify``.

Pipeline (cf. ``LETTER_A_ARCHITECTURE.md`` §6):

    1. Resolve the text source (--text | --text-file | --stdin).
    2. Lazy-import :mod:`efe_tool.catalog` and load the catalog.
       The embedder is loaded only if the compiled cache is missing.
    3. Lazy-import :mod:`efe_tool.client` and open a gRPC channel.
    4. Call :meth:`EfeClient.classify`.
    5. Evaluate the strict-mode gates.
    6. Render the output document on stdout.
    7. Close the client.

Each step logs one structured event on stderr. Errors are mapped
through :func:`efe_tool.cli.errors.from_bridge_error`.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

from ..config import ResolvedConfig, harmonise_budget_and_timeout
from ..errors import (
    CatalogInvalidError,
    CatalogNotFoundError,
    CliError,
    LowConfidenceError,
    RejectedError,
    UsageError,
    from_bridge_error,
)
from ..io_input import read_text_input, redact
from ..logging import Logger
from ..output import (
    DocTiming,
    build_classify_document,
    render,
)


def run(cfg: ResolvedConfig, log: Logger) -> int:
    """Dispatcher entry point for the ``classify`` subcommand."""
    cfg = harmonise_budget_and_timeout(cfg, logger=log)

    text = _resolve_text(cfg, log)
    log.info(
        "classify_start",
        endpoint=cfg.endpoint,
        catalog=str(cfg.catalog_path) if cfg.catalog_path else None,
        text=redact(text),
        budget_ms=cfg.budget_ms,
        max_rounds=cfg.max_rounds,
    )

    catalog = _load_catalog(cfg, log)

    # Lazy import: this is where grpcio/numpy get pulled in.
    from efe_tool.client import EfeClient
    from efe_tool.errors import BridgeError

    client = _build_client(cfg, catalog, log)
    t0 = time.perf_counter()
    try:
        try:
            result = client.classify(
                text,
                catalog,
                request_id=cfg.request_id,
                return_sub_scores=cfg.with_diagnostics,
                max_rounds=cfg.max_rounds,
                budget_wall_clock_ms=cfg.budget_ms,
            )
        except BridgeError as e:
            raise from_bridge_error(e) from e
    finally:
        _safe_close(client, log)
    elapsed_ms = int((time.perf_counter() - t0) * 1000)

    _apply_strict_gate(cfg, catalog, result, log)

    doc = build_classify_document(
        result,
        catalog=catalog,
        timing=DocTiming(
            elapsed_ms=elapsed_ms,
            server_ms=int(getattr(result, "total_ms", 0) or 0),
        ),
        request_id=cfg.request_id,
        with_diagnostics=cfg.with_diagnostics,
    )
    render(doc, format=cfg.format, pretty=cfg.pretty)

    log.info(
        "classify_done",
        label=doc["label"],
        confidence=doc["confidence"],
        reject=doc["reject"],
        rounds=doc["rounds"],
        elapsed_ms=elapsed_ms,
    )
    return 0


# ---------------------------------------------------------------------------
#  Text resolution with backward compatibility shim
# ---------------------------------------------------------------------------


def _resolve_text(cfg: ResolvedConfig, log: Logger) -> str:
    """Resolve the text to classify from the three possible sources.

    Emits a ``deprecated_positional_text`` warning whenever the legacy
    ``efe-admin classify "..."`` positional form was used. The resolver
    (:mod:`efe_tool.cli.config`) has already folded that positional
    into :attr:`ResolvedConfig.text` **and** preserved it verbatim in
    :attr:`ResolvedConfig.legacy_text`, so we can detect the shim here
    without re-parsing anything.
    """
    if cfg.legacy_text is not None:
        log.warn(
            "deprecated_positional_text",
            detail=(
                "the positional TEXT argument is deprecated; use "
                "--text, --text-file, or --stdin instead. It will be "
                "removed in efe-tool 2.0."
            ),
        )

    try:
        return read_text_input(
            text=cfg.text,
            text_file=cfg.text_file,
            use_stdin=cfg.use_stdin,
            logger=log,
        )
    except UsageError:
        raise
    except Exception as e:  # pragma: no cover — unexpected
        raise UsageError(f"could not read input text: {e}") from e


# ---------------------------------------------------------------------------
#  Catalog loading
# ---------------------------------------------------------------------------


def _load_catalog(cfg: ResolvedConfig, log: Logger) -> Any:
    if cfg.catalog_path is None:
        raise UsageError(
            "--catalog is required",
            hint="pass a YAML catalog path or set EFE_CATALOG_PATH",
        )
    if not Path(cfg.catalog_path).exists():
        raise CatalogNotFoundError(
            f"catalog file not found: {cfg.catalog_path}",
            hint="check --catalog or EFE_CATALOG_PATH",
        )

    t0 = time.perf_counter()
    from efe_tool.catalog import AnchorCatalog
    from efe_tool.errors import CatalogError

    # Determine whether we must load an embedder. AnchorCatalog.load
    # tolerates ``embedder=None`` when a fresh ``.compiled.json`` is
    # present. We leverage that to skip the ~1.5 s torch warm-up in the
    # common case (§11.2 of the architecture).
    yaml_path = Path(cfg.catalog_path)
    compiled = yaml_path.with_suffix("") \
        .with_name(f"{yaml_path.stem}.compiled.json")
    if compiled.exists():
        embedder = None
        log.debug("catalog_cache_hit", path=str(compiled))
    else:
        log.debug("catalog_cache_miss", path=str(compiled))
        from efe_tool.errors import BridgeError
        try:
            from efe_tool.embedder import LocalSTEmbedder
            embedder = LocalSTEmbedder(cfg.embedder_model)
        except BridgeError as e:
            # ConfigError (missing sentence-transformers), EmbedderError,
            # etc. — route through the CLI taxonomy.
            raise from_bridge_error(e) from e
        except ImportError as e:
            raise CatalogInvalidError(
                f"cannot import embedder module: {e}",
                hint=(
                    "install the optional compile dependencies: "
                    "`pip install efe-tool[compile]` (once published)."
                ),
                cause=e,
            ) from e

    try:
        catalog = AnchorCatalog.load(yaml_path, embedder=embedder)
    except CatalogError as e:
        raise CatalogInvalidError(str(e), cause=e) from e
    except FileNotFoundError as e:
        raise CatalogNotFoundError(str(e), cause=e) from e

    log.info(
        "catalog_loaded",
        name=getattr(catalog, "name", "?"),
        version=getattr(catalog, "version", "?"),
        fingerprint=getattr(catalog, "embedder_fingerprint", ""),
        classes=len(getattr(catalog, "classes", ())),
        cache_hit=compiled.exists(),
        elapsed_ms=int((time.perf_counter() - t0) * 1000),
    )
    return catalog


# ---------------------------------------------------------------------------
#  Client construction
# ---------------------------------------------------------------------------


def _build_client(cfg: ResolvedConfig, catalog: Any, log: Logger) -> Any:
    """Open a gRPC client against ``cfg.endpoint``.

    We reuse the catalog's embedder fingerprint to drive
    :class:`LocalSTEmbedder`: if the catalog was loaded from a compiled
    cache, we need a minimal embedder instance that advertises the
    same fingerprint, because :meth:`EfeClient.classify` validates
    them against each other. The lowest-cost way to achieve this is
    to re-instantiate ``LocalSTEmbedder`` with the declared model;
    the instance will lazy-load the torch weights on the first
    ``.encode()`` call.
    """
    t0 = time.perf_counter()
    from efe_tool.errors import BridgeError

    try:
        from efe_tool.embedder import LocalSTEmbedder
        embedder = LocalSTEmbedder(cfg.embedder_model)
    except BridgeError as e:
        raise from_bridge_error(e) from e

    from efe_tool.client import EfeClient

    try:
        client = EfeClient(
            endpoint=cfg.endpoint,
            embedder=embedder,
            jwt=cfg.jwt,
            tenant=cfg.tenant,
            timeout_s=cfg.timeout_s,
            insecure=not cfg.tls,
        )
    except BridgeError as e:
        raise from_bridge_error(e) from e
    log.debug(
        "grpc_client_opened",
        endpoint=cfg.endpoint,
        tls=cfg.tls,
        timeout_s=cfg.timeout_s,
        elapsed_ms=int((time.perf_counter() - t0) * 1000),
    )
    return client


def _safe_close(client: Any, log: Logger) -> None:
    try:
        client.close()
    except Exception as e:  # pragma: no cover
        log.warn("grpc_close_failed", error=str(e))


# ---------------------------------------------------------------------------
#  Strict-mode gate
# ---------------------------------------------------------------------------


def _apply_strict_gate(
    cfg: ResolvedConfig,
    catalog: Any,
    result: Any,
    log: Logger,
) -> None:
    if not cfg.strict:
        return
    reject = bool(getattr(result, "reject", False))
    confidence = float(getattr(result, "confidence", 0.0))
    high = float(getattr(
        getattr(catalog, "thresholds", None), "confidence_high", 1.0
    ))

    if reject:
        raise RejectedError(
            f"classification rejected (confidence={confidence:.3f})",
            hint=(
                "the swarm could not commit to a label above the catalog's "
                "confidence_medium threshold"
            ),
            context={"confidence": confidence},
        )
    if confidence < high:
        raise LowConfidenceError(
            f"confidence {confidence:.3f} below confidence_high {high:.3f}",
            hint="raise --max-rounds or revise the catalog",
            context={"confidence": confidence, "confidence_high": high},
        )


__all__ = ["run"]
