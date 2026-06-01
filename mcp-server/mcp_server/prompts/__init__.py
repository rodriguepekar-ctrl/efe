"""MCP Prompts (M17 D44) — 5 pre-built workflow templates."""
from .registry import PROMPT_DESCRIPTORS, get_prompt, render_prompt

__all__ = ["PROMPT_DESCRIPTORS", "get_prompt", "render_prompt"]
