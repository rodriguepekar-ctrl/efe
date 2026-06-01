"""Kubernetes client abstraction.

Why a protocol: the orchestrator must *boot* without a cluster. Tests,
local dev, and CI use :class:`FakeKubernetesClient`, which records
every submitted manifest and lets tests assert on the stored dicts
without spinning up minikube.

Production wires :class:`RealKubernetesClient` (lazy import — needs
the ``kubernetes`` PyPI package, declared as an optional extra in
``pyproject.toml``). If the package isn't installed, the factory
raises a clear :class:`DispatcherError`.

Three verbs:

* ``submit(manifest) → JobHandle``
* ``delete(name, namespace) → None``
* ``status(name, namespace) → JobStatus``

A :class:`JobHandle` is the pair ``(name, namespace)``; anything else
(phase, conditions) is pulled via :meth:`status`.
"""

from __future__ import annotations

import asyncio
from collections.abc import Iterable
from dataclasses import dataclass, field
from typing import Any, Literal, Protocol

from efe_orch.config import K8sSettings
from efe_orch.errors import DispatcherError, WorkerJobRejected

JobPhase = Literal["Pending", "Running", "Succeeded", "Failed", "Unknown"]


@dataclass(frozen=True, slots=True)
class JobHandle:
    name: str
    namespace: str
    created: bool = True


@dataclass(frozen=True, slots=True)
class JobStatus:
    name: str
    namespace: str
    phase: JobPhase
    active: int = 0
    succeeded: int = 0
    failed: int = 0
    conditions: tuple[dict[str, Any], ...] = ()


class KubernetesClient(Protocol):
    async def submit(self, manifest: dict[str, Any]) -> JobHandle:  # pragma: no cover
        ...

    async def delete(self, name: str, namespace: str) -> None:  # pragma: no cover
        ...

    async def status(self, name: str, namespace: str) -> JobStatus:  # pragma: no cover
        ...

    async def list_jobs(
        self, namespace: str, *, label_selector: str | None = None,
    ) -> Iterable[JobStatus]:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
#  Fake (tests / dry-run)
# ---------------------------------------------------------------------------


@dataclass
class FakeKubernetesClient:
    """Deterministic in-memory stub.

    Stores every submitted manifest in ``jobs`` keyed by
    ``(namespace, name)``. ``status()`` returns whatever phase you set
    via :meth:`set_phase` (default: Pending). Supports:

    * idempotent submit (same name → noop + ``created=False``)
    * forced failure (``fail_on_submit=True``) to test the error path
    * explicit status injection for deterministic reaper tests
    """

    jobs: dict[tuple[str, str], dict[str, Any]] = field(default_factory=dict)
    phase: dict[tuple[str, str], JobPhase] = field(default_factory=dict)
    fail_on_submit: bool = False
    submitted: list[dict[str, Any]] = field(default_factory=list)
    deletions: list[tuple[str, str]] = field(default_factory=list)

    async def submit(self, manifest: dict[str, Any]) -> JobHandle:
        if self.fail_on_submit:
            raise WorkerJobRejected(
                "fake client configured to reject submissions",
                detail={"name": manifest["metadata"]["name"]},
            )
        name = manifest["metadata"]["name"]
        namespace = manifest["metadata"]["namespace"]
        key = (namespace, name)
        await asyncio.sleep(0)  # simulate IO yielding
        if key in self.jobs:
            return JobHandle(name=name, namespace=namespace, created=False)
        self.jobs[key] = manifest
        self.submitted.append(manifest)
        self.phase.setdefault(key, "Pending")
        return JobHandle(name=name, namespace=namespace, created=True)

    async def delete(self, name: str, namespace: str) -> None:
        key = (namespace, name)
        self.deletions.append(key)
        self.jobs.pop(key, None)
        self.phase.pop(key, None)

    async def status(self, name: str, namespace: str) -> JobStatus:
        key = (namespace, name)
        if key not in self.jobs:
            return JobStatus(name=name, namespace=namespace, phase="Unknown")
        phase = self.phase.get(key, "Pending")
        active = 1 if phase in ("Pending", "Running") else 0
        succeeded = 1 if phase == "Succeeded" else 0
        failed = 1 if phase == "Failed" else 0
        return JobStatus(
            name=name, namespace=namespace, phase=phase,
            active=active, succeeded=succeeded, failed=failed,
        )

    async def list_jobs(
        self, namespace: str, *, label_selector: str | None = None,
    ) -> Iterable[JobStatus]:
        out: list[JobStatus] = []
        for (ns, name), manifest in self.jobs.items():
            if ns != namespace:
                continue
            if label_selector:
                labels = manifest["metadata"].get("labels") or {}
                key, _, value = label_selector.partition("=")
                if labels.get(key) != value:
                    continue
            st = await self.status(name, ns)
            out.append(st)
        return tuple(out)

    def set_phase(self, name: str, namespace: str, phase: JobPhase) -> None:
        self.phase[(namespace, name)] = phase


# ---------------------------------------------------------------------------
#  Real client (lazy import)
# ---------------------------------------------------------------------------


class RealKubernetesClient:
    """Thin wrapper around the ``kubernetes-asyncio`` library.

    Fully functional but not imported until instantiated, so tests
    don't pull in the dep. If the package isn't installed, the
    constructor raises :class:`DispatcherError`.
    """

    def __init__(self, settings: K8sSettings) -> None:
        try:
            from kubernetes_asyncio import client, config
        except ImportError as exc:  # pragma: no cover — optional dep
            raise DispatcherError(
                "kubernetes-asyncio is required for RealKubernetesClient",
                detail={"install": "pip install kubernetes-asyncio"},
            ) from exc
        self._settings = settings
        self._client = client
        self._config = config
        self._api: Any | None = None

    async def _ensure_api(self) -> Any:
        if self._api is None:
            try:
                await self._config.load_incluster_config()
            except Exception:  # pragma: no cover — covered in int-test env
                await self._config.load_kube_config()
            self._api = self._client.BatchV1Api()
        return self._api

    async def submit(self, manifest: dict[str, Any]) -> JobHandle:
        api = await self._ensure_api()
        name = manifest["metadata"]["name"]
        namespace = manifest["metadata"]["namespace"]
        try:
            await api.create_namespaced_job(namespace, manifest)
            return JobHandle(name=name, namespace=namespace, created=True)
        except Exception as exc:  # pragma: no cover — network path
            raise WorkerJobRejected(
                f"k8s API refused job {name!r}: {exc}",
                detail={"name": name, "namespace": namespace},
            ) from exc

    async def delete(self, name: str, namespace: str) -> None:
        api = await self._ensure_api()
        await api.delete_namespaced_job(
            name, namespace,
            body=self._client.V1DeleteOptions(propagation_policy="Background"),
        )

    async def status(self, name: str, namespace: str) -> JobStatus:
        api = await self._ensure_api()
        job = await api.read_namespaced_job(name, namespace)
        status = getattr(job, "status", None)
        active = int(getattr(status, "active", 0) or 0)
        succeeded = int(getattr(status, "succeeded", 0) or 0)
        failed = int(getattr(status, "failed", 0) or 0)
        phase: JobPhase = "Pending"
        if succeeded > 0:
            phase = "Succeeded"
        elif failed > 0:
            phase = "Failed"
        elif active > 0:
            phase = "Running"
        return JobStatus(
            name=name, namespace=namespace, phase=phase,
            active=active, succeeded=succeeded, failed=failed,
        )

    async def list_jobs(
        self, namespace: str, *, label_selector: str | None = None,
    ) -> Iterable[JobStatus]:
        api = await self._ensure_api()
        res = await api.list_namespaced_job(
            namespace, label_selector=label_selector,
        )
        out: list[JobStatus] = []
        for job in getattr(res, "items", []):
            name = job.metadata.name
            out.append(await self.status(name, namespace))
        return tuple(out)


# ---------------------------------------------------------------------------
#  Factory
# ---------------------------------------------------------------------------


def build_client(
    settings: K8sSettings, *,
    fake: FakeKubernetesClient | None = None,
) -> KubernetesClient:
    """Return a client honouring ``settings.dry_run``.

    * If ``fake`` is passed, it always wins (tests).
    * If ``dry_run`` is True, a fresh :class:`FakeKubernetesClient` is
      returned — safe default for local dev.
    * Else a :class:`RealKubernetesClient` is instantiated.
    """
    if fake is not None:
        return fake
    if settings.dry_run:
        return FakeKubernetesClient()
    return RealKubernetesClient(settings)


__all__ = [
    "FakeKubernetesClient",
    "JobHandle",
    "JobPhase",
    "JobStatus",
    "KubernetesClient",
    "RealKubernetesClient",
    "build_client",
]
