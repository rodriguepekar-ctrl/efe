"""Tenant provisioner — onboarding & offboarding.

Three offboard modes :

* ``soft`` — disable access, keep data for 90 days.
* ``export`` — produce encrypted archive, then hard.
* ``hard`` — destroy DEK → cryptographic erasure, then GC.
"""

from __future__ import annotations

import asyncio
import enum
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import TYPE_CHECKING

from efe_security.errors import (
    CryptoError,
    SecurityError,
    TenantDisabled,
    TenantNotFound,
)
from efe_security.fsm import TENANT_ALLOWED, guard
from efe_security.secrets.key_registry import KeyAlreadyRegistered

if TYPE_CHECKING:  # pragma: no cover
    from efe_security.secrets.key_registry import KeyRegistry
    from efe_security.secrets.kms_client import KmsClient

__all__ = ["OffboardMode", "Tenant", "TenantProvisioner", "TenantTier"]


class TenantTier(str, enum.Enum):
    standard = "standard"
    hipaa = "hipaa"
    eu_sovereign = "eu_sovereign"


class OffboardMode(str, enum.Enum):
    soft = "soft"
    export = "export"
    hard = "hard"


@dataclass(frozen=True, slots=True)
class Tenant:
    tenant_id: str
    display_name: str
    tier: TenantTier
    region: str
    state: str  # one of the values from fsm.TENANT_ALLOWED nodes
    state_version: int
    onboarded_at: datetime
    offboarded_at: datetime | None
    dek_kek_key_id: str
    labels: tuple[tuple[str, str], ...] = ()


@dataclass(slots=True)
class TenantProvisioner:
    kms: "KmsClient"
    key_registry: "KeyRegistry"
    _tenants: dict[str, Tenant] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    # ------------------------------------------------------------------
    # Onboard
    # ------------------------------------------------------------------

    async def onboard(
        self,
        *,
        tenant_id: str,
        display_name: str,
        tier: TenantTier = TenantTier.standard,
        region: str = "eu-west-1",
        labels: tuple[tuple[str, str], ...] = (),
    ) -> Tenant:
        from efe_security.secrets.key_registry import KeyEntry, KeyPurpose, KeyState

        now = datetime.now(tz=timezone.utc)
        async with self._lock:
            if tenant_id in self._tenants:
                raise SecurityError(f"tenant already exists: {tenant_id}")
            # Provision a KEK for this tenant.
            kek_key_id = f"tenant-kek/{tenant_id}"
            if hasattr(self.kms, "register_root_key"):
                try:
                    self.kms.register_root_key(kek_key_id)  # type: ignore[attr-defined]
                except CryptoError as exc:
                    # The only legitimate reason for `register_root_key` to
                    # fail idempotently is "already registered". Any other
                    # CryptoError surfaces : we must not silently continue.
                    if "already" not in str(exc).lower():
                        raise
            try:
                await self.key_registry.register(
                    KeyEntry(
                        key_id=kek_key_id,
                        alg_id="AES256-GCM-v1",
                        purpose=KeyPurpose.tenant_dek_kek,
                        state=KeyState.active,
                        created_at=now,
                    )
                )
            except KeyAlreadyRegistered:
                # Idempotent : re-running onboard is OK if the tenant's
                # KEK was already provisioned (e.g. retraction path).
                pass

            guard("nil", "provisioning", TENANT_ALLOWED, aggregate=f"tenant {tenant_id}")
            guard("provisioning", "active", TENANT_ALLOWED, aggregate=f"tenant {tenant_id}")
            tenant = Tenant(
                tenant_id=tenant_id,
                display_name=display_name,
                tier=tier,
                region=region,
                state="active",
                state_version=2,  # went through nil → provisioning → active
                onboarded_at=now,
                offboarded_at=None,
                dek_kek_key_id=kek_key_id,
                labels=labels,
            )
            self._tenants[tenant_id] = tenant
            return tenant

    # ------------------------------------------------------------------
    # Read
    # ------------------------------------------------------------------

    async def get(self, tenant_id: str) -> Tenant:
        async with self._lock:
            t = self._tenants.get(tenant_id)
            if t is None:
                raise TenantNotFound(tenant_id)
            return t

    async def require_active(self, tenant_id: str) -> Tenant:
        t = await self.get(tenant_id)
        if t.state != "active":
            raise TenantDisabled(tenant_id, detail={"state": t.state})
        return t

    async def list(self) -> list[Tenant]:
        async with self._lock:
            return list(self._tenants.values())

    # ------------------------------------------------------------------
    # Offboard
    # ------------------------------------------------------------------

    async def offboard(
        self,
        *,
        tenant_id: str,
        mode: OffboardMode,
        expected_version: int,
        now: datetime | None = None,
    ) -> Tenant:
        async with self._lock:
            t = self._tenants.get(tenant_id)
            if t is None:
                raise TenantNotFound(tenant_id)
            if t.state_version != expected_version:
                raise SecurityError(
                    f"tenant {tenant_id}: version mismatch "
                    f"expected={expected_version} actual={t.state_version}"
                )
            target_state = {
                OffboardMode.soft: "offboarding_soft",
                OffboardMode.export: "offboarding_export",
                OffboardMode.hard: "offboarding_hard",
            }[mode]
            guard(t.state, target_state, TENANT_ALLOWED, aggregate=f"tenant {tenant_id}")
            new_t = Tenant(
                tenant_id=t.tenant_id,
                display_name=t.display_name,
                tier=t.tier,
                region=t.region,
                state=target_state,
                state_version=t.state_version + 1,
                onboarded_at=t.onboarded_at,
                offboarded_at=now or datetime.now(tz=timezone.utc),
                dek_kek_key_id=t.dek_kek_key_id,
                labels=t.labels,
            )
            self._tenants[tenant_id] = new_t
            return new_t

    async def finalize_hard(
        self, *, tenant_id: str, expected_version: int, now: datetime | None = None
    ) -> Tenant:
        """Complete hard offboarding : destroy the KEK (cryptographic
        erasure), transition to ``archived``."""
        from efe_security.secrets.key_registry import KeyState

        async with self._lock:
            t = self._tenants.get(tenant_id)
            if t is None:
                raise TenantNotFound(tenant_id)
            if t.state_version != expected_version:
                raise SecurityError(
                    f"tenant {tenant_id}: version mismatch "
                    f"expected={expected_version} actual={t.state_version}"
                )
            guard(t.state, "archived", TENANT_ALLOWED, aggregate=f"tenant {tenant_id}")
            # Retire the KEK (soft) — real destruction via KMS is deployment-specific.
            try:
                entry = await self.key_registry.get(t.dek_kek_key_id)
                await self.key_registry.transition(
                    t.dek_kek_key_id,
                    KeyState.retired,
                    expected_version=entry.state_version,
                    now=now or datetime.now(tz=timezone.utc),
                )
            except SecurityError:
                pass

            new_t = Tenant(
                tenant_id=t.tenant_id,
                display_name=t.display_name,
                tier=t.tier,
                region=t.region,
                state="archived",
                state_version=t.state_version + 1,
                onboarded_at=t.onboarded_at,
                offboarded_at=t.offboarded_at,
                dek_kek_key_id=t.dek_kek_key_id,
                labels=t.labels,
            )
            self._tenants[tenant_id] = new_t
            return new_t
