"""Embedder — turns text into a numpy vector, caches in the blob store."""

from __future__ import annotations

from efe_ft.embedder.backends.mock import MockEmbedder
from efe_ft.embedder.worker import EmbedderWorker, EmbeddingBackend

__all__ = ["EmbedderWorker", "EmbeddingBackend", "MockEmbedder"]
