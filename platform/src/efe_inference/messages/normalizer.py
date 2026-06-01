"""Normalisers — convert wire formats to :class:`ChatRequest`.

Supported input formats :

* OpenAI ``/v1/chat/completions`` body.
* Anthropic ``/v1/messages`` body.
* Native EFE format (identity modulo field names).

A normaliser is **pure** — it only validates and rewrites, it does
not touch the DB or any external service. It raises
:class:`RequestError`-derived exceptions on malformed inputs; never
emits partial requests.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any, Literal, Protocol, cast

from efe_inference.errors import (
    RequestError,
    UnsupportedContentKind,
    UnsupportedMessageRole,
    ValidationError,
)
from efe_inference.ids import new_uuid
from efe_inference.messages.request import (
    ChatMessage,
    ChatRequest,
    ImageBlock,
    Role,
    SamplingParams,
    TextBlock,
    ToolDef,
    ToolResultBlock,
    ToolUseBlock,
)


class RequestNormalizer(Protocol):
    def normalize(
        self,
        raw: Mapping[str, Any],
        *,
        tenant_id: str,
        task_id: str,
        request_id: str | None = None,
    ) -> ChatRequest:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# OpenAI format
# ---------------------------------------------------------------------------


class OpenAINormalizer:
    """Accepts the familiar ``/v1/chat/completions`` body."""

    @staticmethod
    def normalize(
        raw: Mapping[str, Any],
        *,
        tenant_id: str,
        task_id: str,
        request_id: str | None = None,
    ) -> ChatRequest:
        if "messages" not in raw or not isinstance(raw["messages"], list):
            raise RequestError("OpenAI body requires a 'messages' list")
        messages: list[ChatMessage] = []
        for m in raw["messages"]:
            messages.append(_openai_message_to_canonical(m))
        sampling = SamplingParams(
            temperature=float(raw.get("temperature", 1.0)),
            top_p=float(raw.get("top_p", 1.0)),
            top_k=int(raw["top_k"]) if "top_k" in raw and raw["top_k"] is not None else None,
            max_tokens=int(raw.get("max_tokens", 1024)),
            stop=_to_tuple_str(raw.get("stop")),
            presence_penalty=float(raw.get("presence_penalty", 0.0)),
            frequency_penalty=float(raw.get("frequency_penalty", 0.0)),
            seed=int(raw["seed"]) if raw.get("seed") is not None else None,
        )
        tools = tuple(_openai_tool_to_def(t) for t in raw.get("tools", []) or [])
        tool_choice = _normalise_tool_choice(raw.get("tool_choice", "auto"))
        stream = bool(raw.get("stream", False))
        model_hint = (
            str(raw["model"]) if raw.get("model") else None
        )
        return ChatRequest(
            tenant_id=tenant_id,
            task_id=task_id,
            request_id=request_id or new_uuid(),
            messages=tuple(messages),
            sampling=sampling,
            tools=tools,
            tool_choice=tool_choice,
            stream=stream,
            model_hint=model_hint,
            user_id=str(raw["user"]) if raw.get("user") else None,
            timeout_budget_s=float(raw.get("timeout_s", 30.0)),
            metadata={
                k: str(v) for k, v in (raw.get("metadata") or {}).items()
            },
        )


def _openai_message_to_canonical(m: Mapping[str, Any]) -> ChatMessage:
    role = m.get("role")
    if role not in {"system", "user", "assistant", "tool"}:
        raise UnsupportedMessageRole(
            f"unsupported OpenAI role {role!r}",
        )
    content_raw = m.get("content")
    blocks: list[Any] = []
    if isinstance(content_raw, str):
        blocks.append(TextBlock(text=content_raw))
    elif isinstance(content_raw, list):
        for item in content_raw:
            if not isinstance(item, Mapping):
                raise RequestError(
                    f"content item must be an object, got {type(item).__name__}",
                )
            kind = item.get("type")
            if kind == "text":
                blocks.append(TextBlock(text=str(item.get("text", ""))))
            elif kind == "image_url":
                url = item.get("image_url", {}).get("url", "")
                # Fix M11 — extract MIME from ``data:`` URI prefix so
                # the canonical form always has a useful mime.
                mime = _mime_from_data_uri(url) or "image/jpeg"
                blocks.append(ImageBlock(image_url=url, mime=mime))
            else:
                raise UnsupportedContentKind(
                    f"unsupported OpenAI content type {kind!r}",
                )
    elif content_raw is None:
        # Assistant with tool_calls only — no textual content.
        pass
    else:
        raise RequestError(
            f"OpenAI content must be str or list, got {type(content_raw).__name__}",
        )
    if role == "assistant":
        for tc in m.get("tool_calls", []) or []:
            blocks.append(ToolUseBlock(
                tool_call_id=str(tc.get("id", "")),
                tool_name=str(tc.get("function", {}).get("name", "")),
                tool_args=_parse_json_args(
                    tc.get("function", {}).get("arguments", "{}"),
                ),
            ))
    if role == "tool":
        blocks.append(ToolResultBlock(
            tool_call_id=str(m.get("tool_call_id", "")),
            content=str(content_raw or ""),
        ))
    name = m.get("name")
    return ChatMessage(
        role=Role(role),
        content=tuple(blocks),
        name=str(name) if name else None,
        tool_call_id=(
            str(m["tool_call_id"]) if role == "tool" else None
        ),
    )


def _openai_tool_to_def(t: Mapping[str, Any]) -> ToolDef:
    if t.get("type") != "function":
        raise RequestError(
            f"unsupported OpenAI tool type {t.get('type')!r}",
        )
    fn = t.get("function") or {}
    return ToolDef(
        name=str(fn.get("name", "")),
        description=str(fn.get("description", "")),
        parameters_json_schema=dict(fn.get("parameters") or {}),
    )


# ---------------------------------------------------------------------------
# Anthropic format
# ---------------------------------------------------------------------------


class AnthropicNormalizer:
    """Accepts Anthropic's ``/v1/messages`` body (system + messages)."""

    @staticmethod
    def normalize(
        raw: Mapping[str, Any],
        *,
        tenant_id: str,
        task_id: str,
        request_id: str | None = None,
    ) -> ChatRequest:
        if "messages" not in raw or not isinstance(raw["messages"], list):
            raise RequestError("Anthropic body requires a 'messages' list")
        messages: list[ChatMessage] = []
        system_val = raw.get("system")
        if system_val:
            sys_text = (
                system_val if isinstance(system_val, str)
                else _flatten_anthropic_content(system_val)
            )
            messages.append(ChatMessage(
                role=Role.SYSTEM,
                content=(TextBlock(text=sys_text),),
            ))
        for m in raw["messages"]:
            role = m.get("role")
            if role not in {"user", "assistant"}:
                raise UnsupportedMessageRole(
                    f"unsupported Anthropic role {role!r}",
                )
            blocks: list[Any] = []
            content = m.get("content", [])
            if isinstance(content, str):
                blocks.append(TextBlock(text=content))
            else:
                for item in content:
                    if not isinstance(item, Mapping):
                        raise RequestError(
                            "anthropic content items must be objects",
                        )
                    t = item.get("type")
                    if t == "text":
                        blocks.append(TextBlock(text=str(item.get("text", ""))))
                    elif t == "image":
                        src = item.get("source") or {}
                        data_url = src.get("data") or src.get("url") or ""
                        blocks.append(ImageBlock(
                            image_url=str(data_url),
                            mime=str(src.get("media_type", "image/png")),
                        ))
                    elif t == "tool_use":
                        blocks.append(ToolUseBlock(
                            tool_call_id=str(item.get("id", "")),
                            tool_name=str(item.get("name", "")),
                            tool_args=dict(item.get("input") or {}),
                        ))
                    elif t == "tool_result":
                        raw_content = item.get("content", "")
                        if isinstance(raw_content, list):
                            raw_content = _flatten_anthropic_content(raw_content)
                        blocks.append(ToolResultBlock(
                            tool_call_id=str(item.get("tool_use_id", "")),
                            content=str(raw_content),
                            is_error=bool(item.get("is_error", False)),
                        ))
                    else:
                        raise UnsupportedContentKind(
                            f"unsupported Anthropic content type {t!r}",
                        )
            messages.append(ChatMessage(
                role=Role(role),
                content=tuple(blocks),
            ))
        sampling = SamplingParams(
            temperature=float(raw.get("temperature", 1.0)),
            top_p=float(raw.get("top_p", 1.0)),
            top_k=int(raw["top_k"]) if raw.get("top_k") is not None else None,
            max_tokens=int(raw.get("max_tokens", 1024)),
            stop=_to_tuple_str(raw.get("stop_sequences")),
        )
        tools = tuple(
            ToolDef(
                name=str(t.get("name", "")),
                description=str(t.get("description", "")),
                parameters_json_schema=dict(t.get("input_schema") or {}),
            )
            for t in raw.get("tools", []) or []
        )
        return ChatRequest(
            tenant_id=tenant_id,
            task_id=task_id,
            request_id=request_id or new_uuid(),
            messages=tuple(messages),
            sampling=sampling,
            tools=tools,
            tool_choice=_normalise_tool_choice(raw.get("tool_choice", "auto")),
            stream=bool(raw.get("stream", False)),
            model_hint=str(raw["model"]) if raw.get("model") else None,
            user_id=(
                str(raw["metadata"]["user_id"])
                if isinstance(raw.get("metadata"), Mapping)
                and raw["metadata"].get("user_id")
                else None
            ),
        )


def _flatten_anthropic_content(items: Any) -> str:
    if isinstance(items, str):
        return items
    out: list[str] = []
    for item in items or []:
        if isinstance(item, Mapping) and item.get("type") == "text":
            out.append(str(item.get("text", "")))
    return "".join(out)


# ---------------------------------------------------------------------------
# Native (identity modulo validation)
# ---------------------------------------------------------------------------


class NativeNormalizer:
    """Accept a JSON body already shaped like :class:`ChatRequest`."""

    @staticmethod
    def normalize(
        raw: Mapping[str, Any],
        *,
        tenant_id: str,
        task_id: str,
        request_id: str | None = None,
    ) -> ChatRequest:
        try:
            msgs = tuple(
                _native_message(m) for m in raw.get("messages", [])
            )
            sampling_raw = raw.get("sampling") or {}
            sampling = SamplingParams(
                temperature=float(sampling_raw.get("temperature", 1.0)),
                top_p=float(sampling_raw.get("top_p", 1.0)),
                top_k=(
                    int(sampling_raw["top_k"])
                    if sampling_raw.get("top_k") is not None else None
                ),
                max_tokens=int(sampling_raw.get("max_tokens", 1024)),
                stop=_to_tuple_str(sampling_raw.get("stop")),
            )
            tools = tuple(
                ToolDef(
                    name=str(t.get("name", "")),
                    description=str(t.get("description", "")),
                    parameters_json_schema=dict(
                        t.get("parameters_json_schema") or {},
                    ),
                )
                for t in raw.get("tools", []) or []
            )
            return ChatRequest(
                tenant_id=tenant_id,
                task_id=task_id,
                request_id=request_id or new_uuid(),
                messages=msgs,
                sampling=sampling,
                tools=tools,
                tool_choice=_normalise_tool_choice(
                    raw.get("tool_choice", "auto"),
                ),
                stream=bool(raw.get("stream", False)),
                model_hint=(
                    str(raw["model_hint"]) if raw.get("model_hint") else None
                ),
                user_id=str(raw["user_id"]) if raw.get("user_id") else None,
                timeout_budget_s=float(raw.get("timeout_budget_s", 30.0)),
                metadata={
                    k: str(v) for k, v in (raw.get("metadata") or {}).items()
                },
            )
        except (TypeError, KeyError, ValueError) as exc:
            raise ValidationError(f"invalid native body: {exc}") from exc


def _native_message(m: Mapping[str, Any]) -> ChatMessage:
    role = m.get("role")
    if role not in {"system", "user", "assistant", "tool"}:
        raise UnsupportedMessageRole(f"unknown role {role!r}")
    # Fix M5 — validate content shape before iterating; accept a plain
    # string or a list of block dicts; reject anything else.
    content_raw = m.get("content", [])
    if isinstance(content_raw, str):
        # Legacy shorthand — wrap into a single TextBlock.
        iterable = [{"kind": "text", "text": content_raw}]
    elif isinstance(content_raw, list):
        iterable = content_raw
    else:
        raise UnsupportedContentKind(
            f"native message content must be str or list, got "
            f"{type(content_raw).__name__}",
        )
    blocks: list[Any] = []
    for b in iterable:
        if not isinstance(b, Mapping):
            raise UnsupportedContentKind(
                f"content block must be a mapping, got {type(b).__name__}",
            )
        kind = b.get("kind")
        if kind == "text":
            blocks.append(TextBlock(text=str(b.get("text", ""))))
        elif kind == "image":
            blocks.append(ImageBlock(
                image_url=str(b.get("image_url", "")),
                mime=str(b.get("mime", "image/png")),
            ))
        elif kind == "tool_use":
            tool_args_raw: Any = b.get("tool_args") or {}
            blocks.append(ToolUseBlock(
                tool_call_id=str(b.get("tool_call_id", "")),
                tool_name=str(b.get("tool_name", "")),
                tool_args=dict(tool_args_raw)
                if isinstance(tool_args_raw, Mapping) else {},
            ))
        elif kind == "tool_result":
            blocks.append(ToolResultBlock(
                tool_call_id=str(b.get("tool_call_id", "")),
                content=str(b.get("content", "")),
                is_error=bool(b.get("is_error", False)),
            ))
        else:
            raise UnsupportedContentKind(f"unsupported block kind {kind!r}")
    # If role == "tool", ChatMessage requires tool_call_id. Derive it
    # from the first ToolResultBlock if present.
    tool_call_id: str | None = None
    if role == "tool":
        tcid_raw = m.get("tool_call_id")
        if tcid_raw:
            tool_call_id = str(tcid_raw)
        else:
            for b in blocks:
                if isinstance(b, ToolResultBlock) and b.tool_call_id:
                    tool_call_id = b.tool_call_id
                    break
        if not tool_call_id:
            raise UnsupportedContentKind(
                "role='tool' requires tool_call_id either at message "
                "level or inside a tool_result block",
            )
    return ChatMessage(
        role=Role(role),
        content=tuple(blocks),
        tool_call_id=tool_call_id,
    )


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _mime_from_data_uri(url: str) -> str | None:
    """Parse ``data:<mime>;base64,...`` and return the MIME part."""
    if not url.startswith("data:"):
        return None
    try:
        header = url[5:].split(",", 1)[0]
        mime = header.split(";", 1)[0].strip()
        return mime or None
    except Exception:
        return None


def _to_tuple_str(v: Any) -> tuple[str, ...]:
    if v is None:
        return ()
    if isinstance(v, str):
        return (v,)
    return tuple(str(x) for x in v)


_ToolChoice = Literal["auto", "required", "none"]


def _normalise_tool_choice(v: Any) -> _ToolChoice:
    if isinstance(v, Mapping):
        # OpenAI-style {"type": "function", "function": {"name": "..."}}
        if v.get("type") == "function":
            return "required"
        return "auto"
    if v in ("auto", "required", "none"):
        return cast(_ToolChoice, v)
    return "auto"


def _parse_json_args(raw: Any) -> dict[str, Any]:
    import json

    if isinstance(raw, Mapping):
        return dict(raw)
    if isinstance(raw, str):
        try:
            parsed = json.loads(raw or "{}")
        except json.JSONDecodeError:
            return {}
        return dict(parsed) if isinstance(parsed, Mapping) else {}
    return {}


__all__ = [
    "AnthropicNormalizer",
    "NativeNormalizer",
    "OpenAINormalizer",
    "RequestNormalizer",
]
