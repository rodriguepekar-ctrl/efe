"""Main input loop for the REPL.

Public surface:

* :func:`run_session` — boot → loop → shutdown, top-level entry used
  by the CLI dispatcher.
* :func:`run_query`   — execute one classification query; exported for
  the explicit ``:classify`` command.
* :func:`read_line`   — read one logical line (with multi-line support);
  exposed for tests.

Design notes
------------

* **Single-threaded** (ADR-B-009). No locks, no worker threads.

* **Readline** is imported defensively; on platforms where it's
  missing (Windows without pyreadline3), the loop still works but
  loses line editing and tab completion.

* **Signal handling**: we use a small state machine via a nonlocal
  flag set inside a SIGINT handler. During ``input()``, a SIGINT
  raises ``KeyboardInterrupt`` naturally; during an RPC, we catch
  it at the loop level and cancel the request (best effort — the
  underlying ``grpcio`` library doesn't expose fine-grained
  cancellation from Python; we rely on ``channel.close``).

* **Batch mode**: if stdin is not a TTY (e.g. piped via
  ``echo 'hello\\n:quit' | efe-admin repl``), the loop skips the
  prompt, reads lines one by one and exits on EOF.  This makes the
  REPL testable via ``subprocess``.
"""

from __future__ import annotations

import os
import signal
import sys
import time
from dataclasses import replace
from typing import Any, Optional, TextIO

from ..cli.config import ResolvedConfig, harmonise_budget_and_timeout
from ..cli.errors import (
    CliError,
    CatalogInvalidError,
    CatalogNotFoundError,
    InterruptedError as CliInterruptedError,
    NetworkError,
    UsageError,
    from_bridge_error,
)
from ..cli.logging import Logger, setup_logging
from . import commands as _cmds
from .commands.session import QuitSignal
from .history import History, resolve_history_path
from .parser import CommandToken, EmptyToken, QueryToken, parse_line
from .prompt import (
    DEFAULT_TEMPLATE,
    build_prompt,
    farewell_line,
    welcome_banner,
)
from .render import (
    render_error_human,
    render_query_result_delegate,
    render_query_result_human,
)
from .session import SessionState


# ---------------------------------------------------------------------------
#  Public API
# ---------------------------------------------------------------------------


def run_session(
    cfg: ResolvedConfig,
    *,
    log: Optional[Logger] = None,
    history_path: Optional[str] = None,
    history_enabled: bool = True,
    prompt_template: str = DEFAULT_TEMPLATE,
    stdin: Optional[TextIO] = None,
    stdout: Optional[TextIO] = None,
    stderr: Optional[TextIO] = None,
    bootstrap_catalog: bool = True,
) -> int:
    """Run a full REPL session. Returns a POSIX exit code.

    Args:
        cfg: boot config (immutable, frozen).
        log: a configured CLI logger; constructed from ``cfg`` if None.
        history_path: explicit path; resolved via XDG if None.
        history_enabled: False disables all persistence.
        prompt_template: format string for the prompt.
        stdin / stdout / stderr: override streams for tests.
        bootstrap_catalog: if False, skip catalog loading at boot
            (used by tests that want to drive the state manually).
    """
    log = log or setup_logging(cfg)
    _stdin = stdin if stdin is not None else sys.stdin
    _stdout = stdout if stdout is not None else sys.stdout
    _stderr = stderr if stderr is not None else sys.stderr

    state = SessionState.from_boot(cfg)

    # --- Boot: embedder, client, catalog ---
    _boot_resources(state, cfg, log, bootstrap_catalog=bootstrap_catalog)

    # --- History ---
    hist = History(
        path=resolve_history_path(history_path),
        enabled=history_enabled,
    )
    hist.load()
    hist.populate_readline()

    # --- Completer ---
    _install_completer(state)

    # --- Banner ---
    if _is_tty(_stderr):
        for line in welcome_banner(
            state, version=_efe_version(), force_color=not state.no_color,
        ):
            print(line, file=_stderr)

    # --- Signal handling for main loop ---
    _prev_sigint = signal.getsignal(signal.SIGINT)
    # We rely on Python's default SIGINT → KeyboardInterrupt behaviour
    # at the prompt; the only tweak we need is to make sure that a
    # spurious SIGINT during a blocking ``input()`` doesn't get lost.
    signal.signal(signal.SIGINT, signal.default_int_handler)

    log.info(
        "repl_boot",
        catalog=state.catalog_name,
        endpoint=state.endpoint,
        embedder=state.embedder_model,
        version=_efe_version(),
    )
    log.info("repl_ready")

    try:
        exit_code = _main_loop(
            state, log, hist,
            prompt_template=prompt_template,
            stdin=_stdin, stdout=_stdout, stderr=_stderr,
        )
    finally:
        signal.signal(signal.SIGINT, _prev_sigint)
        _shutdown(state, log, hist, stderr=_stderr)
    return exit_code


def run_query(text: str, state: SessionState, log: Logger) -> str:
    """Execute one classification query. Returns the string to print
    on stdout. Raises CliError on failure.
    """
    if state.catalog is None:
        raise UsageError(
            "no catalog loaded",
            hint="use `:catalog PATH` before classifying",
        )
    if state.client is None:
        raise UsageError(
            "no gRPC client initialised",
            hint="use `:endpoint HOST:PORT` to configure one",
        )

    # Mirror Lettre A's timeout safety: bump timeout if < budget/1000 + 0.5.
    min_timeout = state.budget_ms / 1000.0 + 0.5
    if state.timeout_s < min_timeout:
        log.warn(
            "timeout_bumped",
            old_timeout_s=state.timeout_s,
            new_timeout_s=round(min_timeout, 3),
            budget_ms=state.budget_ms,
        )
        state.timeout_s = round(min_timeout, 3)

    from efe_tool.errors import BridgeError

    request_id = f"{state.session_id}-{state.queries_count + 1}"
    log.info(
        "classify_start",
        text_len=len(text),
        budget_ms=state.budget_ms,
        max_rounds=state.max_rounds,
        request_id=request_id,
    )

    t0 = time.perf_counter()
    try:
        result = state.client.classify(
            text,
            state.catalog,
            request_id=request_id,
            return_sub_scores=state.diagnostics,
            max_rounds=state.max_rounds,
            budget_wall_clock_ms=state.budget_ms,
        )
    except BridgeError as e:
        raise from_bridge_error(e) from e
    elapsed_ms = (time.perf_counter() - t0) * 1000.0

    # Strict mode: mark result with a trailing ⚠ but do NOT raise in
    # the REPL (quitting on low confidence would defeat the tuning use
    # case). The marker is added in the render step.
    state.record_query(text, result, elapsed_ms)

    log.info(
        "classify_done",
        label=getattr(result, "label", None),
        confidence=float(getattr(result, "confidence", 0.0) or 0.0),
        rounds=int(getattr(result, "rounds", 0) or 0),
        elapsed_ms=round(elapsed_ms, 2),
        request_id=request_id,
    )

    return _format_query_result(result, state, elapsed_ms, request_id)


def read_line(
    state: SessionState,
    *,
    prompt_template: str,
    stdin: TextIO,
    stdout: TextIO,
) -> Optional[str]:
    """Read one logical line (with multi-line support). Returns ``None``
    on EOF.

    On TTY, uses ``input()`` with a formatted prompt (readline-friendly
    ANSI wrapping). On non-TTY, reads from ``stdin`` without prompting.
    """
    tty = _is_tty(stdin) and _is_tty(stdout)

    if tty:
        first_prompt = build_prompt(
            state,
            template=prompt_template,
            for_readline=True,
        )
    else:
        first_prompt = ""

    try:
        line = _readline_from(stdin, first_prompt, tty=tty)
    except EOFError:
        return None

    if line is None:
        return None

    # Triple-quoted block?
    stripped = line.strip()
    if stripped == '"""' or stripped.startswith('"""'):
        return _read_triple_quoted(
            state, line, stdin=stdin, tty=tty,
            prompt_template=prompt_template,
        )

    # Backslash continuation?
    acc = [line.rstrip("\n")]
    while acc[-1].endswith("\\"):
        acc[-1] = acc[-1][:-1]  # strip trailing backslash
        cont_prompt = "...  " if tty else ""
        try:
            nxt = _readline_from(stdin, cont_prompt, tty=tty)
        except EOFError:
            break
        if nxt is None:
            break
        acc.append(nxt.rstrip("\n"))
    return "\n".join(acc)


# ---------------------------------------------------------------------------
#  Boot
# ---------------------------------------------------------------------------


def _boot_resources(
    state: SessionState,
    cfg: ResolvedConfig,
    log: Logger,
    *,
    bootstrap_catalog: bool,
) -> None:
    """Load embedder, client and optionally a catalog at startup."""
    from efe_tool.errors import BridgeError

    try:
        from efe_tool.embedder import LocalSTEmbedder
        state.embedder = LocalSTEmbedder(state.embedder_model)
    except BridgeError as e:
        # Non-fatal: the REPL can still start and the user can load
        # a different embedder via ``:embedder``. Log the reason.
        log.warn(
            "repl_embedder_unavailable",
            error=str(e),
            model=state.embedder_model,
        )

    # The client requires a non-None embedder (it validates catalog
    # fingerprint against ``embedder.fingerprint()`` on every classify).
    # If the embedder could not be loaded, do NOT construct a client —
    # ``run_query`` checks ``state.client is None`` and returns a clean
    # UsageError instead of AttributeError at call time.
    if state.embedder is None:
        log.warn(
            "repl_skipping_client_no_embedder",
            hint="use :embedder MODEL to load an embedder, "
                 "then :reload or :catalog to resume",
        )
    else:
        try:
            from efe_tool.client import EfeClient
            state.client = EfeClient(
                endpoint=state.endpoint,
                embedder=state.embedder,
                jwt=state.jwt,
                tenant=state.tenant,
                timeout_s=state.timeout_s,
                insecure=not state.tls,
            )
        except BridgeError as e:
            log.warn(
                "repl_client_unavailable",
                error=str(e),
                endpoint=state.endpoint,
            )

    if bootstrap_catalog and cfg.catalog_path is not None:
        try:
            from .commands.catalog import _load_into_state
            _load_into_state(state, cfg.catalog_path, log)
        except CliError as e:
            log.warn(
                "repl_catalog_bootstrap_failed",
                error=e.error_code,
                message=e.message,
            )


# ---------------------------------------------------------------------------
#  Main loop
# ---------------------------------------------------------------------------


def _main_loop(
    state: SessionState,
    log: Logger,
    hist: History,
    *,
    prompt_template: str,
    stdin: TextIO,
    stdout: TextIO,
    stderr: TextIO,
) -> int:
    """Read-parse-eval-print loop. Returns an exit code."""
    while True:
        try:
            line = read_line(
                state,
                prompt_template=prompt_template,
                stdin=stdin,
                stdout=stdout,
            )
        except KeyboardInterrupt:
            # SIGINT at empty prompt: print a newline + hint, loop.
            print(file=stderr)
            print("(use :quit or Ctrl-D to exit)", file=stderr)
            continue

        if line is None:
            # EOF (Ctrl-D) — quit gracefully.
            break

        try:
            token = parse_line(line)
        except ValueError as e:
            _print_error(stderr, state, UsageError(str(e)))
            continue

        try:
            if isinstance(token, EmptyToken):
                continue
            if isinstance(token, CommandToken):
                try:
                    result = _cmds.dispatch(token.name, list(token.args),
                                            state, log)
                except QuitSignal:
                    hist.append(token.signature())
                    state.record_command(
                        token.name, list(token.args), outcome="ok",
                    )
                    return 0
                except CliError as e:
                    _print_error(stderr, state, e)
                    state.record_error(e.error_code, e.message)
                    state.record_command(
                        token.name, list(token.args),
                        outcome="error", details=e.message,
                    )
                    hist.append(token.signature())
                    continue
                else:
                    hist.append(token.signature())
                    state.record_command(
                        token.name, list(token.args), outcome="ok",
                        details=result,
                    )
                    if result:
                        print(result, file=stdout, flush=True)
                continue

            if isinstance(token, QueryToken):
                try:
                    out = run_query(token.text, state, log)
                except CliError as e:
                    _print_error(stderr, state, e)
                    state.record_error(e.error_code, e.message)
                    hist.append(token.text)
                    continue
                hist.append(token.text)
                if out:
                    print(out, file=stdout, flush=True)
                continue
        except KeyboardInterrupt:
            # SIGINT during command / query. We already got the tuning
            # done in ``run_query``; cancel the ongoing RPC if we can
            # and report.
            _cancel_inflight(state, log)
            log.info("repl_query_interrupted")
            print("(interrupted)", file=stderr)
            continue
        except Exception as e:
            # The line of last defense (P3 / I4). Under no circumstance
            # does an exception propagate out of the loop.
            log.error(
                "repl_unexpected",
                error=str(e),
                type=type(e).__name__,
            )
            _print_error(stderr, state, CliError(
                f"unexpected error: {type(e).__name__}: {e}", cause=e,
            ))
            state.record_error("BUG", f"{type(e).__name__}: {e}")
            continue

    return 0


# ---------------------------------------------------------------------------
#  Output helpers
# ---------------------------------------------------------------------------


def _format_query_result(
    result: Any, state: SessionState, elapsed_ms: float, request_id: str,
) -> str:
    if state.output_format == "human":
        out = render_query_result_human(
            result,
            catalog=state.catalog,
            elapsed_ms=elapsed_ms,
            diagnostics=state.diagnostics,
            use_color=not state.no_color,
            state=state,
        )
        if state.strict and (
            bool(getattr(result, "reject", False))
            or _is_low_conf(result, state)
        ):
            out += "  ⚠ strict mode: result flagged"
        return out
    return render_query_result_delegate(
        result,
        catalog=state.catalog,
        elapsed_ms=elapsed_ms,
        request_id=request_id,
        diagnostics=state.diagnostics,
        output_format=state.output_format,
        pretty=state.pretty,
    )


def _is_low_conf(result: Any, state: SessionState) -> bool:
    high = state.effective_high_threshold
    if high is None:
        return False
    return float(getattr(result, "confidence", 0.0) or 0.0) < high


def _print_error(stderr: TextIO, state: SessionState, err: CliError) -> None:
    stderr.write(
        render_error_human(
            err.error_code, err.message, hint=err.hint,
            use_color=not state.no_color,
        )
        + "\n"
    )
    stderr.flush()


def _cancel_inflight(state: SessionState, log: Logger) -> None:
    """Best-effort cancellation of an in-flight gRPC call."""
    client = getattr(state, "client", None)
    if client is None:
        return
    chan = getattr(client, "_channel", None)
    if chan is None:
        return
    try:
        chan.close()
        client._channel = None
        client._stub = None
    except Exception as e:  # pragma: no cover
        log.warn("repl_cancel_failed", error=str(e))


# ---------------------------------------------------------------------------
#  Completer installation
# ---------------------------------------------------------------------------


def _install_completer(state: SessionState) -> None:
    from . import completer
    from .commands import canonical_names

    enum_values = {
        "format": ("human", "text", "json", "table"),
        "log": ("debug", "info", "warn", "error", "off", "reset"),
        "pretty": ("on", "off", "reset"),
        "strict": ("on", "off", "reset"),
        "diagnostics": ("on", "off", "reset"),
    }
    path_args = ("catalog", "save", "history")
    c = completer.Completer(
        command_names=canonical_names(),
        enum_values=enum_values,
        path_args=path_args,
    )
    completer.install(c)


# ---------------------------------------------------------------------------
#  Reading primitives
# ---------------------------------------------------------------------------


def _readline_from(
    stream: TextIO, prompt: str, *, tty: bool,
) -> Optional[str]:
    """Wrapper around ``input``/``readline`` that gracefully handles
    non-TTY batch mode."""
    if tty:
        # Use ``input`` so readline history/edit work.
        return input(prompt)
    # Batch mode: read one line from ``stream``.
    line = stream.readline()
    if line == "":
        raise EOFError()
    return line.rstrip("\n")


def _read_triple_quoted(
    state: SessionState,
    first_line: str,
    *,
    stdin: TextIO,
    tty: bool,
    prompt_template: str,
) -> str:
    """Accumulate lines until a terminating ``\"\"\"`` appears."""
    buf: list[str] = []
    body = first_line.strip()
    if body.startswith('"""') and body.endswith('"""') and len(body) >= 6:
        return body[3:-3]
    # Multi-line block
    if body.startswith('"""'):
        buf.append(body[3:])
    while True:
        cont_prompt = "...  " if tty else ""
        try:
            nxt = _readline_from(stdin, cont_prompt, tty=tty)
        except EOFError:
            break
        if nxt is None:
            break
        stripped = nxt.strip()
        if stripped.endswith('"""'):
            buf.append(nxt.rstrip("\n")[:-3].rstrip())
            break
        buf.append(nxt.rstrip("\n"))
    return "\n".join(ln for ln in buf if ln != "")


# ---------------------------------------------------------------------------
#  Shutdown
# ---------------------------------------------------------------------------


def _shutdown(
    state: SessionState,
    log: Logger,
    hist: History,
    *,
    stderr: TextIO,
) -> None:
    # Sync readline → history object so lines edited interactively are
    # persisted even if not captured by append().
    hist.sync_from_readline()
    hist.flush()
    if state.client is not None:
        try:
            state.client.close()
        except Exception as e:  # pragma: no cover
            log.warn("repl_client_close_failed", error=str(e))
    log.info(
        "repl_shutdown",
        queries=state.queries_count,
        commands=state.commands_count,
        errors=state.error_count,
        duration_s=round(state.uptime_s, 2),
    )
    if _is_tty(stderr):
        print(farewell_line(state), file=stderr)


# ---------------------------------------------------------------------------
#  Misc
# ---------------------------------------------------------------------------


def _is_tty(stream: Any) -> bool:
    try:
        return bool(stream.isatty())
    except Exception:
        return False


def _efe_version() -> str:
    try:
        from efe_tool import __version__
        return __version__
    except Exception:  # pragma: no cover
        return "?"


__all__ = ["run_session", "run_query", "read_line"]
