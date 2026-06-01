"""FSM tables shared across consent / key rotation / audit / DSAR / tenant.

Each aggregate exposes an ``ALLOWED`` set and reuses :func:`guard` to
block illegal transitions at the application layer.
"""

from __future__ import annotations

from typing import TypeVar

__all__ = [
    "AUDIT_READ_STATES",
    "CONSENT_ALLOWED",
    "DSAR_ALLOWED",
    "INCIDENT_ALLOWED",
    "KEY_ALLOWED",
    "TENANT_ALLOWED",
    "guard",
    "is_allowed",
]


S = TypeVar("S")


def guard(
    current: S,
    target: S,
    allowed: frozenset[tuple[S, S]],
    *,
    aggregate: str = "aggregate",
) -> None:
    if (current, target) not in allowed:
        from efe_security.errors import SecurityError

        class InvalidStateTransition(SecurityError):
            code = "security.invalid_state_transition"
            http_status = 409

        raise InvalidStateTransition(
            f"{aggregate}: transition {current} → {target} is not allowed",
            detail={"current": str(current), "target": str(target)},
        )


def is_allowed(current: S, target: S, allowed: frozenset[tuple[S, S]]) -> bool:
    return (current, target) in allowed


# ---------------------------------------------------------------------------
# Tenants :
#   nil → provisioning → active ↔ suspended → offboarding_soft/export/hard → archived
# ---------------------------------------------------------------------------

TENANT_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("nil", "provisioning"),
        ("provisioning", "active"),
        ("provisioning", "archived"),
        ("active", "suspended"),
        ("suspended", "active"),
        ("active", "offboarding_soft"),
        ("active", "offboarding_export"),
        ("active", "offboarding_hard"),
        ("suspended", "offboarding_soft"),
        ("suspended", "offboarding_hard"),
        ("offboarding_soft", "offboarding_hard"),
        ("offboarding_export", "offboarding_hard"),
        ("offboarding_soft", "active"),  # retraction within 90 days
        ("offboarding_hard", "archived"),
    }
)


# ---------------------------------------------------------------------------
# Consent records :
#   nil → active → withdrawn → expired ; active → expired
# ---------------------------------------------------------------------------

CONSENT_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("nil", "active"),
        ("active", "withdrawn"),
        ("active", "expired"),
        ("withdrawn", "expired"),
    }
)


# ---------------------------------------------------------------------------
# DSAR requests :
#   received → in_progress → completed
#                        → rejected
# ---------------------------------------------------------------------------

DSAR_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("received", "in_progress"),
        ("in_progress", "completed"),
        ("in_progress", "rejected"),
        ("received", "rejected"),
    }
)


# ---------------------------------------------------------------------------
# Crypto keys :
#   nil → active → retiring → retired
# ---------------------------------------------------------------------------

KEY_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("nil", "active"),
        ("active", "retiring"),
        ("retiring", "retired"),
        ("active", "retired"),  # emergency revoke
    }
)


# ---------------------------------------------------------------------------
# Incidents :
#   detected → triaged → contained → resolved
#                   → escalated → resolved
# ---------------------------------------------------------------------------

INCIDENT_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("detected", "triaged"),
        ("triaged", "contained"),
        ("triaged", "escalated"),
        ("contained", "resolved"),
        ("escalated", "resolved"),
        ("escalated", "contained"),
    }
)

# ---------------------------------------------------------------------------
# Audit reads — marker only, for the meta-audit producer.
# ---------------------------------------------------------------------------

AUDIT_READ_STATES: frozenset[str] = frozenset({"started", "completed"})
