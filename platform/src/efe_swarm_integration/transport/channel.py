"""gRPC channel factory.

Falls back to a lightweight stub when ``grpcio`` is not installed. The
rest of the transport layer uses the ``RpcDriver`` abstraction that
works against both real gRPC and the :class:`FakeSwarmServer`.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_swarm_integration.config import TransportConfig
from efe_swarm_integration.errors import MutualTlsFailed
from efe_swarm_integration.transport.credentials import CredentialsLoader

__all__ = ["ChannelKey", "SwarmChannelFactory"]


try:  # pragma: no cover - optional dep
    import grpc  # type: ignore[import-not-found]
    import grpc.aio  # type: ignore[import-not-found]

    _GRPC_AVAILABLE = True
except ImportError:  # pragma: no cover
    _GRPC_AVAILABLE = False


@dataclass(frozen=True, slots=True)
class ChannelKey:
    target: str
    mtls: bool


class SwarmChannelFactory:
    """Returns one channel per ``ChannelKey`` (single-process pool)."""

    __slots__ = ("_channels", "_config", "_credentials")

    def __init__(
        self,
        config: TransportConfig,
        credentials: CredentialsLoader,
    ) -> None:
        self._config = config
        self._credentials = credentials
        self._channels: dict[ChannelKey, Any] = {}

    def channel(self, *, target: str | None = None) -> Any:
        """Return the (possibly cached) gRPC channel."""
        if not _GRPC_AVAILABLE:
            # Tests use the FakeSwarmServer via RpcDriver directly ;
            # caller should not reach here without grpcio in prod.
            raise MutualTlsFailed(
                "grpcio not installed ; channel() unavailable",
                detail={"target": target or self._config.target},
            )
        target = target or self._config.target
        key = ChannelKey(target=target, mtls=self._config.mtls_required)
        if key in self._channels:
            return self._channels[key]

        options = [
            ("grpc.keepalive_time_ms", self._config.keepalive_time_ms),
            ("grpc.keepalive_timeout_ms", self._config.keepalive_timeout_ms),
            ("grpc.keepalive_permit_without_calls", 1),
            ("grpc.max_send_message_length", self._config.max_send_message_length),
            ("grpc.max_receive_message_length", self._config.max_receive_message_length),
            ("grpc.enable_retries", 0),
        ]

        if self._config.mtls_required:
            try:
                creds = self._credentials.load_mtls()
            except Exception as exc:
                raise MutualTlsFailed(
                    f"failed to load mTLS material: {exc}"
                ) from exc
            ssl_creds = grpc.ssl_channel_credentials(
                root_certificates=creds.ca_pem,
                private_key=creds.client_key_pem,
                certificate_chain=creds.client_cert_pem,
            )
            # Optional JWT via call credentials.
            call_creds = _AuthMetadataPlugin.build(self._credentials)
            composite = grpc.composite_channel_credentials(ssl_creds, call_creds)
            channel = grpc.aio.secure_channel(target, composite, options=options)
        else:
            channel = grpc.aio.insecure_channel(target, options=options)

        self._channels[key] = channel
        return channel

    async def close_all(self) -> None:
        for channel in list(self._channels.values()):
            try:
                await channel.close(grace=2.0)
            except Exception:  # pragma: no cover
                pass
        self._channels.clear()


# ---------------------------------------------------------------------------
# Auth metadata plugin (real gRPC path)
# ---------------------------------------------------------------------------


class _AuthMetadataPlugin:
    """Adapter that injects the Bearer JWT on every RPC."""

    @staticmethod
    def build(credentials: CredentialsLoader) -> Any:  # pragma: no cover
        if not _GRPC_AVAILABLE:
            raise MutualTlsFailed("grpc not available")
        from grpc import AuthMetadataPlugin as _P, AuthMetadataContext, AuthMetadataPluginCallback  # type: ignore[attr-defined]

        class _Impl(_P):  # type: ignore[misc]
            def __call__(
                self,
                context: AuthMetadataContext,
                callback: AuthMetadataPluginCallback,
            ) -> None:
                token = credentials.bearer_token()
                metadata = (
                    (("authorization", f"Bearer {token}"),) if token else ()
                )
                callback(metadata, None)

        return grpc.metadata_call_credentials(_Impl())
