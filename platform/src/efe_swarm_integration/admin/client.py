"""AdminClient — façade over ``efe.v1.admin.AdminService`` via :class:`RpcDriver`."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_swarm_integration.contract.adapter_admin import (
    admin_evolve_op_to_dict,
    admin_register_cone_to_dict,
    parse_cone_handle,
)
from efe_swarm_integration.errors import UnsupportedRpc
from efe_swarm_integration.transport.rpc_driver import RpcDriver
from efe_swarm_integration.types import (
    ConeEvolutionOp,
    ConeHandle,
    ConeSpec,
)

__all__ = ["AdminClient"]


@dataclass(slots=True)
class AdminClient:
    driver: RpcDriver

    async def register_cone(
        self,
        spec: ConeSpec,
        *,
        tenant_id: str,
        actor_id: str,
        deadline_s: float = 5.0,
    ) -> ConeHandle:
        payload = admin_register_cone_to_dict(spec)
        metadata = (
            ("x-efe-tenant-id", tenant_id),
            ("x-efe-actor-id", actor_id),
        )
        resp = await self.driver.unary(
            method="/efe.v1.admin.AdminService/RegisterCone",
            payload=payload,
            metadata=metadata,
            deadline_s=deadline_s,
        )
        return parse_cone_handle(resp.payload)

    async def evolve_cone(
        self,
        *,
        from_key: str,
        op: ConeEvolutionOp,
        tenant_id: str,
        actor_id: str,
        deadline_s: float = 5.0,
    ) -> ConeHandle:
        payload = {
            "from_key": from_key,
            "op": admin_evolve_op_to_dict(op),
        }
        metadata = (
            ("x-efe-tenant-id", tenant_id),
            ("x-efe-actor-id", actor_id),
        )
        resp = await self.driver.unary(
            method="/efe.v1.admin.AdminService/EvolveCone",
            payload=payload,
            metadata=metadata,
            deadline_s=deadline_s,
        )
        return parse_cone_handle(resp.payload)

    async def get_cone(
        self,
        *,
        key: str,
        tenant_id: str,
        deadline_s: float = 5.0,
    ) -> dict[str, Any]:
        resp = await self.driver.unary(
            method="/efe.v1.admin.AdminService/GetCone",
            payload={"key": key},
            metadata=(("x-efe-tenant-id", tenant_id),),
            deadline_s=deadline_s,
        )
        return resp.payload

    async def list_versions(
        self,
        *,
        tenant: str,
        task: str,
        actor_tenant: str,
        deadline_s: float = 5.0,
    ) -> list[str]:
        resp = await self.driver.unary(
            method="/efe.v1.admin.AdminService/ListVersions",
            payload={"tenant": tenant, "task": task},
            metadata=(("x-efe-tenant-id", actor_tenant),),
            deadline_s=deadline_s,
        )
        return [str(k) for k in (resp.payload.get("keys") or [])]

    async def delete_cone(
        self,
        *,
        key: str,
        tenant_id: str,
        actor_id: str,
        force: bool = False,
        deadline_s: float = 5.0,
    ) -> None:
        metadata = (
            ("x-efe-tenant-id", tenant_id),
            ("x-efe-actor-id", actor_id),
        )
        resp = await self.driver.unary(
            method="/efe.v1.admin.AdminService/DeleteCone",
            payload={"key": key, "force": bool(force)},
            metadata=metadata,
            deadline_s=deadline_s,
        )
        # Empty response — nothing to parse.
        if resp.status != "OK":
            raise UnsupportedRpc(
                f"DeleteCone failed with status {resp.status}",
                detail={"payload": resp.payload},
            )

    async def breach_history(
        self,
        *,
        tenant_id: str,
        tenant: str | None = None,
        task: str | None = None,
        min_magnitude: float = 0.0,
        limit: int = 100,
        deadline_s: float = 5.0,
    ) -> list[dict[str, Any]]:
        payload = {
            "limit": int(limit),
            "min_magnitude": float(min_magnitude),
        }
        if tenant is not None:
            payload["tenant"] = tenant
        if task is not None:
            payload["task"] = task
        resp = await self.driver.unary(
            method="/efe.v1.admin.AdminService/BreachHistory",
            payload=payload,
            metadata=(("x-efe-tenant-id", tenant_id),),
            deadline_s=deadline_s,
        )
        return list(resp.payload.get("reports") or [])
