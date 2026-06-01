"""ABAC — attribute-based conditions.

Conditions are small declarative predicates over ``{principal,
resource, context}``. They stack on top of RBAC : a request may be
RBAC-allowed but ABAC-denied (e.g. editing your own resource only).

Supported comparators : ``eq``, ``neq``, ``in``, ``not_in``, ``matches``
(glob), ``between`` (numeric), ``time_in_range``, ``ip_in_cidr``.
"""

from __future__ import annotations

import fnmatch
import re
from dataclasses import dataclass, field
from datetime import datetime, time
from ipaddress import ip_address, ip_network
from typing import Any, Iterable, Literal

from efe_security.context import Principal, Resource

__all__ = [
    "AbacCondition",
    "AbacRule",
    "AbacRuleSet",
    "evaluate_rules",
]


Comparator = Literal[
    "eq", "neq", "in", "not_in", "matches",
    "between", "time_in_range", "ip_in_cidr",
]


@dataclass(frozen=True, slots=True)
class AbacCondition:
    path: str  # e.g. "principal.tenant_id", "resource.owner_id", "context.ip"
    comparator: Comparator
    value: Any

    def evaluate(
        self,
        *,
        principal: Principal,
        resource: Resource | None,
        context: dict[str, Any],
    ) -> bool:
        actual = _resolve(self.path, principal=principal, resource=resource, context=context)
        if self.comparator == "eq":
            return actual == self.value
        if self.comparator == "neq":
            return actual != self.value
        if self.comparator == "in":
            return actual in self.value
        if self.comparator == "not_in":
            return actual not in self.value
        if self.comparator == "matches":
            if actual is None:
                return False
            return fnmatch.fnmatchcase(str(actual), str(self.value))
        if self.comparator == "between":
            lo, hi = self.value
            return lo <= actual <= hi
        if self.comparator == "time_in_range":
            lo, hi = self.value  # "HH:MM" strings
            if not isinstance(actual, datetime):
                return False
            t = actual.time()
            lo_t = _parse_hhmm(lo)
            hi_t = _parse_hhmm(hi)
            if lo_t <= hi_t:
                return lo_t <= t <= hi_t
            # wrap around midnight
            return t >= lo_t or t <= hi_t
        if self.comparator == "ip_in_cidr":
            if actual is None:
                return False
            try:
                return ip_address(actual) in ip_network(self.value)
            except ValueError:
                return False
        return False


def _parse_hhmm(s: str) -> time:
    m = re.match(r"^(\d{2}):(\d{2})$", s)
    if not m:
        raise ValueError(f"invalid HH:MM: {s}")
    return time(hour=int(m.group(1)), minute=int(m.group(2)))


def _resolve(
    path: str,
    *,
    principal: Principal,
    resource: Resource | None,
    context: dict[str, Any],
) -> Any:
    head, _, tail = path.partition(".")
    if head == "principal":
        obj: Any = principal
    elif head == "resource":
        obj = resource
    elif head == "context":
        return _dict_lookup(context, tail)
    else:
        return None
    if obj is None:
        return None
    for part in tail.split("."):
        if not part:
            continue
        if isinstance(obj, dict):
            obj = obj.get(part)
        else:
            obj = getattr(obj, part, None)
        if obj is None:
            return None
    return obj


def _dict_lookup(obj: dict[str, Any], path: str) -> Any:
    if not path:
        return obj
    parts = path.split(".")
    cur: Any = obj
    for p in parts:
        if isinstance(cur, dict):
            cur = cur.get(p)
        else:
            return None
        if cur is None:
            return None
    return cur


@dataclass(frozen=True, slots=True)
class AbacRule:
    """A rule composed of conditions ANDed together.

    ``name`` is free-form ; ``allow`` flips the rule into an implicit
    deny when false.
    """

    name: str
    action_pattern: str  # fnmatch on action, e.g. "audit:*"
    conditions: tuple[AbacCondition, ...]
    allow: bool = True
    obligations: tuple[str, ...] = ()

    def applies_to(self, action: str) -> bool:
        return fnmatch.fnmatchcase(action, self.action_pattern)


@dataclass(slots=True)
class AbacRuleSet:
    rules: tuple[AbacRule, ...] = field(default_factory=tuple)

    def extend(self, rule: AbacRule) -> "AbacRuleSet":
        return AbacRuleSet(rules=self.rules + (rule,))


def evaluate_rules(
    ruleset: AbacRuleSet,
    *,
    action: str,
    principal: Principal,
    resource: Resource | None,
    context: dict[str, Any],
) -> tuple[bool, list[str], list[str]]:
    """Return ``(allow, reasons, obligations)``.

    Fail-closed semantics : if any rule applying to ``action`` has
    ``allow=False`` with all conditions matching → deny. Otherwise, if
    at least one ``allow=True`` rule matches → allow with accumulated
    obligations. Else → no opinion → ``True`` (ABAC is a filter on top
    of RBAC, so absence of rule = pass-through).
    """
    reasons: list[str] = []
    obligations: list[str] = []
    denied = False
    any_allow = False
    for rule in ruleset.rules:
        if not rule.applies_to(action):
            continue
        conds_ok = all(
            c.evaluate(principal=principal, resource=resource, context=context)
            for c in rule.conditions
        )
        if conds_ok:
            if rule.allow:
                any_allow = True
                obligations.extend(rule.obligations)
                reasons.append(f"abac:{rule.name}:allow")
            else:
                denied = True
                reasons.append(f"abac:{rule.name}:deny")
    if denied:
        return False, reasons, obligations
    # If a rule matched positively, allow explicitly ; otherwise,
    # "no opinion" returns True so RBAC remains the gate.
    return True, reasons or ["abac:no-rule"], obligations


_ = Iterable
