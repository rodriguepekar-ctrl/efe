"""Sprint D10 — assert that every public symbol in
``efe_tool.__all__`` is documented in the API reference.

The reference page (`docs/docs/reference/api.md`) uses
mkdocstrings' `::: efe_tool.<module>` directives to auto-include
docstrings. This test parses the markdown and asserts that the
union of all `::: ...` directives covers ≥ 95 % of the
``__all__`` symbols.

Doesn't actually call mkdocs — pure static analysis. The full
mkdocs build is gated by ``.github/workflows/docs.yml::build-and-lint``.
"""

from __future__ import annotations

import importlib
import re
import sys
from pathlib import Path


_REPO = Path(__file__).resolve().parents[2]
_SDK_PATH = _REPO / "clients" / "python"
_API_MD = _REPO / "docs" / "docs" / "reference" / "api.md"

sys.path.insert(0, str(_SDK_PATH))


def _parse_api_reference_directives() -> set[str]:
    """Extract the set of fully-qualified symbols referenced in
    ``api.md`` via ``::: <fqn>`` directives.

    Returns the union of:
      * ``::: efe_tool`` → all top-level `__all__` exports.
      * ``::: efe_tool.<module>`` with `members:` → those members.
      * ``::: efe_tool.<module>`` without `members:` → all the
        module's documented public symbols (we approximate this by
        treating it as a wildcard match for any symbol in that
        module).
    """
    text = _API_MD.read_text(encoding="utf-8")
    referenced: set[str] = set()
    # Match ``::: efe_tool.module`` blocks and their optional members:.
    block_pat = re.compile(
        r"^:::\s+(?P<fqn>efe_tool[\w.]*)\s*$"
        r"(?P<options>(?:\n[ \t]+.*)*)",
        re.MULTILINE,
    )
    # NOTE — the trailing `\n?` on the last item is critical: when the
    # last members entry is the LAST line of the captured block (no
    # trailing newline), strict `\n` would fail to match.
    members_pat = re.compile(
        r"^[ \t]+members:\s*\n"
        r"(?P<list>(?:[ \t]+-\s+\w+\n?)+)",
        re.MULTILINE,
    )
    for m in block_pat.finditer(text):
        fqn = m.group("fqn")
        options = m.group("options") or ""
        members_match = members_pat.search(options)
        if members_match:
            for line in members_match.group("list").splitlines():
                line = line.strip().lstrip("-").strip()
                if line:
                    referenced.add(line)
        else:
            # Wildcard for the module — mark all `__all__` of that
            # module (best-effort).
            module_name = fqn
            try:
                module = importlib.import_module(module_name)
                if hasattr(module, "__all__"):
                    for sym in module.__all__:
                        referenced.add(sym)
            except ImportError:
                # The module may not be importable (e.g. _async
                # requires grpc.aio). Skip — coverage will be lower
                # but the test still validates the rest.
                pass
    return referenced


def test_api_reference_covers_at_least_95pct_of_public_surface():
    efe_tool = importlib.import_module("efe_tool")
    public_symbols = set(efe_tool.__all__)
    documented = _parse_api_reference_directives()

    # Some symbols are dunders or version-related — exempt.
    EXEMPT = {"__version__", "__version_tuple__"}

    expected_documented = public_symbols - EXEMPT
    actual_documented = documented & expected_documented
    coverage = len(actual_documented) / len(expected_documented)

    missing = expected_documented - actual_documented

    assert coverage >= 0.95, (
        f"D10-DOC-2 SLO violation: API reference covers "
        f"{coverage * 100:.1f}% of public symbols (target ≥ 95%).\n"
        f"Missing from `docs/docs/reference/api.md`: {sorted(missing)}\n"
        f"Add the missing symbols to a relevant `::: efe_tool.<module>` "
        f"block's `members:` list."
    )


def test_api_md_exists():
    assert _API_MD.exists(), f"API reference page missing: {_API_MD}"


def test_api_md_uses_mkdocstrings_syntax():
    text = _API_MD.read_text(encoding="utf-8")
    # At least one ``::: efe_tool`` directive must be present.
    assert re.search(r"^:::\s+efe_tool", text, re.MULTILINE), (
        "api.md does not use mkdocstrings — expected `::: efe_tool…` directives"
    )


def test_api_md_includes_each_top_level_module():
    """Sanity: every key public module has at least one `:::` block."""
    text = _API_MD.read_text(encoding="utf-8")
    expected_modules = [
        "efe_tool",
        "efe_tool.client",
        "efe_tool.catalog",
        "efe_tool.embedder",
        "efe_tool.types",
        "efe_tool.errors",
        "efe_tool.integrations.hybrid_decoder",
    ]
    for mod in expected_modules:
        # Match `::: efe_tool` exactly (not as prefix of e.g.
        # `efe_tool.client`).
        pat = rf"^:::\s+{re.escape(mod)}\s*$"
        assert re.search(pat, text, re.MULTILINE), (
            f"`{mod}` has no `:::` block in api.md — its public "
            f"surface won't be auto-documented."
        )
