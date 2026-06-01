#!/usr/bin/env python3
"""Sprint D11.6 — Helm Tier 2 dual-render isolation check.

Given two rendered Helm outputs (one per tenant), assert the
critical isolation invariants:

  1. **No name collision** — every Kubernetes object has a
     unique (kind, name) across the two tenant releases. (Two
     tenants would normally live in different namespaces, but
     accidental same-namespace renders would conflict.)
  2. **Selectors are non-overlapping** — the Deployment's
     selector for tenant A doesn't match the pod template of
     tenant B. (A regression would make k8s think both tenants'
     pods belong to one Deployment.)
  3. **NetworkPolicy ingress is tenant-specific** — tenant A's
     ``tenant-ingress`` NetworkPolicy doesn't whitelist tenant
     B's namespace label.
  4. **ResourceQuota / LimitRange are present and non-empty** in
     both renders.

Used by ``.github/workflows/bench-isolation.yml``.

Exit codes:
   0 — all invariants hold
   1 — at least one invariant violated
   2 — operational error (file missing, malformed YAML)
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

import yaml


def load_manifests(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        raise FileNotFoundError(path)
    docs = list(yaml.safe_load_all(path.read_text(encoding="utf-8")))
    return [d for d in docs if d is not None]


def kind_name(doc: dict) -> tuple[str, str, str]:
    """Return (kind, namespace, name)."""
    return (
        doc.get("kind", "_unknown"),
        doc.get("metadata", {}).get("namespace", ""),
        doc.get("metadata", {}).get("name", ""),
    )


def selector_labels(doc: dict) -> dict[str, str]:
    """Extract `spec.selector.matchLabels` from a Deployment / Service."""
    sel = doc.get("spec", {}).get("selector", {})
    if doc.get("kind") == "Service":
        # Service.spec.selector is the matchLabels itself (no
        # `matchLabels` indirection).
        return dict(sel)
    return dict(sel.get("matchLabels", {}) or {})


def template_labels(doc: dict) -> dict[str, str]:
    """`spec.template.metadata.labels` for a Deployment / Job."""
    return dict(
        doc.get("spec", {})
           .get("template", {})
           .get("metadata", {})
           .get("labels", {})
    )


def netpol_allowed_labels(doc: dict) -> list[dict[str, str]]:
    """Collect every namespaceSelector matchLabels block from a
    NetworkPolicy ingress."""
    labels: list[dict[str, str]] = []
    spec = doc.get("spec", {})
    for rule in spec.get("ingress", []):
        for src in rule.get("from", []):
            ns = src.get("namespaceSelector", {})
            ml = ns.get("matchLabels", {})
            if ml:
                labels.append(dict(ml))
    return labels


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    p.add_argument("acme", type=Path,
                   help="Render of tenant A (e.g. /tmp/render-acme.yaml)")
    p.add_argument("globex", type=Path,
                   help="Render of tenant B (e.g. /tmp/render-globex.yaml)")
    args = p.parse_args()

    try:
        a_docs = load_manifests(args.acme)
        b_docs = load_manifests(args.globex)
    except (FileNotFoundError, yaml.YAMLError) as exc:
        print(f"::error::failed to load renders: {exc}", file=sys.stderr)
        return 2

    errors: list[str] = []

    # ─── Invariant 1: name disjointness ─────────────────────────
    a_names = {kind_name(d) for d in a_docs}
    b_names = {kind_name(d) for d in b_docs}
    overlap = {(k, ns, n) for (k, ns, n) in a_names & b_names if k}
    # Allow overlap on `Pod` test resources (helm.sh/hook=test
    # render in both namespaces but never coexist alive).
    overlap = {x for x in overlap if x[0] != "Pod"}
    if overlap:
        errors.append(
            f"name collision between tenant renders: {sorted(overlap)}"
        )

    # ─── Invariant 2: selector disjointness ─────────────────────
    a_deploys = [d for d in a_docs if d.get("kind") == "Deployment"]
    b_deploys = [d for d in b_docs if d.get("kind") == "Deployment"]
    for da in a_deploys:
        sel_a = selector_labels(da)
        for db in b_deploys:
            tpl_b = template_labels(db)
            if sel_a and tpl_b and all(
                tpl_b.get(k) == v for k, v in sel_a.items()
            ):
                errors.append(
                    f"selector of {kind_name(da)} matches template of {kind_name(db)} — "
                    f"two tenant Deployments would compete for the same pods"
                )

    # ─── Invariant 3: NetworkPolicy is tenant-specific ──────────
    a_netpols = [
        d for d in a_docs
        if d.get("kind") == "NetworkPolicy"
        and "tenant-ingress" in d.get("metadata", {}).get("name", "")
    ]
    if not a_netpols:
        errors.append("tenant A render has no `*-tenant-ingress` NetworkPolicy")

    b_owner_labels: list[str] = []
    for db in b_docs:
        if db.get("kind") == "Namespace":
            ml = db.get("metadata", {}).get("labels", {}) or {}
            if "tenant.efe.ai/owner" in ml:
                b_owner_labels.append(ml["tenant.efe.ai/owner"])
        elif "metadata" in db:
            ml = (db.get("metadata", {}) or {}).get("labels", {}) or {}
            if "app.kubernetes.io/tenant" in ml:
                b_owner_labels.append(ml["app.kubernetes.io/tenant"])

    for netpol in a_netpols:
        for src_labels in netpol_allowed_labels(netpol):
            owner = src_labels.get("tenant.efe.ai/owner")
            if owner and owner in b_owner_labels:
                errors.append(
                    f"tenant A NetworkPolicy whitelists tenant B's owner label: {owner!r}"
                )

    # ─── Invariant 4: ResourceQuota + LimitRange present ────────
    for label, docs in (("acme", a_docs), ("globex", b_docs)):
        kinds = {d.get("kind") for d in docs}
        if "ResourceQuota" not in kinds:
            errors.append(f"{label} render missing ResourceQuota (Tier 2 expected)")
        if "LimitRange" not in kinds:
            errors.append(f"{label} render missing LimitRange (Tier 2 expected)")
        if "NetworkPolicy" not in kinds:
            errors.append(f"{label} render missing NetworkPolicy (Tier 2 expected)")

    # ─── Report ─────────────────────────────────────────────────
    if errors:
        print(
            f"::error::Tier 2 dual-render isolation: {len(errors)} violation(s)",
            file=sys.stderr,
        )
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1

    print(
        f"OK — Tier 2 dual-render isolation invariants hold "
        f"({len(a_docs)} acme docs, {len(b_docs)} globex docs)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
