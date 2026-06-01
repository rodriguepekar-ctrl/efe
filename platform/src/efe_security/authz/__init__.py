"""AuthZ : PDP central + RBAC + ABAC + RLS + decorator + break-glass."""

from __future__ import annotations

from efe_security.authz.abac import (
    AbacCondition,
    AbacRule,
    AbacRuleSet,
    evaluate_rules,
)
from efe_security.authz.break_glass import (
    BREAK_GLASS_ALLOWED_SCOPES,
    BreakGlassGrant,
    BreakGlassManager,
    BreakGlassState,
)
from efe_security.authz.enforcer import requires
from efe_security.authz.pdp import (
    PolicyDecision,
    PolicyDecisionPoint,
    PolicyRequest,
)
from efe_security.authz.rbac import RbacRegistry, RoleGrant
from efe_security.authz.rls_bridge import RlsBridge, validate_tenant_id

__all__ = [
    "AbacCondition",
    "AbacRule",
    "AbacRuleSet",
    "BREAK_GLASS_ALLOWED_SCOPES",
    "BreakGlassGrant",
    "BreakGlassManager",
    "BreakGlassState",
    "PolicyDecision",
    "PolicyDecisionPoint",
    "PolicyRequest",
    "RbacRegistry",
    "RlsBridge",
    "RoleGrant",
    "evaluate_rules",
    "requires",
    "validate_tenant_id",
]
