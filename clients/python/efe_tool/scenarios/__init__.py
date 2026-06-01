"""``efe_tool.scenarios`` — Letter-C scenario library (public API).

See ``LETTER_C_ARCHITECTURE.md`` §16 for the design.

Minimal surface::

    from efe_tool.scenarios import (
        Scenario, Prompt, GoldenCase,
        load_prompt, load_scenario, list_builtin_scenarios,
        scenario_summary,
        run_scenario, ScenarioRunResult,
        bench_scenario, BenchReport,
        scaffold_scenario,
        build_transcript, write_transcript,
    )

The public symbols fall into four groups:

* **Loading**      — ``load_prompt``, ``load_scenario``, discovery.
* **Running**      — ``run_scenario`` orchestrating Anthropic + EFE.
* **Benching**     — ``bench_scenario`` over golden cases.
* **Authoring**    — ``scaffold_scenario`` for new scenarios.
* **Persisting**   — ``build_transcript`` / ``write_transcript``.
"""

from __future__ import annotations

from .bench import BenchCaseOutcome, BenchReport, bench_scenario
from .loader import (
    GoldenCase,
    Prompt,
    Scenario,
    list_builtin_scenarios,
    load_golden,
    load_prompt,
    load_scenario,
    resolve_scenario_dir,
    scenario_summary,
)
from .runner import (
    CatalogResolver,
    ScenarioRunResult,
    default_catalog_resolver,
    run_scenario,
)
from .scaffold import scaffold_scenario, validate_name
from .transcript import (
    TRANSCRIPT_SCHEMA_VERSION,
    build_transcript,
    write_transcript,
)


__all__ = [
    # Loading
    "Scenario",
    "Prompt",
    "GoldenCase",
    "load_prompt",
    "load_golden",
    "load_scenario",
    "list_builtin_scenarios",
    "resolve_scenario_dir",
    "scenario_summary",
    # Running
    "ScenarioRunResult",
    "CatalogResolver",
    "default_catalog_resolver",
    "run_scenario",
    # Benching
    "BenchCaseOutcome",
    "BenchReport",
    "bench_scenario",
    # Authoring
    "scaffold_scenario",
    "validate_name",
    # Persisting
    "TRANSCRIPT_SCHEMA_VERSION",
    "build_transcript",
    "write_transcript",
]
