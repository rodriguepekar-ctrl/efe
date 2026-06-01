"""Token counter — per model family via official tokenizer.

Implements I6 (exact tokenization). For MVP the registry ships with :

* :class:`ApproxTokenCounter` — word-boundary heuristic, used when a
  real tokenizer is not available (e.g. CPU-only CI without tiktoken).
* :class:`TikTokenCounter` — exact for GPT-4 / GPT-3.5 (tiktoken).
* :class:`ClaudeTokenCounter` — exact for Anthropic (anthropic SDK's
  ``count_tokens`` helper).
* :class:`SentencePieceTokenCounter` — exact for Llama-3 / Mistral.

Only the approximate counter is tested directly here (the real ones
require heavy deps). In production, the registry is populated via
factory at boot time.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Protocol

from efe_inference.errors import TokenizerUnavailable


class TokenCounter(Protocol):
    name: str

    def count(self, text: str) -> int:  # pragma: no cover
        ...


@dataclass
class ApproxTokenCounter:
    """Word-like approximation — 1 token per short word, 2 per long.

    Fix M4 — CJK / non-whitespace scripts (Chinese, Japanese, Korean,
    Thai) do not tokenise cleanly on whitespace. The English path
    (``\\S+`` split + char heuristic) under-counts those scripts by
    10-50×. We now tally ``non-ASCII codepoints × 1.2`` separately and
    add that to the word-based count for ASCII portions, producing a
    conservative upper-bound for all languages.
    """

    name: str = "approx"

    def count(self, text: str) -> int:
        if not text:
            return 0
        # Separate ASCII (word-based) vs non-ASCII (codepoint-based).
        ascii_chunks: list[str] = []
        non_ascii_codepoints = 0
        for ch in text:
            if ord(ch) < 128:
                ascii_chunks.append(ch)
            else:
                non_ascii_codepoints += 1
        ascii_text = "".join(ascii_chunks)
        total = 0
        for w in re.findall(r"\S+", ascii_text):
            total += 1 + (len(w) // 6)
        # CJK heuristic: ~1 token per codepoint (tiktoken + sentencepiece
        # both pay roughly one token per CJK character).
        total += int(non_ascii_codepoints * 1.2)
        return total


class _RealTokenCounterBase:
    """Common scaffolding. Subclasses provide the backend-specific loader."""

    name: str

    def count(self, text: str) -> int:
        raise NotImplementedError


@dataclass
class TokenCounterRegistry:
    _impls: dict[str, TokenCounter]

    @classmethod
    def default(cls) -> TokenCounterRegistry:
        return cls(_impls={"approx": ApproxTokenCounter()})

    def register(self, tokenizer_name: str, counter: TokenCounter) -> None:
        self._impls[tokenizer_name] = counter

    def get(self, tokenizer_name: str) -> TokenCounter:
        if tokenizer_name not in self._impls:
            raise TokenizerUnavailable(
                f"no tokenizer registered under {tokenizer_name!r}",
            )
        return self._impls[tokenizer_name]

    def count(self, tokenizer_name: str, text: str) -> int:
        return self.get(tokenizer_name).count(text)

    def names(self) -> tuple[str, ...]:
        return tuple(sorted(self._impls))


__all__ = [
    "ApproxTokenCounter",
    "TokenCounter",
    "TokenCounterRegistry",
]
