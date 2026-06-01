#!/usr/bin/env python3
"""Sprint D13 — Grafana org provisioning for multi-tenant deployments.

Creates a Grafana org per tenant, attaches the EFE folder + the
6 D13 dashboards, configures the Prometheus datasource with the
``X-Scope-OrgID`` header so Mimir / Cortex / Thanos filter
server-side. Cf. architecture D13 § 12.

Usage::

    python scripts/observability/grafana_org_provision.py \\
        --grafana-url http://grafana:3000 \\
        --token "$GRAFANA_ADMIN_TOKEN" \\
        --tenant tenant-acme \\
        --prometheus-url http://mimir:9009/prometheus

Exit codes :

* 0 — org created (or already exists with the right config).
* 1 — Grafana API error (4xx / 5xx).
* 2 — usage / config error.

Idempotent : re-running yields the same end state.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from typing import Any


def grafana_request(
    base_url: str,
    method: str,
    path: str,
    token: str,
    body: Any | None = None,
    org_id: int | None = None,
    timeout: float = 10.0,
) -> tuple[int, dict | list | None]:
    url = f"{base_url.rstrip('/')}{path}"
    headers = {
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/json",
    }
    if org_id is not None:
        headers["X-Grafana-Org-Id"] = str(org_id)
    data = json.dumps(body).encode("utf-8") if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8")
            return resp.status, json.loads(raw) if raw else None
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode("utf-8"))
        except Exception:
            return e.code, None


def find_or_create_org(
    base_url: str, token: str, name: str,
) -> int:
    """Return the org id, creating it if missing."""
    code, body = grafana_request(base_url, "GET", f"/api/orgs/name/{name}", token)
    if code == 200 and isinstance(body, dict) and "id" in body:
        return int(body["id"])
    if code == 404:
        code, body = grafana_request(
            base_url, "POST", "/api/orgs", token,
            body={"name": name},
        )
        if code in (200, 409):
            if isinstance(body, dict) and "orgId" in body:
                return int(body["orgId"])
        # Fallback : look up again.
        code, body = grafana_request(base_url, "GET", f"/api/orgs/name/{name}", token)
        if code == 200 and isinstance(body, dict) and "id" in body:
            return int(body["id"])
    raise SystemExit(f"FATAL: cannot find/create org {name!r} (HTTP {code})")


def upsert_prometheus_datasource(
    base_url: str, token: str, org_id: int,
    *,
    prom_url: str, tenant_id: str,
) -> None:
    """Create or update the Prometheus datasource with the
    X-Scope-OrgID header set to the tenant id (Mimir multi-tenancy)."""
    payload = {
        "name": "Prometheus",
        "type": "prometheus",
        "access": "proxy",
        "url": prom_url,
        "isDefault": True,
        "jsonData": {
            "httpHeaderName1": "X-Scope-OrgID",
            "timeInterval": "30s",
            "exemplarTraceIdDestinations": [
                {"name": "trace_id", "datasourceUid": "tempo"},
            ],
        },
        "secureJsonData": {
            "httpHeaderValue1": tenant_id,
        },
    }
    # Try create first ; if it already exists, fall back to update.
    code, _ = grafana_request(
        base_url, "POST", "/api/datasources", token,
        body=payload, org_id=org_id,
    )
    if code in (200, 201):
        return
    if code == 409:
        # Already exists — fetch by name then PUT.
        code, body = grafana_request(
            base_url, "GET", "/api/datasources/name/Prometheus", token,
            org_id=org_id,
        )
        if code == 200 and isinstance(body, dict) and "id" in body:
            ds_id = body["id"]
            payload["id"] = ds_id
            code, _ = grafana_request(
                base_url, "PUT", f"/api/datasources/{ds_id}", token,
                body=payload, org_id=org_id,
            )
            if code in (200, 201):
                return
    raise SystemExit(
        f"FATAL: cannot upsert Prometheus datasource for org "
        f"{org_id} (HTTP {code})"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    parser.add_argument("--grafana-url", required=True)
    parser.add_argument("--token",       required=True,
                        help="Grafana ADMIN API token")
    parser.add_argument("--tenant",      required=True,
                        help="Tenant id, e.g. tenant-acme")
    parser.add_argument("--prometheus-url", required=True,
                        help="Prometheus / Mimir endpoint")
    parser.add_argument("--org-name", default=None,
                        help="Grafana org name (default : derived from --tenant)")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    org_name = args.org_name or args.tenant

    if args.dry_run:
        print(f"DRY RUN : would create org {org_name!r}, "
              f"Prometheus datasource pointing to {args.prometheus_url} "
              f"with X-Scope-OrgID={args.tenant}")
        return 0

    print(f"[1/2] Creating / finding org {org_name!r}…")
    org_id = find_or_create_org(args.grafana_url, args.token, org_name)
    print(f"      org_id = {org_id}")

    print(f"[2/2] Upserting Prometheus datasource (X-Scope-OrgID={args.tenant})…")
    upsert_prometheus_datasource(
        args.grafana_url, args.token, org_id,
        prom_url=args.prometheus_url, tenant_id=args.tenant,
    )
    print("      ✓ done")
    print()
    print(f"Org {org_name!r} ready. Dashboards land via Helm (sidecar).")
    print(f"To verify : curl -H 'Authorization: Bearer ...' "
          f"-H 'X-Grafana-Org-Id: {org_id}' "
          f"{args.grafana_url}/api/dashboards/uid/efe-d13-01-operator-overview")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
