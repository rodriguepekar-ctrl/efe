"""Mutable session state for the EFE REPL.

A single :class:`SessionState` instance lives for the whole process
lifetime. Every command that the user types mutates fields on it.
No lock is required because the REPL is single-threaded
(see ``LETTER_B_ARCHITECTURE.md`` ADR-B-009).

The state is distinct from the *boot config* (``cli.config.ResolvedConfig``)
which is immutable and frozen at startup. The ``boot_cfg`` attribute
on the session keeps a reference to the origin config so a user can
reset a field with ``:xxx reset`` semantics.
"""

from __future__ import annotations

import os
import sys
import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Optional

from ..cli.config import ResolvedConfig


def _stdout_is_tty() -> bool:
    """Return True iff stdout is attached to a TTY and colour is allowed
    by the :envvar:`NO_COLOR` convention.

    Used by :meth:`SessionState.from_boot` to auto-disable ANSI colour
    output when the REPL is piped into another tool.
    """
    if "NO_COLOR" in os.environ:
        return False
    try:
        return bool(sys.stdout.isatty())
    except Exception:  # pragma: no cover
        return False


# ---------------------------------------------------------------------------
#  Dataclass
# ---------------------------------------------------------------------------


@dataclass
class SessionState:
    """Mutable in-memory state of one REPL session.

    Attributes are grouped thematically. See
    ``LETTER_B_ARCHITECTURE.md`` §5.4 for the canonical description.
    """

    # --- identity -----------------------------------------------------
    session_id: str
    started_at: float                             # time.monotonic() at boot
    started_at_wall: float                        # time.time()     at boot (UTC s)
    boot_cfg: ResolvedConfig                      # frozen, source of resets

    # --- long-lived resources ----------------------------------------
    embedder: Any = None                          # LocalSTEmbedder | None
    client: Any = None                            # EfeClient | None
    catalog: Any = None                           # AnchorCatalog | None

    # --- network / runtime tunables ----------------------------------
    endpoint: str = ""
    embedder_model: str = ""
    budget_ms: int = 5_000
    timeout_s: float = 10.0
    max_rounds: int = 16
    strict: bool = False
    tls: bool = False
    tls_ca: Optional[str] = None
    jwt: Optional[str] = None
    tenant: Optional[str] = None

    # --- output preferences (runtime) --------------------------------
    output_format: str = "human"                  # human | text | json | table
    pretty: bool = False
    diagnostics: bool = False
    no_color: bool = False

    # --- threshold overrides (session-local) -------------------------
    threshold_high_override: Optional[float] = None
    threshold_medium_override: Optional[float] = None

    # --- logging -----------------------------------------------------
    log_level: str = "info"

    # --- metrics -----------------------------------------------------
    queries_count: int = 0
    commands_count: int = 0
    error_count: int = 0
    label_counts: dict[str, int] = field(default_factory=dict)
    latencies_ms: list[float] = field(default_factory=list)

    # --- history / transcript ----------------------------------------
    history: list[str] = field(default_factory=list)      # input lines
    transcript: list[dict[str, Any]] = field(default_factory=list)
    last_results: list[Any] = field(default_factory=list)  # last ClassifyResult

    # --- caps --------------------------------------------------------
    history_cap: int = 10_000
    transcript_cap: int = 10_000
    last_results_cap: int = 100

    # ------------------------------------------------------------------
    #  Factory
    # ------------------------------------------------------------------

    @classmethod
    def from_boot(cls, cfg: ResolvedConfig) -> "SessionState":
        """Build a fresh :class:`SessionState` from a boot config.

        Heavy singletons (embedder / client / catalog) are wired by
        the boot phase in :mod:`efe_tool.cli.commands.repl` — not here,
        to keep this constructor pure.

        Color auto-detection: if ``cfg.no_color`` is ``True``, colors
        are disabled. Otherwise, we additionally check that stdout is
        a TTY — mirroring the Lettre A convention so a piped REPL
        session never emits ANSI escapes into ``jq`` or ``less``.
        """
        no_color = bool(cfg.no_color) or not _stdout_is_tty()
        return cls(
            session_id=str(uuid.uuid4()),
            started_at=time.monotonic(),
            started_at_wall=time.time(),
            boot_cfg=cfg,
            endpoint=cfg.endpoint,
            embedder_model=cfg.embedder_model,
            budget_ms=cfg.budget_ms,
            timeout_s=cfg.timeout_s,
            max_rounds=cfg.max_rounds,
            strict=cfg.strict,
            tls=cfg.tls,
            tls_ca=cfg.tls_ca,
            jwt=cfg.jwt,
            tenant=cfg.tenant,
            output_format="human",  # REPL-specific default, see ADR-B-005
            pretty=cfg.pretty,
            diagnostics=cfg.with_diagnostics,
            no_color=no_color,
            log_level=cfg.log_level,
        )

    # ------------------------------------------------------------------
    #  Derived views
    # ------------------------------------------------------------------

    @property
    def uptime_s(self) -> float:
        return time.monotonic() - self.started_at

    @property
    def catalog_name(self) -> str:
        """Short name for the active catalog, or a sentinel."""
        if self.catalog is None:
            return "(no catalog)"
        return str(getattr(self.catalog, "name", "?"))

    @property
    def effective_high_threshold(self) -> Optional[float]:
        """Effective confidence_high threshold in force for this session."""
        if self.threshold_high_override is not None:
            return self.threshold_high_override
        if self.catalog is None:
            return None
        th = getattr(self.catalog, "thresholds", None)
        return getattr(th, "confidence_high", None) if th is not None else None

    @property
    def effective_medium_threshold(self) -> Optional[float]:
        if self.threshold_medium_override is not None:
            return self.threshold_medium_override
        if self.catalog is None:
            return None
        th = getattr(self.catalog, "thresholds", None)
        return getattr(th, "confidence_medium", None) if th is not None else None

    # ------------------------------------------------------------------
    #  Mutation helpers (used by commands)
    # ------------------------------------------------------------------

    def record_query(self, text: str, result: Any, elapsed_ms: float) -> None:
        """Update counters, label distribution and latency window after
        a successful classify."""
        self.queries_count += 1
        self.history.append(text)
        self._cap(self.history, self.history_cap)
        label = getattr(result, "label", None)
        if isinstance(label, str):
            self.label_counts[label] = self.label_counts.get(label, 0) + 1
        self.latencies_ms.append(float(elapsed_ms))
        self._cap(self.latencies_ms, self.last_results_cap * 4)
        self.last_results.append(result)
        self._cap(self.last_results, self.last_results_cap)
        self.transcript.append({
            "kind": "query",
            "at": time.time(),
            "text": text,
            "label": label,
            "confidence": float(getattr(result, "confidence", 0.0) or 0.0),
            "rounds": int(getattr(result, "rounds", 0) or 0),
            "elapsed_ms": float(elapsed_ms),
        })
        self._cap(self.transcript, self.transcript_cap)

    def record_command(
        self, name: str, args: list[str], outcome: str = "ok",
        details: Optional[str] = None,
    ) -> None:
        self.commands_count += 1
        self.history.append(f":{name}" + (" " + " ".join(args) if args else ""))
        self._cap(self.history, self.history_cap)
        self.transcript.append({
            "kind": "command",
            "at": time.time(),
            "name": name,
            "args": list(args),
            "outcome": outcome,
            "details": details,
        })
        self._cap(self.transcript, self.transcript_cap)

    def record_error(self, code: str, message: str) -> None:
        self.error_count += 1
        self.transcript.append({
            "kind": "error",
            "at": time.time(),
            "code": code,
            "message": message,
        })
        self._cap(self.transcript, self.transcript_cap)

    def reset_threshold_overrides(self) -> None:
        self.threshold_high_override = None
        self.threshold_medium_override = None

    # ------------------------------------------------------------------
    #  Internals
    # ------------------------------------------------------------------

    @staticmethod
    def _cap(seq: list, cap: int) -> None:
        """Trim ``seq`` in place to at most ``cap`` entries (FIFO)."""
        over = len(seq) - cap
        if over > 0:
            del seq[:over]


__all__ = ["SessionState"]
