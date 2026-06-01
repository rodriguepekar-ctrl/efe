"""@requires decorator — thin sugar over the PDP.

Applied on FastAPI routes / handler functions. Reads the
:class:`SecurityContext` from the call kwargs, invokes the PDP, and
either raises :class:`ScopeMissing` / :class:`TenantMismatch` or calls
the wrapped function.
"""

from __future__ import annotations

import functools
import inspect
from typing import Any, Awaitable, Callable

from efe_security.authz.pdp import PolicyDecisionPoint, PolicyRequest
from efe_security.context import Resource, SecurityContext
from efe_security.errors import ActionForbidden, ScopeMissing, TenantMismatch

__all__ = ["requires"]


def requires(
    *,
    action: str,
    resource_kind: str | None = None,
    pdp_attr: str = "pdp",
) -> Callable[[Callable[..., Awaitable[Any]]], Callable[..., Awaitable[Any]]]:
    """Authorization decorator.

    Usage::

        @requires(action="schedule.create", resource_kind="schedule")
        async def create_schedule(body, ctx: SecurityContext, pdp):
            ...

    Conventions :

    * the wrapped coroutine MUST accept a ``ctx: SecurityContext`` kwarg.
    * the PDP is found via the ``pdp_attr`` name (default ``"pdp"``) in
      the kwargs ; handlers typically receive it from the app's
      dependency container.
    * the PDP's verdict is applied BEFORE the wrapped body runs.
    """

    def deco(fn: Callable[..., Awaitable[Any]]) -> Callable[..., Awaitable[Any]]:
        sig = inspect.signature(fn)
        if "ctx" not in sig.parameters:
            raise ValueError(
                f"@requires: {fn.__name__} must declare a 'ctx' parameter"
            )

        @functools.wraps(fn)
        async def wrapper(*args: Any, **kwargs: Any) -> Any:
            ctx = kwargs.get("ctx")
            if not isinstance(ctx, SecurityContext):
                raise ActionForbidden("missing SecurityContext")
            pdp = kwargs.get(pdp_attr)
            if not isinstance(pdp, PolicyDecisionPoint):
                raise ActionForbidden(
                    f"missing PolicyDecisionPoint (kwarg '{pdp_attr}')"
                )
            resource: Resource | None = kwargs.get("resource")
            if resource is None and resource_kind is not None:
                resource = Resource(kind=resource_kind, tenant_id=ctx.principal.tenant_id)

            req = PolicyRequest(
                principal=ctx.principal,
                action=action,
                resource=resource,
                context={
                    "client_ip": ctx.client_ip,
                    "channel": ctx.channel,
                    "request_id": ctx.request_id,
                },
            )
            decision = await pdp.decide(req)
            if not decision.allow:
                # Discriminate on the first reason to emit a precise error.
                first = decision.reasons[0] if decision.reasons else ""
                if first.startswith("authz:tenant_mismatch"):
                    raise TenantMismatch(
                        "tenant mismatch", detail={"reasons": list(decision.reasons)}
                    )
                if first.startswith("rbac:missing:"):
                    scope = first[len("rbac:missing:"):]
                    raise ScopeMissing(
                        f"missing scope {scope!r}",
                        detail={"reasons": list(decision.reasons)},
                    )
                raise ActionForbidden(
                    "authorization denied",
                    detail={"reasons": list(decision.reasons)},
                )
            # Propagate obligations into the ctx passed downstream.
            kwargs["ctx"] = ctx.with_obligations(*decision.obligations)
            return await fn(*args, **kwargs)

        return wrapper

    return deco
