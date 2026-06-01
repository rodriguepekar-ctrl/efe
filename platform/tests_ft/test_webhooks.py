"""Tests for :class:`LabelingWebhookDispatcher`."""

from __future__ import annotations

import hashlib
import hmac
import json

import pytest

from efe_ft.labeling.webhooks import LabelingWebhookDispatcher, WebhookEvent


async def test_event_canonical_body_sorted() -> None:
    ev = WebhookEvent(
        event="label.submitted",
        tenant_id="acme",
        payload={"b": 2, "a": 1},
        event_id="evt_1",
    )
    body = ev.canonical_body()
    parsed = json.loads(body)
    # keys must be sorted
    assert list(parsed.keys()) == sorted(parsed.keys())
    assert parsed["payload"] == {"a": 1, "b": 2}


async def test_dispatcher_signs_and_posts() -> None:
    called: list[tuple[str, bytes, dict[str, str]]] = []

    async def fake_poster(url: str, body: bytes, headers: dict[str, str]) -> int:
        called.append((url, body, headers))
        return 200

    def endpoint_for(tenant: str) -> tuple[str, str] | None:
        if tenant == "acme":
            return ("https://example.com/hook", "secret-key")
        return None

    d = LabelingWebhookDispatcher(
        endpoint_for_tenant=endpoint_for, poster=fake_poster,
    )
    ok = await d.send(WebhookEvent(
        event="label.submitted", tenant_id="acme",
        payload={"sample_id": "s1"}, event_id="evt_1",
    ))
    assert ok
    assert len(called) == 1
    url, body, headers = called[0]
    assert url == "https://example.com/hook"
    # Signature present
    sig_header = headers["x-efe-ft-signature"]
    assert sig_header.startswith("sha256=")
    expected = hmac.new(b"secret-key", body, hashlib.sha256).hexdigest()
    assert sig_header == f"sha256={expected}"


async def test_unknown_tenant_skipped() -> None:
    async def poster(url: str, body: bytes, headers: dict[str, str]) -> int:
        raise AssertionError("poster should not be called for unknown tenant")

    d = LabelingWebhookDispatcher(
        endpoint_for_tenant=lambda t: None, poster=poster,
    )
    ok = await d.send(WebhookEvent(
        event="label.submitted", tenant_id="ghost",
        payload={}, event_id="evt_0",
    ))
    assert not ok


async def test_retry_then_success() -> None:
    calls = {"n": 0}

    async def poster(url: str, body: bytes, headers: dict[str, str]) -> int:
        calls["n"] += 1
        return 500 if calls["n"] < 3 else 200

    d = LabelingWebhookDispatcher(
        endpoint_for_tenant=lambda t: ("http://x", "k"),
        poster=poster, initial_backoff_s=0.001, max_retries=3,
    )
    ok = await d.send(WebhookEvent(
        event="label.submitted", tenant_id="acme",
        payload={}, event_id="evt_x",
    ))
    assert ok
    assert calls["n"] == 3


async def test_retry_exhausted_dead_letter() -> None:
    async def poster(url: str, body: bytes, headers: dict[str, str]) -> int:
        return 503

    d = LabelingWebhookDispatcher(
        endpoint_for_tenant=lambda t: ("http://x", "k"),
        poster=poster, initial_backoff_s=0.001, max_retries=2,
    )
    ok = await d.send(WebhookEvent(
        event="label.submitted", tenant_id="acme",
        payload={}, event_id="evt_y",
    ))
    assert not ok
    assert len(d.dead_letter) == 1
    _, reason = d.dead_letter[0]
    assert "503" in reason


async def test_poster_exception_retried() -> None:
    calls = {"n": 0}

    async def poster(url: str, body: bytes, headers: dict[str, str]) -> int:
        calls["n"] += 1
        if calls["n"] < 2:
            raise ConnectionError("boom")
        return 200

    d = LabelingWebhookDispatcher(
        endpoint_for_tenant=lambda t: ("http://x", "k"),
        poster=poster, initial_backoff_s=0.001,
    )
    ok = await d.send(WebhookEvent(
        event="label.submitted", tenant_id="acme",
        payload={}, event_id="evt_z",
    ))
    assert ok
    assert calls["n"] == 2
