"""Network isolation helpers — emit NetworkPolicy hints.

The helper does NOT apply K8s manifests directly ; it produces declarative
hints that the deployment tooling translates into NetworkPolicy /
Namespace / LabelSelector YAML.
"""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from typing import Any

__all__ = ["NetworkPolicyHint", "NetworkPolicyKind"]


class NetworkPolicyKind(str, enum.Enum):
    deny_all = "deny_all"
    allow_egress_kms = "allow_egress_kms"
    allow_egress_idp = "allow_egress_idp"
    allow_egress_objectstore = "allow_egress_objectstore"
    allow_ingress_gateway = "allow_ingress_gateway"
    allow_mesh_mtls = "allow_mesh_mtls"


@dataclass(frozen=True, slots=True)
class NetworkPolicyHint:
    tenant_id: str
    kind: NetworkPolicyKind
    labels: tuple[tuple[str, str], ...] = ()
    extra: tuple[tuple[str, str], ...] = ()

    def to_manifest(self) -> dict[str, Any]:
        """Produce a representative JSON blob for tooling."""
        return {
            "tenant_id": self.tenant_id,
            "kind": self.kind.value,
            "labels": {k: v for k, v in self.labels},
            "extra": {k: v for k, v in self.extra},
        }


DEFAULT_POLICIES: tuple[NetworkPolicyKind, ...] = (
    NetworkPolicyKind.deny_all,
    NetworkPolicyKind.allow_ingress_gateway,
    NetworkPolicyKind.allow_mesh_mtls,
    NetworkPolicyKind.allow_egress_kms,
    NetworkPolicyKind.allow_egress_idp,
    NetworkPolicyKind.allow_egress_objectstore,
)


def default_hints(tenant_id: str) -> list[NetworkPolicyHint]:
    return [
        NetworkPolicyHint(tenant_id=tenant_id, kind=k, labels=(("managed-by", "efe-security"),))
        for k in DEFAULT_POLICIES
    ]
