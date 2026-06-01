"""Render the Kubernetes Job manifest for a single worker.

Pure function — no I/O. Callers hand it a :class:`WorkerJobSpec` and
receive a dict that matches the ``batch/v1 Job`` schema. The client
layer (:mod:`efe_orch.dispatcher.client`) is responsible for actually
POSTing it to the API server.

What's baked into the manifest:

* **SecurityContext** — non-root, read-only rootfs, drop all caps.
* **NetworkPolicy** — marked via the ``efe.io/network-policy`` label
  so the NP controller targets the Job's pods.
* **ServiceAccount** — the orchestrator-owned SA that M06 binds to
  the result-artifact bucket.
* **Env**: OTEL endpoint, tenant scope, run id, snapshot id, worker
  id, orchestrator callback URL. The worker picks these up on boot.
* **Resource requests/limits** — pinned by the chosen
  :class:`PoolSettings`. Requests == limits for GPU pools (no CPU
  burst), requests < limits for CPU pools (25% headroom).

The output is a plain dict so tests can assert on the shape without
depending on the Kubernetes client library.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from efe_orch.config import K8sSettings, PoolSettings


@dataclass(frozen=True, slots=True)
class WorkerJobSpec:
    """Everything the renderer needs to produce a manifest."""

    run_id: str
    tenant_id: str
    task_id: str
    snapshot_id: str
    worker_id: str
    pool: PoolSettings
    base_model: str
    recipe_version: str
    hyperparameters: dict[str, Any] = field(default_factory=dict)
    resume_from_checkpoint: str | None = None
    callback_url: str = "http://efe-orchestrator.efe-ft.svc.cluster.local:8080"
    image: str | None = None  # override; else uses K8sSettings.image
    extra_env: dict[str, str] = field(default_factory=dict)


def _job_name(run_id: str, attempt: int | None = None) -> str:
    short = run_id.replace("-", "")[:16]
    suffix = f"-a{attempt}" if attempt is not None else ""
    return f"efe-ft-{short}{suffix}"


def render_worker_job(
    spec: WorkerJobSpec,
    k8s: K8sSettings,
    *,
    attempt: int | None = None,
) -> dict[str, Any]:
    """Return a ``batch/v1 Job`` dict for ``spec``.

    The manifest is deliberately verbose — every field is explicit so
    review is obvious and there is no reliance on server-side defaults.
    """
    pool = spec.pool
    image = spec.image or k8s.image

    base_env = [
        {"name": "EFE_RUN_ID", "value": spec.run_id},
        {"name": "EFE_TENANT_ID", "value": spec.tenant_id},
        {"name": "EFE_TASK_ID", "value": spec.task_id},
        {"name": "EFE_SNAPSHOT_ID", "value": spec.snapshot_id},
        {"name": "EFE_WORKER_ID", "value": spec.worker_id},
        {"name": "EFE_BASE_MODEL", "value": spec.base_model},
        {"name": "EFE_RECIPE_VERSION", "value": spec.recipe_version},
        {"name": "EFE_POOL_NAME", "value": pool.name},
        {"name": "EFE_CALLBACK_URL", "value": spec.callback_url},
        {"name": "OTEL_SERVICE_NAME", "value": "efe-worker"},
        {
            "name": "OTEL_EXPORTER_OTLP_ENDPOINT",
            "value": "http://otel-collector.observability:4317",
        },
    ]
    if spec.resume_from_checkpoint:
        base_env.append({
            "name": "EFE_RESUME_CHECKPOINT",
            "value": spec.resume_from_checkpoint,
        })
    for k, v in sorted(spec.extra_env.items()):
        base_env.append({"name": k, "value": v})

    resources: dict[str, Any] = {
        "requests": {
            "cpu": f"{pool.cpu_request}",
            "memory": f"{pool.memory_gib:.1f}Gi",
        },
        "limits": {
            "cpu": f"{pool.cpu_request * 1.25:.2f}" if pool.gpu is None
                   else f"{pool.cpu_request}",
            "memory": f"{pool.memory_gib:.1f}Gi",
        },
    }
    if pool.gpu is not None and pool.gpu_count > 0:
        resources["requests"][pool.gpu] = str(pool.gpu_count)
        resources["limits"][pool.gpu] = str(pool.gpu_count)

    labels = {
        "app.kubernetes.io/name": "efe-worker",
        "app.kubernetes.io/component": "finetune",
        "efe.io/tenant-id": spec.tenant_id,
        "efe.io/run-id-short": spec.run_id.replace("-", "")[:16],
        "efe.io/pool": pool.name,
        "efe.io/network-policy": k8s.network_policy,
        "efe.io/worker-id": spec.worker_id,
    }
    annotations = {
        "efe.io/run-id": spec.run_id,
        "efe.io/snapshot-id": spec.snapshot_id,
        "efe.io/recipe-version": spec.recipe_version,
    }

    manifest: dict[str, Any] = {
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": _job_name(spec.run_id, attempt),
            "namespace": k8s.namespace,
            "labels": labels,
            "annotations": annotations,
        },
        "spec": {
            "backoffLimit": k8s.backoff_limit,
            "activeDeadlineSeconds": min(
                k8s.active_deadline_seconds, pool.max_runtime_s,
            ),
            "ttlSecondsAfterFinished": k8s.ttl_seconds_after_finished,
            "parallelism": 1,
            "completions": 1,
            "template": {
                "metadata": {
                    "labels": labels,
                    "annotations": annotations,
                },
                "spec": {
                    "serviceAccountName": k8s.service_account,
                    "restartPolicy": "Never",
                    "securityContext": {
                        "runAsNonRoot": True,
                        "runAsUser": 1000,
                        "runAsGroup": 1000,
                        "fsGroup": 1000,
                        "seccompProfile": {"type": "RuntimeDefault"},
                    },
                    "containers": [
                        {
                            "name": "worker",
                            "image": image,
                            "imagePullPolicy": k8s.image_pull_policy,
                            "command": ["python", "-m", "efe_worker.main"],
                            "env": base_env,
                            "resources": resources,
                            "securityContext": {
                                "allowPrivilegeEscalation": False,
                                "readOnlyRootFilesystem": True,
                                "capabilities": {"drop": ["ALL"]},
                            },
                            "volumeMounts": [
                                {
                                    "name": "scratch",
                                    "mountPath": "/var/efe/scratch",
                                },
                                {
                                    "name": "tmp",
                                    "mountPath": "/tmp",
                                },
                            ],
                        }
                    ],
                    "volumes": [
                        {"name": "scratch", "emptyDir": {"sizeLimit": "20Gi"}},
                        {"name": "tmp", "emptyDir": {"sizeLimit": "2Gi"}},
                    ],
                },
            },
        },
    }
    return manifest


__all__ = ["WorkerJobSpec", "render_worker_job"]
