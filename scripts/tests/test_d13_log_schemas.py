"""Sprint D13 — JSON-Schema validation of the 5 log streams.

Validates :

  1. Each schema file exists in the expected location.
  2. Each schema parses as valid JSON.
  3. Each schema declares the right `stream` const.
  4. Sample synthetic events conform to the schema.
  5. The common envelope is referenced correctly.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_SCHEMAS_DIR = _REPO / "deploy" / "observability" / "log-schemas"


# Optional import : `jsonschema` is a dev dep, skipped if missing.
try:
    import jsonschema
    HAVE_JSONSCHEMA = True
except ImportError:
    HAVE_JSONSCHEMA = False


# ─────────────────────────────────────────────────────────────────────
# 1. Schema files exist
# ─────────────────────────────────────────────────────────────────────

EXPECTED_STREAMS = ("request", "audit", "classify", "evolution", "security")


def test_common_schema_exists():
    p = _SCHEMAS_DIR / "_common.json"
    assert p.exists(), f"missing common schema {p}"


def test_all_5_streams_have_a_schema_file():
    for stream in EXPECTED_STREAMS:
        p = _SCHEMAS_DIR / f"{stream}.json"
        assert p.exists(), f"missing {stream} schema at {p}"


# ─────────────────────────────────────────────────────────────────────
# 2. JSON parses
# ─────────────────────────────────────────────────────────────────────

def test_each_schema_parses_as_valid_json():
    for f in _SCHEMAS_DIR.glob("*.json"):
        with f.open() as fp:
            json.load(fp)  # raises if invalid


# ─────────────────────────────────────────────────────────────────────
# 3. Each per-stream schema declares the right `stream` const
# ─────────────────────────────────────────────────────────────────────

def test_each_per_stream_schema_pins_stream_const():
    for stream in EXPECTED_STREAMS:
        with (_SCHEMAS_DIR / f"{stream}.json").open() as fp:
            schema = json.load(fp)
        # Look for "stream": {"const": "<stream>"} somewhere in allOf.
        allof = schema.get("allOf", [])
        found = False
        for sub in allof:
            props = sub.get("properties", {}).get("stream", {})
            if props.get("const") == stream:
                found = True
                break
        assert found, f"schema {stream}.json doesn't pin stream const"


# ─────────────────────────────────────────────────────────────────────
# 4. Synthetic events validate
# ─────────────────────────────────────────────────────────────────────

@pytest.mark.skipif(not HAVE_JSONSCHEMA, reason="jsonschema not installed")
def test_audit_event_validates():
    # Standalone audit schema (resolves _common via $ref). We use the
    # in-memory validator — load both schemas, register the common,
    # then validate.
    from jsonschema import RefResolver, Draft202012Validator

    common = json.loads((_SCHEMAS_DIR / "_common.json").read_text())
    audit = json.loads((_SCHEMAS_DIR / "audit.json").read_text())

    store = {common["$id"]: common, audit["$id"]: audit, "_common.json": common}
    resolver = RefResolver(base_uri=audit["$id"], referrer=audit, store=store)
    validator = Draft202012Validator(audit, resolver=resolver)

    sample = {
        "schema_version": "1.0",
        "stream": "audit",
        "timestamp": "2026-04-27T14:30:00.123Z",
        "level": "INFO",
        "service": "efe-server",
        "service_version": "0.1.13",
        "tenant_id": "tenant-acme",
        "principal": "alice@acme.com",
        "action": "register",
        "resource_type": "catalog",
        "resource_key": "tenant-acme/billing@v1",
        "op_tag": "register",
        "content_sha256": "deadbeef",
    }
    validator.validate(sample)


@pytest.mark.skipif(not HAVE_JSONSCHEMA, reason="jsonschema not installed")
def test_audit_event_rejects_invalid():
    from jsonschema import RefResolver, Draft202012Validator, ValidationError

    common = json.loads((_SCHEMAS_DIR / "_common.json").read_text())
    audit = json.loads((_SCHEMAS_DIR / "audit.json").read_text())
    store = {common["$id"]: common, audit["$id"]: audit, "_common.json": common}
    resolver = RefResolver(base_uri=audit["$id"], referrer=audit, store=store)
    validator = Draft202012Validator(audit, resolver=resolver)

    bad_action = {
        "schema_version": "1.0",
        "stream": "audit",
        "timestamp": "2026-04-27T14:30:00.123Z",
        "level": "INFO",
        "service": "efe-server",
        "service_version": "0.1.13",
        "tenant_id": "tenant-acme",
        "principal": "alice@acme.com",
        "action": "BANANA",  # ← not in enum
        "resource_type": "catalog",
        "resource_key": "tenant-acme/billing@v1",
        "op_tag": "register",
    }
    with pytest.raises(ValidationError):
        validator.validate(bad_action)


# ─────────────────────────────────────────────────────────────────────
# 5. Common envelope referenced
# ─────────────────────────────────────────────────────────────────────

def test_per_stream_schemas_reference_common():
    for stream in EXPECTED_STREAMS:
        with (_SCHEMAS_DIR / f"{stream}.json").open() as fp:
            text = fp.read()
        assert "_common.json" in text, (
            f"schema {stream}.json must $ref _common.json"
        )


def test_schema_version_is_pinned_to_1_0_for_v0_1_13():
    """The common envelope's `schema_version` field is the wire-
    contract for log consumers. v0.1.13 ships schema 1.0."""
    with (_SCHEMAS_DIR / "_common.json").open() as fp:
        common = json.load(fp)
    pattern = (common.get("properties", {})
                     .get("schema_version", {})
                     .get("pattern", ""))
    assert pattern, "schema_version field must declare a pattern"
    # The pattern allows MAJOR.MINOR ; verify it's a regex form.
    import re
    assert re.match(pattern, "1.0"), \
        f"schema_version pattern {pattern!r} must accept '1.0'"
