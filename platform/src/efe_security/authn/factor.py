"""Authentication factor requirements (MFA / step-up).

M11 does not implement MFA itself — it only enforces that the IdP has
asserted the right factors. :func:`require_mfa` raises
:class:`MfaRequired` otherwise.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable

from efe_security.context import Principal
from efe_security.errors import MfaRequired

__all__ = ["MfaRequirement", "require_mfa"]


@dataclass(frozen=True, slots=True)
class MfaRequirement:
    """Declarative requirement for an action."""

    required_factors: frozenset[str]  # e.g. {"mfa","hwk"}
    min_acr: str | None = None  # e.g. "urn:efe:acr:high"


def require_mfa(principal: Principal, req: MfaRequirement) -> None:
    actor = principal.actor
    if req.required_factors and not req.required_factors.issubset(set(actor.amr)):
        missing = req.required_factors - set(actor.amr)
        raise MfaRequired(
            f"missing authentication factors: {sorted(missing)}",
            detail={"required": sorted(req.required_factors), "present": sorted(actor.amr)},
        )
    if req.min_acr is not None and (actor.acr or "") < req.min_acr:
        raise MfaRequired(
            f"ACR below required level: got={actor.acr!r} min={req.min_acr!r}"
        )


_ = Iterable
