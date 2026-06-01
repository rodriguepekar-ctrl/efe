"""Shared fixtures for tests_worker."""

from __future__ import annotations

from pathlib import Path

import pytest

from efe_worker.clock import FakeClock
from efe_worker.config import RunIdentity, WorkerSettings
from efe_worker.context import RunContext
from efe_worker.data.tokenizer import FakeTokenizer
from efe_worker.transport.grpc_client import FakeOrchestratorClient
from efe_worker.transport.s3 import FakeBlobStore


@pytest.fixture
def fake_clock() -> FakeClock:
    return FakeClock()


@pytest.fixture
def fake_tokenizer() -> FakeTokenizer:
    return FakeTokenizer(vocab_size=256, max_seq_len=32)


@pytest.fixture
def fake_transport() -> FakeOrchestratorClient:
    return FakeOrchestratorClient()


@pytest.fixture
def fake_blob_store() -> FakeBlobStore:
    return FakeBlobStore()


@pytest.fixture
def run_identity() -> RunIdentity:
    return RunIdentity(
        run_id="11111111-2222-3333-4444-555555555555",
        tenant_id="acme",
        task_id="routing",
        snapshot_id="sn-1",
        worker_id="worker-alpha",
        snapshot_uri="file:///snap/sn-1/manifest.json",
        base_model="sentence-transformers/all-MiniLM-L6-v2",
        recipe_version="v1",
        pool_name="cpu-small",
    )


@pytest.fixture
def worker_settings(
    run_identity: RunIdentity, tmp_path: Path,
) -> WorkerSettings:
    return WorkerSettings.from_env(overrides={
        "environment": "test",
        "method_name": "classif",
        "identity": run_identity.model_dump(),
        "transport": {"fake_transport": True},
        "checkpoint": {
            "local_dir": str(tmp_path / "ckpts"),
            "remote_uri_prefix": "file:///remote/ckpts",
            "every_n_steps": 2,
            "keep_local": 2,
        },
        "training": {
            "max_steps": 4, "batch_size": 4,
            "eval_every_n_steps": 2,
        },
    })


@pytest.fixture
def run_context(
    worker_settings: WorkerSettings,
    fake_transport: FakeOrchestratorClient,
    fake_blob_store: FakeBlobStore,
    fake_clock: FakeClock,
) -> RunContext:
    return RunContext(
        settings=worker_settings,
        clock=fake_clock,
        transport=fake_transport,
        blob_store=fake_blob_store,
    )
