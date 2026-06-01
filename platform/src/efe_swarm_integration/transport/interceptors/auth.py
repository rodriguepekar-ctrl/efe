"""AuthInterceptor — ensures a Bearer is in metadata if no mTLS.

Real mTLS is enforced at the channel level ; this interceptor is the
belt-and-braces at the RPC level.
"""

from __future__ import annotations

from dataclasses import dataclass

from efe_swarm_integration.errors import AuthNError
from efe_swarm_integration.transport.credentials import CredentialsLoader

__all__ = ["AuthInterceptor"]


@dataclass(slots=True)
class AuthInterceptor:
    credentials: CredentialsLoader
    require_bearer: bool = False

    def apply(
        self, *, metadata: tuple[tuple[str, str], ...]
    ) -> tuple[tuple[str, str], ...]:
        token = self.credentials.bearer_token()
        if token:
            return metadata + (("authorization", f"Bearer {token}"),)
        if self.require_bearer:
            raise AuthNError("Bearer JWT required but CredentialsLoader has none")
        return metadata
