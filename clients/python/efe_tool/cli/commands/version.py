"""Implementation of ``efe-admin version``.

Unlike ``--version`` (which uses argparse's built-in action and prints
a non-structured string), the subcommand produces a schema-versioned
document that can be consumed by pipelines.
"""

from __future__ import annotations

from efe_tool import __version__

from ..config import ResolvedConfig
from ..logging import Logger
from ..output import build_version_document, render


def run(cfg: ResolvedConfig, log: Logger) -> int:
    log.debug("version_start")
    doc = build_version_document(__version__)
    render(doc, format=cfg.format, pretty=cfg.pretty)
    return 0


__all__ = ["run"]
