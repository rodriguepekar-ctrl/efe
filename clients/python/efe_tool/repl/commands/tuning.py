"""Classification-tuning commands: ``:budget``, ``:timeout``, ``:rounds``,
``:strict``, ``:thresholds``.

All of these mutate session-local fields. They never touch the gRPC
channel or the catalog. ``reset`` reverts to the boot config value.
"""

from __future__ import annotations

from typing import Any

from ...cli.errors import UsageError
from . import Command, register


# ---------------------------------------------------------------------------
#  Shared helpers
# ---------------------------------------------------------------------------


def _parse_int(raw: str, *, name: str, lo: int = 1, hi: int = 2**31 - 1) -> int:
    try:
        v = int(raw)
    except ValueError:
        raise UsageError(f":{name} expects an integer, got {raw!r}")
    if not lo <= v <= hi:
        raise UsageError(f":{name} must be in [{lo}, {hi}], got {v}")
    return v


def _parse_float(raw: str, *, name: str, lo: float = 1e-3) -> float:
    try:
        v = float(raw)
    except ValueError:
        raise UsageError(f":{name} expects a number, got {raw!r}")
    if v < lo:
        raise UsageError(f":{name} must be ≥ {lo}, got {v}")
    return v


def _parse_on_off(raw: str, *, name: str) -> bool:
    s = raw.lower()
    if s in {"on", "true", "1", "yes", "enable"}:
        return True
    if s in {"off", "false", "0", "no", "disable"}:
        return False
    raise UsageError(f":{name} expects on|off, got {raw!r}")


# ---------------------------------------------------------------------------
#  :budget
# ---------------------------------------------------------------------------


def _cmd_budget(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"budget: {state.budget_ms} ms"
    if args[0] == "reset":
        state.budget_ms = state.boot_cfg.budget_ms
        return f"budget reset to {state.budget_ms} ms"
    state.budget_ms = _parse_int(args[0], name="budget", lo=1, hi=3_600_000)
    return f"budget: {state.budget_ms} ms"


register(Command(
    name="budget",
    run=_cmd_budget,
    doc="Without arg: show budget_ms. With MS: set. 'reset' → boot value.",
    group="tuning",
))


# ---------------------------------------------------------------------------
#  :timeout
# ---------------------------------------------------------------------------


def _cmd_timeout(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"timeout: {state.timeout_s:.2f} s"
    if args[0] == "reset":
        state.timeout_s = state.boot_cfg.timeout_s
        return f"timeout reset to {state.timeout_s:.2f} s"
    state.timeout_s = _parse_float(args[0], name="timeout", lo=0.01)
    return f"timeout: {state.timeout_s:.2f} s"


register(Command(
    name="timeout",
    run=_cmd_timeout,
    doc="Without arg: show timeout_s. With S: set. 'reset' → boot value.",
    group="tuning",
))


# ---------------------------------------------------------------------------
#  :rounds
# ---------------------------------------------------------------------------


def _cmd_rounds(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"max_rounds: {state.max_rounds}"
    if args[0] == "reset":
        state.max_rounds = state.boot_cfg.max_rounds
        return f"max_rounds reset to {state.max_rounds}"
    state.max_rounds = _parse_int(args[0], name="rounds", lo=1, hi=256)
    return f"max_rounds: {state.max_rounds}"


register(Command(
    name="rounds",
    run=_cmd_rounds,
    doc="Without arg: show max_rounds. With N in [1,256]: set. 'reset' → boot.",
    group="tuning",
))


# ---------------------------------------------------------------------------
#  :strict
# ---------------------------------------------------------------------------


def _cmd_strict(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"strict: {'on' if state.strict else 'off'}"
    if args[0] == "reset":
        state.strict = state.boot_cfg.strict
        return f"strict reset to {'on' if state.strict else 'off'}"
    state.strict = _parse_on_off(args[0], name="strict")
    return f"strict: {'on' if state.strict else 'off'}"


register(Command(
    name="strict",
    run=_cmd_strict,
    doc="Toggle strict mode (on|off|reset). Marks low-conf results with ⚠.",
    group="tuning",
))


# ---------------------------------------------------------------------------
#  :thresholds
# ---------------------------------------------------------------------------


def _cmd_thresholds(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return _describe_thresholds(state)
    if args[0] == "reset":
        state.reset_threshold_overrides()
        return "thresholds reset to catalog defaults"
    # Parse pairs: `high X`, `medium Y` (in any order, both optional).
    i = 0
    changed: list[str] = []
    while i < len(args):
        key = args[i].lower()
        if key not in {"high", "medium"}:
            raise UsageError(
                f":thresholds unknown key {key!r}",
                hint="use 'high' and/or 'medium', or 'reset'",
            )
        if i + 1 >= len(args):
            raise UsageError(f":thresholds {key} requires a value")
        val = _parse_float(args[i + 1], name=f"thresholds.{key}", lo=0.0)
        if not 0.0 <= val <= 1.0:
            raise UsageError(
                f":thresholds {key} must be in [0, 1], got {val}"
            )
        if key == "high":
            state.threshold_high_override = val
        else:
            state.threshold_medium_override = val
        changed.append(f"{key}={val}")
        i += 2
    # Sanity: high ≥ medium
    eff_h = state.effective_high_threshold
    eff_m = state.effective_medium_threshold
    if eff_h is not None and eff_m is not None and eff_h < eff_m:
        raise UsageError(
            f"invalid threshold order: high ({eff_h:.3f}) < medium ({eff_m:.3f})",
            hint="adjust so high ≥ medium",
        )
    return "thresholds updated: " + ", ".join(changed)


def _describe_thresholds(state: Any) -> str:
    h = state.effective_high_threshold
    m = state.effective_medium_threshold
    if h is None or m is None:
        return "(catalog not loaded)"
    lines = [f"high:   {h:.3f}", f"medium: {m:.3f}"]
    overrides = []
    if state.threshold_high_override is not None:
        overrides.append("high")
    if state.threshold_medium_override is not None:
        overrides.append("medium")
    if overrides:
        lines.append(f"(overrides: {', '.join(overrides)})")
    return "\n".join(lines)


register(Command(
    name="thresholds",
    run=_cmd_thresholds,
    doc="Inspect or override decision thresholds. ':thresholds high 0.8 medium 0.6'",
    group="tuning",
))


__all__: list[str] = []
