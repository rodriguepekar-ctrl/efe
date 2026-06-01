"""M25 agent runtime tools.

Exposes a `TOOLS` tuple of `Tool` instances, all gated by
``EFE_FEATURE_AGENT_RUNTIME_ENABLED``. Wire into
``mcp-server/mcp_server/tools/registry.py`` :

```python
from .agent_runtime import TOOLS as _AR_TOOLS
_ALL_TOOLS = (..., *_AR_TOOLS)
```

See § 19 of EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md.
"""
from __future__ import annotations

from ..types import Tool
from . import (
    approve_agent_step,
    create_agent_run,
    get_agent_run,
    terminate_agent_run,
)

TOOLS: tuple[Tool, ...] = (
    create_agent_run.TOOL,
    get_agent_run.TOOL,
    approve_agent_step.TOOL,
    terminate_agent_run.TOOL,
)
