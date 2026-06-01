"""Configuration dataclass for M12."""

from __future__ import annotations

from dataclasses import dataclass, field

__all__ = [
    "CircuitBreakerConfig",
    "RetryConfig",
    "SwarmIntegrationConfig",
    "TransportConfig",
    "DEFAULTS",
]


# ---------------------------------------------------------------------------
# Bounds
# ---------------------------------------------------------------------------

MAX_DIM: int = 4096
MAX_POPULATION: int = 50_000
MAX_ROUNDS: int = 256
MAX_ANCHORS: int = 10_000
MAX_HBM_BYTES: int = 6 * 1024**3
MAX_CLIENT_DEADLINE_S: int = 60
MAX_INFLIGHT_PER_TENANT: int = 32
MAX_MSG_BYTES_CLIENT: int = 32 * 1024**2
MAX_MSG_BYTES_SERVER: int = 64 * 1024**2
NETWORK_MARGIN_MS: int = 200


@dataclass(frozen=True, slots=True)
class TransportConfig:
    target: str = "efe-server:50051"
    mtls_required: bool = True
    ca_path: str | None = None
    client_cert_path: str | None = None
    client_key_path: str | None = None
    keepalive_time_ms: int = 20_000
    keepalive_timeout_ms: int = 5_000
    max_send_message_length: int = MAX_MSG_BYTES_CLIENT
    max_receive_message_length: int = MAX_MSG_BYTES_SERVER
    max_client_deadline_s: int = MAX_CLIENT_DEADLINE_S


@dataclass(frozen=True, slots=True)
class CircuitBreakerConfig:
    error_rate_threshold: float = 0.25
    window_s: int = 30
    min_samples: int = 20
    open_cooldown_s: int = 10


@dataclass(frozen=True, slots=True)
class RetryConfig:
    max_attempts_unavailable: int = 5
    max_attempts_resource_exhausted: int = 3
    max_attempts_deadline: int = 2  # only if idempotent
    base_delay_ms: int = 200
    max_delay_ms: int = 5_000


@dataclass(frozen=True, slots=True)
class SwarmIntegrationConfig:
    transport: TransportConfig = field(default_factory=TransportConfig)
    retry: RetryConfig = field(default_factory=RetryConfig)
    circuit_breaker: CircuitBreakerConfig = field(default_factory=CircuitBreakerConfig)
    max_inflight_per_tenant: int = MAX_INFLIGHT_PER_TENANT
    max_queue_wait_ms: int = 50
    pii_linter_enabled: bool = True
    contract_schema_version: str = "efe.v1"
    breach_tailer_poll_s: float = 5.0
    hbm_descspec_threshold_ratio: float = 0.5  # call DescribeSpec if est > 50 %


DEFAULTS: SwarmIntegrationConfig = SwarmIntegrationConfig()
