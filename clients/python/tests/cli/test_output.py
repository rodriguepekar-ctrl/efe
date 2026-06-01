"""Tests for :mod:`efe_tool.cli.output`."""

from __future__ import annotations

import io
import json
from types import SimpleNamespace

import pytest

from efe_tool.cli.errors import UsageError, CatalogNotFoundError
from efe_tool.cli.metadata import SCHEMA_VERSION
from efe_tool.cli.output import (
    DocTiming,
    build_classify_document,
    build_compile_document,
    build_healthcheck_document,
    build_validate_document,
    build_version_document,
    render,
    render_error,
)


# ---------------------------------------------------------------------------
#  build_*_document
# ---------------------------------------------------------------------------


class TestClassifyDocument:
    def test_canonical(self):
        result = SimpleNamespace(
            label="fraud",
            confidence=0.575,
            alignment_delta=0.12,
            rounds=4,
            reject=False,
            total_ms=800,
        )
        catalog = SimpleNamespace(
            name="support_triage",
            version="1.0.0",
            embedder_fingerprint="sha256:abc",
        )
        doc = build_classify_document(
            result, catalog=catalog,
            timing=DocTiming(elapsed_ms=820, server_ms=800),
            request_id="rid",
        )
        assert doc["schema_version"] == SCHEMA_VERSION
        assert doc["label"] == "fraud"
        assert doc["confidence"] == 0.575
        assert doc["rounds"] == 4
        assert doc["catalog"]["name"] == "support_triage"
        assert doc["timing"]["elapsed_ms"] == 820
        assert doc["request_id"] == "rid"
        assert "diagnostics" not in doc

    def test_with_diagnostics(self):
        sub = SimpleNamespace(label="fraud", cosine=0.8)
        result = SimpleNamespace(
            label="fraud", confidence=0.5, alignment_delta=0.1,
            rounds=2, reject=False, total_ms=100,
            sub_scores=[sub], swarm_id="s-1", text_hash="h",
            top_cosine=0.85, extra={"k": "v"},
        )
        catalog = SimpleNamespace(
            name="c", version="1", embedder_fingerprint="fp"
        )
        doc = build_classify_document(
            result, catalog=catalog,
            timing=DocTiming(elapsed_ms=1, server_ms=1),
            request_id="r", with_diagnostics=True,
        )
        assert doc["diagnostics"]["swarm_id"] == "s-1"
        assert doc["diagnostics"]["sub_scores"][0]["label"] == "fraud"
        assert doc["diagnostics"]["extra"] == {"k": "v"}


class TestHealthcheckDocument:
    def test_serving(self):
        doc = build_healthcheck_document(
            serving=True, latency_ms=12,
            endpoint="127.0.0.1:50051", request_id="r",
        )
        assert doc["status"] == "SERVING"
        assert doc["latency_ms"] == 12
        assert doc["schema_version"] == SCHEMA_VERSION

    def test_not_serving(self):
        doc = build_healthcheck_document(
            serving=False, latency_ms=999,
            endpoint="h:1", request_id="r",
        )
        assert doc["status"] == "NOT_SERVING"


class TestCompileDocument:
    def test_ok(self):
        classes = [
            SimpleNamespace(name="a", canonical_sentences=("x", "y")),
            SimpleNamespace(name="b", canonical_sentences=("z",)),
        ]
        cat = SimpleNamespace(
            name="c", version="1.0", embedder_fingerprint="fp",
            dim=4, classes=classes,
        )
        doc = build_compile_document(cat, request_id="r")
        assert doc["catalog"]["classes"][0] == {"name": "a", "sentences": 2}
        assert doc["catalog"]["dim"] == 4


class TestValidateDocument:
    def test_ok(self):
        doc = build_validate_document(
            catalog_path="x.yaml", ok=True,
            issues=(), warnings=(), request_id="r",
        )
        assert doc["valid"] is True
        assert doc["issues"] == []

    def test_with_issues(self):
        doc = build_validate_document(
            catalog_path="x.yaml", ok=False,
            issues=({"code": "X"},),
            warnings=(), request_id="r",
        )
        assert doc["valid"] is False
        assert doc["issues"][0]["code"] == "X"


class TestVersionDocument:
    def test_basic(self):
        doc = build_version_document("1.2.3")
        assert doc["version"] == "1.2.3"
        assert doc["program"] == "efe-admin"
        assert doc["schema_version"] == SCHEMA_VERSION


# ---------------------------------------------------------------------------
#  Rendering
# ---------------------------------------------------------------------------


class TestRenderJson:
    def test_compact_by_default(self):
        buf = io.StringIO()
        render({"a": 1, "b": 2}, stream=buf)
        # Compact JSON: no spaces after separators
        assert '"a":1' in buf.getvalue()
        assert buf.getvalue().endswith("\n")

    def test_pretty_indented(self):
        buf = io.StringIO()
        render({"a": 1}, pretty=True, stream=buf)
        assert "\n  " in buf.getvalue()

    def test_unicode(self):
        buf = io.StringIO()
        render({"msg": "Ça va ?"}, stream=buf)
        assert "Ça" in buf.getvalue()


class TestRenderText:
    def test_key_value_alignment(self):
        buf = io.StringIO()
        render({"short": 1, "much_longer_key": 2}, format="text", stream=buf)
        out = buf.getvalue()
        # The ``short`` line should be padded so colons align.
        lines = out.splitlines()
        assert any(line.startswith("short") for line in lines)


class TestRenderTable:
    def test_two_column(self):
        buf = io.StringIO()
        render({"a": 1, "b": 2}, format="table", stream=buf)
        out = buf.getvalue()
        assert "KEY" in out and "VALUE" in out
        assert "| a" in out


# ---------------------------------------------------------------------------
#  Error rendering
# ---------------------------------------------------------------------------


class TestRenderError:
    def test_text_format(self):
        buf = io.StringIO()
        render_error(UsageError("bad", hint="--help"),
                     format="text", stream=buf)
        out = buf.getvalue()
        assert "USAGE" in out
        assert "bad" in out
        assert "--help" in out

    def test_json_format(self):
        buf = io.StringIO()
        render_error(CatalogNotFoundError("nope", hint="check path"),
                     format="json", stream=buf, request_id="rid")
        parsed = json.loads(buf.getvalue())
        assert parsed["error"]["code"] == "CATALOG_NOT_FOUND"
        assert parsed["error"]["exit_code"] == 3
        assert parsed["error"]["hint"] == "check path"
        assert parsed["request_id"] == "rid"
        assert parsed["schema_version"] == SCHEMA_VERSION
