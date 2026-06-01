"""Dagster integration for Letter D.

Provides ``efe_resource``, a Dagster :class:`ResourceDefinition` that
constructs and tears down an :class:`EfeFacade` per run (or per
execution, depending on how the job is wired).

Usage::

    from dagster import job, op
    from efe_tool.integrations.dagster import efe_resource

    @op(required_resource_keys={"efe"})
    def classify_batch(context, msgs):
        efe = context.resources.efe
        return [efe.classify(m, catalog="support_triage").as_dict() for m in msgs]

    @job(resource_defs={"efe": efe_resource})
    def nightly(): classify_batch([...])
"""

from __future__ import annotations

import logging
from typing import Any, Optional

from . import EfeConfig, EfeFacade


_log = logging.getLogger("efe_tool.integrations.dagster")


def _init_fn(init_context: Any) -> EfeFacade:
    raw = getattr(init_context, "resource_config", {}) or {}
    if raw:
        cfg = EfeConfig.from_dict({**_default_cfg_dict(), **raw})
    else:
        cfg = EfeConfig.from_env()
    facade = EfeFacade(cfg)
    facade.preload()
    _log.info("efe_facade_ready_in_dagster_resource",
              extra={"endpoint": cfg.endpoint})
    return facade


def _teardown_fn(facade: EfeFacade) -> None:
    try:
        facade.close(timeout_s=10.0)
    except Exception:  # pragma: no cover
        _log.exception("efe_facade_close_error_in_dagster_resource")


def _default_cfg_dict() -> dict[str, Any]:
    cfg = EfeConfig.from_env()
    return {
        "endpoint": cfg.endpoint,
        "catalogs": list(cfg.catalogs),
        "catalog_search_paths": list(cfg.catalog_search_paths),
        "embedder_model": cfg.embedder_model,
    }


def efe_resource() -> Any:
    """Return a Dagster :class:`ResourceDefinition` or ``None`` when
    dagster is not installed.

    Implemented as a function (not a module-level constant) so importing
    this adapter does not require dagster.
    """
    try:
        from dagster import ResourceDefinition  # type: ignore
    except ImportError as e:  # pragma: no cover
        raise RuntimeError("dagster is not installed") from e
    return ResourceDefinition(
        resource_fn=_init_fn,
        dispose_fn=_teardown_fn,
        description="EfeFacade — Letter D integration of EFE Mode 1.",
    )


__all__ = ["efe_resource"]
