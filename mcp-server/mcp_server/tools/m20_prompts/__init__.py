"""M20 prompt templates — versioned, evaluated, provider-agnostic.

Each template is a Markdown file with YAML front-matter declaring :
  - name           — stable id (e.g. `summarize_classification_v1`)
  - version        — integer, bumps on breaking change
  - license        — proprietary
  - provider_compat — list of providers tested with this template
  - eval_pass      — bool, true if eval against gold dataset passed
  - eval_accuracy  — float, [0, 1]

The body is split into `# System` and `# User` sections by the loader.
Variable substitution uses `string.Template` (`$var` / `${var}`) — no
Jinja2, deliberately, to keep dependency surface minimal and avoid
arbitrary code execution in templates.

Pin tests : tests/test_m20_prompts_loader.py.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from string import Template
from typing import Optional

import yaml


_TEMPLATES_DIR = Path(__file__).parent
_CACHE: dict[str, "PromptTemplate"] = {}


@dataclass
class PromptTemplate:
    """Parsed prompt template ready for substitution."""

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
    metadata: dict = field(default_factory=dict)


class PromptTemplateError(RuntimeError):
    """Raised on malformed templates or missing variables."""


def load(template_id: str) -> PromptTemplate:
    """Load and cache a template by its stable id (filename without .md)."""
    if template_id in _CACHE:
        return _CACHE[template_id]

    path = _TEMPLATES_DIR / f"{template_id}.md"
    if not path.exists():
        raise PromptTemplateError(
            f"template not found : {template_id} (expected {path})"
        )

    raw = path.read_text(encoding="utf-8")
    if not raw.startswith("---"):
        raise PromptTemplateError(
            f"template {template_id} : missing YAML front-matter"
        )

    parts = raw.split("---", 2)
    if len(parts) < 3:
        raise PromptTemplateError(
            f"template {template_id} : malformed front-matter delimiter"
        )

    _, header_yaml, body = parts
    try:
        header = yaml.safe_load(header_yaml) or {}
    except yaml.YAMLError as e:
        raise PromptTemplateError(
            f"template {template_id} : invalid YAML header : {e}"
        ) from e

    system_body, user_body = _split_system_user(body, template_id)

    template = PromptTemplate(
        name=header["name"],
        version=int(header["version"]),
        license=header.get("license", "proprietary"),
        provider_compat=list(header.get("provider_compat", [])),
        eval_pass=bool(header.get("eval_pass", False)),
        eval_accuracy=header.get("eval_accuracy"),
        system_body=system_body,
        user_body=user_body,
        notes=header.get("notes", ""),
        min_context_tokens=int(header.get("min_context_tokens", 4000)),
        max_response_tokens=int(header.get("max_response_tokens", 1024)),
        metadata=header,
    )
    _CACHE[template_id] = template
    return template


def render(template_id: str, **kwargs) -> tuple[str, str]:
    """Render `(system_prompt, user_prompt)` with `$variable` substitution.

    Uses `string.Template.safe_substitute` so missing variables become
    literal `$varname` (no exception) — this lets callers provide partial
    contexts and the unsubstituted markers serve as runtime signals to
    fix at the prompt-template development stage.
    """
    tmpl = load(template_id)
    sys = Template(tmpl.system_body).safe_substitute(**kwargs)
    usr = Template(tmpl.user_body).safe_substitute(**kwargs)
    return sys, usr


def list_templates() -> list[str]:
    """List all template ids available in this directory."""
    return sorted(p.stem for p in _TEMPLATES_DIR.glob("*.md"))


def clear_cache() -> None:
    """Clear the load cache (test helper)."""
    _CACHE.clear()


def _split_system_user(body: str, template_id: str) -> tuple[str, str]:
    """Split a template body into system + user sections.

    Format :
        # System
        ...system instructions...

        # User
        ...user-side prompt with $vars...
    """
    sections = {"system": "", "user": ""}
    current: Optional[str] = None
    for line in body.splitlines():
        s = line.strip()
        if s.lower().startswith("# system"):
            current = "system"
            continue
        if s.lower().startswith("# user"):
            current = "user"
            continue
        if current is not None:
            sections[current] += line + "\n"

    if not sections["user"]:
        raise PromptTemplateError(
            f"template {template_id} : missing `# User` section"
        )

    return sections["system"].strip(), sections["user"].strip()


__all__ = [
    "PromptTemplate",
    "PromptTemplateError",
    "load",
    "render",
    "list_templates",
    "clear_cache",
]
