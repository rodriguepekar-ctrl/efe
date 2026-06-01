"""Layered configuration resolution for the ``efe-admin`` CLI.

This module is the **single source of truth** for translating raw
CLI + environment inputs into a strictly-typed :class:`ResolvedConfig`
dataclass. Every subcommand consumes that dataclass; no subcommand
reads ``os.environ`` or inspects an argparse namespace directly.

Precedence (see ``LETTER_A_ARCHITECTURE.md`` §7.1):

    1. Flag CLI explicitly passed
    2. Environment variable (``EFE_*`` + the ``NO_COLOR`` convention)
    3. Default compiled in :mod:`efe_tool.cli.metadata`

This module is deliberately side-effect-free:

* No logging (the logger is configured *after* resolution).
* No I/O apart from reading ``os.environ``.
* No import of heavy dependencies.

That makes it trivially unit-testable.
"""

from __future__ import annotations

import os
import uuid
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any, Final, Mapping, Optional

from .errors import UsageError
from .metadata import (
    DEFAULTS,
    ENV_KEYS,
    LOG_FORMATS,
    LOG_LEVELS,
    OUTPUT_FORMATS,
    PROGRESS_MODES,
    no_color_from_env,
    parse_bool_env,
)


# Sentinel that represents "not provided on the command line". We
# cannot use ``None`` because ``None`` is a legitimate value for some
# flags (e.g. ``--jwt``).
_MISSING: Final[object] = object()


# ---------------------------------------------------------------------------
# The dataclass
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ResolvedConfig:
    """Fully resolved configuration consumed by every subcommand.

    Instances are **frozen**: subcommands that need a derived variant
    (e.g. a new ``request_id`` per retry) use :meth:`dataclasses.replace`
    rather than mutating in place.
    """

    # --- selected subcommand ---
    command: str                          # "classify" | "compile-catalog" | ...

    # --- network ---
    endpoint: str
    tls: bool
    tls_ca: Optional[str]
    jwt: Optional[str]
    tenant: Optional[str]

    # --- catalog ---
    catalog_path: Optional[Path]
    embedder_model: str

    # --- classification ---
    budget_ms: int
    timeout_s: float
    max_rounds: int
    strict: bool

    # --- output ---
    format: str
    pretty: bool
    with_diagnostics: bool

    # --- logging ---
    log_level: str
    log_format: str
    quiet: bool
    no_color: bool
    request_id: str
    progress: str

    # --- subcommand-specific inputs (raw, validated later) ---
    text: Optional[str] = None
    text_file: Optional[Path] = None
    use_stdin: bool = False
    out_path: Optional[Path] = None       # compile-catalog / eval
    force: bool = False                   # compile-catalog
    golden_path: Optional[Path] = None    # eval
    strict_thresholds: bool = False       # validate
    shell: Optional[str] = None           # completions

    # --- deprecation shim: positional TEXT for classify (remove in 2.0) ---
    legacy_text: Optional[str] = None

    # --- REPL-specific (Lettre B) ---
    history_path: Optional[str] = None
    no_history: bool = False
    prompt_template: Optional[str] = None

    # --- Scenario-specific (Lettre C) ---
    scenario_action: Optional[str] = None          # list | show | run | bench | scaffold
    scenario_name: Optional[str] = None
    scenario_user_text: Optional[str] = None
    scenario_transcript_path: Optional[str] = None
    scenario_cost_ceiling_usd: Optional[float] = None
    scenario_max_rounds: Optional[int] = None
    scenario_redact_user: bool = False
    scenario_scaffold_catalog: Optional[str] = None

    # --- Catalog admin (Sprint D12) ---
    # The new ``catalog`` subcommand has 6 actions: register, evolve,
    # list, versions, get, delete. The fields below carry per-action
    # inputs ; only those relevant to the chosen action are populated.
    catalog_action: Optional[str] = None             # register | evolve | list | versions | get | delete
    catalog_name: Optional[str] = None               # register / versions
    catalog_version: Optional[str] = None            # register (semver-shaped, informational)
    catalog_key: Optional[str] = None                # get / delete / evolve (from_key)
    catalog_content_path: Optional[Path] = None      # register (YAML body)
    catalog_op: Optional[str] = None                 # evolve (append-class | remove-class | …)
    catalog_op_payload_path: Optional[Path] = None   # evolve (op-specific YAML/JSON)
    catalog_description: Optional[str] = None        # register / evolve audit note
    catalog_labels_path: Optional[Path] = None       # register (JSON labels)
    catalog_include_compiled: bool = False           # get
    catalog_force: bool = False                      # delete
    catalog_page_size: Optional[int] = None          # list

    # --- misc / extensibility (rarely used; prefer named fields) ---
    extra: Mapping[str, Any] = field(default_factory=dict)

    # ------------------------------------------------------------------
    #  Conveniences
    # ------------------------------------------------------------------

    @property
    def effective_log_level(self) -> str:
        """Apply the ``--quiet`` override."""
        return "error" if self.quiet else self.log_level

    def with_request_id(self, request_id: str) -> "ResolvedConfig":
        """Return a copy with a different request_id (for retries)."""
        return replace(self, request_id=request_id)


# ---------------------------------------------------------------------------
#  Resolver
# ---------------------------------------------------------------------------


def resolve_config(
    args: Any,
    env: Mapping[str, str] | None = None,
) -> ResolvedConfig:
    """Turn an argparse namespace + environment into a :class:`ResolvedConfig`.

    Args:
        args:  The namespace returned by ``argparse.ArgumentParser.parse_args``.
               Every flag must have been declared with ``default=_MISSING``
               (see :mod:`efe_tool.cli.parser`) so that the resolver can
               distinguish "not provided" from "provided with the default value".
        env:   Environment mapping. Defaults to ``os.environ``.

    Raises:
        UsageError: if any resolved value is out of its documented domain.
    """
    env_map: Mapping[str, str] = env if env is not None else os.environ

    def pick(key: str) -> Any:
        """Apply precedence for one key: flag > env > default."""
        flag_val = getattr(args, key, _MISSING)
        if flag_val is not _MISSING:
            return flag_val
        env_name = ENV_KEYS.get(key)
        if env_name is not None:
            raw = env_map.get(env_name)
            if raw is not None and raw != "":
                try:
                    return _coerce_env(key, raw)
                except ValueError as e:
                    raise UsageError(
                        f"env var {env_name}={raw!r} is invalid: {e}",
                        hint=f"valid values for {env_name}: see --help",
                    )
        return DEFAULTS[key]

    # --- keys we can handle uniformly ---
    endpoint       = pick("endpoint")
    tls            = pick("tls")
    tls_ca         = pick("tls_ca")
    jwt            = pick("jwt")
    tenant         = pick("tenant")
    catalog_path   = pick("catalog_path")
    embedder_model = pick("embedder_model")
    budget_ms      = pick("budget_ms")
    timeout_s      = pick("timeout_s")
    max_rounds     = pick("max_rounds")
    strict         = pick("strict")
    fmt            = pick("format")
    pretty         = pick("pretty")
    with_diag      = pick("with_diagnostics")
    log_level      = pick("log_level")
    log_format     = pick("log_format")
    quiet          = pick("quiet")
    request_id     = pick("request_id")
    progress       = pick("progress")

    # NO_COLOR has special "presence-only" semantics: the mere presence
    # of the env var — even with empty value — disables colors. We
    # read from ``env_map`` (not ``os.environ``) so tests can pass a
    # fixture env.
    no_color_flag = getattr(args, "no_color", _MISSING)
    if _is_set(no_color_flag):
        no_color = bool(no_color_flag)
    else:
        no_color = "NO_COLOR" in env_map

    # ---- request_id default ----
    if not request_id:
        request_id = str(uuid.uuid4())

    # ---- path coercion ----
    if catalog_path is not None and not isinstance(catalog_path, Path):
        catalog_path = Path(str(catalog_path))

    # ---- subcommand-specific inputs (never env-backed) ----
    text_raw  = getattr(args, "text", None)
    text      = text_raw if _is_set(text_raw) else None
    text_file = _path_or_none(getattr(args, "text_file", None))
    use_stdin = _bool_or_false(getattr(args, "stdin", False))
    out_path  = _path_or_none(getattr(args, "out", None))
    force     = _bool_or_false(getattr(args, "force", False))
    golden_path = _path_or_none(getattr(args, "golden", None))
    strict_thr  = _bool_or_false(getattr(args, "strict_thresholds", False))
    shell_raw   = getattr(args, "shell", None)
    shell       = shell_raw if _is_set(shell_raw) else None

    # ---- Positional path fold-in ----
    # ``compile-catalog PATH`` and ``validate PATH`` declare a positional
    # ``path`` argument. Map it onto ``catalog_path`` unless --catalog
    # has already provided one explicitly. This keeps every subcommand's
    # consumer reading a single field (``cfg.catalog_path``).
    positional_path = getattr(args, "path", None)
    if positional_path is not None and catalog_path is None:
        catalog_path = Path(str(positional_path))

    # ---- Deprecated positional TEXT for classify ----
    # Kept for backward compatibility with the AS-IS CLI. Stored under
    # ``legacy_text`` so the ``classify`` subcommand can emit a clear
    # deprecation warning on stderr. If no --text was passed, this value
    # also doubles as the effective text input.
    legacy_text_raw = getattr(args, "legacy_text", None)
    legacy_text     = str(legacy_text_raw) if legacy_text_raw else None
    if legacy_text is not None and text is None:
        text = legacy_text

    # ---- REPL-specific fields ----
    history_path_raw   = getattr(args, "history_path", None)
    history_path_val   = (
        str(history_path_raw) if _is_set(history_path_raw) else None
    )
    no_history_val     = _bool_or_false(getattr(args, "no_history", False))
    prompt_template_raw = getattr(args, "prompt_template", None)
    prompt_template_val = (
        str(prompt_template_raw) if _is_set(prompt_template_raw) else None
    )

    # ---- Scenario-specific fields (Lettre C) ----
    scen_action = getattr(args, "scenario_action", None)
    scen_name   = getattr(args, "scenario_name", None)
    scen_user   = getattr(args, "scenario_user_text", None)
    scen_tr     = getattr(args, "scenario_transcript_path", None)
    scen_cost   = getattr(args, "scenario_cost_ceiling_usd", None)
    scen_rounds = getattr(args, "scenario_max_rounds", None)
    scen_redact = getattr(args, "scenario_redact_user", None)
    scen_scat   = getattr(args, "scenario_scaffold_catalog", None)

    scenario_action_val = str(scen_action) if _is_set(scen_action) else None
    scenario_name_val   = str(scen_name) if _is_set(scen_name) else None
    scenario_user_val   = str(scen_user) if _is_set(scen_user) else None
    scenario_tr_val     = str(scen_tr) if _is_set(scen_tr) else None
    scenario_cost_val   = float(scen_cost) if _is_set(scen_cost) else None
    scenario_rounds_val = int(scen_rounds) if _is_set(scen_rounds) else None
    scenario_redact_val = _bool_or_false(scen_redact)
    scenario_scat_val   = str(scen_scat) if _is_set(scen_scat) else None

    # ---- Catalog admin fields (Sprint D12) ----
    cat_action  = getattr(args, "catalog_action", None)
    cat_name    = getattr(args, "catalog_name", None)
    cat_version = getattr(args, "catalog_version", None)
    cat_key     = getattr(args, "catalog_key", None)
    cat_content = getattr(args, "catalog_content_path", None)
    cat_op      = getattr(args, "catalog_op", None)
    cat_payload = getattr(args, "catalog_op_payload_path", None)
    cat_descr   = getattr(args, "catalog_description", None)
    cat_labels  = getattr(args, "catalog_labels_path", None)
    cat_incl_c  = getattr(args, "catalog_include_compiled", None)
    cat_force_v = getattr(args, "catalog_force", None)
    cat_psize   = getattr(args, "catalog_page_size", None)

    catalog_action_val          = str(cat_action) if _is_set(cat_action) else None
    catalog_name_val            = str(cat_name) if _is_set(cat_name) else None
    catalog_version_val         = str(cat_version) if _is_set(cat_version) else None
    catalog_key_val             = str(cat_key) if _is_set(cat_key) else None
    catalog_content_path_val    = _path_or_none(cat_content)
    catalog_op_val              = str(cat_op) if _is_set(cat_op) else None
    catalog_op_payload_path_val = _path_or_none(cat_payload)
    catalog_description_val     = str(cat_descr) if _is_set(cat_descr) else None
    catalog_labels_path_val     = _path_or_none(cat_labels)
    catalog_include_compiled_val = _bool_or_false(cat_incl_c)
    catalog_force_val           = _bool_or_false(cat_force_v)
    catalog_page_size_val       = int(cat_psize) if _is_set(cat_psize) else None

    cfg = ResolvedConfig(
        command=getattr(args, "cmd", "") or "",
        endpoint=str(endpoint),
        tls=bool(tls),
        tls_ca=_opt_str(tls_ca),
        jwt=_opt_str(jwt),
        tenant=_opt_str(tenant),
        catalog_path=catalog_path,
        embedder_model=str(embedder_model),
        budget_ms=int(budget_ms),
        timeout_s=float(timeout_s),
        max_rounds=int(max_rounds),
        strict=bool(strict),
        format=str(fmt),
        pretty=bool(pretty),
        with_diagnostics=bool(with_diag),
        log_level=str(log_level),
        log_format=str(log_format),
        quiet=bool(quiet),
        no_color=bool(no_color),
        request_id=str(request_id),
        progress=str(progress),
        text=text,
        text_file=text_file,
        use_stdin=use_stdin,
        out_path=out_path,
        force=force,
        golden_path=golden_path,
        strict_thresholds=strict_thr,
        shell=shell,
        legacy_text=legacy_text,
        history_path=history_path_val,
        no_history=no_history_val,
        prompt_template=prompt_template_val,
        scenario_action=scenario_action_val,
        scenario_name=scenario_name_val,
        scenario_user_text=scenario_user_val,
        scenario_transcript_path=scenario_tr_val,
        scenario_cost_ceiling_usd=scenario_cost_val,
        scenario_max_rounds=scenario_rounds_val,
        scenario_redact_user=scenario_redact_val,
        scenario_scaffold_catalog=scenario_scat_val,
        catalog_action=catalog_action_val,
        catalog_name=catalog_name_val,
        catalog_version=catalog_version_val,
        catalog_key=catalog_key_val,
        catalog_content_path=catalog_content_path_val,
        catalog_op=catalog_op_val,
        catalog_op_payload_path=catalog_op_payload_path_val,
        catalog_description=catalog_description_val,
        catalog_labels_path=catalog_labels_path_val,
        catalog_include_compiled=catalog_include_compiled_val,
        catalog_force=catalog_force_val,
        catalog_page_size=catalog_page_size_val,
    )
    validate_config(cfg)
    return cfg


# ---------------------------------------------------------------------------
#  Validation
# ---------------------------------------------------------------------------


def validate_config(cfg: ResolvedConfig) -> None:
    """Enforce domain constraints on a resolved config.

    Called automatically by :func:`resolve_config`. Also invokable from
    tests that construct a :class:`ResolvedConfig` directly.
    """
    # Enum constraints
    if cfg.log_level not in LOG_LEVELS:
        raise UsageError(
            f"invalid --log-level {cfg.log_level!r}",
            hint=f"allowed values: {', '.join(LOG_LEVELS)}",
        )
    if cfg.log_format not in LOG_FORMATS:
        raise UsageError(
            f"invalid --log-format {cfg.log_format!r}",
            hint=f"allowed values: {', '.join(LOG_FORMATS)}",
        )
    if cfg.format not in OUTPUT_FORMATS:
        raise UsageError(
            f"invalid --format {cfg.format!r}",
            hint=f"allowed values: {', '.join(OUTPUT_FORMATS)}",
        )
    if cfg.progress not in PROGRESS_MODES:
        raise UsageError(
            f"invalid --progress {cfg.progress!r}",
            hint=f"allowed values: {', '.join(PROGRESS_MODES)}",
        )

    # Numeric constraints
    if cfg.budget_ms <= 0:
        raise UsageError(f"--budget-ms must be > 0, got {cfg.budget_ms}")
    if cfg.timeout_s <= 0:
        raise UsageError(f"--timeout-s must be > 0, got {cfg.timeout_s}")
    if not 1 <= cfg.max_rounds <= 256:
        raise UsageError(
            f"--max-rounds must be in [1, 256], got {cfg.max_rounds}"
        )

    # Endpoint shape: host:port, no scheme
    if cfg.endpoint.startswith(("http://", "https://", "grpc://", "grpcs://")):
        raise UsageError(
            f"--endpoint must be host:port, got {cfg.endpoint!r}",
            hint="gRPC endpoints do not carry a URL scheme",
        )
    if ":" not in cfg.endpoint:
        raise UsageError(
            f"--endpoint must include a port, got {cfg.endpoint!r}",
            hint="e.g. --endpoint 127.0.0.1:50051",
        )


def harmonise_budget_and_timeout(
    cfg: ResolvedConfig,
    *,
    logger: Any | None = None,
) -> ResolvedConfig:
    """Guarantee ``timeout_s >= budget_ms/1000 + 0.5`` by raising ``timeout_s``
    if necessary. This prevents a very common pitfall where the client
    hangs up before the server has even had a chance to return the
    answer.

    The fix is applied automatically with a log warning rather than a
    hard error, because it's strictly conservative and makes the CLI
    forgiving.
    """
    min_timeout_s = cfg.budget_ms / 1000.0 + 0.5
    if cfg.timeout_s < min_timeout_s:
        new_timeout = round(min_timeout_s, 3)
        if logger is not None:
            logger.warn(
                "timeout_bumped",
                old_timeout_s=cfg.timeout_s,
                new_timeout_s=new_timeout,
                budget_ms=cfg.budget_ms,
                reason=(
                    "timeout_s was smaller than budget_ms/1000 + 0.5s "
                    "safety margin"
                ),
            )
        return replace(cfg, timeout_s=new_timeout)
    return cfg


# ---------------------------------------------------------------------------
#  Internals
# ---------------------------------------------------------------------------


def _coerce_env(key: str, raw: str) -> Any:
    """Coerce a raw env var string to the right Python type for ``key``.

    Uses :data:`DEFAULTS` as a shape oracle.
    """
    default = DEFAULTS[key]
    if isinstance(default, bool) or default is None and key in {
        "strict", "pretty", "with_diagnostics", "quiet", "no_color", "tls",
    }:
        parsed = parse_bool_env(raw)
        if parsed is None:
            # Empty env var is treated as "unset"
            raise ValueError("empty")
        return parsed
    if isinstance(default, int):
        try:
            return int(raw)
        except ValueError:
            raise ValueError("expected integer")
    if isinstance(default, float):
        try:
            return float(raw)
        except ValueError:
            raise ValueError("expected float")
    if default is None and key in {"catalog_path", "tls_ca"}:
        return raw  # path strings — Path() wrap happens later
    if default is None and key in {"jwt", "tenant", "request_id"}:
        return raw
    # string enum or plain string
    return raw


def _opt_str(v: Any) -> Optional[str]:
    if v is None or v is _MISSING:
        return None
    s = str(v)
    return s if s else None


def _is_set(v: Any) -> bool:
    """True iff ``v`` is neither ``None`` nor the MISSING sentinel."""
    return v is not None and v is not _MISSING


def _path_or_none(v: Any) -> Optional[Path]:
    if not _is_set(v):
        return None
    if isinstance(v, Path):
        return v
    return Path(str(v))


def _bool_or_false(v: Any) -> bool:
    if not _is_set(v):
        return False
    return bool(v)


# ---------------------------------------------------------------------------
#  Public symbol for tests
# ---------------------------------------------------------------------------

__all__ = [
    "MISSING",
    "ResolvedConfig",
    "resolve_config",
    "validate_config",
    "harmonise_budget_and_timeout",
]

# Expose the sentinel under a public, documented name so that
# :mod:`efe_tool.cli.parser` can use it consistently.
MISSING: Final[object] = _MISSING
