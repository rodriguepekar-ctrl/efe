"""EFE MCP Server (M16) — Model Context Protocol bridge for EFE 1.0 GA.

Exposes EFE classification + catalog admin + audit RPCs as MCP tools so any
MCP-compatible LLM client (Claude Desktop, Cursor, Zed, Continue, Cline, …)
can call EFE without a custom plugin.

Compliance posture is preserved : the LLM is the user's choice in their
own MCP client ; the MCP server itself adds no new sub-processors.

See `docs/EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md` for the full design.
"""

__version__ = "0.1.0"
__all__ = ["__version__"]
