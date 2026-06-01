"""Implementation of ``efe-admin catalog``.

Sprint D12 (v0.1.12) — multi-tenant catalog admin via the gRPC
``CatalogService`` (``protos/efe/v1/catalog.proto``). Wraps the
:class:`efe_tool.admin.EfeAdminClient` six new methods :

  * ``register_catalog`` ↔ ``efe-admin catalog register``
  * ``evolve_catalog``   ↔ ``efe-admin catalog evolve``
  * ``list_catalogs``    ↔ ``efe-admin catalog list``
  * ``list_catalog_versions`` ↔ ``efe-admin catalog versions``
  * ``get_catalog``      ↔ ``efe-admin catalog get``
  * ``delete_catalog``   ↔ ``efe-admin catalog delete``

Exit codes (cf. ``cli/errors.py``) :

  * ``0`` — success.
  * ``2`` — usage error (missing flag, malformed key).
  * ``3`` — ``EfeNotFoundError`` mapped to :class:`CatalogNotFoundError`.
  * ``4`` — ``EfeInvalidArgumentError`` mapped to :class:`CatalogInvalidError`.
  * ``5`` — gRPC transport failure (unreachable, TLS, etc.).
  * ``6`` — generic server error (auth issue, rate limit, internal).

The command always emits a schema-versioned document on stdout —
even a successful ``delete`` produces ``{"deleted": true, "key": ...}``
so a CI script can pipe to ``jq``.
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

from ..config import ResolvedConfig
from ..errors import (
    CatalogInvalidError,
    CatalogNotFoundError,
    CliError,
    NetworkError,
    ServerError,
    UsageError,
    from_bridge_error,
)
from ..logging import Logger
from ..metadata import SCHEMA_VERSION
from ..output import render


# ---------------------------------------------------------------------------
#  Top-level dispatcher
# ---------------------------------------------------------------------------


def run(cfg: ResolvedConfig, log: Logger) -> int:
    action = cfg.catalog_action
    if not action:
        raise UsageError(
            "no catalog action ; expected one of: "
            "register, evolve, list, versions, get, delete",
            hint="run `efe-admin catalog --help`",
        )
    if action == "register":
        return _run_register(cfg, log)
    if action == "evolve":
        return _run_evolve(cfg, log)
    if action == "list":
        return _run_list(cfg, log)
    if action == "versions":
        return _run_versions(cfg, log)
    if action == "get":
        return _run_get(cfg, log)
    if action == "delete":
        return _run_delete(cfg, log)
    raise UsageError(f"unknown catalog action: {action!r}")


# ---------------------------------------------------------------------------
#  Per-action handlers
# ---------------------------------------------------------------------------


def _run_register(cfg: ResolvedConfig, log: Logger) -> int:
    if not cfg.catalog_name:
        raise UsageError("--name is required for `catalog register`")
    if not cfg.catalog_content_path:
        raise UsageError("--content is required for `catalog register`")

    content = _load_yaml_or_json(cfg.catalog_content_path)
    labels = _load_json_or_yaml(cfg.catalog_labels_path) if cfg.catalog_labels_path else {}
    if not isinstance(labels, dict):
        raise UsageError(
            f"--labels file must decode to a JSON object ; got "
            f"{type(labels).__name__}",
        )

    log.info(
        "catalog_register_start",
        endpoint=cfg.endpoint, name=cfg.catalog_name,
        version=cfg.catalog_version or "(unset)",
        content_path=str(cfg.catalog_content_path),
        n_labels=len(labels),
    )
    admin = _build_admin(cfg)

    t0 = time.perf_counter()
    try:
        registered = admin.register_catalog(
            tenant=cfg.tenant or _tenant_or_die(admin, cfg),
            name=cfg.catalog_name,
            version=cfg.catalog_version or "v1",
            content=content,
            description=cfg.catalog_description or "",
            labels=dict(labels),
        )
    except Exception as exc:  # noqa: BLE001 — re-raise typed below
        raise _translate_admin_error(exc, action="register") from exc
    finally:
        try:
            admin.close()
        except Exception:  # pragma: no cover
            pass
    latency_ms = int((time.perf_counter() - t0) * 1000)

    doc = _build_registered_document(registered, action="register",
                                     latency_ms=latency_ms,
                                     request_id=cfg.request_id)
    render(doc, format=cfg.format, pretty=cfg.pretty)
    log.info("catalog_register_done", key=registered.key,
             version=registered.version, latency_ms=latency_ms)
    return 0


def _run_evolve(cfg: ResolvedConfig, log: Logger) -> int:
    if not cfg.catalog_key:
        raise UsageError("--from-key is required for `catalog evolve`")
    if not cfg.catalog_op:
        raise UsageError("--op is required for `catalog evolve`")
    if not cfg.catalog_op_payload_path:
        raise UsageError("--payload is required for `catalog evolve`")

    op = _resolve_evolve_op(cfg.catalog_op)
    payload = _load_yaml_or_json(cfg.catalog_op_payload_path)
    if not isinstance(payload, dict):
        raise UsageError(
            f"--payload must decode to an object ; got "
            f"{type(payload).__name__}",
        )

    log.info(
        "catalog_evolve_start",
        endpoint=cfg.endpoint, from_key=cfg.catalog_key,
        op=cfg.catalog_op,
        payload_path=str(cfg.catalog_op_payload_path),
    )
    admin = _build_admin(cfg)
    t0 = time.perf_counter()
    try:
        registered = admin.evolve_catalog(
            from_key=cfg.catalog_key,
            op=op,
            op_payload=payload,
            description=cfg.catalog_description or "",
        )
    except Exception as exc:  # noqa: BLE001
        raise _translate_admin_error(exc, action="evolve") from exc
    finally:
        try:
            admin.close()
        except Exception:  # pragma: no cover
            pass
    latency_ms = int((time.perf_counter() - t0) * 1000)

    doc = _build_registered_document(registered, action="evolve",
                                     latency_ms=latency_ms,
                                     request_id=cfg.request_id,
                                     extra={"from_key": cfg.catalog_key,
                                            "op": cfg.catalog_op})
    render(doc, format=cfg.format, pretty=cfg.pretty)
    log.info("catalog_evolve_done", key=registered.key,
             version=registered.version, latency_ms=latency_ms)
    return 0


def _run_list(cfg: ResolvedConfig, log: Logger) -> int:
    log.info("catalog_list_start", endpoint=cfg.endpoint)
    admin = _build_admin(cfg)
    t0 = time.perf_counter()
    try:
        page_size = cfg.catalog_page_size or 100
        refs = admin.list_catalogs(
            tenant=cfg.tenant,
            page_size=page_size,
        )
    except Exception as exc:  # noqa: BLE001
        raise _translate_admin_error(exc, action="list") from exc
    finally:
        try:
            admin.close()
        except Exception:  # pragma: no cover
            pass
    latency_ms = int((time.perf_counter() - t0) * 1000)

    doc = {
        "schema_version": SCHEMA_VERSION,
        "request_id": cfg.request_id,
        "action": "list",
        "endpoint": cfg.endpoint,
        "latency_ms": latency_ms,
        "n_catalogs": len(refs),
        "catalogs": [
            {
                "tenant": r.tenant,
                "name": r.name,
                "latest_version": r.version,
                "num_classes": r.num_classes,
                "num_anchors": r.num_anchors,
            }
            for r in refs
        ],
    }
    render(doc, format=cfg.format, pretty=cfg.pretty)
    log.info("catalog_list_done", n=len(refs), latency_ms=latency_ms)
    return 0


def _run_versions(cfg: ResolvedConfig, log: Logger) -> int:
    if not cfg.catalog_name:
        raise UsageError("--name is required for `catalog versions`")
    log.info("catalog_versions_start", endpoint=cfg.endpoint,
             name=cfg.catalog_name)
    admin = _build_admin(cfg)
    t0 = time.perf_counter()
    try:
        keys = admin.list_catalog_versions(
            tenant=cfg.tenant or _tenant_or_die(admin, cfg),
            name=cfg.catalog_name,
        )
    except Exception as exc:  # noqa: BLE001
        raise _translate_admin_error(exc, action="versions") from exc
    finally:
        try:
            admin.close()
        except Exception:  # pragma: no cover
            pass
    latency_ms = int((time.perf_counter() - t0) * 1000)

    doc = {
        "schema_version": SCHEMA_VERSION,
        "request_id": cfg.request_id,
        "action": "versions",
        "endpoint": cfg.endpoint,
        "name": cfg.catalog_name,
        "latency_ms": latency_ms,
        "n_versions": len(keys),
        "keys": list(keys),
    }
    render(doc, format=cfg.format, pretty=cfg.pretty)
    log.info("catalog_versions_done", n=len(keys), latency_ms=latency_ms)
    return 0


def _run_get(cfg: ResolvedConfig, log: Logger) -> int:
    if not cfg.catalog_key:
        raise UsageError("KEY positional is required for `catalog get`")
    log.info("catalog_get_start", endpoint=cfg.endpoint,
             key=cfg.catalog_key,
             include_compiled=cfg.catalog_include_compiled)
    admin = _build_admin(cfg)
    t0 = time.perf_counter()
    try:
        registered = admin.get_catalog(
            cfg.catalog_key,
            include_compiled=cfg.catalog_include_compiled,
        )
    except Exception as exc:  # noqa: BLE001
        raise _translate_admin_error(exc, action="get") from exc
    finally:
        try:
            admin.close()
        except Exception:  # pragma: no cover
            pass
    latency_ms = int((time.perf_counter() - t0) * 1000)

    doc = _build_registered_document(registered, action="get",
                                     latency_ms=latency_ms,
                                     request_id=cfg.request_id)
    render(doc, format=cfg.format, pretty=cfg.pretty)
    log.info("catalog_get_done", key=registered.key,
             version=registered.version, latency_ms=latency_ms)
    return 0


def _run_delete(cfg: ResolvedConfig, log: Logger) -> int:
    if not cfg.catalog_key:
        raise UsageError("KEY positional is required for `catalog delete`")
    log.info("catalog_delete_start", endpoint=cfg.endpoint,
             key=cfg.catalog_key, force=cfg.catalog_force)
    admin = _build_admin(cfg)
    t0 = time.perf_counter()
    try:
        admin.delete_catalog(cfg.catalog_key, force=cfg.catalog_force)
    except Exception as exc:  # noqa: BLE001
        raise _translate_admin_error(exc, action="delete") from exc
    finally:
        try:
            admin.close()
        except Exception:  # pragma: no cover
            pass
    latency_ms = int((time.perf_counter() - t0) * 1000)

    doc = {
        "schema_version": SCHEMA_VERSION,
        "request_id": cfg.request_id,
        "action": "delete",
        "endpoint": cfg.endpoint,
        "deleted": True,
        "key": cfg.catalog_key,
        "force": cfg.catalog_force,
        "latency_ms": latency_ms,
    }
    render(doc, format=cfg.format, pretty=cfg.pretty)
    log.info("catalog_delete_done", key=cfg.catalog_key,
             latency_ms=latency_ms)
    return 0


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _build_admin(cfg: ResolvedConfig) -> Any:
    """Construct an :class:`EfeAdminClient` from the resolved CLI config."""
    from efe_tool.admin import EfeAdminClient

    if not cfg.jwt:
        raise UsageError(
            "--jwt (or env EFE_JWT) is required for catalog admin operations",
            hint="catalog endpoints carry sensitive PII labels and require "
                 "an authenticated tenant",
        )
    return EfeAdminClient(
        endpoint=cfg.endpoint,
        jwt=cfg.jwt,
        insecure=not cfg.tls,
        timeout_s=cfg.timeout_s,
    )


def _tenant_or_die(admin: Any, cfg: ResolvedConfig) -> str:
    """Best-effort tenant resolution when --tenant is not on the CLI.

    Falls back to the SDK's ``_tenant_from_jwt`` helper which decodes
    the JWT payload (no signature check). Raises a usage error if the
    JWT lacks a ``tenant_id`` claim.
    """
    try:
        return admin._tenant_from_jwt()  # noqa: SLF001 — same package
    except Exception as exc:
        raise UsageError(
            "no --tenant was provided and the JWT has no tenant_id claim ; "
            f"cannot scope the request: {exc}",
            hint="pass --tenant explicitly, or use a JWT with a tenant_id "
                 "claim",
        ) from exc


def _resolve_evolve_op(name: str) -> Any:
    """Map the CLI flag string to an :class:`EvolveOp` enum variant."""
    from efe_tool.admin import EvolveOp

    table = {
        "append-class": EvolveOp.APPEND_CLASS,
        "remove-class": EvolveOp.REMOVE_CLASS,
        "update-thresholds": EvolveOp.UPDATE_THRESHOLDS,
        "replace-class-anchors": EvolveOp.REPLACE_CLASS_ANCHORS,
        "recompile": EvolveOp.RECOMPILE,
    }
    if name not in table:
        raise UsageError(
            f"unknown --op {name!r} ; expected one of "
            f"{', '.join(sorted(table))}",
        )
    return table[name]


def _load_yaml_or_json(path: Path) -> Any:
    """Load a YAML or JSON file. YAML is tried first ; JSON is the
    fallback so callers can use either format interchangeably.

    Empty files (whitespace-only, or YAML that decodes to ``None``)
    are explicitly rejected so the caller doesn't silently send a
    `null` payload to the server.
    """
    text = Path(path).read_text(encoding="utf-8")
    parsed: Any
    try:
        import yaml  # noqa: PLC0415 — local to keep CLI startup lean
    except ImportError:
        # PyYAML is in the SDK's pyproject extras ; if missing, fall
        # back to JSON-only mode (which is what the catalog YAMLs are
        # in production anyway).
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError as exc:
            raise UsageError(
                f"file {path} is not valid JSON (PyYAML not installed, "
                f"so YAML fallback unavailable): {exc}",
            ) from exc
    else:
        try:
            parsed = yaml.safe_load(text)
        except yaml.YAMLError as yaml_exc:
            # YAML parse failed ; try JSON before giving up.
            try:
                parsed = json.loads(text)
            except json.JSONDecodeError as json_exc:
                raise UsageError(
                    f"file {path} is neither valid YAML "
                    f"({yaml_exc}) nor valid JSON ({json_exc})",
                ) from yaml_exc
    if parsed is None:
        raise UsageError(
            f"file {path} is empty (or YAML decoded to null) ; "
            f"expected a JSON / YAML object",
        )
    return parsed


def _load_json_or_yaml(path: Path) -> Any:
    """Variant that prefers JSON (used for ``--labels``).

    Same empty-file rejection as :func:`_load_yaml_or_json`.
    """
    text = Path(path).read_text(encoding="utf-8")
    parsed: Any
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError as json_exc:
        try:
            import yaml  # noqa: PLC0415
        except ImportError as imp_exc:
            raise UsageError(
                f"file {path} is not valid JSON ({json_exc}) and "
                f"PyYAML is not installed for the YAML fallback",
            ) from imp_exc
        try:
            parsed = yaml.safe_load(text)
        except yaml.YAMLError as yaml_exc:
            raise UsageError(
                f"file {path} is neither valid JSON "
                f"({json_exc}) nor valid YAML ({yaml_exc})",
            ) from yaml_exc
    if parsed is None:
        raise UsageError(
            f"file {path} is empty (or decoded to null) ; "
            f"expected a JSON / YAML object",
        )
    return parsed


def _build_registered_document(
    registered: Any, *,
    action: str,
    latency_ms: int,
    request_id: str,
    extra: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Render a :class:`RegisteredCatalog` to the schema-versioned doc."""
    meta = registered.metadata
    doc: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "request_id": request_id,
        "action": action,
        "latency_ms": latency_ms,
        "key": registered.key,
        "tenant": registered.tenant,
        "name": registered.name,
        "version": registered.version,
        "metadata": {
            "created_at": meta.created_at.isoformat(),
            "created_by": meta.created_by,
            "description": meta.description,
            "parent_key": meta.parent_key,
            "evolution_op": meta.evolution_op,
            "labels": dict(meta.labels),
            "content_sha256": meta.content_sha256,
        },
        "content": registered.content,
    }
    if registered.compiled is not None:
        doc["compiled"] = [
            {
                "class_name": c.class_name,
                "num_anchors": c.num_anchors,
                "embedder_fingerprint": c.embedder_fingerprint,
            }
            for c in registered.compiled
        ]
    if extra:
        doc.update(extra)
    return doc


def _translate_admin_error(exc: BaseException, *, action: str) -> CliError:
    """Map an :class:`efe_tool.errors.EfeError` (or related) to CliError.

    We don't reuse :func:`from_bridge_error` because its message is
    classify-centric (talks about ``--budget-ms``, swarm convergence,
    etc.) ; catalog errors deserve their own hints.
    """
    from efe_tool.errors import (
        EfeAuthError,
        EfeError,
        EfeInvalidArgumentError,
        EfeNotFoundError,
        EfeRateLimitError,
        EfeUnavailableError,
    )
    from efe_tool.admin import AdminError

    if isinstance(exc, CliError):
        return exc  # pre-wrapped — pass through

    if isinstance(exc, EfeNotFoundError):
        return CatalogNotFoundError(
            f"catalog `{action}` failed — not found: {exc}",
            hint="check the canonical key with "
                 "`efe-admin catalog versions --name <NAME>`",
            cause=exc,
        )
    if isinstance(exc, EfeInvalidArgumentError):
        return CatalogInvalidError(
            f"catalog `{action}` failed — invalid argument: {exc}",
            hint="run `efe-admin validate <content.yaml>` if registering "
                 "; check the schema in protos/efe/v1/catalog.proto",
            cause=exc,
        )
    if isinstance(exc, EfeAuthError):
        return ServerError(
            f"catalog `{action}` failed — authentication: {exc}",
            hint="check that the JWT carries the correct scope: "
                 "admin:catalog:read for read ops, admin:catalog:write "
                 "for register/evolve/delete",
            cause=exc,
        )
    if isinstance(exc, EfeRateLimitError):
        return ServerError(
            f"catalog `{action}` failed — rate-limited: {exc}",
            hint="back off and retry ; per-tenant quotas are documented "
                 "in docs/operations/quotas.md",
            cause=exc,
        )
    if isinstance(exc, EfeUnavailableError):
        return NetworkError(
            f"catalog `{action}` failed — server unavailable: {exc}",
            hint="check `efe-admin healthcheck --endpoint <ep>`",
            cause=exc,
        )
    if isinstance(exc, AdminError):
        return UsageError(f"admin client error: {exc}", cause=exc)
    if isinstance(exc, EfeError):
        return ServerError(
            f"catalog `{action}` failed: {exc}", cause=exc,
        )
    # Fallback: route through the existing classify-centric translator.
    return from_bridge_error(exc)


__all__ = ["run"]
