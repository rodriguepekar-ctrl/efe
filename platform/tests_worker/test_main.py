"""Tests for :mod:`efe_worker.__main__` CLI."""

from __future__ import annotations

import json

import pytest

from efe_worker.__main__ import _parse_args, main


def test_parse_run() -> None:
    ns = _parse_args(["run"])
    assert ns.command == "run"


def test_parse_missing_subcommand() -> None:
    with pytest.raises(SystemExit):
        _parse_args([])


def test_list_methods(capsys: pytest.CaptureFixture[str]) -> None:
    rc = main(["list-methods"])
    assert rc == 0
    out = capsys.readouterr().out
    for name in ("infonce", "triplet", "classif", "lora", "distill"):
        assert name in out


def test_describe_known_method(
    capsys: pytest.CaptureFixture[str],
) -> None:
    rc = main(["describe-method", "classif"])
    assert rc == 0
    parsed = json.loads(capsys.readouterr().out)
    assert parsed["name"] == "classif"


def test_describe_unknown_method(
    capsys: pytest.CaptureFixture[str],
) -> None:
    rc = main(["describe-method", "galaxy"])
    assert rc == 1


def test_config_dumps_valid_json(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    for k, v in [
        ("EFE_RUN_ID", "r"), ("EFE_TENANT_ID", "t"),
        ("EFE_TASK_ID", "tt"), ("EFE_SNAPSHOT_ID", "s"),
        ("EFE_WORKER_ID", "w"), ("EFE_ENVIRONMENT", "test"),
    ]:
        monkeypatch.setenv(k, v)
    rc = main(["config"])
    assert rc == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["environment"] == "test"
    assert payload["identity"]["run_id"] == "r"
