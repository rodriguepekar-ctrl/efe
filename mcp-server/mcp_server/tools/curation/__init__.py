"""M23 curation tools for the EFE MCP server.

Exposes a `TOOLS` tuple of `Tool` instances, each gated by the
``EFE_FEATURE_M23_CURATION_ENABLED`` env var (per-tool ``feature_flag``).
The registry filters out feature-flagged-OFF tools at registry-build
time, so when the env var is unset/false, ``list_tools`` does not
expose these handlers.

Wire into ``mcp-server/mcp_server/tools/registry.py`` by adding to the
imports and to the ``_ALL_TOOLS`` tuple :

```python
from .curation import TOOLS as _CURATION_TOOLS
_ALL_TOOLS: tuple[Tool, ...] = (..., *_CURATION_TOOLS)
```

See § 16 of EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md.
"""
from __future__ import annotations

from ..types import Tool
from . import (
    audit_vs_gold,
    explain_misclassification,
    generate_synth,
    propose_split,
    suggest_merge,
)

TOOLS: tuple[Tool, ...] = (
    propose_split.TOOL,
    explain_misclassification.TOOL,
    generate_synth.TOOL,
    suggest_merge.TOOL,
    audit_vs_gold.TOOL,
)

__all__ = ["TOOLS"]
