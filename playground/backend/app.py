#!/usr/bin/env python3
"""
EFE Playground — Flask backend.

Provides REST endpoints mimicking the EFE gRPC API surface, for
local trial via the bundled HTML/JS frontend.

Endpoints :
  GET  /health                       — liveness probe
  GET  /info                         — backend info (embedder, version, etc.)
  POST /auth/token                   — issue demo token
  GET  /auth/tenants                 — list demo tenants
  GET  /auth/me                      — verify token + return identity

  POST /catalog/register             — create catalog version
  GET  /catalog                      — list catalogs (per tenant)
  GET  /catalog/<name>               — get latest catalog
  GET  /catalog/<name>/versions/<v>  — get specific version
  DELETE /catalog/<name>             — delete catalog (admin only)

  POST /classify                     — classify a text against a catalog
  POST /encode                       — encode a text → embedding vector

  GET  /audit                        — list audit log events (tenant-scoped)
  GET  /audit/integrity              — verify HMAC chain integrity

  GET  /samples                      — list pre-built sample catalogs
  POST /samples/<sample_name>/load   — load sample into current tenant

Authentication : Bearer token in `Authorization` header.

Usage :
  python3 playground/backend/app.py [--host HOST] [--port PORT]

  Then open browser :
    http://localhost:8765/
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

# Add backend/ to path so module imports work
sys.path.insert(0, str(Path(__file__).resolve().parent))

from flask import Flask, jsonify, request, send_from_directory  # noqa: E402
from flask_cors import CORS  # noqa: E402

from audit import AuditChain  # noqa: E402
from auth import (  # noqa: E402
    AuthIdentity,
    DEMO_TENANTS,
    issue_demo_token,
    list_demo_tenants,
    parse_token,
)
from catalog import (  # noqa: E402
    CatalogStore,
    QuotaExceededError,
    MAX_NAMES_PER_TENANT,
    MAX_VERSIONS_PER_NAME,
    MAX_CLASSES_PER_CATALOG,
    MAX_SENTENCES_PER_CLASS,
)
from embedder import make_embedder, DIM  # noqa: E402
from middleware import (  # noqa: E402
    IdempotencyCache,
    RateLimiter,
    scrub_pii,
)

# ---------------------------------------------------------------------------
#  App initialization
# ---------------------------------------------------------------------------

app = Flask(__name__, static_folder=None)
CORS(app)

# Globals — single-process, single-thread playground
EMBEDDER = make_embedder()
CATALOG = CatalogStore()
AUDIT = AuditChain()
RATE_LIMITER = RateLimiter()           # M15-A3 (M11 rate limit)
IDEMPOTENCY = IdempotencyCache()       # M15-A6 (M11 idempotency)
PII_HASH_ENABLED = os.environ.get("EFE_PLAYGROUND_PII_HASH", "false").lower() == "true"  # M15-A4 (D13)
SAMPLES_DIR = Path(__file__).resolve().parents[1] / "samples"
FRONTEND_DIR = Path(__file__).resolve().parents[1] / "frontend"

VERSION = "1.0.0-playground"


# ---------------------------------------------------------------------------
#  Auth middleware
# ---------------------------------------------------------------------------


def get_identity() -> AuthIdentity | None:
    """Extract + validate Bearer token from Authorization header."""
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer "):
        return None
    token = auth[7:].strip()
    return parse_token(token)


def require_auth():
    ident = get_identity()
    if ident is None:
        return None, (jsonify({"error": "unauthorized", "message": "Bearer token required"}), 401)
    return ident, None


def require_admin():
    ident, err = require_auth()
    if err:
        return None, err
    if not ident.is_admin:
        return None, (jsonify({"error": "forbidden", "message": "admin role required"}), 403)
    return ident, None


def check_rate_limit(ident):
    """Audit fix M15-A3 — token-bucket rate limit per tenant.

    Returns (None, None) if allowed, else (None, error_response_tuple).
    """
    allowed, info = RATE_LIMITER.check(ident.tenant_id, cost=1.0)
    if not allowed:
        return None, (jsonify({
            "error": "rate_limited",
            "message": f"tenant {ident.tenant_id} exceeded rate limit",
            "rate_limit_info": info,
        }), 429)
    return info, None


def check_idempotency(ident):
    """Audit fix M15-A6 — return cached response if Idempotency-Key matches."""
    key = request.headers.get("Idempotency-Key", "").strip()
    if not key:
        return None
    body = request.get_data()
    cached = IDEMPOTENCY.lookup(
        tenant_id=ident.tenant_id,
        idempotency_key=key,
        body=body,
    )
    if cached is None:
        return None
    resp = jsonify(cached.response)
    resp.status_code = cached.status_code
    resp.headers["X-EFE-Idempotency-Replay"] = "true"
    return resp


def store_idempotency(ident, response_data, status_code):
    """Audit fix M15-A6 — store the response under the Idempotency-Key."""
    key = request.headers.get("Idempotency-Key", "").strip()
    if not key:
        return
    IDEMPOTENCY.store(
        tenant_id=ident.tenant_id,
        idempotency_key=key,
        body=request.get_data(),
        response=response_data,
        status_code=status_code,
    )


# ---------------------------------------------------------------------------
#  Endpoints
# ---------------------------------------------------------------------------


@app.route("/health", methods=["GET"])
def health():
    return jsonify({"status": "ok", "service": "efe-playground", "version": VERSION})


@app.route("/info", methods=["GET"])
def info():
    return jsonify({
        "service": "efe-playground",
        "version": VERSION,
        "embedder": {
            "backend": EMBEDDER.info.backend,
            "model_name": EMBEDDER.info.model_name,
            "dim": EMBEDDER.info.dim,
            "device": EMBEDDER.info.device,
            "notes": EMBEDDER.info.notes,
        },
        # Audit fix M15-A2 — actual D12 policy quotas
        "limits": {
            "max_seq_len": 256,
            "max_names_per_tenant": MAX_NAMES_PER_TENANT,
            "max_versions_per_name": MAX_VERSIONS_PER_NAME,
            "max_classes_per_catalog": MAX_CLASSES_PER_CATALOG,
            "max_sentences_per_class": MAX_SENTENCES_PER_CLASS,
        },
        "audit_chain": {
            "events_total": len(AUDIT._events),
        },
        # Audit fix M15-A3 + A6 — middleware status surfaced
        "rate_limit": {
            "default_capacity": RATE_LIMITER.default_capacity,
            "default_refill_per_sec": RATE_LIMITER.default_refill_per_sec,
            "per_tenant_buckets": len(RATE_LIMITER._buckets),
        },
        "idempotency": IDEMPOTENCY.stats(),
        # Audit fix M15-A4 — PII hash mode (D13)
        "pii_hash_enabled": PII_HASH_ENABLED,
        "uptime_seconds": time.time() - START_TIME,
    })


# ─── Auth ─────────────────────────────────────────────────────────────────


@app.route("/auth/token", methods=["POST"])
def auth_token():
    data = request.get_json(silent=True) or {}
    tenant_id = data.get("tenant_id")
    role = data.get("role", "admin")
    if not tenant_id:
        return jsonify({"error": "tenant_id required"}), 400
    try:
        token = issue_demo_token(tenant_id, role)
    except ValueError as e:
        return jsonify({"error": str(e)}), 400
    AUDIT.emit(
        tenant_id=tenant_id,
        actor=f"{tenant_id}-actor",
        action="auth.issue_token",
        resource="auth",
    )
    return jsonify({
        "token": token,
        "tenant_id": tenant_id,
        "role": role,
        "expires_in_seconds": None,  # demo token has no expiry
    })


@app.route("/auth/tenants", methods=["GET"])
def auth_tenants():
    return jsonify({"tenants": list_demo_tenants()})


@app.route("/auth/me", methods=["GET"])
def auth_me():
    ident, err = require_auth()
    if err:
        return err
    return jsonify({
        "tenant_id": ident.tenant_id,
        "role": ident.role,
        "actor": ident.actor,
        "is_admin": ident.is_admin,
    })


# ─── Catalog ──────────────────────────────────────────────────────────────


@app.route("/catalog/register", methods=["POST"])
def catalog_register():
    ident, err = require_admin()
    if err:
        return err
    data = request.get_json(silent=True) or {}
    name = data.get("name")
    classes = data.get("classes")
    if not name or not classes:
        return jsonify({"error": "name + classes required"}), 400
    try:
        cv = CATALOG.register(
            tenant_id=ident.tenant_id,
            name=name,
            classes_input=classes,
            embedder=EMBEDDER,
        )
    except QuotaExceededError as e:
        # Audit fix M15-A2 — explicit 429 for D12 quota errors
        return jsonify({"error": "quota_exceeded", "message": str(e)}), 429
    except (ValueError, TypeError) as e:
        return jsonify({"error": str(e)}), 400
    AUDIT.emit(
        tenant_id=ident.tenant_id,
        actor=ident.actor,
        action="catalog.register",
        resource=name,
        metadata={
            "version": cv.version,
            "fingerprint": cv.fingerprint,
            "class_count": len(cv.classes),
        },
    )
    return jsonify({
        "version_id": cv.version_id,
        "name": cv.name,
        "version": cv.version,
        "fingerprint": cv.fingerprint,
        "class_count": len(cv.classes),
        "classes": [
            {
                "name": c.name,
                "example_count": len(c.examples),
                "confidence_threshold": c.confidence_threshold,
            }
            for c in cv.classes
        ],
        "embedder_info": cv.embedder_info,
    }), 201


@app.route("/catalog", methods=["GET"])
def catalog_list():
    ident, err = require_auth()
    if err:
        return err
    catalogs = CATALOG.list_catalogs(ident.tenant_id)
    return jsonify({"catalogs": catalogs})


@app.route("/catalog/<name>", methods=["GET"])
def catalog_get(name):
    ident, err = require_auth()
    if err:
        return err
    cv = CATALOG.get_latest(ident.tenant_id, name)
    if cv is None:
        return jsonify({"error": "not_found"}), 404
    return jsonify({
        "name": cv.name,
        "version": cv.version,
        "fingerprint": cv.fingerprint,
        "created_at": cv.created_at,
        "classes": [
            {
                "name": c.name,
                "examples": c.examples,
                "confidence_threshold": c.confidence_threshold,
                "metadata": c.metadata,
            }
            for c in cv.classes
        ],
        "embedder_info": cv.embedder_info,
    })


@app.route("/catalog/<name>/versions/<int:version>", methods=["GET"])
def catalog_get_version(name, version):
    ident, err = require_auth()
    if err:
        return err
    cv = CATALOG.get_version(ident.tenant_id, name, version)
    if cv is None:
        return jsonify({"error": "not_found"}), 404
    return jsonify({
        "name": cv.name,
        "version": cv.version,
        "fingerprint": cv.fingerprint,
        "created_at": cv.created_at,
        "classes": [
            {"name": c.name, "examples": c.examples,
             "confidence_threshold": c.confidence_threshold}
            for c in cv.classes
        ],
    })


@app.route("/catalog/<name>/evolve", methods=["POST"])
def catalog_evolve(name):
    """Audit fix M15-A1 (HIGH) — D12 Evolve RPC.

    Body : {"additional_examples": {<class_name>: [<new examples>], ...}}
    Adds examples to existing classes (or creates new classes) ;
    recomputes centroids ; registers as new version.
    """
    ident, err = require_admin()
    if err:
        return err
    data = request.get_json(silent=True) or {}
    additional = data.get("additional_examples", {})
    if not isinstance(additional, dict) or not additional:
        return jsonify({
            "error": "additional_examples (dict[class_name → list[str]]) required"
        }), 400

    try:
        cv = CATALOG.evolve(
            tenant_id=ident.tenant_id,
            name=name,
            additional_examples_per_class=additional,
            embedder=EMBEDDER,
        )
    except KeyError as e:
        return jsonify({"error": "catalog_not_found", "message": str(e)}), 404
    except QuotaExceededError as e:
        return jsonify({"error": "quota_exceeded", "message": str(e)}), 429
    except (ValueError, TypeError) as e:
        return jsonify({"error": str(e)}), 400

    AUDIT.emit(
        tenant_id=ident.tenant_id,
        actor=ident.actor,
        action="catalog.evolve",
        resource=name,
        metadata={
            "version": cv.version,
            "fingerprint": cv.fingerprint,
            "added_classes_or_examples": list(additional.keys()),
        },
    )
    return jsonify({
        "name": cv.name,
        "version": cv.version,
        "fingerprint": cv.fingerprint,
        "class_count": len(cv.classes),
    }), 201


@app.route("/catalog/<name>", methods=["DELETE"])
def catalog_delete(name):
    ident, err = require_admin()
    if err:
        return err
    deleted = CATALOG.delete(ident.tenant_id, name)
    if not deleted:
        return jsonify({"error": "not_found"}), 404
    AUDIT.emit(
        tenant_id=ident.tenant_id,
        actor=ident.actor,
        action="catalog.delete",
        resource=name,
    )
    return jsonify({"deleted": True, "name": name})


# ─── Classify + encode ────────────────────────────────────────────────────


@app.route("/classify", methods=["POST"])
def classify():
    ident, err = require_auth()
    if err:
        return err

    # Audit fix M15-A6 — idempotency check (replay if cached)
    cached = check_idempotency(ident)
    if cached is not None:
        return cached

    # Audit fix M15-A3 — rate limit
    _, rl_err = check_rate_limit(ident)
    if rl_err:
        return rl_err

    data = request.get_json(silent=True) or {}
    catalog_name = data.get("catalog")
    text = data.get("text")
    top_k = int(data.get("top_k", 3))
    if not catalog_name or not text:
        return jsonify({"error": "catalog + text required"}), 400

    # Audit fix M15-A4 — optional PII scrub before encoding (D13)
    pii_info = None
    if PII_HASH_ENABLED:
        text, pii_info = scrub_pii(text)

    try:
        result = CATALOG.classify(
            tenant_id=ident.tenant_id,
            catalog_name=catalog_name,
            text=text,
            embedder=EMBEDDER,
            top_k=top_k,
        )
    except KeyError as e:
        return jsonify({"error": "catalog_not_found", "message": str(e)}), 404

    if pii_info is not None:
        result["pii_scrub"] = pii_info

    AUDIT.emit(
        tenant_id=ident.tenant_id,
        actor=ident.actor,
        action="classify",
        resource=catalog_name,
        metadata={
            "text_len": len(text),
            "top_class": result["top_k"][0]["class_name"] if result["top_k"] else None,
            "top_score": result["top_k"][0]["score"] if result["top_k"] else None,
            "pii_scrubbed": pii_info is not None and any(pii_info.values()),
        },
    )
    # Audit fix M15-A6 — store response under Idempotency-Key
    store_idempotency(ident, result, 200)
    return jsonify(result)


@app.route("/encode", methods=["POST"])
def encode():
    ident, err = require_auth()
    if err:
        return err

    # Audit fix M15-A6 — idempotency
    cached = check_idempotency(ident)
    if cached is not None:
        return cached

    # Audit fix M15-A3 — rate limit
    _, rl_err = check_rate_limit(ident)
    if rl_err:
        return rl_err

    data = request.get_json(silent=True) or {}
    text = data.get("text")
    if not text:
        return jsonify({"error": "text required"}), 400

    # Audit fix M15-A4 — PII scrub
    pii_info = None
    if PII_HASH_ENABLED:
        text, pii_info = scrub_pii(text)

    vec = EMBEDDER.encode(text)
    AUDIT.emit(
        tenant_id=ident.tenant_id,
        actor=ident.actor,
        action="encode",
        resource="embedder",
        metadata={
            "text_len": len(text),
            "pii_scrubbed": pii_info is not None and any(pii_info.values()),
        },
    )
    response = {
        "embedding": vec.tolist(),
        "dim": int(vec.shape[0]),
        "norm": float((vec * vec).sum() ** 0.5),
        "embedder_info": {
            "backend": EMBEDDER.info.backend,
            "model_name": EMBEDDER.info.model_name,
        },
    }
    if pii_info is not None:
        response["pii_scrub"] = pii_info
    store_idempotency(ident, response, 200)
    return jsonify(response)


# ─── Audit ────────────────────────────────────────────────────────────────


@app.route("/audit", methods=["GET"])
def audit_list():
    ident, err = require_auth()
    if err:
        return err
    limit = int(request.args.get("limit", 50))
    events = AUDIT.list(tenant_id=ident.tenant_id, limit=limit)
    return jsonify({"events": events, "count": len(events)})


@app.route("/audit/integrity", methods=["GET"])
def audit_integrity():
    ident, err = require_auth()
    if err:
        return err
    return jsonify(AUDIT.verify_integrity())


# ─── Admin / bundle info (D27) ────────────────────────────────────────────


@app.route("/admin/bundle-info", methods=["GET"])
def admin_bundle_info():
    """Audit fix M15-A5 — D27 admin/bundle-info equivalent.

    In production EFE this is the endpoint the M12 D27 bundle-reload
    Helm hook polls. Returns the active bundle's metadata so operators
    + monitoring can verify which bundle the pod is currently serving.

    In the playground, the "bundle" is simulated by the active embedder.
    """
    ident, err = require_auth()
    if err:
        return err
    return jsonify({
        "bundle_slug": "playground-embedder-1.0.0",
        "bundle_id": (
            f"playground/{EMBEDDER.info.model_name}@1.0.0/"
            f"{('onnx' if EMBEDDER.info.backend == 'sentence-transformers' else 'hash-sim')}/"
            f"{EMBEDDER.info.dim}"
        ),
        "framework": (
            "onnx" if EMBEDDER.info.backend == "sentence-transformers"
            else "hash-simulator"
        ),
        "dim": EMBEDDER.info.dim,
        "manifestSha256": _bundle_fingerprint(),
        "is_dev_unsigned": EMBEDDER.info.backend == "hash",
        "embedder": {
            "backend": EMBEDDER.info.backend,
            "model_name": EMBEDDER.info.model_name,
            "device": EMBEDDER.info.device,
            "notes": EMBEDDER.info.notes,
        },
        "production_path": (
            "In production, this endpoint returns the OCI-pulled "
            "bundle's manifest.sha256 + framework + dim. "
            "Monitored by the M12 D27 bundle-reload Helm hook."
        ),
    })


def _bundle_fingerprint() -> str:
    """Compute a stable fingerprint for the active embedder backend."""
    import hashlib
    canon = (
        f"{EMBEDDER.info.backend}|"
        f"{EMBEDDER.info.model_name}|"
        f"{EMBEDDER.info.dim}|"
        f"playground-1.0.0"
    )
    return hashlib.sha256(canon.encode("utf-8")).hexdigest()


@app.route("/admin/quotas", methods=["GET"])
def admin_quotas():
    """Audit fix M15-A2 — surface current per-tenant quota usage."""
    ident, err = require_auth()
    if err:
        return err
    name_count = len(CATALOG._store.get(ident.tenant_id, {}))
    catalog_versions = {
        name: len(versions)
        for name, versions in CATALOG._store.get(ident.tenant_id, {}).items()
    }
    return jsonify({
        "tenant_id": ident.tenant_id,
        "current_usage": {
            "names_count": name_count,
            "max_names_per_tenant": MAX_NAMES_PER_TENANT,
            "names_remaining": MAX_NAMES_PER_TENANT - name_count,
            "versions_per_name": catalog_versions,
        },
        "limits": {
            "max_names_per_tenant": MAX_NAMES_PER_TENANT,
            "max_versions_per_name": MAX_VERSIONS_PER_NAME,
            "max_classes_per_catalog": MAX_CLASSES_PER_CATALOG,
            "max_sentences_per_class": MAX_SENTENCES_PER_CLASS,
        },
    })


# ─── Samples ──────────────────────────────────────────────────────────────


@app.route("/samples", methods=["GET"])
def samples_list():
    samples = []
    if SAMPLES_DIR.is_dir():
        for f in sorted(SAMPLES_DIR.glob("*.json")):
            try:
                data = json.loads(f.read_text(encoding="utf-8"))
                samples.append({
                    "id": f.stem,
                    "name": data.get("name", f.stem),
                    "description": data.get("description", ""),
                    "class_count": len(data.get("classes", [])),
                    "language": data.get("language", "en"),
                })
            except Exception:
                continue
    return jsonify({"samples": samples})


@app.route("/samples/<sample_id>", methods=["GET"])
def samples_get(sample_id):
    f = SAMPLES_DIR / f"{sample_id}.json"
    if not f.is_file():
        return jsonify({"error": "not_found"}), 404
    try:
        return jsonify(json.loads(f.read_text(encoding="utf-8")))
    except Exception as e:
        return jsonify({"error": "parse_failed", "message": str(e)}), 500


@app.route("/samples/<sample_id>/load", methods=["POST"])
def samples_load(sample_id):
    ident, err = require_admin()
    if err:
        return err
    f = SAMPLES_DIR / f"{sample_id}.json"
    if not f.is_file():
        return jsonify({"error": "sample_not_found"}), 404
    try:
        data = json.loads(f.read_text(encoding="utf-8"))
    except Exception as e:
        return jsonify({"error": "parse_failed", "message": str(e)}), 500

    name = data.get("name", sample_id)
    classes = data.get("classes", [])
    cv = CATALOG.register(
        tenant_id=ident.tenant_id,
        name=name,
        classes_input=classes,
        embedder=EMBEDDER,
    )
    AUDIT.emit(
        tenant_id=ident.tenant_id,
        actor=ident.actor,
        action="samples.load",
        resource=sample_id,
        metadata={"catalog_name": name, "version": cv.version},
    )
    return jsonify({
        "loaded": True,
        "sample_id": sample_id,
        "catalog_name": name,
        "version": cv.version,
        "fingerprint": cv.fingerprint,
        "class_count": len(cv.classes),
    }), 201


# ─── Frontend static ──────────────────────────────────────────────────────


@app.route("/", methods=["GET"])
def frontend_index():
    return send_from_directory(str(FRONTEND_DIR), "index.html")


@app.route("/<path:filename>", methods=["GET"])
def frontend_asset(filename):
    # Only serve files that exist under FRONTEND_DIR
    safe_path = FRONTEND_DIR / filename
    if safe_path.is_file():
        return send_from_directory(str(FRONTEND_DIR), filename)
    return jsonify({"error": "not_found"}), 404


# ---------------------------------------------------------------------------
#  Driver
# ---------------------------------------------------------------------------


START_TIME = time.time()


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=8765)
    p.add_argument("--debug", action="store_true")
    args = p.parse_args(argv)

    print(f"╔═══════════════════════════════════════════════════════╗", flush=True)
    print(f"║  EFE Playground — backend starting                    ║", flush=True)
    print(f"║                                                       ║", flush=True)
    print(f"║  Embedder backend  : {EMBEDDER.info.backend:<32s}║", flush=True)
    print(f"║  Embedder model    : {EMBEDDER.info.model_name[:32]:<32s}║", flush=True)
    print(f"║  Embedder dim      : {EMBEDDER.info.dim:<32d}║", flush=True)
    print(f"║                                                       ║", flush=True)
    print(f"║  → Open : http://{args.host}:{args.port}/                  ║", flush=True)
    print(f"║                                                       ║", flush=True)
    print(f"╚═══════════════════════════════════════════════════════╝", flush=True)

    app.run(host=args.host, port=args.port, debug=args.debug, use_reloader=False)


if __name__ == "__main__":
    main()
