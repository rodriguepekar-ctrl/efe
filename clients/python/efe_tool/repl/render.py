"""Human-first renderers for the EFE REPL.

The ``human`` format is the REPL default. Two-column layout, aligned,
optionally colored. Additionally, we delegate to the Lettre A formatters
for ``text``, ``json`` and ``table`` formats when the user opts in
via ``:format``.

No function in this module performs I/O. The caller (:mod:`input_loop`
or a command handler) prints the returned string to stdout.
"""

from __future__ import annotations

from typing import Any, Iterable, Mapping, Optional, Sequence

from ..cli import output as cli_output
from ..cli.output import DocTiming
from .prompt import (
    BLUE,
    BOLD,
    CYAN,
    DIM,
    GREEN,
    MAGENTA,
    RED,
    RESET,
    YELLOW,
)


# ---------------------------------------------------------------------------
#  Label color palette
# ---------------------------------------------------------------------------


# Fallback colors when a class has no explicit ``color`` in the catalog.
_FALLBACK_PALETTE: tuple[str, ...] = (CYAN, YELLOW, MAGENTA, GREEN, BLUE, RED)


#: Mapping from catalog color name → ANSI escape.
_COLOR_NAMES: dict[str, str] = {
    "red": RED, "green": GREEN, "blue": BLUE,
    "yellow": YELLOW, "orange": YELLOW, "cyan": CYAN,
    "magenta": MAGENTA, "purple": MAGENTA,
}


def color_for_label(
    label: str, catalog: Any, use_color: bool,
) -> str:
    """Return the ANSI escape to open a label's color, or empty string."""
    if not use_color:
        return ""
    if catalog is not None:
        classes = getattr(catalog, "classes", ())
        for i, c in enumerate(classes):
            if getattr(c, "name", None) == label:
                cname = getattr(c, "color", None)
                if cname and cname.lower() in _COLOR_NAMES:
                    return _COLOR_NAMES[cname.lower()]
                return _FALLBACK_PALETTE[i % len(_FALLBACK_PALETTE)]
    return ""


def _verdict(result: Any, state: Any = None) -> str:
    """Compute the human verdict for a classify result.

    When ``state`` is provided, we apply the session's effective
    thresholds (which may include ``:thresholds`` overrides) instead
    of relying on the client/server-side zone. This is what makes
    ``:thresholds high 0.95 medium 0.80`` actually change the verdict
    of a subsequent query.

    When ``state`` is ``None`` (unit tests / non-REPL callers), we
    fall back to the flags the server returned.
    """
    conf = float(getattr(result, "confidence", 0.0) or 0.0)

    if state is not None:
        eff_high = state.effective_high_threshold
        eff_medium = state.effective_medium_threshold
        if eff_high is not None and eff_medium is not None:
            if conf < eff_medium:
                return "reject"
            if conf < eff_high:
                return "warn"
            return "trusted"
        # No catalog / no overrides yet → fall through.

    if bool(getattr(result, "reject", False)):
        return "reject"
    extra = getattr(result, "extra", None)
    if isinstance(extra, Mapping):
        zone = extra.get("zone")
        if zone in {"trusted", "warn", "reject"}:
            return str(zone)
    return "trusted"


# ---------------------------------------------------------------------------
#  Human rendering
# ---------------------------------------------------------------------------


def render_query_result_human(
    result: Any,
    *,
    catalog: Any,
    elapsed_ms: float,
    diagnostics: bool,
    use_color: bool,
    state: Any = None,
) -> str:
    """Return the multi-line human representation of a ClassifyResult.

    If ``state`` is passed, verdict computation honours session-level
    threshold overrides. Otherwise it falls back to the server's flags.
    """
    label = str(getattr(result, "label", "?"))
    conf = float(getattr(result, "confidence", 0.0) or 0.0)
    rounds = int(getattr(result, "rounds", 0) or 0)
    verdict = _verdict(result, state)

    lab_color = color_for_label(label, catalog, use_color)
    reset = RESET if use_color else ""

    lines: list[str] = []
    lines.append(
        f"  label:        {lab_color}{label}{reset:<5}"
        f" {DIM if use_color else ''}({BOLD if use_color else ''}confidence "
        f"{conf:.3f}{reset if use_color else ''}"
        f"{DIM if use_color else ''}, rounds {rounds}){reset}"
    )
    lines.append(
        f"  verdict:      {_colored_verdict(verdict, use_color)}"
        f"  {DIM if use_color else ''}({elapsed_ms:.0f} ms){reset}"
    )
    if diagnostics:
        lines += _diag_lines(result, catalog, use_color)
    return "\n".join(lines)


def render_error_human(
    code: str, message: str, *, hint: Optional[str] = None,
    use_color: bool = True,
) -> str:
    color_err = RED + BOLD if use_color else ""
    dim = DIM if use_color else ""
    reset = RESET if use_color else ""
    lines = [f"{color_err}error{reset} {code}: {message}"]
    if hint:
        lines.append(f"  {dim}hint:{reset} {hint}")
    return "\n".join(lines)


def render_info(state: Any, *, use_color: bool) -> str:
    """Render the ``:info`` output."""
    bold = BOLD if use_color else ""
    dim = DIM if use_color else ""
    reset = RESET if use_color else ""
    cat = state.catalog
    cat_descr = (
        f"{getattr(cat, 'name', '?')}@{getattr(cat, 'version', '?')} "
        f"(fingerprint {getattr(cat, 'embedder_fingerprint', '?')})"
        if cat is not None else "(no catalog)"
    )
    high = state.effective_high_threshold
    medium = state.effective_medium_threshold
    rows = [
        ("session", state.session_id),
        ("catalog", cat_descr),
        ("endpoint", state.endpoint),
        ("embedder", state.embedder_model),
        ("budget", f"{state.budget_ms} ms"),
        ("timeout", f"{state.timeout_s:.1f} s"),
        ("rounds", str(state.max_rounds)),
        ("format", state.output_format + (" (pretty)" if state.pretty else "")),
        ("strict", "on" if state.strict else "off"),
        ("diagnostics", "on" if state.diagnostics else "off"),
        ("log", state.log_level),
        (
            "thresholds",
            (
                f"high {high:.3f}, medium {medium:.3f}"
                if high is not None and medium is not None
                else "(catalog not loaded)"
            ),
        ),
    ]
    w = max(len(k) for k, _ in rows)
    out = []
    for k, v in rows:
        out.append(f"{bold}{k.ljust(w)}{reset}  {v}")
    return "\n".join(out)


def render_stats(state: Any, *, use_color: bool) -> str:
    bold = BOLD if use_color else ""
    dim = DIM if use_color else ""
    reset = RESET if use_color else ""
    lines = []
    lines.append(f"{bold}Queries:{reset}    {state.queries_count}")
    lines.append(f"{bold}Commands:{reset}   {state.commands_count}")
    lines.append(f"{bold}Errors:{reset}     {state.error_count}")
    if state.label_counts:
        items = sorted(state.label_counts.items(),
                       key=lambda kv: (-kv[1], kv[0]))
        parts = [f"{k}={v}" for k, v in items]
        lines.append(f"{bold}Labels:{reset}     " + ", ".join(parts))
    if state.latencies_ms:
        xs = sorted(state.latencies_ms)
        p50 = xs[len(xs) // 2]
        p95 = xs[max(0, int(len(xs) * 0.95) - 1)]
        lines.append(f"{bold}Latency p50:{reset} {p50:.0f} ms")
        lines.append(f"{bold}Latency p95:{reset} {p95:.0f} ms")
    s = state.uptime_s
    h = int(s // 3600); m = int((s % 3600) // 60); sec = int(s % 60)
    lines.append(f"{bold}Session:{reset}    {h}h {m:02d}m {sec:02d}s")
    return "\n".join(lines)


def render_classes(catalog: Any, *, use_color: bool) -> str:
    if catalog is None:
        return "(no catalog loaded)"
    classes = getattr(catalog, "classes", ())
    if not classes:
        return "(catalog has no classes)"
    out: list[str] = []
    for c in classes:
        name = getattr(c, "name", "?")
        desc = getattr(c, "description", "") or ""
        sents = len(getattr(c, "canonical_sentences", ()))
        lab_color = color_for_label(name, catalog, use_color)
        reset = RESET if use_color else ""
        out.append(
            f"  {lab_color}{name}{reset:<12}"
            f"  ({sents} sentences)  {desc}"
        )
    return "\n".join(out)


# ---------------------------------------------------------------------------
#  Delegation to the lettre-A formatters for text/json/table
# ---------------------------------------------------------------------------


def render_query_result_delegate(
    result: Any,
    *,
    catalog: Any,
    elapsed_ms: float,
    request_id: str,
    diagnostics: bool,
    output_format: str,
    pretty: bool,
) -> str:
    """Render via the Lettre-A formatters for non-human formats.

    Returns a string (no trailing newline) that the caller can print.
    """
    doc = cli_output.build_classify_document(
        result,
        catalog=catalog,
        timing=DocTiming(
            elapsed_ms=int(elapsed_ms),
            server_ms=int(getattr(result, "total_ms", 0) or 0),
        ),
        request_id=request_id,
        with_diagnostics=diagnostics,
    )
    import io as _io
    buf = _io.StringIO()
    cli_output.render(doc, format=output_format, pretty=pretty, stream=buf)
    return buf.getvalue().rstrip("\n")


# ---------------------------------------------------------------------------
#  Internals
# ---------------------------------------------------------------------------


def _colored_verdict(v: str, use_color: bool) -> str:
    if not use_color:
        return v
    if v == "trusted":
        return f"{GREEN}{v}{RESET}"
    if v == "warn":
        return f"{YELLOW}{v}{RESET}"
    return f"{RED}{BOLD}{v}{RESET}"


def _diag_lines(result: Any, catalog: Any, use_color: bool) -> list[str]:
    sub = getattr(result, "sub_scores", None)
    if not sub:
        return ["  diagnostics:", "    (no sub_scores returned)"]
    top_label = str(getattr(result, "label", ""))
    lines = ["  diagnostics:"]
    # Sort by descending cosine, but preserve top's marker.
    items = sorted(
        [(getattr(s, "label", "?"), float(getattr(s, "cosine", 0.0))) for s in sub],
        key=lambda kv: (-kv[1], kv[0]),
    )
    for name, score in items:
        marker = " ← top" if name == top_label else ""
        color = color_for_label(name, catalog, use_color)
        reset = RESET if use_color else ""
        lines.append(f"    {color}{name:<10}{reset}  {score:.4f}{marker}")
    return lines


__all__ = [
    "color_for_label",
    "render_query_result_human",
    "render_error_human",
    "render_info",
    "render_stats",
    "render_classes",
    "render_query_result_delegate",
]
