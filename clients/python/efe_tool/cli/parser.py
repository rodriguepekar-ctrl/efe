"""Argparse parser construction for the ``efe-admin`` CLI.

Every flag declares ``default=MISSING`` (the sentinel exported from
:mod:`efe_tool.cli.config`). The resolver then applies the full
precedence chain (flag > env > default) deterministically.

Why not click/typer?
    * Zero extra runtime dependency.
    * Full control over the help string (layout, grouping, examples).
    * Parsing takes under 15 ms cold on a M3 Pro — well below the
      150 ms cold-start budget (§11 of the architecture).
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
from typing import Any, Sequence

from efe_tool import __version__

from .config import MISSING
from .errors import UsageError
from .metadata import (
    DEFAULTS,
    LOG_FORMATS,
    LOG_LEVELS,
    OUTPUT_FORMATS,
    PROG_CANONICAL,
    PROG_DESCRIPTION,
    PROG_TAGLINE,
    PROGRESS_MODES,
)


# ---------------------------------------------------------------------------
#  Typed helpers
# ---------------------------------------------------------------------------


def _positive_int(raw: str) -> int:
    try:
        v = int(raw)
    except ValueError:
        raise argparse.ArgumentTypeError(f"not an integer: {raw!r}")
    if v <= 0:
        raise argparse.ArgumentTypeError(f"must be > 0, got {v}")
    return v


def _positive_float(raw: str) -> float:
    try:
        v = float(raw)
    except ValueError:
        raise argparse.ArgumentTypeError(f"not a float: {raw!r}")
    if v <= 0:
        raise argparse.ArgumentTypeError(f"must be > 0, got {v}")
    return v


def _bounded_int(lo: int, hi: int) -> Any:
    def parse(raw: str) -> int:
        try:
            v = int(raw)
        except ValueError:
            raise argparse.ArgumentTypeError(f"not an integer: {raw!r}")
        if not lo <= v <= hi:
            raise argparse.ArgumentTypeError(
                f"must be in [{lo}, {hi}], got {v}"
            )
        return v
    return parse


def _existing_path(raw: str) -> Path:
    p = Path(raw)
    if not p.exists():
        raise argparse.ArgumentTypeError(f"path does not exist: {raw}")
    return p


# ---------------------------------------------------------------------------
#  A custom ArgumentParser that raises UsageError instead of exiting
# ---------------------------------------------------------------------------


class _RaisingParser(argparse.ArgumentParser):
    """An ``ArgumentParser`` that raises :class:`UsageError` on parse
    errors, rather than calling ``sys.exit(2)``.

    This lets the top-level dispatcher in :mod:`efe_tool.cli.main` treat
    usage errors like any other :class:`CliError` and render them
    through the canonical formatter.

    We still honour ``--help`` and ``--version`` by letting argparse
    call :meth:`exit` for them; those are non-error terminations.
    """

    def error(self, message: str) -> None:  # type: ignore[override]
        raise UsageError(
            message,
            hint=f"run `{self.prog} --help` for usage",
        )


# ---------------------------------------------------------------------------
#  Builder
# ---------------------------------------------------------------------------


def build_parser(prog: str = PROG_CANONICAL) -> argparse.ArgumentParser:
    """Construct the full argparse tree for ``efe-admin``.

    Global flags (``--log-level``, ``--log-format``, ``--quiet``,
    ``--no-color``, ``--request-id``) are accepted **both** before and
    after the subcommand token. This matches the ergonomics of
    kubectl / gh / git and avoids the classic argparse footgun where
    ``efe-admin validate x.yaml --log-format json`` errored out. The
    mechanism is an argparse ``parents=[…]`` injection that's shared
    across every subparser.
    """
    global_parent = _build_global_parent()

    parser = _RaisingParser(
        prog=prog,
        description=PROG_DESCRIPTION,
        epilog=(
            PROG_TAGLINE + "\n\n"
            "Examples:\n"
            f"  {prog} classify --text 'hello' --catalog catalogs/support_triage.yaml\n"
            f"  {prog} healthcheck --endpoint 127.0.0.1:50051\n"
            f"  echo 'hello' | {prog} classify --stdin --catalog c.yaml\n"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        parents=[global_parent],
    )
    parser.add_argument(
        "--version", action="version",
        version=f"%(prog)s {__version__}",
    )

    sub = parser.add_subparsers(
        dest="cmd",
        required=True,
        metavar="COMMAND",
        title="Subcommands",
    )

    _build_classify(sub, global_parent)
    _build_compile_catalog(sub, global_parent)
    _build_validate(sub, global_parent)
    _build_healthcheck(sub, global_parent)
    _build_eval(sub, global_parent)
    _build_version(sub, global_parent)
    _build_completions(sub, global_parent)
    _build_repl(sub, global_parent)
    _build_scenario(sub, global_parent)
    _build_catalog(sub, global_parent)

    return parser


def _build_global_parent() -> argparse.ArgumentParser:
    """Parent parser carrying every global flag.

    Returned with ``add_help=False`` so that every subparser can
    ``parents=[it]`` without duplicating the ``-h`` action.
    """
    gp = argparse.ArgumentParser(add_help=False)
    _add_global_flags(gp)
    return gp


# ---------------------------------------------------------------------------
#  Global flags
# ---------------------------------------------------------------------------


def _add_global_flags(parser: argparse.ArgumentParser) -> None:
    """Install the global flags on ``parser``.

    We use ``default=argparse.SUPPRESS`` rather than our ``MISSING``
    sentinel here because these flags are inherited by every subparser
    via ``parents=[global_parent]``. With a regular default, the
    subparser would *reset* the attribute to the default when parsing,
    overwriting any value set by the main parser. ``SUPPRESS`` tells
    argparse not to set the attribute at all when the flag isn't
    passed — so ``getattr(args, key, MISSING)`` still returns MISSING
    and the resolver behaves identically.
    """
    g = parser.add_argument_group("logging")
    g.add_argument(
        "--log-level", dest="log_level",
        choices=LOG_LEVELS, default=argparse.SUPPRESS,
        help=f"log level on stderr (default: {DEFAULTS['log_level']})",
    )
    g.add_argument(
        "--log-format", dest="log_format",
        choices=LOG_FORMATS, default=argparse.SUPPRESS,
        help=f"log format on stderr (default: {DEFAULTS['log_format']})",
    )
    g.add_argument(
        "--quiet", "-q", dest="quiet",
        action="store_const", const=True, default=argparse.SUPPRESS,
        help="shortcut for --log-level error",
    )
    g.add_argument(
        "--no-color", dest="no_color",
        action="store_const", const=True, default=argparse.SUPPRESS,
        help="disable colored stderr (NO_COLOR honored too)",
    )
    g.add_argument(
        "--request-id", dest="request_id",
        default=argparse.SUPPRESS,
        help="client-chosen request id; defaults to a fresh UUID",
    )
    g.add_argument(
        "--progress", dest="progress",
        choices=PROGRESS_MODES, default=argparse.SUPPRESS,
        help=(
            f"stderr progress indicator mode (default: {DEFAULTS['progress']}). "
            "'auto' shows a spinner on TTY for operations > 500 ms."
        ),
    )


def _add_network_flags(p: argparse.ArgumentParser) -> None:
    """Flags shared by every subcommand that talks gRPC."""
    p.add_argument(
        "--endpoint", dest="endpoint", default=MISSING,
        help=f"gRPC endpoint (default: {DEFAULTS['endpoint']})",
    )
    p.add_argument(
        "--tls", dest="tls", action="store_const", const=True, default=MISSING,
        help="enable TLS for the gRPC channel",
    )
    p.add_argument(
        "--tls-ca", dest="tls_ca", default=MISSING,
        help="custom CA bundle path (implies --tls)",
    )
    p.add_argument(
        "--jwt", dest="jwt", default=MISSING,
        help="bearer JWT for authorization header",
    )
    p.add_argument(
        "--tenant", dest="tenant", default=MISSING,
        help="tenant id for the x-tenant-id metadata",
    )


def _add_output_flags(p: argparse.ArgumentParser) -> None:
    """Flags shared by every subcommand that produces a stdout document."""
    p.add_argument(
        "--format", dest="format",
        choices=OUTPUT_FORMATS, default=MISSING,
        help=f"stdout format (default: {DEFAULTS['format']})",
    )
    p.add_argument(
        "--pretty", dest="pretty",
        action="store_const", const=True, default=MISSING,
        help="indent JSON output for humans",
    )


# ---------------------------------------------------------------------------
#  classify
# ---------------------------------------------------------------------------


def _build_classify(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "classify",
        help="Classify a single text via the EFE server.",
        description="Classify a text against a catalog and print the verdict.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  efe-admin classify --text 'hello' --catalog catalogs/x.yaml\n"
            "  echo 'hello' | efe-admin classify --stdin --catalog x.yaml\n"
            "  efe-admin classify --text-file msg.txt --catalog x.yaml --pretty\n"
        ),
        parents=[global_parent],
    )

    # Input source — mutually exclusive
    src = p.add_argument_group("input source (exactly one)")
    src.add_argument("--text", dest="text", default=MISSING,
                     help="inline text to classify")
    src.add_argument("--text-file", dest="text_file", default=MISSING,
                     help="UTF-8 file to classify")
    src.add_argument("--stdin", dest="stdin",
                     action="store_const", const=True, default=MISSING,
                     help="read text from stdin until EOF")
    # Back-compat: allow a positional TEXT with a deprecation shim in
    # classify.run. We accept zero-or-one positional so `--stdin` alone
    # keeps working.
    p.add_argument("legacy_text", nargs="?", default=None,
                   help=argparse.SUPPRESS)

    # Catalog
    cat = p.add_argument_group("catalog")
    cat.add_argument("--catalog", dest="catalog_path",
                     default=MISSING,
                     help="catalog YAML path (required unless EFE_CATALOG_PATH is set)")
    cat.add_argument("--embedder", dest="embedder_model",
                     default=MISSING,
                     help="sentence-transformers model name (must match the catalog)")

    # Tuning
    tun = p.add_argument_group("tuning")
    tun.add_argument("--budget-ms", dest="budget_ms",
                     type=_positive_int, default=MISSING,
                     help=f"swarm wall-clock budget in ms (default {DEFAULTS['budget_ms']})")
    tun.add_argument("--timeout-s", dest="timeout_s",
                     type=_positive_float, default=MISSING,
                     help=f"gRPC client timeout in seconds (default {DEFAULTS['timeout_s']})")
    tun.add_argument("--max-rounds", dest="max_rounds",
                     type=_bounded_int(1, 256), default=MISSING,
                     help=f"max fusion rounds in [1,256] (default {DEFAULTS['max_rounds']})")
    tun.add_argument("--strict", dest="strict",
                     action="store_const", const=True, default=MISSING,
                     help="non-zero exit if reject=true or confidence<high")

    # Output
    _add_output_flags(p)
    p.add_argument("--with-diagnostics", dest="with_diagnostics",
                   action="store_const", const=True, default=MISSING,
                   help="include debug fields (sub_scores, swarm_id, …) in JSON output")

    # Network (shared)
    _add_network_flags(p)


# ---------------------------------------------------------------------------
#  compile-catalog
# ---------------------------------------------------------------------------


def _build_compile_catalog(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "compile-catalog",
        help="Compile a YAML catalog into anchors and cache it.",
        description=(
            "Encode canonical sentences using the declared embedder, compute "
            "anchor centroids, and write the `.compiled.json` cache next to "
            "the YAML file."
        ),
        parents=[global_parent],
    )
    p.add_argument("path", help="catalog YAML path")
    p.add_argument("--embedder", dest="embedder_model", default=MISSING,
                   help="sentence-transformers model name")
    p.add_argument("--out", dest="out", default=MISSING,
                   help="override cache output path (default: next to YAML)")
    p.add_argument("--force", dest="force",
                   action="store_const", const=True, default=MISSING,
                   help="recompile even if cache is fresh")
    _add_output_flags(p)


# ---------------------------------------------------------------------------
#  validate
# ---------------------------------------------------------------------------


def _build_validate(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "validate",
        help="Validate a catalog YAML without loading an embedder.",
        description=(
            "Lint a catalog YAML against the documented schema "
            "(apiVersion, kind, classes, thresholds, cone_policy). Fast; does "
            "not require a running server or a sentence-transformers install."
        ),
        parents=[global_parent],
    )
    p.add_argument("path", help="catalog YAML path")
    p.add_argument("--strict-thresholds", dest="strict_thresholds",
                   action="store_const", const=True, default=MISSING,
                   help="warn when confidence_high - confidence_medium < 0.05")
    _add_output_flags(p)


# ---------------------------------------------------------------------------
#  healthcheck
# ---------------------------------------------------------------------------


def _build_healthcheck(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "healthcheck",
        help="Probe the EFE server health (grpc-health or DescribeSpec).",
        description="Return SERVING / NOT_SERVING. Exit 0 if serving, else 5.",
        parents=[global_parent],
    )
    _add_network_flags(p)
    p.add_argument("--timeout-s", dest="timeout_s",
                   type=_positive_float, default=MISSING,
                   help="per-call timeout (default 2.0)")
    _add_output_flags(p)


# ---------------------------------------------------------------------------
#  eval
# ---------------------------------------------------------------------------


def _build_eval(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "eval",
        help="Run a golden-set evaluation and print metrics.",
        description=(
            "Delegate to `eval/scripts/run_eval.py` and bridge its metrics "
            "to the CLI schema. Useful for CI gates."
        ),
        parents=[global_parent],
    )
    p.add_argument("--catalog", dest="catalog_path",
                   default=MISSING, required=False,
                   help="catalog YAML path")
    p.add_argument("--golden", dest="golden",
                   default=MISSING,
                   help="golden-set JSON or JSONL path")
    # Accept both ``--out`` (new canonical) and ``--output`` (AS-IS
    # backwards compat). Both store to the same dest so downstream
    # code reads a single field.
    p.add_argument("--out", "--output", dest="out", default=MISSING,
                   help="optional JSON report output path")
    _add_network_flags(p)
    _add_output_flags(p)


# ---------------------------------------------------------------------------
#  version
# ---------------------------------------------------------------------------


def _build_version(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "version",
        help="Print the package version (JSON-aware).",
        description=(
            "Equivalent to `--version`, but works as a subcommand so it "
            "can produce a structured document when --format json is "
            "requested (useful for runbooks)."
        ),
        parents=[global_parent],
    )
    _add_output_flags(p)


# ---------------------------------------------------------------------------
#  completions
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
#  repl
# ---------------------------------------------------------------------------


def _build_repl(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "repl",
        help="Start an interactive REPL session (Letter B).",
        description=(
            "Launch a persistent process that loads the embedder and "
            "gRPC client once, then reads classification queries "
            "interactively. Commands prefixed with ':' switch catalog, "
            "reload, change tuning parameters, etc. Type :help inside "
            "the REPL for the full command list."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  efe-admin repl --catalog catalogs/support_triage.yaml\n"
            "  efe-admin repl  # start with no catalog, :catalog PATH inside\n"
            "  echo -e 'hello\\n:quit' | efe-admin repl --catalog x.yaml\n"
        ),
        parents=[global_parent],
    )

    # Catalog (optional — REPL can boot without one and accept :catalog later)
    p.add_argument("--catalog", dest="catalog_path", default=MISSING,
                   help="optional catalog YAML path to load at boot")
    p.add_argument("--embedder", dest="embedder_model", default=MISSING,
                   help="sentence-transformers model name")

    # Classification tuning (initial values, mutable via :budget etc.)
    p.add_argument("--budget-ms", dest="budget_ms",
                   type=_positive_int, default=MISSING,
                   help=f"initial swarm wall-clock budget (default {DEFAULTS['budget_ms']})")
    p.add_argument("--timeout-s", dest="timeout_s",
                   type=_positive_float, default=MISSING,
                   help=f"initial gRPC timeout (default {DEFAULTS['timeout_s']})")
    p.add_argument("--max-rounds", dest="max_rounds",
                   type=_bounded_int(1, 256), default=MISSING,
                   help=f"initial max fusion rounds (default {DEFAULTS['max_rounds']})")
    p.add_argument("--strict", dest="strict",
                   action="store_const", const=True, default=MISSING,
                   help="start with strict mode enabled")

    # Output / logging initial values
    _add_output_flags(p)
    p.add_argument("--with-diagnostics", dest="with_diagnostics",
                   action="store_const", const=True, default=MISSING,
                   help="start with diagnostics on (show sub_scores)")

    # Network
    _add_network_flags(p)

    # REPL-specific knobs — surfaced as typed fields on ResolvedConfig.
    p.add_argument("--history", dest="history_path", default=MISSING,
                   help="path to the persistent history file "
                        "(default: $XDG_DATA_HOME/efe/history)")
    p.add_argument("--no-history", dest="no_history",
                   action="store_const", const=True, default=MISSING,
                   help="disable history persistence for this session")
    p.add_argument("--prompt", dest="prompt_template", default=MISSING,
                   help="prompt template (default: 'EFE {catalog}> ')")


# ---------------------------------------------------------------------------
#  scenario
# ---------------------------------------------------------------------------


def _build_scenario(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    """Mount the ``scenario`` subcommand with its own sub-sub-command table.

    Usage (synopsis)::

        efe-admin scenario list
        efe-admin scenario show   NAME
        efe-admin scenario run    NAME --user TEXT [--transcript-path P]
        efe-admin scenario bench  NAME
        efe-admin scenario scaffold NAME [--out DIR] [--force]

    Every sub-sub-command understands the global flags (log-level, etc.)
    and the output flags (--format, --pretty).
    """
    p = sub.add_parser(
        "scenario",
        help="Run or manage Letter-C scenario bundles.",
        description=(
            "Orchestrate a scenario: system-prompt + catalog + golden cases "
            "wired through Claude + EFE. See LETTER_C_ARCHITECTURE.md for "
            "the full design."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  efe-admin scenario list\n"
            "  efe-admin scenario show support_triage_bilingual --pretty\n"
            "  efe-admin scenario run support_triage_bilingual \\\n"
            "      --user 'My card was charged without my consent'\n"
            "  efe-admin scenario bench support_triage_bilingual\n"
            "  efe-admin scenario scaffold my_new_scenario --out scenarios/\n"
        ),
        parents=[global_parent],
    )
    action_sub = p.add_subparsers(
        dest="scenario_action",
        required=True,
        metavar="ACTION",
        title="Scenario actions",
    )

    # --- list ---
    p_list = action_sub.add_parser(
        "list", help="List available scenarios (builtin + discoverable).",
        parents=[global_parent],
    )
    _add_output_flags(p_list)

    # --- show ---
    p_show = action_sub.add_parser(
        "show", help="Print a scenario's metadata and a preview.",
        parents=[global_parent],
    )
    p_show.add_argument("scenario_name", help="Scenario name (e.g. support_triage_bilingual)")
    _add_output_flags(p_show)

    # --- run ---
    p_run = action_sub.add_parser(
        "run",
        help="Run one user message through a scenario (live LLM call).",
        description=(
            "Requires ANTHROPIC_API_KEY and a running efe-server. "
            "Produces a JSON summary on stdout and an optional transcript "
            "file via --transcript-path."
        ),
        parents=[global_parent],
    )
    p_run.add_argument("scenario_name", help="Scenario name")
    p_run.add_argument(
        "--user", dest="scenario_user_text", required=True,
        help="User message to classify (the initial prompt).",
    )
    p_run.add_argument(
        "--transcript-path", dest="scenario_transcript_path",
        default=MISSING,
        help="Where to dump the full transcript JSON (optional).",
    )
    p_run.add_argument(
        "--cost-ceiling-usd", dest="scenario_cost_ceiling_usd",
        type=float, default=MISSING,
        help="Hard budget in USD. Exit 24 if exceeded.",
    )
    p_run.add_argument(
        "--max-rounds", dest="scenario_max_rounds",
        type=_bounded_int(1, 64), default=MISSING,
        help="Override the default max rounds (1..64).",
    )
    p_run.add_argument(
        "--redact-user", dest="scenario_redact_user",
        action="store_const", const=True, default=MISSING,
        help="Replace the user's text in the transcript with a length+hash placeholder.",
    )
    _add_output_flags(p_run)

    # --- bench ---
    p_bench = action_sub.add_parser(
        "bench",
        help="Run every golden case of a scenario against the live LLM.",
        description=(
            "Iterates all golden cases, runs each through the scenario, "
            "and prints accuracy / cost / latency. Exit 6 if any case errors."
        ),
        parents=[global_parent],
    )
    p_bench.add_argument("scenario_name", help="Scenario name")
    p_bench.add_argument(
        "--cost-ceiling-usd", dest="scenario_cost_ceiling_usd",
        type=float, default=MISSING,
        help="Per-case hard budget in USD.",
    )
    p_bench.add_argument(
        "--max-rounds", dest="scenario_max_rounds",
        type=_bounded_int(1, 64), default=MISSING,
        help="Override the default max rounds (1..64).",
    )
    _add_output_flags(p_bench)

    # --- scaffold ---
    p_scaf = action_sub.add_parser(
        "scaffold", help="Generate a skeleton directory for a new scenario.",
        parents=[global_parent],
    )
    p_scaf.add_argument("scenario_name", help="Scenario name (snake_case)")
    p_scaf.add_argument(
        "--out", dest="out",
        default=MISSING,
        help="Parent directory for the new scenario (default: ./scenarios).",
    )
    p_scaf.add_argument(
        "--catalog", dest="scenario_scaffold_catalog",
        default=MISSING,
        help="Placeholder catalog name to embed in the prompt.",
    )
    p_scaf.add_argument(
        "--force", dest="force",
        action="store_const", const=True, default=MISSING,
        help="Overwrite an existing directory.",
    )
    _add_output_flags(p_scaf)


# ---------------------------------------------------------------------------
#  catalog (Sprint D12 — multi-tenant catalog admin)
# ---------------------------------------------------------------------------


def _build_catalog(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    """Mount the ``catalog`` subcommand with its own action table.

    Sprint D12 (v0.1.12). Wraps the gRPC ``CatalogService`` from
    ``protos/efe/v1/catalog.proto``. RBAC scopes (cf.
    ``docs/bench/d12_catalog_admin_versioning_architecture.md`` § 6):

      * ``admin:catalog:read``  — list / versions / get
      * ``admin:catalog:write`` — register / evolve / delete

    Synopsis::

        efe-admin catalog register  --name N --version V --content content.yaml
                                    [--description ...] [--labels labels.json]
        efe-admin catalog evolve    --from-key KEY --op OP --payload op.yaml
                                    [--description ...]
        efe-admin catalog list      [--page-size N]
        efe-admin catalog versions  --name N
        efe-admin catalog get       KEY [--include-compiled]
        efe-admin catalog delete    KEY [--force]

    Every action requires a JWT (``--jwt`` or ``EFE_JWT``) and a
    server endpoint (``--endpoint`` or ``EFE_ENDPOINT``).
    """
    p = sub.add_parser(
        "catalog",
        help="Manage versioned catalogs (D12 multi-tenant admin).",
        description=(
            "Register, evolve, get, list, version, or delete a catalog "
            "via the gRPC CatalogService. See docs/howto/catalog-admin.md."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  efe-admin catalog register --name billing --version v1 \\\n"
            "      --content catalogs/billing.yaml --description 'Initial import'\n"
            "  efe-admin catalog evolve --from-key acme/billing@v1 \\\n"
            "      --op update-thresholds --payload ops/raise_thresholds.yaml\n"
            "  efe-admin catalog list --pretty\n"
            "  efe-admin catalog versions --name billing\n"
            "  efe-admin catalog get acme/billing@v3 --include-compiled\n"
            "  efe-admin catalog delete acme/billing@v1\n"
        ),
        parents=[global_parent],
    )
    action_sub = p.add_subparsers(
        dest="catalog_action",
        required=True,
        metavar="ACTION",
        title="Catalog actions",
    )

    # --- register ---
    p_reg = action_sub.add_parser(
        "register",
        help="Register a new catalog version (atomic).",
        description=(
            "POST a YAML-shaped catalog to the server. The server "
            "assigns the integer version monotonically per "
            "(tenant, name)."
        ),
        parents=[global_parent],
    )
    p_reg.add_argument("--name", dest="catalog_name", required=True,
                       help="catalog name (e.g. 'billing', 'support_triage')")
    p_reg.add_argument("--version", dest="catalog_version", default=MISSING,
                       help="SemVer-shaped version tag (informational ; "
                            "the server assigns the integer version)")
    p_reg.add_argument("--content", dest="catalog_content_path",
                       type=_existing_path, required=True,
                       help="path to the catalog YAML (efe.mode1/v1 schema)")
    p_reg.add_argument("--description", dest="catalog_description",
                       default=MISSING, help="audit note (free-form text)")
    p_reg.add_argument("--labels", dest="catalog_labels_path",
                       type=_existing_path, default=MISSING,
                       help="optional JSON file with key-value labels")
    _add_network_flags(p_reg)
    _add_output_flags(p_reg)

    # --- evolve ---
    p_evo = action_sub.add_parser(
        "evolve",
        help="Apply an evolution op, returning a new version.",
        description=(
            "Evolution is monotonic: the server always produces a new "
            "(tenant, name) version. The 5 supported ops mirror the "
            "proto oneof : append-class, remove-class, "
            "update-thresholds, replace-class-anchors, recompile."
        ),
        parents=[global_parent],
    )
    p_evo.add_argument("--from-key", dest="catalog_key", required=True,
                       help="canonical key of the source version "
                            "('tenant/name@vN')")
    p_evo.add_argument("--op", dest="catalog_op", required=True,
                       choices=(
                           "append-class", "remove-class",
                           "update-thresholds", "replace-class-anchors",
                           "recompile",
                       ),
                       help="evolution op (see proto §8.1)")
    p_evo.add_argument("--payload", dest="catalog_op_payload_path",
                       type=_existing_path, required=True,
                       help="op-specific payload (YAML or JSON ; see "
                            "docs/howto/catalog-evolution.md)")
    p_evo.add_argument("--description", dest="catalog_description",
                       default=MISSING, help="audit note (free-form text)")
    _add_network_flags(p_evo)
    _add_output_flags(p_evo)

    # --- list ---
    p_list = action_sub.add_parser(
        "list",
        help="List catalogs visible to the caller's tenant.",
        description=(
            "Returns a one-row-per-(tenant,name) summary with the "
            "latest version, class count, and aggregate anchor count. "
            "Server-side filtered by JWT tenant_id."
        ),
        parents=[global_parent],
    )
    p_list.add_argument("--page-size", dest="catalog_page_size",
                        type=_bounded_int(1, 500), default=MISSING,
                        help="max results per page (default 100, max 500)")
    _add_network_flags(p_list)
    _add_output_flags(p_list)

    # --- versions ---
    p_vers = action_sub.add_parser(
        "versions",
        help="List all versions for a (tenant, name).",
        description=(
            "Returns canonical keys 'tenant/name@vN' sorted ascending."
        ),
        parents=[global_parent],
    )
    p_vers.add_argument("--name", dest="catalog_name", required=True,
                        help="catalog name")
    _add_network_flags(p_vers)
    _add_output_flags(p_vers)

    # --- get ---
    p_get = action_sub.add_parser(
        "get",
        help="Fetch one catalog version by canonical key.",
        description=(
            "Print the full canonical body (content + audit metadata). "
            "With --include-compiled, also emit per-class compiled "
            "anchor refs (requires admin:catalog:read)."
        ),
        parents=[global_parent],
    )
    p_get.add_argument("catalog_key", help="canonical key 'tenant/name@vN'")
    p_get.add_argument("--include-compiled", dest="catalog_include_compiled",
                       action="store_const", const=True, default=MISSING,
                       help="include compiled anchor refs in the response")
    _add_network_flags(p_get)
    _add_output_flags(p_get)

    # --- delete ---
    p_del = action_sub.add_parser(
        "delete",
        help="Delete a specific catalog version.",
        description=(
            "Without --force, the server refuses to delete a version "
            "still referenced by another (e.g. a parent_key)."
        ),
        parents=[global_parent],
    )
    p_del.add_argument("catalog_key", help="canonical key 'tenant/name@vN'")
    p_del.add_argument("--force", dest="catalog_force",
                       action="store_const", const=True, default=MISSING,
                       help="cascade-delete (use with care — breaks "
                            "downstream classifiers that reference this version)")
    _add_network_flags(p_del)
    _add_output_flags(p_del)


# ---------------------------------------------------------------------------
#  completions
# ---------------------------------------------------------------------------


def _build_completions(
    sub: argparse._SubParsersAction[Any],
    global_parent: argparse.ArgumentParser,
) -> None:
    p = sub.add_parser(
        "completions",
        help="Emit a shell completion script (bash/zsh/fish).",
        description=(
            "Write a completion script to stdout. Pipe it into the "
            "appropriate location for your shell."
        ),
        epilog=(
            "Examples:\n"
            "  efe-admin completions bash > ~/.bash_completion.d/efe-admin\n"
            "  efe-admin completions zsh  > ~/.zsh/completions/_efe-admin\n"
            "  efe-admin completions fish > ~/.config/fish/completions/efe-admin.fish\n"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        parents=[global_parent],
    )
    p.add_argument(
        "shell",
        choices=("bash", "zsh", "fish"),
        help="target shell",
    )


# ---------------------------------------------------------------------------
#  Public API
# ---------------------------------------------------------------------------


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    """Parse ``argv`` and return a raw namespace.

    Passing ``None`` (the default) delegates to ``sys.argv[1:]``.

    Raises:
        UsageError: on any parse failure.
    """
    parser = build_parser()
    return parser.parse_args(argv)


__all__ = ["build_parser", "parse_args"]
