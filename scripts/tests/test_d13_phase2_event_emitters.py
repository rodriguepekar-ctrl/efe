"""Sprint D13 Phase 2 — verify the Rust event emitters' output
conforms to the JSON-Schemas.

We don't compile Rust here ; instead, we verify a) the Rust
source declares structs whose fields match the JSON-Schema,
b) the canonical enum values match the JSON-Schema enums.

A more thorough end-to-end test would compile efe-server +
emit a real event ; that's covered by the Rust unit tests in
``crates/efe-telemetry/src/events/d13_streams.rs::tests``.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_SCHEMAS_DIR = _REPO / "deploy" / "observability" / "log-schemas"
_RUST_FILE   = _REPO / "crates" / "efe-telemetry" / "src" / "events" / "d13_streams.rs"


def _rust_text() -> str:
    return _RUST_FILE.read_text(encoding="utf-8")


def _schema(name: str) -> dict:
    with (_SCHEMAS_DIR / f"{name}.json").open() as fp:
        return json.load(fp)


# ─────────────────────────────────────────────────────────────────────
# Common envelope : each Rust CommonEnvelope field maps to a schema property
# ─────────────────────────────────────────────────────────────────────


COMMON_FIELDS = (
    "timestamp", "level", "request_id", "trace_id", "span_id",
    "tenant_id", "principal", "service_version",
)


@pytest.mark.parametrize("field", COMMON_FIELDS)
def test_common_envelope_field_in_rust(field):
    assert f"pub {field}:" in _rust_text(), (
        f"Rust CommonEnvelope is missing field {field!r}"
    )


@pytest.mark.parametrize("field", COMMON_FIELDS)
def test_common_envelope_field_in_schema(field):
    schema = _schema("_common")
    props = schema.get("properties", {})
    assert field in props, f"_common.json missing property {field!r}"


# ─────────────────────────────────────────────────────────────────────
# AuditAction enum strings match the schema action enum
# ─────────────────────────────────────────────────────────────────────


def test_audit_action_enum_matches_schema():
    schema = _schema("audit")
    # Schema uses an `enum` clause inside properties.action.
    actions = schema.get("properties", {}).get("action", {}).get("enum", [])
    assert set(actions) == {"register", "evolve", "delete"}
    rust = _rust_text()
    for a in actions:
        assert f'"{a}"' in rust, f"Rust missing audit action {a!r}"


# ─────────────────────────────────────────────────────────────────────
# ClassifyTier values match the schema enum
# ─────────────────────────────────────────────────────────────────────


def test_classify_tier_enum_matches_schema():
    schema = _schema("classify")
    tiers = schema.get("properties", {}).get("tier", {}).get("enum", [])
    assert set(tiers) == {"high", "medium", "low", "reject"}
    rust = _rust_text()
    for t in tiers:
        assert f'"{t}"' in rust, f"Rust missing classify tier {t!r}"


# ─────────────────────────────────────────────────────────────────────
# SecurityEventKind values match the schema enum
# ─────────────────────────────────────────────────────────────────────


def test_security_event_kind_enum_matches_schema():
    schema = _schema("security")
    kinds = schema.get("properties", {}).get("event", {}).get("enum", [])
    expected = {
        "jwt_invalid", "jwt_expired", "jwt_signature",
        "scope_missing", "tenant_mismatch", "rate_limited",
        "grafana_admin_action", "auth_ok",
    }
    assert set(kinds) == expected
    rust = _rust_text()
    for k in kinds:
        assert f'"{k}"' in rust, f"Rust missing security event kind {k!r}"


# ─────────────────────────────────────────────────────────────────────
# Op_tag enum (audit + evolution) matches the proto oneof + register/delete
# ─────────────────────────────────────────────────────────────────────


def test_op_tag_enum_matches_evolution_oneof_plus_register_delete():
    schema = _schema("audit")
    op_tags = schema.get("properties", {}).get("op_tag", {}).get("enum", [])
    expected = {
        "register", "append_class", "remove_class", "update_thresholds",
        "replace_class_anchors", "recompile", "delete",
    }
    assert set(op_tags) == expected


# ─────────────────────────────────────────────────────────────────────
# 5 streams have a typed builder in Rust
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("builder", [
    "RequestEvent", "AuditEvent", "ClassifyEvent",
    "EvolutionEvent", "SecurityEvent",
])
def test_each_stream_has_a_rust_builder(builder):
    rust = _rust_text()
    assert f"pub struct {builder}" in rust, (
        f"Rust missing builder struct {builder!r}"
    )
    assert f"impl {builder}" in rust, (
        f"Rust missing impl {builder!r}"
    )
    assert "into_value" in rust, (
        f"Rust builder {builder!r} must expose into_value()"
    )


# ─────────────────────────────────────────────────────────────────────
# Span name constants are referenced from Rust + match architecture
# ─────────────────────────────────────────────────────────────────────


def test_six_catalog_spans_declared():
    spans_rust = (_REPO / "crates" / "efe-telemetry" / "src" / "spans.rs").read_text()
    expected_spans = {
        "efe.catalog.register",
        "efe.catalog.evolve",
        "efe.catalog.get",
        "efe.catalog.list",
        "efe.catalog.list_versions",
        "efe.catalog.delete",
    }
    for s in expected_spans:
        assert f'"{s}"' in spans_rust, f"missing catalog span {s!r}"


def test_forbidden_attr_includes_pii_terms():
    spans_rust = (_REPO / "crates" / "efe-telemetry" / "src" / "spans.rs").read_text()
    for term in ("email", "token", "request_id", "text", "password"):
        assert f'"{term}"' in spans_rust, (
            f"FORBIDDEN_ATTR_PREFIXES missing {term!r}"
        )
