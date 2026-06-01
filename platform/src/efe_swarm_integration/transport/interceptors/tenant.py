"""TenantInterceptor — injects metadata headers from the SecurityContext."""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["TenantInterceptor", "build_tenant_metadata"]


def build_tenant_metadata(
    *,
    tenant_id: str,
    task_id: str | None = None,
    request_id: str | None = None,
    idempotency_key: str | None = None,
    actor_id: str | None = None,
) -> tuple[tuple[str, str], ...]:
    md: list[tuple[str, str]] = [("x-efe-tenant-id", tenant_id)]
    if task_id:
        md.append(("x-efe-task-id", task_id))
    if request_id:
        md.append(("x-efe-request-id", request_id))
    if idempotency_key:
        md.append(("x-efe-idempotency-key", idempotency_key))
    if actor_id:
        md.append(("x-efe-actor-id", actor_id))
    return tuple(md)


@dataclass(slots=True)
class TenantInterceptor:
    """Pure metadata shaper ; callable."""

    def apply(
        self,
        *,
        metadata: tuple[tuple[str, str], ...],
        tenant_id: str,
        task_id: str | None = None,
        request_id: str | None = None,
        idempotency_key: str | None = None,
        actor_id: str | None = None,
    ) -> tuple[tuple[str, str], ...]:
        base = build_tenant_metadata(
            tenant_id=tenant_id,
            task_id=task_id,
            request_id=request_id,
            idempotency_key=idempotency_key,
            actor_id=actor_id,
        )
        # dedupe : existing metadata prefix wins.
        existing_keys = {k for (k, _) in metadata}
        return tuple(
            list(metadata)
            + [kv for kv in base if kv[0] not in existing_keys]
        )
