"""Session-management commands: help, quit, info, version, save, stats,
history, clear, healthcheck.

Every function here has the signature
``fn(state, args, log) -> str | None`` and is registered at import time.
"""

from __future__ import annotations

import json
import os
import time
from pathlib import Path
from typing import Any

from ...cli.errors import (
    NetworkError,
    UsageError,
    from_bridge_error,
)
from . import Command, REGISTRY, canonical_names, register


# ---------------------------------------------------------------------------
#  :help
# ---------------------------------------------------------------------------


def _cmd_help(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return _render_help_index()
    target = args[0].lstrip(":")
    cmd = REGISTRY.get(target)
    if cmd is None:
        raise UsageError(f"unknown command :{target}")
    lines = [f":{cmd.name} — {cmd.doc}"]
    if cmd.aliases:
        lines.append("  aliases: " + ", ".join(f":{a}" for a in cmd.aliases))
    if cmd.needs_catalog:
        lines.append("  requires: a loaded catalog")
    lines.append(f"  group: {cmd.group}")
    return "\n".join(lines)


def _render_help_index() -> str:
    groups: dict[str, list[Command]] = {}
    seen: set[str] = set()
    for cmd in REGISTRY.values():
        if cmd.name in seen:
            continue
        seen.add(cmd.name)
        groups.setdefault(cmd.group, []).append(cmd)
    order = ("catalog", "classify", "tuning", "output", "logging",
             "network", "session")
    lines = ["Available commands (type ':help <cmd>' for detail):"]
    for g in order:
        cmds = sorted(groups.get(g, []), key=lambda c: c.name)
        if not cmds:
            continue
        lines.append(f"  [{g}]")
        for c in cmds:
            first = c.doc.splitlines()[0] if c.doc else ""
            lines.append(f"    :{c.name:<14} {first}")
    # Catch any leftover groups not in the ordered list.
    for g, cmds in sorted(groups.items()):
        if g in order:
            continue
        lines.append(f"  [{g}]")
        for c in sorted(cmds, key=lambda c: c.name):
            first = c.doc.splitlines()[0] if c.doc else ""
            lines.append(f"    :{c.name:<14} {first}")
    return "\n".join(lines)


register(Command(
    name="help",
    run=_cmd_help,
    doc="Show all commands or detail for one: ':help :reload'",
    group="session",
))


# ---------------------------------------------------------------------------
#  :quit (aliases: :exit, :q)
# ---------------------------------------------------------------------------


class QuitSignal(Exception):
    """Sentinel exception raised by ``:quit`` to exit the REPL loop."""


def _cmd_quit(state: Any, args: list[str], log: Any) -> str | None:
    raise QuitSignal()


register(Command(
    name="quit",
    run=_cmd_quit,
    doc="Exit the REPL (same as Ctrl-D)",
    aliases=("exit", "q"),
    group="session",
))


# ---------------------------------------------------------------------------
#  :version
# ---------------------------------------------------------------------------


def _cmd_version(state: Any, args: list[str], log: Any) -> str:
    from efe_tool import __version__
    return f"efe-tool {__version__} — REPL"


register(Command(
    name="version",
    run=_cmd_version,
    doc="Show the current efe-tool version",
    group="session",
))


# ---------------------------------------------------------------------------
#  :clear
# ---------------------------------------------------------------------------


def _cmd_clear(state: Any, args: list[str], log: Any) -> str | None:
    if state.no_color or not _stdout_is_tty():
        return "(clear: no TTY, no-op)"
    # ANSI CSI 2J (erase entire screen) + CSI H (cursor home).
    import sys
    sys.stdout.write("\x1b[2J\x1b[H")
    sys.stdout.flush()
    return None


register(Command(
    name="clear",
    run=_cmd_clear,
    doc="Clear the screen (ANSI; no-op on non-TTY)",
    group="session",
))


# ---------------------------------------------------------------------------
#  :info
# ---------------------------------------------------------------------------


def _cmd_info(state: Any, args: list[str], log: Any) -> str:
    from ..render import render_info
    return render_info(state, use_color=not state.no_color)


register(Command(
    name="info",
    run=_cmd_info,
    doc="Show the full runtime state of the session",
    group="session",
))


# ---------------------------------------------------------------------------
#  :stats
# ---------------------------------------------------------------------------


def _cmd_stats(state: Any, args: list[str], log: Any) -> str:
    from ..render import render_stats
    return render_stats(state, use_color=not state.no_color)


register(Command(
    name="stats",
    run=_cmd_stats,
    doc="Show counts, label distribution and latency percentiles",
    group="session",
))


# ---------------------------------------------------------------------------
#  :history
# ---------------------------------------------------------------------------


def _cmd_history(state: Any, args: list[str], log: Any) -> str:
    if args and args[0] == "clear":
        state.history.clear()
        return "history cleared"
    n = 20
    if args:
        try:
            n = max(1, min(10_000, int(args[0])))
        except ValueError:
            raise UsageError(f":history expects an integer or 'clear', got {args[0]!r}")
    recent = state.history[-n:]
    if not recent:
        return "(history empty)"
    width = len(str(len(state.history)))
    out = []
    for i, line in enumerate(recent, start=len(state.history) - len(recent) + 1):
        out.append(f"  {i:{width}}  {line}")
    return "\n".join(out)


register(Command(
    name="history",
    run=_cmd_history,
    doc="Show recent inputs. ':history 50' for 50 lines, ':history clear' to wipe",
    group="session",
))


# ---------------------------------------------------------------------------
#  :save PATH
# ---------------------------------------------------------------------------


def _cmd_save(state: Any, args: list[str], log: Any) -> str:
    if not args:
        raise UsageError(":save PATH — specify a target file")
    target = Path(os.path.expanduser(args[0]))
    payload = _build_transcript_payload(state)
    target.parent.mkdir(parents=True, exist_ok=True)
    tmp = target.with_suffix(target.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    tmp.replace(target)
    return f"saved {len(payload['entries'])} entries to {target}"


def _build_transcript_payload(state: Any) -> dict[str, Any]:
    from efe_tool import __version__
    return {
        "schema_version": 1,
        "session_id": state.session_id,
        "program_version": __version__,
        "boot_config": {
            "catalog_path": (
                str(state.boot_cfg.catalog_path)
                if state.boot_cfg.catalog_path else None
            ),
            "endpoint": state.boot_cfg.endpoint,
            "embedder": state.boot_cfg.embedder_model,
        },
        "started_at": _iso(state.started_at_wall),
        "ended_at": _iso(time.time()),
        "duration_s": round(state.uptime_s, 3),
        "entries": list(state.transcript),
        "stats": {
            "queries": state.queries_count,
            "commands": state.commands_count,
            "errors": state.error_count,
            "labels": dict(state.label_counts),
            "latency_p50_ms": _percentile(state.latencies_ms, 0.50),
            "latency_p95_ms": _percentile(state.latencies_ms, 0.95),
        },
    }


def _iso(t: float) -> str:
    """Convert a POSIX timestamp (seconds since epoch) to ISO 8601 UTC."""
    gm = time.gmtime(t)
    return (
        f"{gm.tm_year:04d}-{gm.tm_mon:02d}-{gm.tm_mday:02d}"
        f"T{gm.tm_hour:02d}:{gm.tm_min:02d}:{gm.tm_sec:02d}Z"
    )


def _percentile(xs: list[float], q: float) -> float:
    if not xs:
        return 0.0
    ys = sorted(xs)
    idx = max(0, min(len(ys) - 1, int(len(ys) * q)))
    return round(ys[idx], 3)


register(Command(
    name="save",
    run=_cmd_save,
    doc="Save the session transcript as a versioned JSON file",
    group="session",
))


# ---------------------------------------------------------------------------
#  :healthcheck
# ---------------------------------------------------------------------------


def _cmd_healthcheck(state: Any, args: list[str], log: Any) -> str:
    if state.client is None:
        raise UsageError("healthcheck: no client available (endpoint not wired)")
    from efe_tool.errors import BridgeError
    t0 = time.perf_counter()
    try:
        serving = bool(state.client.healthcheck())
    except BridgeError as e:
        raise from_bridge_error(e) from e
    ms = int((time.perf_counter() - t0) * 1000)
    status = "SERVING" if serving else "NOT_SERVING"
    if not serving:
        raise NetworkError(
            f"server not serving at {state.endpoint}",
            hint="check the endpoint and try `:endpoint HOST:PORT` to switch",
            context={"endpoint": state.endpoint, "latency_ms": ms},
        )
    return f"{status} (latency {ms} ms)"


register(Command(
    name="healthcheck",
    run=_cmd_healthcheck,
    doc="Probe the gRPC server; prints SERVING + latency",
    group="session",
))


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _stdout_is_tty() -> bool:
    import sys
    try:
        return sys.stdout.isatty()
    except Exception:  # pragma: no cover
        return False


__all__ = ["QuitSignal"]
