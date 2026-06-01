"""Credentials loading — mTLS certificates + JWT refresh hook.

Works even when ``grpcio`` is absent : the class methods return opaque
byte triples that the channel factory passes into the real gRPC call
when available. The JWT hook is a callable returning the current Bearer.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

__all__ = ["CredentialsLoader", "mTlsCredentials"]


@dataclass(frozen=True, slots=True)
class mTlsCredentials:
    ca_pem: bytes
    client_cert_pem: bytes
    client_key_pem: bytes


@dataclass(slots=True)
class CredentialsLoader:
    """Load cert + key bytes from disk ; reload on demand."""

    ca_path: str | None = None
    cert_path: str | None = None
    key_path: str | None = None
    jwt_provider: Callable[[], str] | None = None
    _cached_mtls: mTlsCredentials | None = field(default=None, init=False)

    def load_mtls(self) -> mTlsCredentials:
        if self._cached_mtls is not None:
            return self._cached_mtls
        if not (self.ca_path and self.cert_path and self.key_path):
            raise ValueError(
                "CredentialsLoader requires ca_path, cert_path and key_path "
                "for mTLS"
            )
        ca = Path(self.ca_path).read_bytes()
        cert = Path(self.cert_path).read_bytes()
        key = Path(self.key_path).read_bytes()
        out = mTlsCredentials(ca_pem=ca, client_cert_pem=cert, client_key_pem=key)
        self._cached_mtls = out
        return out

    def bearer_token(self) -> str | None:
        if self.jwt_provider is None:
            return None
        return self.jwt_provider()

    def invalidate(self) -> None:
        """Force a re-load on the next :meth:`load_mtls` call."""
        self._cached_mtls = None
