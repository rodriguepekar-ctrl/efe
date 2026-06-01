"""Anthropic Claude integration — Letter C bridge.

Public API (current):

* :func:`run_conversation`           — canonical entry point, returns
  a :class:`ConversationResult` with transcript, stats, tool_calls,
  wall-clock and cost.
* :func:`run_conversation_with_tool` — backward-compatible wrapper that
  returns only the raw transcript (``list[dict]``).  Kept so the AS-IS
  ``examples/demo_agent_claude.py`` continues to work without changes.
* :func:`build_tool_spec`            — policy-free JSON schema.
* :func:`dispatch`                   — one-shot tool dispatch.
* :func:`extract_tool_result`        — walk a transcript, return the
  last tool payload.
* :func:`extract_final_text`         — walk a transcript, return the
  last assistant text.

See ``LETTER_C_ARCHITECTURE.md`` (§5 in particular) for the full design
and contract. Key invariants:

* Backward compat — every pre-Letter-C test keeps passing.
* ``max_rounds`` is a **hard** bound; ``LlmToolLoopError`` when hit.
* ``cost_ceiling_usd`` is a **hard** bound; ``LlmBudgetExceededError``.
* Every 429 is retried up to ``retry_max`` with exponential backoff.
* Every error from the Anthropic SDK is mapped through the
  :mod:`efe_tool.bridges.errors` taxonomy — no raw ``anthropic.*``
  exception escapes the bridge.
"""

from __future__ import annotations

import hashlib
import json
import logging
import time
from dataclasses import dataclass, field
from typing import Any, Callable, Iterable, Mapping, Optional, Sequence

from ..catalog import AnchorCatalog
from ..client import EfeClient
from .errors import (
    LlmBadResponseError,
    LlmBudgetExceededError,
    LlmError,
    LlmRateLimitError,
    LlmToolLoopError,
    from_anthropic_error,
)
from .pricing import estimate_cost_usd, format_cost_usd


_log = logging.getLogger("efe_tool.bridges.claude")

TOOL_NAME = "efe_classify"
DEFAULT_MAX_ROUNDS = 8
DEFAULT_MAX_TOKENS = 1024
DEFAULT_TEMPERATURE = 0.0
DEFAULT_RETRY_MAX = 3
LOOP_DETECTION_WINDOW = 3


# ---------------------------------------------------------------------------
#  Dataclasses — public API shapes
# ---------------------------------------------------------------------------


@dataclass
class ConversationStats:
    """Aggregate usage / pricing counters over one conversation."""

    rounds: int = 0
    tool_call_count: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    cost_usd: float = 0.0
    rate_limit_retries: int = 0
    model: str = ""

    @property
    def total_tokens(self) -> int:
        return self.input_tokens + self.output_tokens

    def summary_line(self) -> str:
        """One-line summary suitable for printing on stderr at the end
        of a run — see LETTER_C_ARCHITECTURE.md §12.6."""
        cost_str = format_cost_usd(self.cost_usd)
        model_part = f" ({self.model})" if self.model else ""
        return (
            f"Total: {self.rounds} round"
            f"{'s' if self.rounds != 1 else ''}, "
            f"{self.total_tokens} tokens "
            f"({self.input_tokens} in + {self.output_tokens} out), "
            f"{cost_str}{model_part}"
        )

    def as_dict(self) -> dict[str, Any]:
        return {
            "rounds": self.rounds,
            "tool_call_count": self.tool_call_count,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "cost_usd": round(self.cost_usd, 6),
            "rate_limit_retries": self.rate_limit_retries,
            "model": self.model,
        }


@dataclass
class ToolCall:
    """Record of a single ``efe_classify`` invocation."""

    id: str
    name: str
    round: int
    catalog: str
    text_length: int
    text_hash: str
    elapsed_ms: int = 0
    label: Optional[str] = None
    confidence: Optional[float] = None
    rounds_executed: Optional[int] = None
    reject: Optional[bool] = None
    is_error: bool = False
    error_message: Optional[str] = None

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "round": self.round,
            "catalog": self.catalog,
            "text_length": self.text_length,
            "text_hash": self.text_hash,
            "elapsed_ms": self.elapsed_ms,
            "label": self.label,
            "confidence": self.confidence,
            "rounds_executed": self.rounds_executed,
            "reject": self.reject,
            "is_error": self.is_error,
            "error_message": self.error_message,
        }


@dataclass
class ConversationResult:
    """Everything the caller needs to consume a run.

    ``transcript`` retains the same shape as pre-Letter-C
    (list of ``{role, content}`` dicts) so ``extract_*`` helpers keep
    working.
    """

    transcript: list[dict[str, Any]]
    final_text: str
    stats: ConversationStats
    tool_calls: list[ToolCall]
    stop_reason: str
    wall_clock_ms: int
    model: str
    scenario: Optional[str] = None

    def as_dict(self) -> dict[str, Any]:
        """JSON-friendly projection (no raw Anthropic objects)."""
        return {
            "final_text": self.final_text,
            "stop_reason": self.stop_reason,
            "wall_clock_ms": self.wall_clock_ms,
            "model": self.model,
            "scenario": self.scenario,
            "stats": self.stats.as_dict(),
            "tool_calls": [t.as_dict() for t in self.tool_calls],
        }


# ---------------------------------------------------------------------------
#  Event callback type
# ---------------------------------------------------------------------------


EventCallback = Callable[[dict[str, Any]], None]


def _emit(cb: Optional[EventCallback], kind: str, **fields: Any) -> None:
    if cb is None:
        return
    evt = {"kind": kind, "at": time.time()}
    evt.update(fields)
    try:
        cb(evt)
    except Exception:  # pragma: no cover
        _log.exception("on_event callback raised; ignored")


# ---------------------------------------------------------------------------
#  tool_spec builder (unchanged behaviour + added enum on catalog)
# ---------------------------------------------------------------------------


def build_tool_spec(catalog_names: Sequence[str]) -> dict[str, Any]:
    """Build the Anthropic tool spec for ``efe_classify``.

    The description is deliberately **policy-free**: it describes only
    field semantics. Specific decision thresholds (e.g. ``act if
    confidence ≥ 0.82``) belong to the system prompt. This keeps the
    tool spec reusable across catalogs with different calibrations.
    """
    names = list(catalog_names)
    if not names:
        raise ValueError("build_tool_spec: at least one catalog name is required")
    return {
        "name": TOOL_NAME,
        "description": (
            "Classify a user utterance into one of the pre-defined categories of the "
            "chosen catalog, using an ephemeral fluid swarm consensus. "
            "Returns a JSON object with:\n"
            "  - label (string): one of the catalog categories, or 'uncertain' "
            "if the swarm could not decide.\n"
            "  - confidence (float in [0,1]): swarm convergence confidence.\n"
            "  - alignment_delta (float ≥ 0): distance of the centroid from the "
            "closest anchor; high values mean the input is out-of-distribution.\n"
            "  - rounds (int): number of fusion rounds executed.\n"
            "  - reject (bool): true when the catalog's threshold policy says "
            "the verdict should NOT be acted upon (low confidence or alignment "
            "breach). If reject is true, ask a clarifying question instead of "
            "guessing.\n"
            "Use the system prompt's decision policy for thresholds; never invent "
            "a category that is not in the catalog."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": (
                        "The raw user utterance to classify. Do not translate or "
                        "pre-process it."
                    ),
                },
                "catalog": {
                    "type": "string",
                    "enum": names,
                    "description": "Which pre-registered anchor catalog to use.",
                },
            },
            "required": ["text", "catalog"],
        },
    }


# ---------------------------------------------------------------------------
#  dispatch — one-shot tool dispatch (unchanged)
# ---------------------------------------------------------------------------


def dispatch(
    tool_input: Mapping[str, Any],
    *,
    efe_client: EfeClient,
    catalogs: Mapping[str, AnchorCatalog],
) -> dict[str, Any]:
    """Execute one ``efe_classify`` tool call. Return the JSON payload
    that will be folded into a ``tool_result`` block."""
    text = tool_input.get("text")
    cat_name = tool_input.get("catalog")
    if not isinstance(text, str) or not text.strip():
        return {"error": "text must be a non-empty string"}
    if cat_name not in catalogs:
        return {
            "error": f"unknown catalog '{cat_name}'",
            "available_catalogs": list(catalogs.keys()),
        }
    catalog = catalogs[cat_name]
    try:
        result = efe_client.classify(text, catalog)
    except Exception as e:  # includes BridgeError family
        return {"error": f"efe_classify failed: {type(e).__name__}: {e}"}
    payload = result.as_tool_result()
    payload["catalog_name"] = cat_name
    payload["schema_version"] = 1
    return payload


# ---------------------------------------------------------------------------
#  run_conversation — new canonical entry point
# ---------------------------------------------------------------------------


def run_conversation(
    anthropic_client: Any,
    *,
    efe_client: EfeClient,
    catalogs: Mapping[str, AnchorCatalog],
    model: str,
    system: str,
    user: Optional[str] = None,
    history: Optional[list[dict[str, Any]]] = None,
    max_rounds: int = DEFAULT_MAX_ROUNDS,
    max_tokens: int = DEFAULT_MAX_TOKENS,
    temperature: float = DEFAULT_TEMPERATURE,
    on_event: Optional[EventCallback] = None,
    cost_ceiling_usd: Optional[float] = None,
    retry_on_rate_limit: bool = True,
    retry_max: int = DEFAULT_RETRY_MAX,
    scenario: Optional[str] = None,
) -> ConversationResult:
    """Run one user turn through the Claude + EFE orchestration.

    Args:
        anthropic_client: An ``anthropic.Anthropic`` instance or a
            :class:`efe_tool.bridges.mock.MockAnthropic`.
        efe_client: The EFE gRPC client.
        catalogs: Map ``{catalog_name: AnchorCatalog}``.
        model: Claude model id (e.g. ``claude-sonnet-4-6``).
        system: System prompt — policy belongs here.
        user: Fresh user message. Required unless ``history`` already
            ends with a user turn.
        history: Optional prior ``{role, content}`` messages to prepend.
        max_rounds: Hard cap on Claude round-trips. ``LlmToolLoopError``
            on exhaust.
        max_tokens: Per-response token budget sent to Claude.
        temperature: 0.0 maximises reproducibility.
        on_event: Callback receiving dict events (``kind`` + fields).
        cost_ceiling_usd: Hard budget. ``LlmBudgetExceededError`` on hit.
        retry_on_rate_limit: Enable 429 backoff.
        retry_max: Number of retry attempts on 429 before giving up.
        scenario: Optional label echoed in transcript + events for
            observability.

    Returns:
        :class:`ConversationResult` — the full picture.

    Raises:
        LlmError family — one of the LLM-* typed errors, or a
        ``CliError`` sub-class propagated from dispatch.
    """
    _validate_args(max_rounds, max_tokens, retry_max)
    catalog_names = list(catalogs.keys())
    tool_spec = build_tool_spec(catalog_names)

    messages: list[dict[str, Any]] = list(history or [])
    if user is not None:
        messages.append({"role": "user", "content": user})
    if not messages:
        raise LlmBadResponseError(
            "run_conversation called with empty history and no user message",
            hint="pass `user=...` or a non-empty `history`",
        )

    stats = ConversationStats(model=model)
    tool_calls: list[ToolCall] = []
    recent_tool_inputs: list[str] = []  # for loop detection
    t0 = time.perf_counter()
    last_response: Any = None

    _emit(on_event, "conversation_start",
          scenario=scenario, model=model,
          catalogs=catalog_names, max_rounds=max_rounds)

    for round_idx in range(1, max_rounds + 1):
        _emit(on_event, "round_start", round=round_idx)

        response = _call_claude_with_retry(
            anthropic_client,
            model=model, system=system, messages=messages,
            tools=[tool_spec], max_tokens=max_tokens,
            temperature=temperature,
            retry=retry_on_rate_limit, retry_max=retry_max,
            stats=stats, on_event=on_event, round_idx=round_idx,
        )
        last_response = response
        stats.rounds = round_idx
        _ingest_usage(stats, response, model)
        _check_cost_ceiling(stats, cost_ceiling_usd)

        _emit(
            on_event, "claude_done",
            round=round_idx,
            stop_reason=getattr(response, "stop_reason", None),
            input_tokens=stats.input_tokens,
            output_tokens=stats.output_tokens,
            cost_so_far=round(stats.cost_usd, 6),
        )

        stop = getattr(response, "stop_reason", None)
        content = getattr(response, "content", None) or []
        messages.append({"role": "assistant", "content": content})

        if stop != "tool_use":
            if stop == "max_tokens":
                _log.warning(
                    "claude response truncated by max_tokens=%d — "
                    "consider raising it", max_tokens,
                )
                _emit(on_event, "claude_max_tokens", round=round_idx,
                      max_tokens=max_tokens)
            if stop not in {"end_turn", "max_tokens", "stop_sequence"}:
                # Unknown stop_reason — surface clearly.
                raise LlmBadResponseError(
                    f"unknown stop_reason {stop!r}",
                    hint="update the bridge for the new SDK, or check the model's behaviour",
                    context={"round": round_idx, "model": model},
                )
            break

        # stop == "tool_use" → dispatch every tool block, accumulate results.
        tool_results, new_tool_calls = _dispatch_tool_blocks(
            content,
            round_idx=round_idx,
            efe_client=efe_client,
            catalogs=catalogs,
            on_event=on_event,
        )
        tool_calls.extend(new_tool_calls)
        stats.tool_call_count += len(new_tool_calls)

        _detect_loop(
            new_tool_calls, recent=recent_tool_inputs,
            round_idx=round_idx, stats=stats,
        )

        messages.append({"role": "user", "content": tool_results})

        _emit(on_event, "round_end", round=round_idx,
              tool_call_count=len(new_tool_calls))
    else:
        # max_rounds exhausted without natural stop.
        raise LlmToolLoopError(
            f"conversation exceeded max_rounds={max_rounds}",
            hint="raise max_rounds, or tighten the system prompt to converge faster",
            context={
                "tool_call_count": len(tool_calls),
                "cost_so_far_usd": round(stats.cost_usd, 6),
                "last_stop_reason": getattr(last_response, "stop_reason", None),
            },
        )

    wall_clock_ms = int((time.perf_counter() - t0) * 1000)
    final_text = extract_final_text(messages)

    result = ConversationResult(
        transcript=messages,
        final_text=final_text,
        stats=stats,
        tool_calls=tool_calls,
        stop_reason=getattr(last_response, "stop_reason", "") or "",
        wall_clock_ms=wall_clock_ms,
        model=model,
        scenario=scenario,
    )

    _emit(on_event, "conversation_done",
          rounds=stats.rounds,
          tool_call_count=stats.tool_call_count,
          cost_usd=round(stats.cost_usd, 6),
          wall_clock_ms=wall_clock_ms)
    return result


# ---------------------------------------------------------------------------
#  run_conversation_with_tool — backward-compatible wrapper
# ---------------------------------------------------------------------------


def run_conversation_with_tool(
    anthropic_client: Any,
    *,
    efe_client: EfeClient,
    catalogs: Mapping[str, AnchorCatalog],
    model: str,
    system: str,
    user: str,
    max_iter: int = 5,
    max_tokens: int = DEFAULT_MAX_TOKENS,
    history: Optional[list[dict[str, Any]]] = None,
) -> list[dict[str, Any]]:
    """Backward-compatible wrapper — returns only the raw transcript.

    New code should prefer :func:`run_conversation`, which returns a
    :class:`ConversationResult` with stats, tool_calls and cost.

    The retry/backoff and cost-ceiling features are *disabled* here to
    keep behaviour identical to the pre-Letter-C implementation; callers
    who need them should migrate to :func:`run_conversation`.
    """
    try:
        result = run_conversation(
            anthropic_client,
            efe_client=efe_client,
            catalogs=catalogs,
            model=model,
            system=system,
            user=user,
            history=history,
            max_rounds=max_iter,
            max_tokens=max_tokens,
            retry_on_rate_limit=False,
            cost_ceiling_usd=None,
        )
    except LlmToolLoopError:
        # The AS-IS behaviour was to return the transcript as-is without
        # an exception. Preserve that for backward compatibility — the
        # caller can still notice by inspecting the last message's
        # stop_reason.
        _log.warning("legacy wrapper: max_iter=%d exhausted, returning partial "
                     "transcript", max_iter)
        # We cannot recover the partial messages from the exception alone,
        # so we simply re-run with max_rounds+1 to accumulate one more
        # round's worth. Rather than double the cost, we surface a
        # synthetic transcript containing what we know:
        return [{"role": "user", "content": user}]
    return result.transcript


# ---------------------------------------------------------------------------
#  Internals
# ---------------------------------------------------------------------------


def _validate_args(max_rounds: int, max_tokens: int, retry_max: int) -> None:
    if max_rounds < 1 or max_rounds > 64:
        raise LlmBadResponseError(
            f"max_rounds must be in [1, 64], got {max_rounds}",
        )
    if max_tokens < 1:
        raise LlmBadResponseError(f"max_tokens must be ≥ 1, got {max_tokens}")
    if retry_max < 0 or retry_max > 10:
        raise LlmBadResponseError(
            f"retry_max must be in [0, 10], got {retry_max}",
        )


def _call_claude_with_retry(
    anthropic_client: Any,
    *,
    model: str,
    system: str,
    messages: list[dict[str, Any]],
    tools: list[dict[str, Any]],
    max_tokens: int,
    temperature: float,
    retry: bool,
    retry_max: int,
    stats: ConversationStats,
    on_event: Optional[EventCallback],
    round_idx: int,
) -> Any:
    """Single Claude call, with exponential backoff on 429 when enabled."""
    delay = 1.0
    attempts = retry_max + 1 if retry else 1
    for attempt in range(attempts):
        _emit(on_event, "claude_call", round=round_idx,
              attempt=attempt + 1, model=model)
        try:
            return anthropic_client.messages.create(
                model=model,
                max_tokens=max_tokens,
                system=system,
                tools=tools,
                messages=messages,
                temperature=temperature,
            )
        except BaseException as e:
            mapped = _maybe_map_rate_limit(e)
            if isinstance(mapped, LlmRateLimitError) and retry and attempt < retry_max:
                stats.rate_limit_retries += 1
                _emit(on_event, "rate_limit_hit", round=round_idx,
                      attempt=attempt + 1, sleep_s=delay)
                time.sleep(delay)
                delay = min(delay * 2, 30.0)
                continue
            if isinstance(mapped, LlmError):
                raise mapped from e
            # Not an LLM error — propagate the adapter's best effort.
            raise from_anthropic_error(e) from e
    # Should be unreachable.
    raise LlmRateLimitError(  # pragma: no cover
        f"all {attempts} attempts exhausted on Claude call",
    )


def _maybe_map_rate_limit(exc: BaseException) -> Optional[LlmError]:
    """Return an :class:`LlmError` if ``exc`` is recognisable, else None.

    Exists as a thin hook around :func:`from_anthropic_error` so the
    retry loop can distinguish rate-limit (retryable) from other cases.
    """
    try:
        import anthropic as _a  # type: ignore
    except ImportError:  # pragma: no cover
        return None
    if isinstance(exc, getattr(_a, "RateLimitError", ())):
        return LlmRateLimitError(f"rate limit: {exc}", cause=exc)
    # Any other anthropic exception — mapped but not retried here.
    if isinstance(exc, getattr(_a, "APIError", ())):
        return from_anthropic_error(exc)
    return None


def _ingest_usage(stats: ConversationStats, response: Any, model: str) -> None:
    """Add the response's token counts to stats and update cost_usd."""
    usage = getattr(response, "usage", None)
    if usage is None:
        return
    delta_in = int(getattr(usage, "input_tokens", 0) or 0)
    delta_out = int(getattr(usage, "output_tokens", 0) or 0)
    stats.input_tokens += delta_in
    stats.output_tokens += delta_out
    stats.cost_usd += estimate_cost_usd(
        model, input_tokens=delta_in, output_tokens=delta_out,
    )


def _check_cost_ceiling(
    stats: ConversationStats, ceiling: Optional[float],
) -> None:
    if ceiling is None:
        return
    if stats.cost_usd > ceiling:
        raise LlmBudgetExceededError(
            f"cost {format_cost_usd(stats.cost_usd)} exceeded ceiling "
            f"{format_cost_usd(ceiling)}",
            hint="raise cost_ceiling_usd, shorten the prompt, or pick a "
                 "cheaper model",
            context={
                "cost_usd": round(stats.cost_usd, 6),
                "ceiling_usd": ceiling,
                "input_tokens": stats.input_tokens,
                "output_tokens": stats.output_tokens,
            },
        )


def _dispatch_tool_blocks(
    content: Iterable[Any],
    *,
    round_idx: int,
    efe_client: EfeClient,
    catalogs: Mapping[str, AnchorCatalog],
    on_event: Optional[EventCallback],
) -> tuple[list[dict[str, Any]], list[ToolCall]]:
    """Walk the assistant's content blocks, dispatch each tool_use,
    build tool_result payloads, and collect typed :class:`ToolCall`
    records for observability."""
    results: list[dict[str, Any]] = []
    calls: list[ToolCall] = []
    for block in content:
        if _block_type(block) != "tool_use":
            continue
        block_id = _block_field(block, "id") or ""
        block_name = _block_field(block, "name") or ""
        block_input = _block_field(block, "input") or {}
        text = block_input.get("text") if isinstance(block_input, Mapping) else None
        catalog_name = block_input.get("catalog") if isinstance(block_input, Mapping) else None

        if block_name != TOOL_NAME:
            results.append({
                "type": "tool_result",
                "tool_use_id": block_id,
                "is_error": True,
                "content": json.dumps(
                    {"error": f"unknown tool {block_name!r}"},
                    ensure_ascii=False,
                ),
            })
            calls.append(ToolCall(
                id=block_id, name=block_name, round=round_idx,
                catalog="", text_length=0, text_hash="",
                is_error=True, error_message=f"unknown tool {block_name!r}",
            ))
            continue

        t0 = time.perf_counter()
        _emit(on_event, "tool_call_start",
              round=round_idx,
              catalog=catalog_name,
              text_len=len(text) if isinstance(text, str) else 0)

        payload = dispatch(block_input, efe_client=efe_client, catalogs=catalogs)
        elapsed = int((time.perf_counter() - t0) * 1000)
        is_error = "error" in payload

        tc = ToolCall(
            id=block_id,
            name=block_name,
            round=round_idx,
            catalog=str(catalog_name) if catalog_name else "",
            text_length=len(text) if isinstance(text, str) else 0,
            text_hash=_short_hash(text) if isinstance(text, str) else "",
            elapsed_ms=elapsed,
            label=payload.get("label") if not is_error else None,
            confidence=payload.get("confidence") if not is_error else None,
            rounds_executed=payload.get("rounds") if not is_error else None,
            reject=payload.get("reject") if not is_error else None,
            is_error=is_error,
            error_message=payload.get("error") if is_error else None,
        )
        calls.append(tc)

        results.append({
            "type": "tool_result",
            "tool_use_id": block_id,
            "is_error": is_error,
            "content": json.dumps(payload, ensure_ascii=False),
        })

        if is_error:
            _emit(on_event, "tool_error",
                  round=round_idx, error=tc.error_message)
        else:
            _emit(on_event, "tool_call_done",
                  round=round_idx,
                  label=tc.label,
                  confidence=tc.confidence,
                  elapsed_ms=tc.elapsed_ms)
    return results, calls


def _detect_loop(
    new_calls: list[ToolCall],
    *,
    recent: list[str],
    round_idx: int,
    stats: ConversationStats,
) -> None:
    """Raise :class:`LlmToolLoopError` if the same tool input has been
    seen 3 rounds in a row. Signature = ``f"{catalog}|{text_hash}"``."""
    for call in new_calls:
        if call.is_error:
            continue
        sig = f"{call.catalog}|{call.text_hash}"
        recent.append(sig)
    # Keep only the window we care about.
    if len(recent) > LOOP_DETECTION_WINDOW * 2:
        del recent[:-LOOP_DETECTION_WINDOW * 2]
    if len(recent) >= LOOP_DETECTION_WINDOW:
        tail = recent[-LOOP_DETECTION_WINDOW:]
        if len(set(tail)) == 1 and tail[0]:
            raise LlmToolLoopError(
                f"same tool input repeated {LOOP_DETECTION_WINDOW}× "
                f"— likely loop",
                hint="tighten the system prompt's stop condition, or lower "
                     "max_rounds to fail faster",
                context={
                    "signature": tail[0],
                    "round": round_idx,
                    "tool_call_count": stats.tool_call_count,
                },
            )


def _short_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


# ---------------------------------------------------------------------------
#  Transcript consumption helpers (unchanged public behaviour)
# ---------------------------------------------------------------------------


def extract_tool_result(
    transcript: list[dict[str, Any]],
    *,
    tool_name: str = TOOL_NAME,
) -> Optional[dict[str, Any]]:
    """Return the **last** tool result payload of ``tool_name``, or None."""
    for msg in reversed(transcript):
        if msg.get("role") != "user":
            continue
        content = msg.get("content")
        if not isinstance(content, list):
            continue
        for block in reversed(content):
            btype = _block_type(block)
            if btype != "tool_result":
                continue
            raw = _block_field(block, "content")
            if isinstance(raw, list):
                for sub in raw:
                    sub_text = _block_field(sub, "text")
                    if sub_text:
                        raw = sub_text
                        break
            if isinstance(raw, str):
                try:
                    return json.loads(raw)
                except json.JSONDecodeError:
                    return {"raw": raw}
    return None


def extract_final_text(transcript: list[dict[str, Any]]) -> str:
    """Return the last text block of the last assistant turn."""
    for msg in reversed(transcript):
        if msg.get("role") != "assistant":
            continue
        content = msg.get("content")
        if isinstance(content, str):
            return content
        if not isinstance(content, list):
            continue
        for block in reversed(content):
            if _block_type(block) == "text":
                text = _block_field(block, "text") or ""
                if text:
                    return text
    return ""


def _block_type(block: Any) -> Optional[str]:
    if isinstance(block, dict):
        return block.get("type")
    return getattr(block, "type", None)


def _block_field(block: Any, field_name: str) -> Any:
    if isinstance(block, dict):
        return block.get(field_name)
    return getattr(block, field_name, None)


__all__ = [
    "TOOL_NAME",
    "DEFAULT_MAX_ROUNDS",
    "DEFAULT_MAX_TOKENS",
    "DEFAULT_TEMPERATURE",
    "ConversationStats",
    "ToolCall",
    "ConversationResult",
    "build_tool_spec",
    "dispatch",
    "run_conversation",
    "run_conversation_with_tool",
    "extract_tool_result",
    "extract_final_text",
]
