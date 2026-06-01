"""Dynamic prompt builder for the REPL.

The default template is ``EFE {catalog}> ``. Users can override via
``--prompt TEMPLATE`` on the command line with placeholders:

* ``{catalog}``  — short catalog name, or ``(no catalog)`` sentinel.
* ``{queries}``  — number of queries since boot.
* ``{uptime}``   — wall-clock uptime as ``HhMMmSSs``.
* ``{endpoint}`` — current gRPC endpoint.
* ``{session}``  — short session id (first 8 chars).

The template is rendered each time a prompt is about to be displayed
so catalog-sensitive changes appear instantly after ``:catalog`` or
``:reload``.

Colors are applied only when stdout is a TTY and ``no_color`` is False.
"""

from __future__ import annotations

import os
import sys
from typing import TYPE_CHECKING

if TYPE_CHECKING:  # pragma: no cover
    from .session import SessionState


# ---------------------------------------------------------------------------
#  ANSI helpers
# ---------------------------------------------------------------------------


RESET = "\x1b[0m"
DIM = "\x1b[2m"
BOLD = "\x1b[1m"
CYAN = "\x1b[36m"
GREEN = "\x1b[32m"
YELLOW = "\x1b[33m"
RED = "\x1b[31m"
BLUE = "\x1b[34m"
MAGENTA = "\x1b[35m"
# readline needs non-printing escape sequences wrapped in \x01 ... \x02
# so it can compute prompt width correctly.
RL_START = "\x01"
RL_END = "\x02"


DEFAULT_TEMPLATE = "EFE {catalog}> "


# ---------------------------------------------------------------------------
#  API
# ---------------------------------------------------------------------------


def build_prompt(
    state: "SessionState",
    *,
    template: str = DEFAULT_TEMPLATE,
    for_readline: bool = True,
    force_color: bool | None = None,
) -> str:
    """Render the prompt string for the current session state.

    Args:
        state: the live :class:`SessionState`.
        template: format string with placeholders.
        for_readline: wrap ANSI sequences with ``\\x01/\\x02`` markers
            so Python ``readline`` measures prompt width correctly.
        force_color: override color auto-detection. ``None`` = auto.
    """
    uptime_s = state.uptime_s
    hours = int(uptime_s // 3600)
    minutes = int((uptime_s % 3600) // 60)
    seconds = int(uptime_s % 60)
    uptime_str = f"{hours}h{minutes:02d}m{seconds:02d}s"

    session_short = state.session_id[:8]

    # Compute color on/off
    if force_color is None:
        use_color = _is_color_ok(state)
    else:
        use_color = bool(force_color)

    # Render — keep catalog separately so we can color only that chunk.
    values = {
        "catalog":  state.catalog_name,
        "queries":  state.queries_count,
        "uptime":   uptime_str,
        "endpoint": state.endpoint,
        "session":  session_short,
    }
    # Paint catalog name cyan + bold when colored.
    if use_color:
        values["catalog"] = _paint_readline_safe(
            state.catalog_name,
            opening=f"{CYAN}{BOLD}",
            closing=RESET,
            for_readline=for_readline,
        )
    try:
        rendered = template.format(**values)
    except KeyError:
        # Unknown placeholder — fall back to the default template.
        rendered = DEFAULT_TEMPLATE.format(**values)
    return rendered


def farewell_line(state: "SessionState") -> str:
    """Short one-liner printed at shutdown."""
    s = state.uptime_s
    h = int(s // 3600)
    m = int((s % 3600) // 60)
    sec = int(s % 60)
    return (
        f"bye — {state.queries_count} queries, "
        f"{state.commands_count} commands, "
        f"{h}h {m:02d}m {sec:02d}s"
    )


def welcome_banner(
    state: "SessionState", version: str, *, force_color: bool | None = None,
) -> list[str]:
    """Multi-line banner shown at boot on stderr."""
    use_color = _is_color_ok(state) if force_color is None else bool(force_color)
    cyan = CYAN if use_color else ""
    dim = DIM if use_color else ""
    reset = RESET if use_color else ""
    lines: list[str] = []
    lines.append(f"{cyan}EFE Mode 1 — interactive REPL{reset} {dim}v{version}{reset}")
    if state.catalog is not None:
        lines.append(
            "catalog: "
            f"{state.catalog_name}@{getattr(state.catalog, 'version', '?')} "
            f"({len(getattr(state.catalog, 'classes', ()))} classes)"
        )
    else:
        lines.append(f"catalog: {dim}(none — use :catalog PATH to load one){reset}")
    scheme = "mTLS" if state.tls else "insecure"
    lines.append(f"endpoint: {state.endpoint} ({scheme})")
    lines.append(
        f"Type {cyan}:help{reset} for commands, "
        f"{cyan}:quit{reset} to exit."
    )
    return lines


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _is_color_ok(state: "SessionState") -> bool:
    if state.no_color:
        return False
    if "NO_COLOR" in os.environ:
        return False
    try:
        return sys.stdout.isatty()
    except Exception:  # pragma: no cover
        return False


def _paint_readline_safe(
    text: str,
    *,
    opening: str,
    closing: str,
    for_readline: bool,
) -> str:
    """Wrap ``text`` in ANSI escapes while respecting readline's
    printable-width accounting.

    In a readline-aware prompt, every non-printing byte must be enclosed
    between ``\\x01`` and ``\\x02`` so the underlying ``rl_expand_prompt``
    can compute the visible width correctly. The naive pattern

        \\x01<ANSI><text><RESET>\\x02

    would tell readline that *the text itself* is non-printing, breaking
    the width calculation. The correct shape is::

        \\x01<ANSI>\\x02<text>\\x01<RESET>\\x02

    That's what this helper emits. Outside a readline context we just
    concatenate the sequences as-is.
    """
    if for_readline:
        return (
            f"{RL_START}{opening}{RL_END}"
            f"{text}"
            f"{RL_START}{closing}{RL_END}"
        )
    return f"{opening}{text}{closing}"


__all__ = [
    "DEFAULT_TEMPLATE",
    "build_prompt",
    "farewell_line",
    "welcome_banner",
    "RESET",
    "CYAN",
    "GREEN",
    "YELLOW",
    "RED",
    "BLUE",
    "MAGENTA",
    "DIM",
    "BOLD",
    "RL_START",
    "RL_END",
]
