"""Tests for the CLI entrypoint."""

from __future__ import annotations

import json

import pytest

from efe_gate.__main__ import _parse_args, main


def test_parse_migrate() -> None:
    ns = _parse_args(["migrate"])
    assert ns.command == "migrate"


def test_parse_missing_subcommand() -> None:
    with pytest.raises(SystemExit):
        _parse_args([])


def test_config_command(
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("EFE_GATE_ENVIRONMENT", "test")
    rc = main(["config"])
    assert rc == 0
    data = json.loads(capsys.readouterr().out)
    assert data["environment"] == "test"


def test_metrics_catalog_command(
    capsys: pytest.CaptureFixture[str],
) -> None:
    rc = main(["metrics-catalog"])
    assert rc == 0
    rows = json.loads(capsys.readouterr().out)
    names = [r["name"] for r in rows]
    assert "accuracy" in names


def test_migrate_command(
    tmp_path: object,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = f"sqlite+aiosqlite:///{tmp_path}/gate.sqlite"  # type: ignore[operator]
    monkeypatch.setenv("EFE_GATE_ENVIRONMENT", "test")
    rc = main(["--db-url", db, "migrate"])
    assert rc == 0
    out = capsys.readouterr().out
    assert "schema created" in out
