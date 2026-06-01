"""Tests for :class:`FakeEfeFacade` and :func:`make_verdict`."""

from __future__ import annotations

import asyncio

import pytest

from efe_tool.integrations import (
    EfeFacadeClosedError,
    EfeFacadeRejectedError,
    EfeFacadeUsageError,
    Verdict,
)
from efe_tool.integrations.testing import FakeEfeFacade, make_verdict


class TestMakeVerdict:
    def test_default(self):
        v = make_verdict()
        assert isinstance(v, Verdict)
        assert v.label == "alpha"
        assert v.zone == "trusted"

    def test_zone_inferred_warn(self):
        v = make_verdict(confidence=0.6)
        assert v.zone == "warn"

    def test_zone_reject(self):
        v = make_verdict(reject=True)
        assert v.zone == "reject"


class TestFakeLifecycle:
    def test_preload_idempotent(self):
        f = FakeEfeFacade()
        f.preload()
        f.preload()
        assert f.is_ready

    def test_close_marks_not_ready(self):
        f = FakeEfeFacade()
        f.close()
        assert f.is_closed
        assert not f.is_ready

    def test_classify_after_close_raises(self):
        f = FakeEfeFacade()
        f.close()
        with pytest.raises(EfeFacadeClosedError):
            f.classify("x", catalog="a")

    def test_healthcheck_shapes(self):
        f = FakeEfeFacade()
        assert f.healthcheck().status == "SERVING"
        f.close()
        assert f.healthcheck().status == "NOT_SERVING"


class TestFakeClassify:
    def test_default_verdict(self):
        f = FakeEfeFacade.with_verdict(label="fraud", confidence=0.95)
        v = f.classify("hi", catalog="support_triage")
        assert v.label == "fraud"
        assert v.catalog == "support_triage"

    def test_records_calls(self):
        f = FakeEfeFacade.with_verdict()
        f.classify("a", catalog="c1")
        f.classify("b", catalog="c2", request_id="rid-1")
        calls = f.classify_calls
        assert len(calls) == 2
        assert calls[0]["text"] == "a"
        assert calls[1]["request_id"] == "rid-1"

    def test_scripted_responses(self):
        from collections import deque
        f = FakeEfeFacade.with_responses({
            "c": [
                make_verdict(label="fraud"),
                make_verdict(label="bug"),
            ],
        })
        assert f.classify("x", catalog="c").label == "fraud"
        assert f.classify("x", catalog="c").label == "bug"
        # Queue exhausted → fall back to default verdict.
        assert f.classify("x", catalog="c").label == "alpha"

    def test_empty_text_raises(self):
        f = FakeEfeFacade()
        with pytest.raises(EfeFacadeUsageError):
            f.classify("  ", catalog="c")

    def test_raise_on_classify(self):
        from efe_tool.integrations import EfeFacadeTimeoutError
        f = FakeEfeFacade(raise_on_classify=EfeFacadeTimeoutError("x"))
        with pytest.raises(EfeFacadeTimeoutError):
            f.classify("a", catalog="c")

    def test_strict_reject(self):
        f = FakeEfeFacade(
            default_verdict=make_verdict(
                confidence=0.1, reject=True, zone="reject",
            ),
        )
        with pytest.raises(EfeFacadeRejectedError):
            f.classify("x", catalog="c", strict=True)


class TestFakeAsync:
    def test_classify_async(self):
        f = FakeEfeFacade.with_verdict(label="bug")
        async def _run():
            return await f.classify_async("x", catalog="c")
        v = asyncio.run(_run())
        assert v.label == "bug"
