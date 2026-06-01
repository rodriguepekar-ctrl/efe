"""mTLS verifier — extract SPIFFE IDs from peer certificates.

The verifier is an adapter : the actual TLS handshake happens in the
reverse proxy (Envoy) or the application server. Here we only interpret
the verified peer identity and turn it into a :class:`Principal`.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_security.context import Actor, ActorKind, Principal
from efe_security.errors import InvalidIssuer, InvalidSignature

__all__ = ["MtlsVerifier", "SpiffeId", "parse_spiffe_id"]


@dataclass(frozen=True, slots=True)
class SpiffeId:
    trust_domain: str
    namespace: str
    service_account: str

    def to_uri(self) -> str:
        return f"spiffe://{self.trust_domain}/{self.namespace}/{self.service_account}"


def parse_spiffe_id(uri: str) -> SpiffeId:
    if not uri.startswith("spiffe://"):
        raise InvalidIssuer(f"not a SPIFFE URI: {uri!r}")
    rest = uri[len("spiffe://") :]
    if "/" not in rest:
        raise InvalidIssuer(f"malformed SPIFFE URI: {uri!r}")
    trust_domain, _, path = rest.partition("/")
    parts = path.split("/")
    if len(parts) < 2:
        raise InvalidIssuer(f"SPIFFE URI missing namespace/service: {uri!r}")
    namespace, service_account = parts[0], parts[1]
    if not trust_domain or not namespace or not service_account:
        raise InvalidIssuer(f"SPIFFE URI has empty component: {uri!r}")
    return SpiffeId(trust_domain=trust_domain, namespace=namespace, service_account=service_account)


@dataclass(slots=True)
class MtlsVerifier:
    """Turn a verified peer SPIFFE URI into a :class:`Principal`."""

    trusted_trust_domain: str
    default_scopes_by_sa: dict[str, frozenset[str]]

    def build_principal(
        self, *, spiffe_uri: str, tenant_id: str | None = None
    ) -> Principal:
        sid = parse_spiffe_id(spiffe_uri)
        if sid.trust_domain != self.trusted_trust_domain:
            raise InvalidSignature(
                f"unknown trust domain: {sid.trust_domain!r}",
                detail={"expected": self.trusted_trust_domain},
            )
        scopes = self.default_scopes_by_sa.get(sid.service_account, frozenset())
        actor = Actor(
            kind=ActorKind.service,
            actor_id=sid.to_uri(),
            display_name=sid.service_account,
        )
        return Principal(
            actor=actor,
            tenant_id=tenant_id,
            session_id=None,
            jti=None,
            scopes=scopes,
            groups=frozenset({f"sa:{sid.service_account}"}),
            expires_at=None,
        )


_ = Any
