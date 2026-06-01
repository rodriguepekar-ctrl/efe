"""Output formatters for the ``efe-admin`` CLI.

Three formats are supported on stdout:

* ``json``  — compact by default, pretty with ``--pretty``.
* ``text``  — key/value, human-friendly, not parseable.
* ``table`` — ASCII table, used by evaluation and multi-row reports.

The same three options apply to the **error document**. Errors always
go to stderr; the success document always goes to stdout (one newline
at the end, no extra blank line).

Invariant (§8.2 of the architecture): stdout contains either the
schema-versioned JSON document, or nothing. Never a log line, never a
progress character, never a banner.
"""

from __future__ import annotations

import json
import sys
import time
from dataclasses import asdict, dataclass
from typing import Any, Iterable, Mapping, Optional, Sequence, TextIO

from .errors import CliError
from .metadata import SCHEMA_VERSION


# ---------------------------------------------------------------------------
#  Document builders
# ---------------------------------------------------------------------------


@dataclass
class DocTiming:
    """Wall-clock timings attached to a classify result."""

    elapsed_ms: int
    server_ms: int


@dataclass
class DocCatalog:
    """Minimal catalog identity for the stdout document."""

    name: str
    version: str
    fingerprint: str


def build_classify_document(
    result: Any,
    *,
    catalog: Any,
    timing: DocTiming,
    request_id: str,
    with_diagnostics: bool = False,
) -> dict[str, Any]:
    """Turn an :class:`efe_tool.types.ClassifyResult` into the canonical
    schema-versioned JSON document.

    The function is pure: it never touches I/O, never raises.
    """
    doc: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "label": _safe_attr(result, "label"),
        "confidence": _safe_float(_safe_attr(result, "confidence")),
        "reject": bool(_safe_attr(result, "reject", False)),
        "alignment_delta": _safe_float(_safe_attr(result, "alignment_delta")),
        "rounds": int(_safe_attr(result, "rounds", 0)),
        "catalog": {
            "name": _safe_attr(catalog, "name", "?"),
            "version": _safe_attr(catalog, "version", "?"),
            "fingerprint": _safe_attr(catalog, "embedder_fingerprint", ""),
        },
        "timing": {
            "elapsed_ms": int(timing.elapsed_ms),
            "server_ms": int(timing.server_ms),
        },
        "request_id": request_id,
    }
    if with_diagnostics:
        doc["diagnostics"] = _build_diagnostics(result)
    return doc


def _build_diagnostics(result: Any) -> dict[str, Any]:
    diag: dict[str, Any] = {}
    for key in ("swarm_id", "text_hash", "top_cosine", "total_ms"):
        v = _safe_attr(result, key, None)
        if v is not None:
            diag[key] = v
    sub_scores = _safe_attr(result, "sub_scores", None)
    if sub_scores:
        diag["sub_scores"] = [
            {
                "label": _safe_attr(s, "label", "?"),
                "cosine": _safe_float(_safe_attr(s, "cosine")),
            }
            for s in sub_scores
        ]
    extra = _safe_attr(result, "extra", None)
    if isinstance(extra, Mapping):
        diag["extra"] = dict(extra)
    return diag


# ---------------------------------------------------------------------------
#  Healthcheck document
# ---------------------------------------------------------------------------


def build_healthcheck_document(
    *,
    serving: bool,
    latency_ms: int,
    endpoint: str,
    request_id: str,
) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "SERVING" if serving else "NOT_SERVING",
        "latency_ms": int(latency_ms),
        "endpoint": endpoint,
        "request_id": request_id,
    }


# ---------------------------------------------------------------------------
#  Compile-catalog document
# ---------------------------------------------------------------------------


def build_compile_document(catalog: Any, *, request_id: str) -> dict[str, Any]:
    classes = _safe_attr(catalog, "classes", ())
    return {
        "schema_version": SCHEMA_VERSION,
        "catalog": {
            "name": _safe_attr(catalog, "name", "?"),
            "version": _safe_attr(catalog, "version", "?"),
            "fingerprint": _safe_attr(catalog, "embedder_fingerprint", ""),
            "dim": int(_safe_attr(catalog, "dim", 0)),
            "classes": [
                {
                    "name": _safe_attr(c, "name", "?"),
                    "sentences": len(_safe_attr(c, "canonical_sentences", ())),
                }
                for c in classes
            ],
        },
        "request_id": request_id,
    }


# ---------------------------------------------------------------------------
#  Validate document
# ---------------------------------------------------------------------------


def build_validate_document(
    *,
    catalog_path: str,
    ok: bool,
    issues: Sequence[Mapping[str, Any]] = (),
    warnings: Sequence[Mapping[str, Any]] = (),
    request_id: str,
) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "valid": bool(ok),
        "catalog_path": catalog_path,
        "issues": [dict(x) for x in issues],
        "warnings": [dict(x) for x in warnings],
        "request_id": request_id,
    }


# ---------------------------------------------------------------------------
#  Version document
# ---------------------------------------------------------------------------


def build_version_document(version_string: str) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "version": version_string,
        "program": "efe-admin",
    }


# ---------------------------------------------------------------------------
#  Rendering dispatch
# ---------------------------------------------------------------------------


def render(
    document: Mapping[str, Any],
    *,
    format: str = "json",
    pretty: bool = False,
    stream: TextIO | None = None,
) -> None:
    """Write ``document`` to stdout according to ``format``.

    Never returns a value and never raises on encoding issues — if the
    stream explodes, it's the process's problem, not the formatter's.
    """
    target = stream if stream is not None else sys.stdout
    if format == "json":
        _render_json(document, target, pretty=pretty)
    elif format == "text":
        _render_text(document, target)
    elif format == "table":
        _render_table(document, target)
    else:  # pragma: no cover — guarded by config validator
        _render_json(document, target, pretty=pretty)


def _render_json(doc: Mapping[str, Any], stream: TextIO, *, pretty: bool) -> None:
    if pretty:
        s = json.dumps(doc, indent=2, ensure_ascii=False, default=str)
    else:
        s = json.dumps(doc, ensure_ascii=False, default=str, separators=(",", ":"))
    stream.write(s + "\n")
    stream.flush()


def _render_text(doc: Mapping[str, Any], stream: TextIO) -> None:
    lines: list[str] = []
    _render_text_rec(doc, prefix="", out=lines)
    stream.write("\n".join(lines) + "\n")
    stream.flush()


def _render_text_rec(obj: Any, *, prefix: str, out: list[str]) -> None:
    if isinstance(obj, Mapping):
        # max key width for alignment
        keys = [str(k) for k in obj.keys()]
        width = max((len(k) for k in keys), default=0)
        for k in keys:
            v = obj[k]
            label = f"{prefix}{k.ljust(width)}"
            if isinstance(v, Mapping):
                out.append(f"{label}:")
                _render_text_rec(v, prefix=prefix + "  ", out=out)
            elif isinstance(v, list):
                out.append(f"{label}:")
                for i, item in enumerate(v):
                    if isinstance(item, Mapping):
                        out.append(f"{prefix}  - [{i}]")
                        _render_text_rec(item, prefix=prefix + "    ", out=out)
                    else:
                        out.append(f"{prefix}  - {_compact(item)}")
            else:
                out.append(f"{label}: {_compact(v)}")
    else:
        out.append(f"{prefix}{_compact(obj)}")


def _render_table(doc: Mapping[str, Any], stream: TextIO) -> None:
    """Best-effort ASCII table.

    For the canonical one-row documents (classify, healthcheck), the
    table is a two-column key/value presentation of the top-level keys.
    Nested structures are JSON-encoded and truncated.
    """
    rows: list[tuple[str, str]] = []
    for k, v in doc.items():
        if isinstance(v, (Mapping, list)):
            val = json.dumps(v, ensure_ascii=False, default=str)
        else:
            val = _compact(v)
        rows.append((str(k), val[:120]))
    w1 = max(len(k) for k, _ in rows)
    w2 = max(len(v) for _, v in rows)
    sep = "+" + "-" * (w1 + 2) + "+" + "-" * (w2 + 2) + "+"
    stream.write(sep + "\n")
    stream.write(f"| {'KEY'.ljust(w1)} | {'VALUE'.ljust(w2)} |\n")
    stream.write(sep + "\n")
    for k, v in rows:
        stream.write(f"| {k.ljust(w1)} | {v.ljust(w2)} |\n")
    stream.write(sep + "\n")
    stream.flush()


# ---------------------------------------------------------------------------
#  Error rendering
# ---------------------------------------------------------------------------


def render_error(
    err: CliError,
    *,
    format: str = "json",
    pretty: bool = False,
    request_id: Optional[str] = None,
    stream: TextIO | None = None,
) -> None:
    """Render an error document to **stderr** in the requested format.

    Unlike :func:`render`, the output goes to stderr by design — stdout
    stays pristine for pipelines even on error.
    """
    target = stream if stream is not None else sys.stderr
    payload = {
        "schema_version": SCHEMA_VERSION,
        "error": err.to_payload(),
    }
    if request_id:
        payload["request_id"] = request_id

    if format == "json":
        if pretty:
            target.write(
                json.dumps(payload, indent=2, ensure_ascii=False, default=str)
                + "\n"
            )
        else:
            target.write(
                json.dumps(payload, ensure_ascii=False, default=str,
                           separators=(",", ":"))
                + "\n"
            )
        target.flush()
        return

    # text format
    lines = [f"error: {err.error_code} (exit {err.exit_code})"]
    lines.append(f"  message: {err.message}")
    if err.hint:
        lines.append(f"  hint: {err.hint}")
    if err.context:
        for k, v in err.context.items():
            lines.append(f"  {k}: {_compact(v)}")
    if request_id:
        lines.append(f"  request_id: {request_id}")
    target.write("\n".join(lines) + "\n")
    target.flush()


# ---------------------------------------------------------------------------
#  Utilities
# ---------------------------------------------------------------------------


def _safe_attr(obj: Any, name: str, default: Any = None) -> Any:
    if isinstance(obj, Mapping):
        return obj.get(name, default)
    return getattr(obj, name, default)


def _safe_float(v: Any) -> float:
    try:
        return float(v)
    except (TypeError, ValueError):
        return 0.0


def _compact(v: Any) -> str:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    if isinstance(v, str):
        return v
    try:
        return json.dumps(v, ensure_ascii=False, default=str)
    except Exception:  # pragma: no cover
        return repr(v)


__all__ = [
    "DocTiming",
    "DocCatalog",
    "build_classify_document",
    "build_healthcheck_document",
    "build_compile_document",
    "build_validate_document",
    "build_version_document",
    "render",
    "render_error",
]
