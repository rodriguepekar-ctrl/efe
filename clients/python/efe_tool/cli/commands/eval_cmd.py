"""Implementation of ``efe-admin eval``.

Thin wrapper around ``eval/scripts/run_eval.py`` in the repo. Preserves
the AS-IS behaviour: locate the script by path, import it via
:mod:`importlib`, forward arguments.

Renamed ``eval_cmd.py`` to avoid shadowing Python's builtin ``eval``
name in tooling.
"""

from __future__ import annotations

import importlib.util
from pathlib import Path
from typing import Any

from ..config import ResolvedConfig
from ..errors import CatalogNotFoundError, UsageError
from ..logging import Logger


def run(cfg: ResolvedConfig, log: Logger) -> int:
    if cfg.catalog_path is None:
        raise UsageError("eval: --catalog is required")
    if cfg.golden_path is None:
        raise UsageError("eval: --golden is required")
    if not Path(cfg.catalog_path).exists():
        raise CatalogNotFoundError(f"catalog not found: {cfg.catalog_path}")
    if not Path(cfg.golden_path).exists():
        raise CatalogNotFoundError(
            f"golden set not found: {cfg.golden_path}",
            context={"path": str(cfg.golden_path)},
        )

    # Locate the evaluation runner. We walk up from the package root
    # to find a sibling ``eval/scripts/run_eval.py``. This matches the
    # AS-IS heuristic in the legacy cli.py.
    script_path = _find_eval_script()
    if script_path is None:
        raise UsageError(
            "eval runner not found (expected eval/scripts/run_eval.py in the repo)",
            hint="run from inside the EFE checkout, or set EFE_EVAL_SCRIPT",
        )

    log.info(
        "eval_start",
        script=str(script_path),
        catalog=str(cfg.catalog_path),
        golden=str(cfg.golden_path),
    )

    spec = importlib.util.spec_from_file_location(
        "efe_eval_run_eval", script_path
    )
    if spec is None or spec.loader is None:
        raise UsageError(
            f"cannot import eval script: {script_path}",
            hint="check file permissions and python version",
        )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    code = int(module.run(  # type: ignore[attr-defined]
        catalog=str(cfg.catalog_path),
        golden=str(cfg.golden_path),
        endpoint=cfg.endpoint,
        output=str(cfg.out_path) if cfg.out_path else None,
        verbose=cfg.log_level == "debug",
    ))

    log.info("eval_done", exit=code)
    return code


def _find_eval_script() -> Path | None:
    import os
    env = os.environ.get("EFE_EVAL_SCRIPT")
    if env:
        p = Path(env)
        if p.exists():
            return p

    here = Path(__file__).resolve()
    for parent in here.parents:
        candidate = parent / "eval" / "scripts" / "run_eval.py"
        if candidate.exists():
            return candidate
    return None


__all__ = ["run"]
