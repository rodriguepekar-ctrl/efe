"""Tokeniser abstraction — real HF + deterministic fake.

The worker never imports :mod:`transformers` at module-load time: the
real HF tokeniser is instantiated lazily only when
:class:`HuggingFaceTokenizer` is used. Tests rely on :class:`FakeTokenizer`
which is pure Python, dependency-free, and deterministic.

Both implement the :class:`Tokenizer` protocol used by every sampler
and method.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_worker.errors import MissingDependency, TokenizationError


@dataclass(frozen=True, slots=True)
class TokenizedBatch:
    """Output of :meth:`Tokenizer.encode_batch`.

    We use plain Python lists to keep the object framework-agnostic;
    methods that need torch tensors wrap on their own.
    """

    input_ids: list[list[int]]
    attention_mask: list[list[int]]
    texts: list[str] = field(default_factory=list)

    @property
    def batch_size(self) -> int:
        return len(self.input_ids)


class Tokenizer(Protocol):
    max_seq_len: int
    vocab_size: int

    def encode(self, text: str) -> list[int]:  # pragma: no cover
        ...

    def encode_batch(
        self, texts: list[str],
    ) -> TokenizedBatch:  # pragma: no cover
        ...

    def decode(self, ids: list[int]) -> str:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# Fake tokenizer — tests
# ---------------------------------------------------------------------------


class FakeTokenizer:
    """Deterministic pure-Python tokeniser.

    Each character in the input string contributes an id based on
    ``ord(c) % vocab_size``. Padding token is 0, mask token is 1.
    """

    PAD_ID = 0
    MASK_ID = 1
    BOS_ID = 2
    EOS_ID = 3

    def __init__(
        self,
        *,
        vocab_size: int = 512,
        max_seq_len: int = 64,
    ) -> None:
        if vocab_size < 32:
            raise TokenizationError("vocab_size must be >= 32")
        self.vocab_size = int(vocab_size)
        self.max_seq_len = int(max_seq_len)

    def _one(self, text: str) -> list[int]:
        ids = [self.BOS_ID]
        for c in text[: self.max_seq_len - 2]:
            ids.append(4 + (ord(c) % (self.vocab_size - 4)))
        ids.append(self.EOS_ID)
        return ids

    def encode(self, text: str) -> list[int]:
        return self._one(text)

    def encode_batch(self, texts: list[str]) -> TokenizedBatch:
        if not texts:
            raise TokenizationError("encode_batch requires >= 1 text")
        encoded = [self._one(t) for t in texts]
        max_len = max(len(ids) for ids in encoded)
        input_ids = [
            ids + [self.PAD_ID] * (max_len - len(ids)) for ids in encoded
        ]
        attention_mask = [
            [1] * len(ids) + [0] * (max_len - len(ids)) for ids in encoded
        ]
        return TokenizedBatch(
            input_ids=input_ids,
            attention_mask=attention_mask,
            texts=list(texts),
        )

    def decode(self, ids: list[int]) -> str:
        # Lossy inverse — used only for logging.
        return "".join(
            chr(i - 4) for i in ids
            if i >= 4 and i < self.vocab_size
        )


# ---------------------------------------------------------------------------
# Real HF tokenizer (lazy)
# ---------------------------------------------------------------------------


class HuggingFaceTokenizer:
    """Thin wrapper around :mod:`transformers.AutoTokenizer`.

    Instantiation is lazy — the ``transformers`` import happens at
    the first :meth:`encode_batch`. This keeps cold-boot + tests
    independent of the heavy dependency.
    """

    def __init__(
        self,
        model_name: str,
        *,
        max_seq_len: int = 128,
        trust_remote_code: bool = False,
    ) -> None:
        self._model_name = model_name
        self.max_seq_len = int(max_seq_len)
        self._trust = bool(trust_remote_code)
        self._tokenizer: Any | None = None
        self._vocab_size: int | None = None

    def _ensure(self) -> Any:
        if self._tokenizer is None:
            try:
                from transformers import AutoTokenizer
            except ImportError as exc:  # pragma: no cover
                raise MissingDependency(
                    "transformers is required for HuggingFaceTokenizer",
                ) from exc
            try:
                self._tokenizer = AutoTokenizer.from_pretrained(
                    self._model_name,
                    trust_remote_code=self._trust,
                )
            except Exception as exc:  # pragma: no cover — network
                raise TokenizationError(
                    f"failed to load tokenizer {self._model_name!r}: {exc}",
                ) from exc
            self._vocab_size = int(self._tokenizer.vocab_size)
        return self._tokenizer

    @property
    def vocab_size(self) -> int:
        if self._vocab_size is None:
            self._ensure()
        return self._vocab_size or 0

    def encode(self, text: str) -> list[int]:
        tk = self._ensure()
        out = tk(
            text,
            max_length=self.max_seq_len,
            truncation=True,
            padding=False,
        )
        return list(out["input_ids"])

    def encode_batch(self, texts: list[str]) -> TokenizedBatch:
        tk = self._ensure()
        out = tk(
            list(texts),
            max_length=self.max_seq_len,
            padding="longest",
            truncation=True,
        )
        ids = [list(x) for x in out["input_ids"]]
        mask = [list(x) for x in out["attention_mask"]]
        return TokenizedBatch(
            input_ids=ids, attention_mask=mask, texts=list(texts),
        )

    def decode(self, ids: list[int]) -> str:
        tk = self._ensure()
        return str(tk.decode(ids, skip_special_tokens=True))


__all__ = [
    "FakeTokenizer",
    "HuggingFaceTokenizer",
    "TokenizedBatch",
    "Tokenizer",
]
