"""Flask integration for Letter D.

Exports:

* :func:`init_efe`  — call inside your application factory.
* :func:`get_efe`   — retrieve the facade inside a request handler.

Usage (application factory)::

    from flask import Flask
    from efe_tool.integrations.flask import init_efe, get_efe

    def create_app(config=None):
        app = Flask(__name__)
        if config: app.config.from_object(config)
        init_efe(app)
        @app.post("/classify")
        def classify():
            v = get_efe().classify(request.json["text"], catalog="support_triage")
            return v.as_dict()
        return app

Works with both sync WSGI and Flask 3.x. Not asyncio-native — wrap
blocking calls in ``asyncio.to_thread`` if using Flask's async-view
feature. The façade is stored on ``app.extensions["efe"]``.
"""

from __future__ import annotations

import logging
from typing import Any, Mapping, Optional

from . import EfeConfig, EfeFacade
from ._errors import EfeFacadeConfigError, EfeFacadeError


_log = logging.getLogger("efe_tool.integrations.flask")


# ---------------------------------------------------------------------------
#  init
# ---------------------------------------------------------------------------


def init_efe(
    app: Any,
    *,
    config: Optional[EfeConfig] = None,
    register_error_handler: bool = True,
    register_health_routes: bool = False,
) -> EfeFacade:
    """Construct, preload and register an :class:`EfeFacade` on ``app``.

    Parameters
    ----------
    app:
        A :class:`flask.Flask` application.
    config:
        Explicit :class:`EfeConfig`. When None, we read
        ``app.config["EFE_*"]`` first, then fall back to environment.
    register_error_handler:
        Install a 4xx/5xx handler for :class:`EfeFacadeError`.
    register_health_routes:
        Install ``/livez`` and ``/readyz`` routes.
    """
    try:
        import flask  # type: ignore  # noqa: F401
    except ImportError as e:  # pragma: no cover
        raise EfeFacadeConfigError(
            "Flask is not installed", hint="pip install flask", cause=e,
        ) from e

    if config is None:
        config = _config_from_app(app)

    facade = EfeFacade(config)
    try:
        facade.preload()
    except Exception:
        _log.exception("efe_facade_preload_failed_in_flask_init")
        # Don't leak a half-initialised facade (open gRPC channel, etc.).
        try:
            facade.close(timeout_s=1.0)
        except Exception:  # pragma: no cover
            pass
        raise

    app.extensions.setdefault("efe", facade)
    # Teardown on app context pop — best-effort; final close on SIGTERM
    # belongs to the WSGI server (gunicorn, uwsgi).
    _register_teardown(app, facade)

    if register_error_handler:
        _register_error_handler(app)
    if register_health_routes:
        _register_health(app)

    _log.info(
        "efe_facade_ready_in_flask",
        extra={"endpoint": config.endpoint, "catalogs": list(config.catalogs)},
    )
    return facade


def get_efe() -> EfeFacade:
    """Return the facade from the current Flask app context."""
    try:
        from flask import current_app  # type: ignore
    except ImportError as e:  # pragma: no cover
        raise EfeFacadeConfigError(
            "Flask is not installed", cause=e,
        ) from e
    facade = current_app.extensions.get("efe") if current_app else None
    if facade is None:
        raise EfeFacadeError(
            "no EfeFacade attached to app.extensions[\"efe\"] — "
            "call init_efe(app) in your factory",
        )
    return facade


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _config_from_app(app: Any) -> EfeConfig:
    """Build an :class:`EfeConfig` from ``app.config["EFE_*"]``.

    Every key prefixed with ``EFE_`` is stripped of the prefix and
    lower-cased. Missing keys fall back to env defaults.
    """
    prefix = "EFE_"
    overrides: dict[str, Any] = {}
    for k, v in app.config.items():
        if isinstance(k, str) and k.startswith(prefix):
            overrides[k[len(prefix):].lower()] = v
    if not overrides:
        return EfeConfig.from_env()
    # Merge env with overrides (overrides win).
    merged = EfeConfig.from_env().to_dict()
    merged.update(overrides)
    return EfeConfig.from_dict(merged)


def _register_teardown(app: Any, facade: EfeFacade) -> None:
    @app.teardown_appcontext
    def _teardown(exc: Optional[BaseException]) -> None:
        # Nothing to do per-request — the facade lives for the whole
        # process life. We leave the hook here so users can swap in a
        # stricter policy by overriding.
        return None


def _register_error_handler(app: Any) -> None:
    from flask import jsonify  # type: ignore

    @app.errorhandler(EfeFacadeError)
    def _handler(exc: EfeFacadeError):  # type: ignore[no-redef]
        payload = exc.to_payload()
        response = jsonify(payload)
        response.status_code = exc.suggested_http_status
        _log.warning(
            "efe_facade_error_on_request",
            extra={
                "error_code": exc.error_code,
                "retryable": exc.retryable,
            },
        )
        return response


def _register_health(app: Any) -> None:
    from flask import jsonify  # type: ignore

    @app.get("/livez")
    def _live():
        return jsonify({"status": "ok"})

    @app.get("/readyz")
    def _ready():
        facade = app.extensions.get("efe")
        if facade is None:
            return jsonify({"status": "not_ready"}), 503
        hs = facade.healthcheck()
        status_code = 200 if hs.serving else 503
        return jsonify(hs.as_dict()), status_code


__all__ = ["init_efe", "get_efe"]
