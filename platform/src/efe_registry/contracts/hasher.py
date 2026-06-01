"""Compute a ``contract_hash`` from (input, output) JSON Schemas."""

from __future__ import annotations

import hashlib
from typing import Any

from efe_registry.contracts.canonicalizer import canonicalize_schema
from efe_registry.hashing import canonical_json_bytes


def contract_hash_from_schemas(
    input_schema: Any, output_schema: Any,
) -> bytes:
    payload = {
        "in": canonicalize_schema(input_schema),
        "out": canonicalize_schema(output_schema),
    }
    return hashlib.sha256(canonical_json_bytes(payload)).digest()


__all__ = ["contract_hash_from_schemas"]
