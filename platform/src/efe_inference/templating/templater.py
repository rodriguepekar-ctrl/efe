"""Prompt templater.

For text-in / text-out families (Llama-3, ChatML, Mistral) we render a
Jinja-like template; for families with a structured chat API (Claude,
GPT, Gemini) we return the canonical messages untouched and let the
backend adapter handle the actual wire format. This lets us keep the
templating layer deterministic and hashable regardless of backend.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_inference.hashing import sha256_hex
from efe_inference.messages.request import (
    ChatMessage,
    ChatRequest,
    TextBlock,
)
from efe_inference.templating.families import (
    BUILTIN_TEMPLATES,
    ModelFamily,
)


@dataclass(frozen=True, slots=True)
class RenderedPrompt:
    family: ModelFamily
    template_version: str
    textual: str | None          # non-None for text families
    messages: tuple[dict[str, Any], ...] | None   # non-None for structured
    hash_hex: str                # SHA-256 of canonical rendered form


class PromptTemplater:
    """Deterministic template renderer.

    Custom templates can be registered via :meth:`register` (e.g. DB-
    loaded ``prompt_templates`` rows); otherwise the built-in set is
    used.
    """

    def __init__(self) -> None:
        self._custom: dict[tuple[ModelFamily, str], str] = {}

    def register(
        self, family: ModelFamily, version: str, content: str,
    ) -> None:
        self._custom[(family, version)] = content

    def render(
        self,
        req: ChatRequest,
        *,
        family: ModelFamily,
        version: str = "1.0",
    ) -> RenderedPrompt:
        if family in {ModelFamily.CLAUDE, ModelFamily.GPT, ModelFamily.GEMINI}:
            # Fix M9 — tool_use blocks must NOT appear inside
            # ``content`` for GPT. For GPT we surface them via a
            # top-level ``tool_calls`` field. Claude and Gemini accept
            # tool_use as a content block.
            messages = tuple(
                _structured_message(m, family=family) for m in req.messages
            )
            fingerprint = _structured_fingerprint(messages)
            return RenderedPrompt(
                family=family,
                template_version=version,
                textual=None,
                messages=messages,
                hash_hex=sha256_hex(fingerprint),
            )
        template = (
            self._custom.get((family, version))
            or BUILTIN_TEMPLATES.get((family, version))
        )
        if template is None:
            raise KeyError(
                f"no template for ({family.value}, {version})",
            )
        textual = _render_simple(template, req)
        return RenderedPrompt(
            family=family,
            template_version=version,
            textual=textual,
            messages=None,
            hash_hex=sha256_hex(textual),
        )


def _structured_message(
    m: ChatMessage,
    *,
    family: ModelFamily = ModelFamily.CLAUDE,
) -> dict[str, Any]:
    """Produce a backend-agnostic dict (M9-aware).

    For GPT, ``tool_use`` blocks move into a top-level ``tool_calls``
    field on assistant messages (OpenAI convention). Claude and Gemini
    accept tool_use inline in ``content``.
    """
    from efe_inference.messages.request import (
        ToolResultBlock,
        ToolUseBlock,
    )

    text_parts = [
        b.text for b in m.content if isinstance(b, TextBlock)
    ]
    rich_parts: list[dict[str, Any]] = []
    tool_uses: list[ToolUseBlock] = []
    tool_results: list[ToolResultBlock] = []
    for b in m.content:
        if isinstance(b, TextBlock):
            continue
        if isinstance(b, ToolUseBlock):
            tool_uses.append(b)
            continue
        if isinstance(b, ToolResultBlock):
            tool_results.append(b)
            continue
        rich_parts.append(_block_to_dict(b))

    out: dict[str, Any] = {"role": m.role.value}
    if family == ModelFamily.GPT:
        if rich_parts or (text_parts and rich_parts):
            out["content"] = (
                [{"type": "text", "text": "".join(text_parts)}, *rich_parts]
                if text_parts else rich_parts
            )
        else:
            out["content"] = "".join(text_parts)
        if tool_uses:
            import json as _json

            out["tool_calls"] = [
                {
                    "id": b.tool_call_id,
                    "type": "function",
                    "function": {
                        "name": b.tool_name,
                        "arguments": _json.dumps(
                            b.tool_args, ensure_ascii=False,
                        ),
                    },
                }
                for b in tool_uses
            ]
        if tool_results:
            out["tool_call_id"] = tool_results[0].tool_call_id
    else:
        combined = [
            *([{"type": "text", "text": "".join(text_parts)}]
              if text_parts else []),
            *rich_parts,
            *[_block_to_dict(b) for b in tool_uses],
            *[_block_to_dict(b) for b in tool_results],
        ]
        if combined and (rich_parts or tool_uses or tool_results):
            out["content"] = combined
        else:
            out["content"] = "".join(text_parts) if text_parts else ""
    if m.tool_call_id and "tool_call_id" not in out:
        out["tool_call_id"] = m.tool_call_id
    if m.name:
        out["name"] = m.name
    return out


def _block_to_dict(b: Any) -> dict[str, Any]:
    from efe_inference.messages.request import (
        ImageBlock,
        ToolResultBlock,
        ToolUseBlock,
    )

    if isinstance(b, ImageBlock):
        return {"type": "image", "image_url": b.image_url, "mime": b.mime}
    if isinstance(b, ToolUseBlock):
        return {
            "type": "tool_use",
            "id": b.tool_call_id,
            "name": b.tool_name,
            "input": dict(b.tool_args),
        }
    if isinstance(b, ToolResultBlock):
        return {
            "type": "tool_result",
            "tool_use_id": b.tool_call_id,
            "content": b.content,
            "is_error": b.is_error,
        }
    return {"type": "unknown"}


def _render_simple(template: str, req: ChatRequest) -> str:
    """Very small Jinja-like renderer.

    We purposely do NOT pull a Jinja2 dependency in the core module;
    the built-in templates use a tiny subset that we evaluate by hand.
    Supported:

    * ``{%- for m in messages -%}...{%- endfor -%}``
    * ``{{ m.role }}`` / ``{{ m.text }}``
    * conditional ``{%- if m.role == 'user' -%}...{%- endif -%}``
      (evaluated per-iteration; no nested conditionals).

    Anything fancier should register a real Jinja2 template via
    :meth:`PromptTemplater.register`.
    """
    rendered_parts: list[str] = []
    loop_start = template.find("{%- for m in messages -%}")
    loop_end = template.find("{%- endfor -%}")
    if loop_start == -1 or loop_end == -1:
        # No loop — treat the whole template as a literal (with variable
        # replacements on ``messages[0]``).
        return template
    prefix = template[:loop_start]
    body = template[loop_start + len("{%- for m in messages -%}"):loop_end]
    suffix = template[loop_end + len("{%- endfor -%}"):]
    rendered_parts.append(prefix)
    for m in req.messages:
        rendered_parts.append(_render_body(body, m))
    rendered_parts.append(suffix)
    return "".join(rendered_parts)


def _render_body(body: str, m: ChatMessage) -> str:
    import re

    out = body
    # Handle one-level conditional on role.
    cond_re = re.compile(
        r"\{%-\s*if m\.role\s*==\s*'(?P<role>\w+)'\s*-%\}"
        r"(?P<body>.*?)"
        r"(?=\{%-\s*el|\{%-\s*endif)",
        re.DOTALL,
    )
    if "{%-" in body:
        # Collect clauses.
        rendered = ""
        pieces = re.split(
            r"\{%-\s*(?:if|elif|else|endif)[^%]*-%\}", body,
        )
        # Non-trivial case: fall back to role==X matches.
        for match in cond_re.finditer(body):
            if match.group("role") == m.role.value:
                rendered += match.group("body")
        out = rendered or pieces[-1]
    out = out.replace("{{ m.role }}", m.role.value)
    out = out.replace("{{ m.text }}", m.text())
    return out


def _structured_fingerprint(messages: tuple[dict[str, Any], ...]) -> str:
    """Stable hash input for structured prompts — sort keys recursively."""
    import json

    return json.dumps(
        list(messages),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    )


__all__ = ["PromptTemplater", "RenderedPrompt"]
