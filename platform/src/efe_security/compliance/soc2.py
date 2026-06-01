"""SOC 2 Common Criteria → M11."""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["SOC2_MAPPING", "Soc2Control"]


@dataclass(frozen=True, slots=True)
class Soc2Control:
    cc: str  # e.g. "CC6.1"
    title: str
    efe_controls: tuple[str, ...]


SOC2_MAPPING: tuple[Soc2Control, ...] = (
    Soc2Control("CC6.1", "Logical access", ("authn", "authz")),
    Soc2Control("CC6.2", "User provisioning", ("authn.service_account", "tenancy.provisioner")),
    Soc2Control("CC6.3", "Role changes", ("audit",)),
    Soc2Control("CC6.6", "Encryption", ("secrets", "crypto")),
    Soc2Control("CC6.7", "Key management", ("secrets.rotation", "crypto.agility")),
    Soc2Control("CC6.8", "Malware", ("external_scanner",)),
    Soc2Control("CC7.1", "Monitoring", ("audit", "incident")),
    Soc2Control("CC7.2", "Anomaly detection", ("incident.detector",)),
)
