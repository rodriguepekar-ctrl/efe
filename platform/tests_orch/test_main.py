"""Tests for the CLI entry point (:mod:`efe_orch.__main__`)."""

from __future__ import annotations

import json

import pytest

from efe_orch.__main__ import _parse_args, main


def test_parse_config() -> None:
    ns = _parse_args(["config"])
    assert ns.command == "config"


def test_parse_reap_once() -> None:
    ns = _parse_args(["reap", "--once"])
    assert ns.command == "reap"
    assert ns.once is True


def test_parse_missing_subcommand_errors() -> None:
    with pytest.raises(SystemExit):
        _parse_args([])


def test_config_command_prints_json(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("EFE_ORCH_ENVIRONMENT", "test")
    rc = main(["config"])
    assert rc == 0
    data = json.loads(capsys.readouterr().out)
    assert data["environment"] == "test"


def test_migrate_command_on_sqlite(
    tmp_path: object, capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = f"sqlite+aiosqlite:///{tmp_path}/orch.sqlite"  # type: ignore[operator]
    monkeypatch.setenv("EFE_ORCH_ENVIRONMENT", "test")
    rc = main(["--db-url", db, "migrate"])
    assert rc == 0
    out = capsys.readouterr().out
    assert "schema created" in out


def test_reap_once_command(
    tmp_path: object, capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = f"sqlite+aiosqlite:///{tmp_path}/orch2.sqlite"  # type: ignore[operator]
    monkeypatch.setenv("EFE_ORCH_ENVIRONMENT", "test")
    main(["--db-url", db, "migrate"])
    capsys.readouterr()  # swallow migrate output
    rc = main(["--db-url", db, "reap", "--once"])
    assert rc == 0
    out = capsys.readouterr().out
    assert "scanned=0" in out
