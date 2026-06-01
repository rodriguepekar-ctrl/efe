"""Tests for :class:`AuditLog`."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from efe_platform.errors import AuditError
from efe_platform.telemetry.audit import AUDIT_SCHEMA_VERSION, AuditEntry, AuditLog


def test_append_and_replay(tmp_path: Path) -> None:
    log = AuditLog(tmp_path / "a.jsonl", fsync_each=False)
    log.append(
        AuditEntry.now(
            request_id="r1", tenant="t", kind="bot_reply", payload={"ok": True}
        )
    )
    log.append(
        AuditEntry.now(
            request_id="r2", tenant="t", kind="error", payload={"msg": "boom"}
        )
    )
    log.close()
    replayed = AuditLog(tmp_path / "a.jsonl", fsync_each=False).replay()
    assert len(replayed) == 2
    assert replayed[0]["request_id"] == "r1"
    assert replayed[1]["kind"] == "error"
    # Schema version is present everywhere.
    for line in replayed:
        assert line["schema_version"] == AUDIT_SCHEMA_VERSION


def test_record_bot_reply_helper(tmp_path: Path) -> None:
    log = AuditLog(tmp_path / "b.jsonl", fsync_each=False)
    log.record_bot_reply(
        request_id="r",
        tenant="acme",
        reply_payload={"final_text": "hi", "ok": True},
    )
    log.close()
    entries = AuditLog(tmp_path / "b.jsonl", fsync_each=False).replay()
    assert entries[0]["kind"] == "bot_reply"
    assert entries[0]["tenant"] == "acme"


def test_redact_prompts(tmp_path: Path) -> None:
    log = AuditLog(tmp_path / "c.jsonl", fsync_each=False, redact_prompts=True)
    log.record_bot_reply(
        request_id="r",
        tenant="t",
        reply_payload={"final_text": "secret-answer", "raw_prompt": "secret-input"},
    )
    log.close()
    entries = AuditLog(tmp_path / "c.jsonl", fsync_each=False).replay()
    payload = entries[0]["payload"]
    assert payload["final_text"].startswith("sha256:")
    assert payload["raw_prompt"].startswith("sha256:")
    assert "secret-answer" not in json.dumps(entries)
    assert "secret-input" not in json.dumps(entries)


def test_malformed_line_detected(tmp_path: Path) -> None:
    p = tmp_path / "corrupt.jsonl"
    p.write_text('{"ok":true}\nnot-json\n', encoding="utf-8")
    with pytest.raises(AuditError, match="malformed audit line 2"):
        AuditLog(p, fsync_each=False).replay()


def test_audit_entry_to_jsonl_is_single_line() -> None:
    e = AuditEntry.now(
        request_id="r", tenant="t", kind="error", payload={"msg": "x\ny"}
    )
    line = e.to_jsonl()
    assert "\n" not in line  # newlines inside content are escaped


def test_close_is_idempotent(tmp_path: Path) -> None:
    log = AuditLog(tmp_path / "d.jsonl", fsync_each=False)
    log.close()
    log.close()  # must not raise
