"""Pydantic schemas for the native REST façade."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field


class NativeChatRequestModel(BaseModel):
    model_config = ConfigDict(extra="forbid")

    task_id: str = Field(min_length=1)
    messages: list[dict[str, Any]]
    sampling: dict[str, Any] | None = None
    tools: list[dict[str, Any]] | None = None
    tool_choice: str = "auto"
    stream: bool = False
    model_hint: str | None = None
    user_id: str | None = None
    timeout_budget_s: float = 30.0
    metadata: dict[str, str] | None = None


class NativeChatResponseModel(BaseModel):
    model_config = ConfigDict(extra="forbid")

    request_id: str
    model: str
    choices: list[dict[str, Any]]
    usage: dict[str, int]
    backend_kind: str
    backend_latency_ms: float
    adapter_overhead_ms: float
    safety_flags: dict[str, list[str]]
    cached: str


__all__ = [
    "NativeChatRequestModel",
    "NativeChatResponseModel",
]
