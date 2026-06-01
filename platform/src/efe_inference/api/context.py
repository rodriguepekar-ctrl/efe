"""ApiContext — bundle collaborator references used by the HTTP routes."""

from __future__ import annotations

from dataclasses import dataclass

from efe_inference.adapter import InferenceAdapter
from efe_inference.config import InferenceSettings


@dataclass
class ApiContext:
    settings: InferenceSettings
    adapter: InferenceAdapter


__all__ = ["ApiContext"]
