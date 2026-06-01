"""Implementation of ``efe-admin scenario`` — Letter-C entrypoint.

Sub-sub-commands
----------------

* ``list``      — print every discoverable scenario (builtin + user-supplied).
* ``show``      — print a scenario's metadata and golden counts.
* ``run``       — run one user message through a scenario (live LLM call).
* ``bench``     — run every golden case of a scenario and report stats.
* ``scaffold``  — generate a skeleton directory for a new scenario.

The CLI wrapper is intentionally thin: all business logic lives in
:mod:`efe_tool.scenarios`.  This file owns argument plumbing, resolution
against the CLI's :class:`ResolvedConfig`, and rendering.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any, Optional

from ..config import ResolvedConfig
from ..errors import (
    CatalogInvalidError,
    CatalogNotFoundError,
    CliError,
    UsageError,
)
from ..logging import Logger
from ..output import render, render_error


# ---------------------------------------------------------------------------
#  Dispatcher entry point (called by cli.main._dispatch)
# ---------------------------------------------------------------------------


def run(cfg: ResolvedConfig, log: Logger) -> int:
    """Route to the selected scenario sub-sub-command."""
    action = getattr(cfg, "scenario_action", None) or \
        (cfg.extra.get("scenario_action") if isinstance(cfg.extra, dict) else None)
    if not action:
        raise UsageError(
            "scenario requires an action: list | show | run | bench | scaffold",
            hint="run `efe-admin scenario --help` for usage",
        )
    if action == "list":
        return _cmd_list(cfg, log)
    if action == "show":
        return _cmd_show(cfg, log)
    if action == "run":
        return _cmd_run(cfg, log)
    if action == "bench":
        return _cmd_bench(cfg, log)
    if action == "scaffold":
        return _cmd_scaffold(cfg, log)
    raise UsageError(f"unknown scenario action: {action!r}")


# ---------------------------------------------------------------------------
#  list
# ---------------------------------------------------------------------------


def _cmd_list(cfg: ResolvedConfig, log: Logger) -> int:
    from efe_tool.scenarios import (
        list_builtin_scenarios, load_scenario, scenario_summary,
    )
    names = list_builtin_scenarios()
    entries: list[dict[str, Any]] = []
    for n in names:
        try:
            scenario = load_scenario(n)
            entries.append(scenario_summary(scenario))
        except Exception as e:
            entries.append({"name": n, "error": str(e)})
    doc = {"schema_version": 1, "count": len(entries), "scenarios": entries}
    render(doc, format=cfg.format, pretty=cfg.pretty)
    return 0


# ---------------------------------------------------------------------------
#  show
# ---------------------------------------------------------------------------


def _cmd_show(cfg: ResolvedConfig, log: Logger) -> int:
    from efe_tool.scenarios import load_scenario, scenario_summary
    name = _scenario_name(cfg)
    scenario = _safe_load_scenario(name)
    summary = scenario_summary(scenario)
    summary["prompt_body_preview"] = _truncate(scenario.prompt.text, 240)
    doc = {"schema_version": 1, **summary,
           "golden_preview": [
               {"user": _truncate(c.user, 80),
                "expected_label": c.expected_label,
                "language": c.language}
               for c in scenario.golden[:5]
           ]}
    render(doc, format=cfg.format, pretty=cfg.pretty)
    return 0


# ---------------------------------------------------------------------------
#  run
# ---------------------------------------------------------------------------


def _cmd_run(cfg: ResolvedConfig, log: Logger) -> int:
    from efe_tool.scenarios import (
        build_transcript, load_scenario, run_scenario, write_transcript,
    )
    name = _scenario_name(cfg)
    user_text = getattr(cfg, "scenario_user_text", None) or \
        (cfg.extra.get("scenario_user_text") if isinstance(cfg.extra, dict) else None)
    if not user_text:
        raise UsageError(
            "scenario run requires --user TEXT (or stdin)",
            hint="try `efe-admin scenario run NAME --user 'your message'`",
        )

    scenario = _safe_load_scenario(name)

    log.info(
        "scenario_run_start",
        scenario=scenario.name,
        prompt_hash=scenario.prompt.hash,
        model=scenario.prompt.model,
    )
    try:
        result = run_scenario(
            scenario,
            user=str(user_text),
            cost_ceiling_usd=_cost_ceiling(cfg),
            max_rounds=_max_rounds(cfg),
        )
    except CliError:
        raise
    except Exception as e:
        raise CliError(
            f"unexpected error running scenario: {type(e).__name__}: {e}",
            cause=e,
        ) from e

    transcript = build_transcript(
        result=result.conversation,
        scenario=scenario,
        boot_config=result.boot_config,
        started_at=result.started_at,
        ended_at=result.ended_at,
        redact_user=_redact_user(cfg),
    )

    # Stdout: JSON summary with final_text + stats (always parseable).
    doc = {
        "schema_version": 1,
        "final_text": result.conversation.final_text,
        "scenario": scenario.name,
        "model": result.conversation.model,
        "stats": result.conversation.stats.as_dict(),
        "wall_clock_ms": result.conversation.wall_clock_ms,
    }
    render(doc, format=cfg.format, pretty=cfg.pretty)

    # Cost line on stderr (human feedback).
    print(result.conversation.stats.summary_line(), file=sys.stderr)

    # Optional transcript dump to disk.
    transcript_path = getattr(cfg, "scenario_transcript_path", None) or \
        (cfg.extra.get("scenario_transcript_path") if isinstance(cfg.extra, dict) else None)
    if transcript_path:
        write_transcript(transcript, transcript_path)
        log.info("scenario_transcript_written", path=str(transcript_path))

    log.info(
        "scenario_run_done",
        scenario=scenario.name,
        rounds=result.conversation.stats.rounds,
        tool_call_count=result.conversation.stats.tool_call_count,
        cost_usd=round(result.conversation.stats.cost_usd, 6),
        wall_clock_ms=result.conversation.wall_clock_ms,
    )
    return 0


# ---------------------------------------------------------------------------
#  bench
# ---------------------------------------------------------------------------


def _cmd_bench(cfg: ResolvedConfig, log: Logger) -> int:
    """Bench every golden case. Requires LLM access (real or mock).

    The CLI path only supports the *real* API — mocks are for unit tests.
    If the user wants a mock bench, they invoke the library directly.
    """
    from efe_tool.scenarios import bench_scenario, load_scenario

    name = _scenario_name(cfg)
    scenario = _safe_load_scenario(name)
    if not scenario.golden:
        raise UsageError(
            f"scenario {name!r} has no golden cases",
            hint="add examples to golden.json first",
        )

    from ..errors import from_bridge_error
    try:
        from efe_tool.scenarios.runner import (
            _make_anthropic, _make_efe_client,
            default_catalog_resolver,
        )
        anthropic_client = _make_anthropic()
        efe_client = _make_efe_client()
        resolver = default_catalog_resolver()
        catalogs = {n: resolver(n) for n in scenario.prompt.catalog_names}
    except CliError:
        raise
    except Exception as e:
        raise from_bridge_error(e) from e

    log.info("scenario_bench_start", scenario=scenario.name,
             case_count=len(scenario.golden))
    report = bench_scenario(
        scenario,
        anthropic_client=anthropic_client,
        efe_client=efe_client,
        catalogs=catalogs,
        cost_ceiling_usd=_cost_ceiling(cfg),
        max_rounds=_max_rounds(cfg),
    )
    doc = {"schema_version": 1, **report.as_dict()}
    render(doc, format=cfg.format, pretty=cfg.pretty)
    print(report.summary_line(), file=sys.stderr)
    log.info("scenario_bench_done", **report.as_dict())
    return 0 if report.errored == 0 else 6  # SERVER_ERROR if any case errored


# ---------------------------------------------------------------------------
#  scaffold
# ---------------------------------------------------------------------------


def _cmd_scaffold(cfg: ResolvedConfig, log: Logger) -> int:
    from efe_tool.scenarios import scaffold_scenario, validate_name

    name = _scenario_name(cfg)
    try:
        validate_name(name)
    except ValueError as e:
        raise UsageError(str(e)) from e
    target = cfg.out_path or Path("scenarios")
    default_catalog = _scaffold_catalog(cfg) or "my_catalog"
    force = cfg.force
    log.info("scenario_scaffold_start",
             name=name, target=str(target), force=force)
    try:
        files = scaffold_scenario(
            name, target_dir=target,
            default_catalog=default_catalog, force=force,
        )
    except FileExistsError as e:
        raise UsageError(
            str(e), hint="pass --force to overwrite",
        ) from e

    doc = {
        "schema_version": 1,
        "scenario": name,
        "target_dir": str(target / name),
        "files": [str(f) for f in files],
    }
    render(doc, format=cfg.format, pretty=cfg.pretty)
    log.info("scenario_scaffold_done",
             name=name, file_count=len(files))
    return 0


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _scenario_name(cfg: ResolvedConfig) -> str:
    name = getattr(cfg, "scenario_name", None) or \
        (cfg.extra.get("scenario_name") if isinstance(cfg.extra, dict) else None)
    if not name:
        raise UsageError(
            "scenario command requires NAME (e.g. support_triage_bilingual)",
        )
    return str(name)


def _cost_ceiling(cfg: ResolvedConfig) -> Optional[float]:
    raw = getattr(cfg, "scenario_cost_ceiling_usd", None) or \
        (cfg.extra.get("scenario_cost_ceiling_usd") if isinstance(cfg.extra, dict) else None)
    if raw is None:
        return None
    try:
        v = float(raw)
    except (TypeError, ValueError):
        raise UsageError(f"invalid --cost-ceiling-usd value: {raw!r}")
    if v <= 0:
        raise UsageError("cost-ceiling-usd must be > 0")
    return v


def _max_rounds(cfg: ResolvedConfig) -> int:
    raw = getattr(cfg, "scenario_max_rounds", None) or \
        (cfg.extra.get("scenario_max_rounds") if isinstance(cfg.extra, dict) else None)
    if raw is None:
        from efe_tool.bridges.claude import DEFAULT_MAX_ROUNDS
        return DEFAULT_MAX_ROUNDS
    try:
        v = int(raw)
    except (TypeError, ValueError):
        raise UsageError(f"invalid --max-rounds value: {raw!r}")
    if not 1 <= v <= 64:
        raise UsageError("--max-rounds must be in [1, 64]")
    return v


def _redact_user(cfg: ResolvedConfig) -> bool:
    return bool(getattr(cfg, "scenario_redact_user", False) or
                (cfg.extra.get("scenario_redact_user") if isinstance(cfg.extra, dict) else False))


def _scaffold_catalog(cfg: ResolvedConfig) -> Optional[str]:
    raw = getattr(cfg, "scenario_scaffold_catalog", None) or \
        (cfg.extra.get("scenario_scaffold_catalog") if isinstance(cfg.extra, dict) else None)
    return str(raw) if raw else None


def _safe_load_scenario(name: str):
    from efe_tool.scenarios import load_scenario
    try:
        return load_scenario(name)
    except FileNotFoundError as e:
        raise CatalogNotFoundError(str(e), cause=e) from e
    except ValueError as e:
        raise CatalogInvalidError(str(e), cause=e) from e


def _truncate(s: str, n: int) -> str:
    s = s or ""
    return s if len(s) <= n else s[:n - 1] + "…"


__all__ = ["run"]
