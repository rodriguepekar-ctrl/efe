"""Prompt template loader for efe-byo-agent (M20).

Templates are stored as Markdown files with YAML front-matter in
this directory. They mirror the mcp-server tools/m20_prompts/ exactly
(same names + same content) so the helper running in direct provider
mode produces identical outputs as the helper called via mcp-server
sampling delegation.

Sync convention : when a template is updated in mcp-server, copy it
here too (CI test `test_prompts_in_sync` enforces).
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from string import Template
from typing import Optional

import yaml


_TEMPLATES_DIR = Path(__file__).parent
_CACHE: dict[str, "PromptTemplate"] = {}


@dataclass
class PromptTemplate:
    name: str
    version: int
    license: str
    provider_compat: list[str]
    eval_pass: bool
    eval_accuracy: Optional[float]
    system_body: str
    user_body: str
    notes: str = ""
    min_context_tokens: int = 4000
    max_response_tokens: int = 1024


class PromptTemplateError(RuntimeError):
    pass


def load(template_id: str) -> PromptTemplate:
    if template_id in _CACHE:
        return _CACHE[template_id]
    path = _TEMPLATES_DIR / f"{template_id}.md"
    if not path.exists():
        raise PromptTemplateError(f"template not found : {template_id}")
    raw = path.read_text(encoding="utf-8")
    if not raw.startswith("---"):
        raise PromptTemplateError(
            f"template {template_id} : missing YAML header"
        )
    parts = raw.split("---", 2)
    if len(parts) < 3:
        raise PromptTemplateError(f"template {template_id} : malformed header")
    _, header_yaml, body = parts
    header = yaml.safe_load(header_yaml) or {}
    sys_body, usr_body = _split_sections(body, template_id)
    t = PromptTemplate(
        name=header["name"],
        version=int(header["version"]),
        license=header.get("license", "proprietary"),
        provider_compat=list(header.get("provider_compat", [])),
        eval_pass=bool(header.get("eval_pass", False)),
        eval_accuracy=header.get("eval_accuracy"),
        system_body=sys_body,
        user_body=usr_body,
        notes=header.get("notes", ""),
        min_context_tokens=int(header.get("min_context_tokens", 4000)),
        max_response_tokens=int(header.get("max_response_tokens", 1024)),
    )
    _CACHE[template_id] = t
    return t


def render(template_id: str, **kwargs) -> tuple[str, str]:
    t = load(template_id)
    return (
        Template(t.system_body).safe_substitute(**kwargs),
        Template(t.user_body).safe_substitute(**kwargs),
    )


def list_templates() -> list[str]:
    return sorted(p.stem for p in _TEMPLATES_DIR.glob("*.md"))


def clear_cache() -> None:
    _CACHE.clear()


def _split_sections(body: str, tid: str) -> tuple[str, str]:
    sys_body, usr_body = "", ""
    current = None
    for line in body.splitlines():
        s = line.strip()
        if s.lower().startswith("# system"):
            current = "system"
            continue
        if s.lower().startswith("# user"):
            current = "user"
            continue
        if current == "system":
            sys_body += line + "\n"
        elif current == "user":
            usr_body += line + "\n"
    if not usr_body:
        raise PromptTemplateError(
            f"template {tid} : missing # User section"
        )
    return sys_body.strip(), usr_body.strip()


__all__ = [
    "PromptTemplate",
    "PromptTemplateError",
    "load",
    "render",
    "list_templates",
    "clear_cache",
]
