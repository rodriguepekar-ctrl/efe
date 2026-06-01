"""LLM-agent-facing wrappers.

Each submodule exposes a single entry point:

* :func:`efe_tool.bridges.claude.build_tool_spec`
* :func:`efe_tool.bridges.openai.build_function_spec`

plus a small run-loop helper that consumes tool calls, invokes the
bridge, and returns the next-turn payload.
"""
