"""Tests for the dispatcher: manifest render + fake client."""

from __future__ import annotations

import pytest

from efe_orch.config import K8sSettings, PoolSettings
from efe_orch.dispatcher import (
    FakeKubernetesClient,
    WorkerJobSpec,
    build_client,
    render_worker_job,
)
from efe_orch.errors import WorkerJobRejected


def _pool_cpu() -> PoolSettings:
    return PoolSettings(
        name="cpu-small",
        cpu_request=1.0,
        memory_gib=2.0,
        max_concurrent_runs=4,
    )


def _pool_gpu() -> PoolSettings:
    return PoolSettings(
        name="gpu-a10",
        cpu_request=4.0,
        memory_gib=32.0,
        gpu="nvidia.com/gpu",
        gpu_count=1,
        max_concurrent_runs=2,
    )


def _spec(pool: PoolSettings, **overrides: object) -> WorkerJobSpec:
    base: dict[str, object] = dict(
        run_id="11111111-2222-3333-4444-555555555555",
        tenant_id="acme",
        task_id="routing",
        snapshot_id="99999999-9999-9999-9999-999999999999",
        worker_id="worker-alpha",
        pool=pool,
        base_model="my-base",
        recipe_version="v1",
    )
    base.update(overrides)
    return WorkerJobSpec(**base)  # type: ignore[arg-type]


def test_render_cpu_manifest_shape() -> None:
    k = K8sSettings(namespace="efe-ft")
    m = render_worker_job(_spec(_pool_cpu()), k, attempt=1)
    assert m["apiVersion"] == "batch/v1"
    assert m["kind"] == "Job"
    assert m["metadata"]["namespace"] == "efe-ft"
    assert m["metadata"]["name"].startswith("efe-ft-")
    assert m["metadata"]["name"].endswith("-a1")
    tpl_spec = m["spec"]["template"]["spec"]
    assert tpl_spec["restartPolicy"] == "Never"
    sc = tpl_spec["securityContext"]
    assert sc["runAsNonRoot"] is True
    assert sc["runAsUser"] == 1000
    env = {e["name"]: e["value"] for e in tpl_spec["containers"][0]["env"]}
    assert env["EFE_RUN_ID"] == "11111111-2222-3333-4444-555555555555"
    assert env["EFE_TENANT_ID"] == "acme"
    assert env["EFE_POOL_NAME"] == "cpu-small"
    resources = tpl_spec["containers"][0]["resources"]
    assert "nvidia.com/gpu" not in resources["requests"]
    assert resources["requests"]["cpu"] == "1.0"


def test_render_gpu_manifest_has_gpu_resource() -> None:
    k = K8sSettings()
    m = render_worker_job(_spec(_pool_gpu()), k)
    resources = (
        m["spec"]["template"]["spec"]["containers"][0]["resources"]
    )
    assert resources["requests"]["nvidia.com/gpu"] == "1"
    assert resources["limits"]["nvidia.com/gpu"] == "1"


def test_render_respects_extra_env_and_checkpoint() -> None:
    k = K8sSettings()
    spec = _spec(
        _pool_cpu(),
        resume_from_checkpoint="s3://bucket/ckpt/step100",
        extra_env={"MY_KEY": "v", "A": "b"},
    )
    m = render_worker_job(spec, k)
    env = {e["name"]: e["value"] for e in
           m["spec"]["template"]["spec"]["containers"][0]["env"]}
    assert env["EFE_RESUME_CHECKPOINT"] == "s3://bucket/ckpt/step100"
    assert env["MY_KEY"] == "v"
    assert env["A"] == "b"


def test_render_active_deadline_is_min_of_k8s_and_pool() -> None:
    k = K8sSettings(active_deadline_seconds=60)
    m = render_worker_job(_spec(_pool_cpu()), k)
    # pool defaults to max_runtime_s=10_800, k8s says 60 → min = 60.
    assert m["spec"]["activeDeadlineSeconds"] == 60


def test_render_ttl_pass_through() -> None:
    k = K8sSettings(ttl_seconds_after_finished=1234)
    m = render_worker_job(_spec(_pool_cpu()), k)
    assert m["spec"]["ttlSecondsAfterFinished"] == 1234


async def test_fake_client_submit_store_manifest() -> None:
    client = FakeKubernetesClient()
    m = render_worker_job(_spec(_pool_cpu()), K8sSettings())
    handle = await client.submit(m)
    assert handle.created is True
    assert handle.name == m["metadata"]["name"]
    assert len(client.submitted) == 1


async def test_fake_client_idempotent_submit() -> None:
    client = FakeKubernetesClient()
    m = render_worker_job(_spec(_pool_cpu()), K8sSettings())
    h1 = await client.submit(m)
    h2 = await client.submit(m)
    assert h1.created is True
    assert h2.created is False


async def test_fake_client_rejection() -> None:
    client = FakeKubernetesClient(fail_on_submit=True)
    m = render_worker_job(_spec(_pool_cpu()), K8sSettings())
    with pytest.raises(WorkerJobRejected):
        await client.submit(m)


async def test_fake_client_phase_control() -> None:
    client = FakeKubernetesClient()
    m = render_worker_job(_spec(_pool_cpu()), K8sSettings(namespace="ns1"))
    await client.submit(m)
    name = m["metadata"]["name"]
    st = await client.status(name, "ns1")
    assert st.phase == "Pending"
    client.set_phase(name, "ns1", "Running")
    st = await client.status(name, "ns1")
    assert st.phase == "Running"
    assert st.active == 1


async def test_fake_client_delete() -> None:
    client = FakeKubernetesClient()
    m = render_worker_job(_spec(_pool_cpu()), K8sSettings())
    await client.submit(m)
    name = m["metadata"]["name"]
    await client.delete(name, m["metadata"]["namespace"])
    st = await client.status(name, m["metadata"]["namespace"])
    assert st.phase == "Unknown"


def test_build_client_dry_run() -> None:
    c = build_client(K8sSettings(dry_run=True))
    assert isinstance(c, FakeKubernetesClient)


def test_build_client_uses_provided_fake() -> None:
    fake = FakeKubernetesClient()
    c = build_client(K8sSettings(dry_run=False), fake=fake)
    assert c is fake
