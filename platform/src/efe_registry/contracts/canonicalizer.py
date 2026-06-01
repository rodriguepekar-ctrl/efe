"""JSON Schema canonicalisation.

Rules (mirrors M06_ARCHITECTURE.md §12.1):

* object keys sorted alphabetically (recursively);
* arrays preserve order but reject ``None`` elements;
* numbers coerced to a single type so ``1`` and ``1.0`` hash the same;
* strings NFC-normalised and right-stripped of whitespace;
* strip ``$comment`` fields;
* UTF-8, no BOM.
"""

from __future__ import annotations

import unicodedata
from collections.abc import Mapping
from typing import Any

from efe_registry.errors import ContractSchemaInvalid

_COMMENT_KEYS = frozenset({"$comment"})


def canonicalize_schema(schema: Any) -> Any:
    """Return a deep-canonicalised copy of ``schema``."""
    return _canon(schema)


def _canon(node: Any) -> Any:
    if isinstance(node, Mapping):
        out: dict[str, Any] = {}
        for k in sorted(node.keys()):
            if not isinstance(k, str):
                raise ContractSchemaInvalid(
                    f"non-string key in schema: {k!r}",
                )
            if k in _COMMENT_KEYS:
                continue
            # Keys themselves are also NFC-normalised so visually
            # identical keys produced by different editors collapse.
            out[unicodedata.normalize("NFC", k)] = _canon(node[k])
        return out
    if isinstance(node, (list, tuple)):
        as_list: list[Any] = []
        for item in node:
            if item is None:
                raise ContractSchemaInvalid(
                    "null entry in schema array — not allowed",
                )
            as_list.append(_canon(item))
        return as_list
    if isinstance(node, bool):
        # ``bool`` is subclass of ``int``; preserve strictly.
        return node
    if isinstance(node, str):
        # NFC normalisation + right-strip of whitespace. We do not
        # ltrim because leading whitespace may be semantically
        # significant in regex patterns etc.
        normalised = unicodedata.normalize("NFC", node)
        return normalised.rstrip()
    if isinstance(node, int):
        # Coerce integer-valued numbers to float for stable hashing —
        # users declaring ``multipleOf: 1`` vs ``multipleOf: 1.0`` now
        # produce the same contract_hash (M3 fix).
        return float(node)
    if isinstance(node, float):
        if node != node:  # NaN
            raise ContractSchemaInvalid("NaN not allowed in schema")
        if node in (float("inf"), float("-inf")):
            raise ContractSchemaInvalid("Inf not allowed in schema")
        return float(node)
    if node is None:
        return None
    # Fallback — coerce to str to stay deterministic.
    return unicodedata.normalize("NFC", str(node)).rstrip()


__all__ = ["canonicalize_schema"]
