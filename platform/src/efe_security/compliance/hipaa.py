"""HIPAA — applicable to opt-in BAA tenants."""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["HIPAA_MAPPING", "HipaaControl"]


@dataclass(frozen=True, slots=True)
class HipaaControl:
    section: str
    title: str
    efe_controls: tuple[str, ...]


HIPAA_MAPPING: tuple[HipaaControl, ...] = (
    HipaaControl("§164.308", "Administrative safeguards", ("authn", "authz", "tenancy.hipaa")),
    HipaaControl("§164.312", "Technical safeguards", ("secrets", "audit")),
    HipaaControl("§164.314", "Organizational requirements", ("baa",)),
)
