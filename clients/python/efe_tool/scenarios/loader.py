"""Prompt and Scenario loaders.

A *scenario* is a versioned bundle of four artefacts:

* ``prompt.md``   — the system prompt, prefixed with YAML frontmatter.
* ``golden.json`` — a list of golden cases (user text + expected label).
* ``README.md``   — prose for humans.
* ``__init__.py`` — empty package marker (so builtin scenarios can be
  imported via :mod:`importlib.resources`).

See ``LETTER_C_ARCHITECTURE.md`` §16 for the design.

This module is **pure** — no network, no LLM calls. Callers combine it
with :mod:`efe_tool.bridges.claude` and :mod:`efe_tool.scenarios.runner`
to actually *run* a scenario.
"""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Mapping, Optional, Sequence


# ---------------------------------------------------------------------------
#  Dataclasses
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Prompt:
    """A versioned system prompt.

    Attributes
    ----------
    text:
        The body that follows the YAML frontmatter.
    metadata:
        Parsed frontmatter (``{name, version, model, catalogs, …}``).
    hash:
        ``sha256:<16 hex chars>`` of ``text`` — a short fingerprint
        safe to log and persist in transcripts.
    source_path:
        Where the file came from. ``None`` when built from a string.
    """

    text: str
    metadata: Mapping[str, Any]
    hash: str
    source_path: Optional[str] = None

    @property
    def name(self) -> str:
        return str(self.metadata.get("name", "?"))

    @property
    def version(self) -> str:
        return str(self.metadata.get("version", "0.0.0"))

    @property
    def model(self) -> str:
        return str(self.metadata.get("model", "claude-sonnet-4-6"))

    @property
    def catalog_names(self) -> tuple[str, ...]:
        raw = self.metadata.get("catalogs") or ()
        if isinstance(raw, str):
            return (raw,)
        return tuple(str(x) for x in raw)


@dataclass(frozen=True)
class GoldenCase:
    """One reference input + expected outcome for bench."""

    user: str
    expected_label: Optional[str] = None
    max_rounds: int = 4
    notes: str = ""
    language: str = "en"


@dataclass(frozen=True)
class Scenario:
    """A scenario bundle loaded from disk or package resources.

    A Scenario is the complete artefact that a Letter-C script consumes:
    prompt, golden cases, readme, plus the directory it came from (so
    the runner can locate catalogs or auxiliary files by convention).
    """

    name: str
    prompt: Prompt
    golden: tuple[GoldenCase, ...]
    readme: str = ""
    source_dir: Optional[str] = None


# ---------------------------------------------------------------------------
#  Frontmatter parsing
# ---------------------------------------------------------------------------


_FRONTMATTER_RE = re.compile(
    r"\A---\s*\n(?P<front>.*?)\n---\s*\n(?P<body>.*)\Z",
    re.DOTALL,
)


def _parse_frontmatter(text: str) -> tuple[dict[str, Any], str]:
    """Split a markdown document with YAML frontmatter.

    Returns ``({frontmatter dict}, body_text)``. If no frontmatter is
    present, returns ``({}, text)``.
    """
    match = _FRONTMATTER_RE.match(text)
    if not match:
        return {}, text
    front_raw = match.group("front")
    body = match.group("body")
    try:
        import yaml  # type: ignore
    except ImportError as e:  # pragma: no cover
        raise RuntimeError(
            "pyyaml is required to parse scenario prompts"
        ) from e
    data = yaml.safe_load(front_raw) or {}
    if not isinstance(data, dict):
        raise ValueError(
            f"frontmatter must be a mapping, got {type(data).__name__}"
        )
    return data, body


# ---------------------------------------------------------------------------
#  load_prompt
# ---------------------------------------------------------------------------


def load_prompt(path_or_text: str | Path, *, is_text: bool = False) -> Prompt:
    """Load a :class:`Prompt` from a path or from a raw markdown string.

    Args:
        path_or_text: Either a filesystem path to a ``.md`` file, or the
            raw markdown text.
        is_text: Set to ``True`` when passing a string literal.

    The ``hash`` field is computed from the body (not the frontmatter),
    so that pure metadata edits (e.g. changing ``version``) do not
    shift the fingerprint.
    """
    if is_text:
        raw = str(path_or_text)
        source_path: Optional[str] = None
    else:
        path = Path(path_or_text)
        raw = path.read_text(encoding="utf-8")
        source_path = str(path)

    metadata, body = _parse_frontmatter(raw)
    body = body.strip("\n")
    digest = hashlib.sha256(body.encode("utf-8")).hexdigest()[:16]
    return Prompt(
        text=body,
        metadata=metadata,
        hash=f"sha256:{digest}",
        source_path=source_path,
    )


# ---------------------------------------------------------------------------
#  load_golden
# ---------------------------------------------------------------------------


def load_golden(path: str | Path) -> tuple[GoldenCase, ...]:
    """Parse a golden case file into a tuple of :class:`GoldenCase`.

    The file is JSON, expected shape: a list of objects with at least
    ``user``. Unknown keys are ignored (forward-compat).
    """
    p = Path(path)
    data = json.loads(p.read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise ValueError(
            f"golden file must contain a JSON list, got {type(data).__name__}"
        )
    out: list[GoldenCase] = []
    for i, item in enumerate(data):
        if not isinstance(item, dict):
            raise ValueError(f"golden[{i}] must be an object")
        user = item.get("user")
        if not isinstance(user, str) or not user.strip():
            raise ValueError(f"golden[{i}].user must be a non-empty string")
        out.append(GoldenCase(
            user=user,
            expected_label=item.get("expected_label"),
            max_rounds=int(item.get("max_rounds", 4)),
            notes=str(item.get("notes", "")),
            language=str(item.get("language", "en")),
        ))
    return tuple(out)


# ---------------------------------------------------------------------------
#  Scenario resolution
# ---------------------------------------------------------------------------


def list_builtin_scenarios() -> list[str]:
    """Return the names of scenarios packaged under
    ``efe_tool.scenarios.builtin``."""
    try:
        from importlib.resources import files
    except ImportError:  # pragma: no cover — Python < 3.9
        return []
    try:
        root = files("efe_tool.scenarios.builtin")
    except (ModuleNotFoundError, FileNotFoundError):  # pragma: no cover
        return []
    names: list[str] = []
    for entry in root.iterdir():
        # ``entry`` is a Traversable. We accept it only if it behaves
        # like a directory with a ``prompt.md`` inside.
        try:
            if entry.is_dir() and (entry / "prompt.md").is_file():
                names.append(entry.name)
        except Exception:  # pragma: no cover — robustness
            continue
    return sorted(names)


def resolve_scenario_dir(name: str, *, search_paths: Sequence[Path] = ()) -> Path:
    """Find a scenario's directory by name.

    Lookup order (first match wins):

    1. Explicit ``search_paths`` (e.g. ``["./scenarios"]``).
    2. The builtin package ``efe_tool.scenarios.builtin``.

    Raises:
        FileNotFoundError: if no matching directory contains a
            ``prompt.md``.
    """
    for root in search_paths:
        candidate = Path(root) / name
        if (candidate / "prompt.md").is_file():
            return candidate

    # Builtin fallback — use filesystem traversal of the package dir
    # so the result is usable as a plain Path (no Traversable surprise).
    try:
        import efe_tool.scenarios.builtin as _b
        package_dir = Path(_b.__file__).parent / name
    except Exception as e:  # pragma: no cover
        raise FileNotFoundError(f"builtin scenarios package unavailable: {e}") from e

    if (package_dir / "prompt.md").is_file():
        return package_dir

    raise FileNotFoundError(
        f"scenario {name!r} not found (searched: "
        f"{[str(p) for p in search_paths] + [str(package_dir.parent)]})"
    )


def load_scenario(
    name: str,
    *,
    search_paths: Sequence[Path | str] = (),
) -> Scenario:
    """Load a scenario bundle by name.

    The returned :class:`Scenario` contains the parsed prompt, the
    golden cases (empty tuple if ``golden.json`` is missing) and the
    README text (empty string if absent).
    """
    norm_paths = tuple(Path(p) for p in search_paths)
    scenario_dir = resolve_scenario_dir(name, search_paths=norm_paths)

    prompt = load_prompt(scenario_dir / "prompt.md")
    golden_path = scenario_dir / "golden.json"
    golden = load_golden(golden_path) if golden_path.is_file() else ()
    readme_path = scenario_dir / "README.md"
    readme = (
        readme_path.read_text(encoding="utf-8")
        if readme_path.is_file() else ""
    )

    metadata_name = prompt.metadata.get("name")
    effective_name = str(metadata_name) if metadata_name else name
    return Scenario(
        name=effective_name,
        prompt=prompt,
        golden=golden,
        readme=readme,
        source_dir=str(scenario_dir),
    )


# ---------------------------------------------------------------------------
#  Serialisation helpers
# ---------------------------------------------------------------------------


def scenario_summary(scenario: Scenario) -> dict[str, Any]:
    """Return a JSON-friendly summary of a scenario, used by ``list``."""
    return {
        "name": scenario.name,
        "version": scenario.prompt.version,
        "model": scenario.prompt.model,
        "catalogs": list(scenario.prompt.catalog_names),
        "prompt_hash": scenario.prompt.hash,
        "golden_cases": len(scenario.golden),
        "source_dir": scenario.source_dir,
    }


__all__ = [
    "Prompt",
    "Scenario",
    "GoldenCase",
    "load_prompt",
    "load_golden",
    "load_scenario",
    "list_builtin_scenarios",
    "resolve_scenario_dir",
    "scenario_summary",
]
