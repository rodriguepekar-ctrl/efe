"""``efe_tool.integrations`` — Letter D public surface.

The façade pattern encapsulating :class:`efe_tool.client.EfeClient`,
:class:`efe_tool.catalog.AnchorCatalog` and
:class:`efe_tool.embedder.LocalSTEmbedder` for use inside a host app
(FastAPI, Django, Flask, Celery, Jupyter, Dagster, …).

See LETTER_D_ARCHITECTURE.md for the full specification.

Minimal usage::

    from efe_tool.integrations import EfeFacade, EfeConfig, Verdict

    facade = EfeFacade(EfeConfig(
        endpoint="127.0.0.1:50051",
        catalogs=("support_triage",),
        catalog_search_paths=("catalogs",),
    ))
    facade.preload()

    verdict: Verdict = facade.classify("Ma carte a été piratée",
                                        catalog="support_triage")
    print(verdict.label, verdict.confidence)

    facade.close()

For framework integrations::

    from efe_tool.integrations.fastapi   import efe_lifespan, get_efe
    from efe_tool.integrations.django    import EfeAppConfig, get_efe
    from efe_tool.integrations.flask     import init_efe, get_efe
    from efe_tool.integrations.celery    import setup_efe_for_worker, get_efe
    from efe_tool.integrations.notebook  import efe_notebook_singleton
    from efe_tool.integrations.testing   import FakeEfeFacade
"""

from __future__ import annotations

from ._config import (
    DEFAULT_EMBEDDER,
    DEFAULT_ENDPOINT,
    EfeConfig,
)
from ._errors import (
    FACADE_EXIT_CODES,
    EfeFacadeCatalogError,
    EfeFacadeClosedError,
    EfeFacadeConfigError,
    EfeFacadeError,
    EfeFacadeInitError,
    EfeFacadeRejectedError,
    EfeFacadeTimeoutError,
    EfeFacadeTransientError,
    EfeFacadeUsageError,
    from_bridge_error,
    from_cli_error,
)
from ._facade import EfeFacade, request_id_var
from ._metrics import MetricsRegistry
from ._tracing import Tracer
from ._types import FacadeStats, HealthStatus, Verdict


__all__ = [
    # Primary
    "EfeFacade",
    "EfeConfig",
    # Public types
    "Verdict",
    "HealthStatus",
    "FacadeStats",
    # Exceptions
    "EfeFacadeError",
    "EfeFacadeConfigError",
    "EfeFacadeInitError",
    "EfeFacadeClosedError",
    "EfeFacadeCatalogError",
    "EfeFacadeUsageError",
    "EfeFacadeRejectedError",
    "EfeFacadeTransientError",
    "EfeFacadeTimeoutError",
    "FACADE_EXIT_CODES",
    "from_cli_error",
    "from_bridge_error",
    # Observability
    "MetricsRegistry",
    "Tracer",
    "request_id_var",
    # Constants
    "DEFAULT_ENDPOINT",
    "DEFAULT_EMBEDDER",
]
