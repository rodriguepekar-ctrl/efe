"""Host-facing configuration dataclass for the Letter-D façade.

``EfeConfig`` is **frozen** — once constructed, no field can change
on the hot path. Per-request overrides (``timeout_s``, per-catalog
budget) pass as function arguments to :meth:`EfeFacade.classify`, not
as attribute mutation.

See LETTER_D_ARCHITECTURE.md §9 for the precedence model and §5.1
for the full dataclass shape.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any, Mapping, Optional

from ._errors import EfeFacadeConfigError


DEFAULT_ENDPOINT: str = "127.0.0.1:50051"
DEFAULT_EMBEDDER: str = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
DEFAULT_REQUEST_TIMEOUT_S: float = 10.0
DEFAULT_CLASSIFY_BUDGET_MS: int = 5_000
DEFAULT_MAX_ROUNDS: int = 16
DEFAULT_CACHE_SIZE: int = 10_000
DEFAULT_CACHE_TTL_S: float = 3600.0
DEFAULT_MAX_TEXT_LENGTH: int = 100_000


@dataclass(frozen=True)
class EfeConfig:
    """Fully-resolved configuration for one :class:`EfeFacade` instance.

    Construction is explicit (kwargs) or via :meth:`from_env`/:meth:`from_dict`.
    No field is lazily filled from the environment at hot-path time.
    """

    # --- network ---
    endpoint: str = DEFAULT_ENDPOINT
    tls: bool = False
    tls_ca: Optional[str] = None
    jwt: Optional[str] = None
    tenant: Optional[str] = None

    # --- catalog registry ---
    catalogs: tuple[str, ...] = ()
    catalog_search_paths: tuple[str, ...] = ()
    embedder_model: str = DEFAULT_EMBEDDER

    # --- tuning ---
    request_timeout_s: float = DEFAULT_REQUEST_TIMEOUT_S
    classify_budget_ms: int = DEFAULT_CLASSIFY_BUDGET_MS
    max_rounds: int = DEFAULT_MAX_ROUNDS
    max_text_length: int = DEFAULT_MAX_TEXT_LENGTH

    # --- caches (EfeClient) ---
    cache_size: int = DEFAULT_CACHE_SIZE
    cache_ttl_s: float = DEFAULT_CACHE_TTL_S

    # --- lifecycle ---
    preload_classify_probe: bool = True

    # --- observability (opt-in, soft deps) ---
    metrics_enabled: bool = True
    tracing_enabled: bool = True
    metrics_namespace: str = "efe"
    log_text_sample: bool = False

    # --- async runtime ---
    async_executor_max_workers: int = 8

    # --- misc extension ---
    extra: Mapping[str, Any] = field(default_factory=dict)

    # ------------------------------------------------------------------
    #  Construction helpers
    # ------------------------------------------------------------------

    def __post_init__(self) -> None:
        # Validation runs once at construction.
        _validate(self)

    @classmethod
    def from_env(
        cls, env: Optional[Mapping[str, str]] = None,
    ) -> "EfeConfig":
        """Build a config from ``EFE_*`` environment variables.

        Recognised keys (all optional, defaults apply):

            EFE_ENDPOINT             host:port
            EFE_TLS                  1|0
            EFE_TLS_CA               path
            EFE_JWT                  bearer token
            EFE_TENANT               tenant id
            EFE_CATALOGS             comma-separated names
            EFE_CATALOG_SEARCH_PATHS colon-separated paths
            EFE_EMBEDDER             sentence-transformers model
            EFE_REQUEST_TIMEOUT_S    float
            EFE_CLASSIFY_BUDGET_MS   int
            EFE_MAX_ROUNDS           int
            EFE_MAX_TEXT_LENGTH      int
            EFE_CACHE_SIZE           int
            EFE_CACHE_TTL_S          float
            EFE_PRELOAD_CLASSIFY_PROBE 1|0
            EFE_METRICS_ENABLED      1|0
            EFE_TRACING_ENABLED      1|0
            EFE_METRICS_NAMESPACE    string
            EFE_LOG_TEXT_SAMPLE      1|0
            EFE_ASYNC_EXECUTOR_MAX_WORKERS int
        """
        e = env if env is not None else os.environ

        def pick(name: str, default: Any) -> Any:
            v = e.get(name)
            return v if v not in (None, "") else default

        return cls(
            endpoint=str(pick("EFE_ENDPOINT", DEFAULT_ENDPOINT)),
            tls=_bool_env(pick("EFE_TLS", "0")),
            tls_ca=_opt(pick("EFE_TLS_CA", None)),
            jwt=_opt(pick("EFE_JWT", None)),
            tenant=_opt(pick("EFE_TENANT", None)),
            catalogs=_tuple_env(pick("EFE_CATALOGS", ""), sep=","),
            catalog_search_paths=_tuple_env(
                pick("EFE_CATALOG_SEARCH_PATHS", ""), sep=":"
            ),
            embedder_model=str(pick("EFE_EMBEDDER", DEFAULT_EMBEDDER)),
            request_timeout_s=float(pick("EFE_REQUEST_TIMEOUT_S", DEFAULT_REQUEST_TIMEOUT_S)),
            classify_budget_ms=int(pick("EFE_CLASSIFY_BUDGET_MS", DEFAULT_CLASSIFY_BUDGET_MS)),
            max_rounds=int(pick("EFE_MAX_ROUNDS", DEFAULT_MAX_ROUNDS)),
            max_text_length=int(pick("EFE_MAX_TEXT_LENGTH", DEFAULT_MAX_TEXT_LENGTH)),
            cache_size=int(pick("EFE_CACHE_SIZE", DEFAULT_CACHE_SIZE)),
            cache_ttl_s=float(pick("EFE_CACHE_TTL_S", DEFAULT_CACHE_TTL_S)),
            preload_classify_probe=_bool_env(pick("EFE_PRELOAD_CLASSIFY_PROBE", "1")),
            metrics_enabled=_bool_env(pick("EFE_METRICS_ENABLED", "1")),
            tracing_enabled=_bool_env(pick("EFE_TRACING_ENABLED", "1")),
            metrics_namespace=str(pick("EFE_METRICS_NAMESPACE", "efe")),
            log_text_sample=_bool_env(pick("EFE_LOG_TEXT_SAMPLE", "0")),
            async_executor_max_workers=int(pick("EFE_ASYNC_EXECUTOR_MAX_WORKERS", 8)),
        )

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> "EfeConfig":
        """Build from a dict (e.g. Django ``settings.EFE`` or pydantic dump).

        Keys are case-insensitive (``ENDPOINT`` or ``endpoint`` both work).
        Unknown keys are collected into :attr:`extra`.
        """
        fields = {f.name for f in cls.__dataclass_fields__.values()}
        known: dict[str, Any] = {}
        extra: dict[str, Any] = {}
        for k, v in d.items():
            norm = k.lower()
            if norm in fields:
                known[norm] = v
            else:
                extra[k] = v
        # Coerce tuple-typed fields from list input.
        for tk in ("catalogs", "catalog_search_paths"):
            if tk in known and not isinstance(known[tk], tuple):
                known[tk] = tuple(known[tk])
        if extra:
            known["extra"] = extra
        return cls(**known)

    def with_override(self, **changes: Any) -> "EfeConfig":
        """Return a new EfeConfig with some fields overridden (for tests)."""
        return replace(self, **changes)

    def to_dict(self) -> dict[str, Any]:
        """Return a plain ``dict`` projection — handy for bridging with
        host frameworks that want a regular mapping (Django settings,
        Flask ``app.config``, Pydantic dumps).

        Tuple fields (``catalogs``, ``catalog_search_paths``) are
        converted to lists for JSON round-trip compatibility.
        """
        return {
            "endpoint": self.endpoint,
            "tls": self.tls,
            "tls_ca": self.tls_ca,
            "jwt": self.jwt,
            "tenant": self.tenant,
            "catalogs": list(self.catalogs),
            "catalog_search_paths": list(self.catalog_search_paths),
            "embedder_model": self.embedder_model,
            "request_timeout_s": self.request_timeout_s,
            "classify_budget_ms": self.classify_budget_ms,
            "max_rounds": self.max_rounds,
            "max_text_length": self.max_text_length,
            "cache_size": self.cache_size,
            "cache_ttl_s": self.cache_ttl_s,
            "preload_classify_probe": self.preload_classify_probe,
            "metrics_enabled": self.metrics_enabled,
            "tracing_enabled": self.tracing_enabled,
            "metrics_namespace": self.metrics_namespace,
            "log_text_sample": self.log_text_sample,
            "async_executor_max_workers": self.async_executor_max_workers,
        }

    # ------------------------------------------------------------------
    #  Derived / convenience
    # ------------------------------------------------------------------

    @property
    def effective_request_timeout_s(self) -> float:
        """Enforce ``timeout_s >= budget_ms/1000 + 0.5`` at read time."""
        floor = self.classify_budget_ms / 1000.0 + 0.5
        return max(self.request_timeout_s, floor)


# ---------------------------------------------------------------------------
#  Validation
# ---------------------------------------------------------------------------


def _validate(cfg: EfeConfig) -> None:
    if not cfg.endpoint:
        raise EfeFacadeConfigError("endpoint is required", hint="host:port form")
    if cfg.endpoint.startswith(("http://", "https://", "grpc://")):
        raise EfeFacadeConfigError(
            f"endpoint must be host:port, got {cfg.endpoint!r}",
            hint="gRPC endpoints do not carry a URL scheme",
        )
    if ":" not in cfg.endpoint:
        raise EfeFacadeConfigError(
            f"endpoint must include a port, got {cfg.endpoint!r}",
        )
    if cfg.request_timeout_s <= 0:
        raise EfeFacadeConfigError(
            f"request_timeout_s must be > 0, got {cfg.request_timeout_s}",
        )
    if cfg.classify_budget_ms <= 0:
        raise EfeFacadeConfigError(
            f"classify_budget_ms must be > 0, got {cfg.classify_budget_ms}",
        )
    if not 1 <= cfg.max_rounds <= 256:
        raise EfeFacadeConfigError(
            f"max_rounds must be in [1, 256], got {cfg.max_rounds}",
        )
    if cfg.max_text_length <= 0:
        raise EfeFacadeConfigError("max_text_length must be > 0")
    if cfg.cache_size < 0:
        raise EfeFacadeConfigError("cache_size must be >= 0")
    if cfg.cache_ttl_s < 0:
        raise EfeFacadeConfigError("cache_ttl_s must be >= 0")
    if cfg.async_executor_max_workers < 1:
        raise EfeFacadeConfigError("async_executor_max_workers must be >= 1")

    if cfg.tls and cfg.tls_ca:
        # Only validate existence if tls_ca is provided (None is fine = system CA).
        if not Path(cfg.tls_ca).expanduser().exists():
            raise EfeFacadeConfigError(
                f"tls_ca path does not exist: {cfg.tls_ca}",
            )

    for name in cfg.catalogs:
        if not name or not all(c.isalnum() or c in "_-" for c in name):
            raise EfeFacadeConfigError(
                f"invalid catalog name {name!r}",
                hint="catalog names must match [A-Za-z0-9_-]+",
            )


def _bool_env(v: Any) -> bool:
    if isinstance(v, bool):
        return v
    if v is None:
        return False
    s = str(v).strip().lower()
    if s in {"", "0", "false", "no", "off", "n", "f"}:
        return False
    return True


def _opt(v: Any) -> Optional[str]:
    if v is None:
        return None
    s = str(v).strip()
    return s if s else None


def _tuple_env(raw: str, *, sep: str) -> tuple[str, ...]:
    if not raw:
        return ()
    return tuple(x.strip() for x in str(raw).split(sep) if x.strip())


__all__ = ["EfeConfig", "DEFAULT_ENDPOINT", "DEFAULT_EMBEDDER"]
