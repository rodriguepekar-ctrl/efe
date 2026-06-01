"""Kubernetes dispatcher — render worker Job manifests + submit them.

Public surface:

    WorkerJobSpec     — immutable input to the renderer
    render_worker_job — pure function → dict (ready for the K8s API)
    JobHandle         — result returned by ``submit()``
    KubernetesClient  — protocol (``submit``, ``delete``, ``status``)
    FakeKubernetesClient — in-memory fake for tests
    build_client      — factory honouring ``K8sSettings.dry_run``
"""

from __future__ import annotations

from efe_orch.dispatcher.client import (
    FakeKubernetesClient,
    JobHandle,
    JobStatus,
    KubernetesClient,
    build_client,
)
from efe_orch.dispatcher.worker_job import (
    WorkerJobSpec,
    render_worker_job,
)

__all__ = [
    "FakeKubernetesClient",
    "JobHandle",
    "JobStatus",
    "KubernetesClient",
    "WorkerJobSpec",
    "build_client",
    "render_worker_job",
]
