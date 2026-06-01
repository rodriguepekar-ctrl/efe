"""Tests for :mod:`efe_tool.cli.logging`."""

from __future__ import annotations

import io
import json

from efe_tool.cli.logging import Logger, setup_logging


class _Cfg:
    """Minimal stand-in for ResolvedConfig."""
    def __init__(self, level="info", format="text", no_color=True, rid="rid"):
        self.log_level = level
        self.log_format = format
        self.no_color = no_color
        self.request_id = rid


class TestLevelFiltering:
    def test_info_filters_debug(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(level="info"), stream=buf)
        log.debug("silent")
        assert buf.getvalue() == ""

    def test_debug_emits(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(level="debug"), stream=buf)
        log.debug("spoken", k="v")
        assert "spoken" in buf.getvalue()

    def test_error_only(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(level="error"), stream=buf)
        log.info("no"); log.warn("no"); log.error("yes")
        assert buf.getvalue().count("\n") == 1
        assert "yes" in buf.getvalue()


class TestJsonFormat:
    def test_json_emits_one_object_per_line(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(format="json"), stream=buf)
        log.info("evt", k=1, s="hi")
        line = buf.getvalue().rstrip("\n")
        parsed = json.loads(line)
        assert parsed["evt"] == "evt"
        assert parsed["lvl"] == "info"
        assert parsed["k"] == 1
        assert parsed["s"] == "hi"
        assert "ts" in parsed
        assert parsed["request_id"] == "rid"

    def test_json_handles_non_serialisable(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(format="json"), stream=buf)

        class X:
            def __str__(self): return "<X>"
        log.info("e", x=X())
        line = buf.getvalue().rstrip("\n")
        assert "<X>" in line


class TestTextFormat:
    def test_text_has_evt_and_fields(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(format="text"), stream=buf)
        log.info("evt", key="val", n=42)
        out = buf.getvalue()
        assert "evt" in out
        assert "key=val" in out
        assert "n=42" in out
        assert "INFO" in out


class TestRequestIdInjection:
    def test_request_id_auto_attached(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(format="json", rid="abc-123"), stream=buf)
        log.info("e")
        parsed = json.loads(buf.getvalue())
        assert parsed["request_id"] == "abc-123"

    def test_request_id_override_per_log(self):
        buf = io.StringIO()
        log = setup_logging(_Cfg(format="json", rid="default"), stream=buf)
        log.info("e", request_id="other")
        parsed = json.loads(buf.getvalue())
        assert parsed["request_id"] == "other"
