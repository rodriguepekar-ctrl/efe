"""M24 active learning tools.

Exposes a `TOOLS` tuple of `Tool` instances, all gated by
``EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED``. Wire into
``mcp-server/mcp_server/tools/registry.py`` :

```python
from .active_learning import TOOLS as _AL_TOOLS
_ALL_TOOLS = (..., *_AL_TOOLS)
```

See § 21 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.
"""
from __future__ import annotations

from ..types import Tool
from . import (
    confirm_gold_label,
    list_gold_label_queue,
    request_gdpr_erasure,
    trigger_bundle_retune,
)

TOOLS: tuple[Tool, ...] = (
    list_gold_label_queue.TOOL,
    confirm_gold_label.TOOL,
    trigger_bundle_retune.TOOL,
    request_gdpr_erasure.TOOL,
)

__all__ = ["TOOLS"]
