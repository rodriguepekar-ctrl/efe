"""Django integration for Letter D.

Exports:

* :class:`EfeAppConfig`   — subclass of :class:`django.apps.AppConfig`
                            that preloads EFE in ``ready()``.
* :func:`get_efe`         — retrieve the facade from the app registry.
* :class:`EfeMiddleware`  — HTTP error mapping for :class:`EfeFacadeError`.
* :func:`get_config_from_settings` — merge ``settings.EFE`` into
                            :class:`EfeConfig`.

The module does **not** import Django at the top. Hosts that never
touch this adapter pay no Django dependency cost.
"""

from __future__ import annotations

import atexit
import logging
import threading
from typing import Any, Mapping, Optional

from . import EfeConfig, EfeFacade
from ._errors import EfeFacadeConfigError, EfeFacadeError


_log = logging.getLogger("efe_tool.integrations.django")

_FACADE: Optional[EfeFacade] = None
_FACADE_LOCK = threading.Lock()


# ---------------------------------------------------------------------------
#  Config bridging
# ---------------------------------------------------------------------------


def get_config_from_settings() -> EfeConfig:
    """Read ``settings.EFE`` and merge with env defaults.

    Supports both uppercase (Django-style) and lowercase keys::

        # settings.py
        EFE = {
            "ENDPOINT": "127.0.0.1:50051",
            "CATALOGS": ["support_triage"],
            "TLS": False,
        }

    Precedence: ``settings.EFE`` values win over env vars.
    """
    try:
        from django.conf import settings  # type: ignore
    except ImportError as e:  # pragma: no cover
        raise EfeFacadeConfigError(
            "Django is not installed",
            hint="pip install django",
            cause=e,
        ) from e
    raw: Mapping[str, Any] = getattr(settings, "EFE", {}) or {}
    # Start from env defaults so any missing Django setting picks up env fallback.
    merged = EfeConfig.from_env().to_dict()
    merged.update({k.lower(): v for k, v in raw.items()})
    return EfeConfig.from_dict(merged)


# ---------------------------------------------------------------------------
#  AppConfig wiring
# ---------------------------------------------------------------------------


class EfeAppReadyMixin:
    """Mixin for a user's ``django.apps.AppConfig`` subclass that
    preloads the EFE facade during Django's ``ready()`` phase.

    Usage::

        from django.apps import AppConfig
        from efe_tool.integrations.django import EfeAppReadyMixin

        class EfeIntegrationConfig(EfeAppReadyMixin, AppConfig):
            name = "apps.efe_integration"

    The mixin chains with a parent ``ready()`` if there is one (so
    users can combine it with other mixins freely).
    """

    def ready(self) -> None:  # type: ignore[override]
        _ensure_facade_ready()
        atexit.register(_atexit_close)
        # Cooperate with any other ``ready()`` in the MRO.
        parent = super()
        if hasattr(parent, "ready") and callable(getattr(parent, "ready")):
            try:
                parent.ready()  # type: ignore[attr-defined]
            except AttributeError:
                pass


def setup_for_django() -> EfeFacade:
    """Imperative alternative for apps that prefer not to subclass
    via the mixin. Call this from anywhere inside ``AppConfig.ready()``
    or manually in a management command::

        from efe_tool.integrations.django import setup_for_django
        setup_for_django()

    Returns the facade singleton so callers can smoke-test it.
    """
    facade = _ensure_facade_ready()
    atexit.register(_atexit_close)
    return facade


def _ensure_facade_ready() -> EfeFacade:
    global _FACADE
    with _FACADE_LOCK:
        if _FACADE is not None:
            return _FACADE
        cfg = get_config_from_settings()
        facade = EfeFacade(cfg)
        try:
            facade.preload()
        except Exception:
            _log.exception("efe_facade_preload_failed_in_django_ready")
            # CRITICAL: close the partially-initialised facade so the
            # gRPC channel it may have opened is released. Without this,
            # a retry on a subsequent request would leak the previous
            # facade (and its thread pool executor if already set up).
            try:
                facade.close(timeout_s=1.0)
            except Exception:  # pragma: no cover
                pass
            raise
        _FACADE = facade
        _log.info("efe_facade_ready_in_django",
                  extra={"endpoint": cfg.endpoint,
                         "catalogs": list(cfg.catalogs)})
        return _FACADE


# ---------------------------------------------------------------------------
#  EfeAppConfig — conditional on Django availability
# ---------------------------------------------------------------------------
#
# Design: the class must be accessible via ``hasattr(module, "EfeAppConfig")``
# even when Django is missing — that's the spec for module-level symbols
# documented in LETTER_D_ARCHITECTURE.md §5.4. We therefore expose a
# placeholder that raises on instantiation, and swap in the real Django
# subclass lazily when the framework is actually present.

try:
    from django.apps import AppConfig as _DjangoAppConfig  # type: ignore
    _DJANGO_AVAILABLE = True
except ImportError:
    _DJANGO_AVAILABLE = False

if _DJANGO_AVAILABLE:
    class EfeAppConfig(EfeAppReadyMixin, _DjangoAppConfig):  # type: ignore[misc]
        """A ready-made Django ``AppConfig`` with EFE preload wired.

        Subclass it and set ``name = "<dotted.path>"``::

            from efe_tool.integrations.django import EfeAppConfig

            class EfeIntegrationConfig(EfeAppConfig):
                name = "apps.efe_integration"
        """
        name = ""
else:  # pragma: no cover — covered by tests that stub Django absent
    class EfeAppConfig:  # type: ignore[no-redef]
        """Stub class when Django is not installed.

        Importing the module is free, but any attempt to instantiate
        this class raises :class:`ImportError` with a clear hint.
        """

        def __init__(self, *args: Any, **kwargs: Any) -> None:
            raise ImportError(
                "EfeAppConfig requires Django. Install it first: `pip install django`.",
            )


def _atexit_close() -> None:
    global _FACADE
    facade = _FACADE
    if facade is None:
        return
    try:
        facade.close(timeout_s=10.0)
    except Exception:  # pragma: no cover
        _log.exception("efe_facade_atexit_close_error")
    _FACADE = None


def get_efe() -> EfeFacade:
    """Return the facade singleton. Builds-and-preloads on first call
    if the host did not use :class:`EfeAppConfig` (best effort)."""
    global _FACADE
    if _FACADE is None:
        _ensure_facade_ready()
    assert _FACADE is not None
    return _FACADE


# ---------------------------------------------------------------------------
#  Middleware
# ---------------------------------------------------------------------------


class EfeMiddleware:
    """Django middleware that turns :class:`EfeFacadeError` into JSON.

    Add to ``MIDDLEWARE`` in ``settings.py`` AFTER common/CSRF::

        MIDDLEWARE = [
            "django.middleware.security.SecurityMiddleware",
            # ...
            "efe_tool.integrations.django.EfeMiddleware",
        ]
    """

    def __init__(self, get_response: Any) -> None:
        self.get_response = get_response

    def __call__(self, request: Any) -> Any:
        return self.get_response(request)

    def process_exception(self, request: Any, exception: BaseException) -> Any:
        if not isinstance(exception, EfeFacadeError):
            return None
        try:
            from django.http import JsonResponse  # type: ignore
        except ImportError:  # pragma: no cover
            return None
        _log.warning(
            "efe_facade_error_on_request",
            extra={
                "error_code": exception.error_code,
                "retryable": exception.retryable,
            },
        )
        return JsonResponse(
            exception.to_payload(),
            status=exception.suggested_http_status,
        )


# ---------------------------------------------------------------------------
#  Test support
# ---------------------------------------------------------------------------


def reset_for_test() -> None:
    """Drop the global facade — used by test tear-down."""
    global _FACADE
    with _FACADE_LOCK:
        if _FACADE is not None:
            try:
                _FACADE.close(timeout_s=1.0)
            except Exception:  # pragma: no cover
                pass
        _FACADE = None


def set_facade_for_test(facade: EfeFacade) -> None:
    """Override the global facade. Intended for pytest fixtures."""
    global _FACADE
    with _FACADE_LOCK:
        _FACADE = facade


__all__ = [
    "EfeAppConfig",            # resolved lazily via module __getattr__
    "EfeAppReadyMixin",
    "setup_for_django",
    "EfeMiddleware",
    "get_efe",
    "get_config_from_settings",
    "reset_for_test",
    "set_facade_for_test",
]
