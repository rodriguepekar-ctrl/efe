"""Tests for :mod:`efe_worker.config`."""

from __future__ import annotations

import pytest

from efe_worker.config import WorkerSettings
from efe_worker.errors import ConfigError


def test_defaults() -> None:
    cfg = WorkerSettings.from_env(overrides={
        "identity": {
            "run_id": "r", "tenant_id": "t", "task_id": "tt",
            "snapshot_id": "s", "worker_id": "w",
        },
    })
    assert cfg.environment == "dev"
    assert cfg.method_name == "classif"
    assert cfg.transport.heartbeat_interval_s == 30.0


def test_identity_required() -> None:
    with pytest.raises(ConfigError):
        WorkerSettings.from_env(overrides={
            "identity": {"run_id": ""},
        })


def test_env_parsing(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_RUN_ID", "r1")
    monkeypatch.setenv("EFE_TENANT_ID", "acme")
    monkeypatch.setenv("EFE_TASK_ID", "routing")
    monkeypatch.setenv("EFE_SNAPSHOT_ID", "sn1")
    monkeypatch.setenv("EFE_WORKER_ID", "w1")
    monkeypatch.setenv("EFE_METHOD_NAME", "triplet")
    monkeypatch.setenv("EFE_CALLBACK_URL", "http://orch:9000")
    monkeypatch.setenv("EFE_HEARTBEAT_INTERVAL_S", "15")
    monkeypatch.setenv(
        "EFE_HYPERPARAMETERS_JSON", '{"margin": 0.3}',
    )
    cfg = WorkerSettings.from_env()
    assert cfg.method_name == "triplet"
    assert cfg.identity.run_id == "r1"
    assert cfg.transport.callback_url == "http://orch:9000"
    assert cfg.transport.heartbeat_interval_s == 15.0
    assert cfg.hyperparameters == {"margin": 0.3}


def test_malformed_hyperparams_json(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    for k, v in [
        ("EFE_RUN_ID", "r"), ("EFE_TENANT_ID", "t"),
        ("EFE_TASK_ID", "tt"), ("EFE_SNAPSHOT_ID", "s"),
        ("EFE_WORKER_ID", "w"),
    ]:
        monkeypatch.setenv(k, v)
    monkeypatch.setenv("EFE_HYPERPARAMETERS_JSON", "not json!")
    with pytest.raises(ConfigError):
        WorkerSettings.from_env()


def test_frozen() -> None:
    cfg = WorkerSettings.from_env(overrides={
        "identity": {
            "run_id": "r", "tenant_id": "t", "task_id": "tt",
            "snapshot_id": "s", "worker_id": "w",
        },
    })
    with pytest.raises(Exception):
        cfg.environment = "prod"  # type: ignore[misc]


def test_extra_forbid() -> None:
    with pytest.raises(ConfigError):
        WorkerSettings.from_env(overrides={
            "identity": {
                "run_id": "r", "tenant_id": "t", "task_id": "tt",
                "snapshot_id": "s", "worker_id": "w",
            },
            "zz_unknown": 1,
        })


def test_method_name_enforced() -> None:
    with pytest.raises(ConfigError):
        WorkerSettings.from_env(overrides={
            "identity": {
                "run_id": "r", "tenant_id": "t", "task_id": "tt",
                "snapshot_id": "s", "worker_id": "w",
            },
            "method_name": "not_a_method",
        })
