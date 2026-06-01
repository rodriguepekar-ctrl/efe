"""Deployment dry-run diff computation (M18 D49).

Computes the semantic diff between the currently-deployed bundle and a
target bundle ; flags risks (LOW/MEDIUM/HIGH/BLOCKER) and surfaces a
recommended action list. Pure-Python ; no kubectl/helm/cosign.

This module is BACKEND-AGNOSTIC : it operates on dicts produced by
either the EFE 1.2 `/admin/deployments/dry-run` endpoint OR by local
composition (current + target bundle metadata).
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


# Risk severity ordering used for `highest_risk_level` summary.
_RISK_SEVERITIES = ("LOW", "MEDIUM", "HIGH", "BLOCKER")


@dataclass(frozen=True)
class DryRunRisk:
    level: str          # LOW | MEDIUM | HIGH | BLOCKER
    category: str       # centroid_drift | dim_change | unsigned_bundle | …
    message: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "level": self.level,
            "category": self.category,
            "message": self.message,
        }


@dataclass(frozen=True)
class DeploymentDiff:
    current_slug: str | None
    current_version: str | None
    current_manifest_sha256: str | None
    target_slug: str
    target_version: str
    target_manifest_sha256: str | None
    manifest_changed: bool
    model_name_changed: bool
    dim_changed: bool
    framework_changed: bool
    expected_replica_restarts: int
    expected_downtime_seconds_p99: int
    risks: list[DryRunRisk] = field(default_factory=list)
    blocking_issues: list[str] = field(default_factory=list)
    recommended_actions: list[str] = field(default_factory=list)

    @property
    def has_blockers(self) -> bool:
        return bool(self.blocking_issues) or any(
            r.level == "BLOCKER" for r in self.risks
        )

    @property
    def highest_risk_level(self) -> str:
        if not self.risks:
            return "LOW"
        # Pick the highest level present.
        for sev in reversed(_RISK_SEVERITIES):
            if any(r.level == sev for r in self.risks):
                return sev
        return "LOW"

    def to_dict(self) -> dict[str, Any]:
        return {
            "current": {
                "bundle": {
                    "slug": self.current_slug,
                    "version": self.current_version,
                },
                "manifest_sha256": self.current_manifest_sha256,
            },
            "target": {
                "bundle": {
                    "slug": self.target_slug,
                    "version": self.target_version,
                },
                "manifest_sha256": self.target_manifest_sha256,
            },
            "diff": {
                "manifest_changed": self.manifest_changed,
                "model_name_changed": self.model_name_changed,
                "dim_changed": self.dim_changed,
                "framework_changed": self.framework_changed,
                "expected_replica_restarts": self.expected_replica_restarts,
                "expected_downtime_seconds_p99": self.expected_downtime_seconds_p99,
            },
            "risks": [r.to_dict() for r in self.risks],
            "blocking_issues": list(self.blocking_issues),
            "recommended_actions": list(self.recommended_actions),
            "highest_risk_level": self.highest_risk_level,
            "has_blockers": self.has_blockers,
        }


def compute_diff(
    *,
    current: dict[str, Any] | None,
    target_manifest: dict[str, Any],
    target_signature_verified: bool | None = None,
    target_signature_unverifiable: bool = False,
    desired_replicas: int = 3,
    rolling_update: bool = True,
) -> DeploymentDiff:
    """Compute the diff between the current deployment and a target bundle.

    Parameters
    ----------
    current
        The `/admin/deployments/current` payload (or None when the
        backend can't tell us — in that case the diff is target-only).
    target_manifest
        The manifest dict for the target bundle (as returned by
        `efe_inspect_bundle`'s `manifest` field).
    target_signature_verified
        Tri-state : True / False / None.
          - True : signature checked out cryptographically
          - False : signature actually FAILED verification (BLOCKER)
          - None : verification not attempted / unknown (MEDIUM)
    target_signature_unverifiable
        Audit fix M18-D49-A7 (HIGH) : when True, the signature is
        unverifiable (e.g. trust store empty, cryptography extra
        missing, bundle has no signature artifact at all). This is
        treated as a MEDIUM risk (unknown), NOT a BLOCKER, to avoid
        false-positives that block legitimate deploys when the
        operator hasn't configured `EFE_OCI_PUBLIC_KEY` yet.
    desired_replicas
        Used to compute `expected_replica_restarts`.
    rolling_update
        When True, downtime estimate is 0 (rolling).
    """
    cur_bundle = (current or {}).get("bundle") or {}
    cur_manifest = (current or {}).get("manifest") or {}

    cur_slug = cur_bundle.get("slug")
    cur_ver = cur_bundle.get("version")
    cur_sha = cur_bundle.get("manifest_sha256") or cur_manifest.get("manifestSha256")

    target_slug = (
        target_manifest.get("slug")
        or target_manifest.get("name")
        or "<target>"
    )
    target_ver = (
        target_manifest.get("version")
        or "<target>"
    )
    target_sha = target_manifest.get("manifestSha256") or target_manifest.get(
        "manifest_sha256"
    )

    manifest_changed = (cur_sha != target_sha) if cur_sha else True

    cur_model = cur_manifest.get("model_name") or cur_manifest.get("modelName")
    target_model = target_manifest.get("model_name") or target_manifest.get(
        "modelName"
    )
    model_name_changed = (
        manifest_changed and cur_model is not None and cur_model != target_model
    )

    cur_dim = cur_manifest.get("dim")
    target_dim = target_manifest.get("dim")
    dim_changed = (
        cur_dim is not None and target_dim is not None and cur_dim != target_dim
    )

    cur_fw = cur_manifest.get("framework")
    target_fw = target_manifest.get("framework")
    framework_changed = (
        cur_fw is not None and target_fw is not None and cur_fw != target_fw
    )

    expected_replica_restarts = int(desired_replicas) if manifest_changed else 0
    if rolling_update:
        expected_downtime_seconds_p99 = 0
    else:
        # 30 seconds per replica for recreate strategy is a coarse
        # heuristic (real values come from D49 audit pass benchmarks).
        expected_downtime_seconds_p99 = expected_replica_restarts * 30

    risks: list[DryRunRisk] = []
    blocking: list[str] = []
    actions: list[str] = []

    if dim_changed:
        risks.append(DryRunRisk(
            level="BLOCKER",
            category="dim_change",
            message=(
                f"Bundle dim changed from {cur_dim} to {target_dim} ; "
                "all classify calls will fail until catalogs are re-encoded. "
                "DO NOT DEPLOY without a re-encode plan."
            ),
        ))
        blocking.append(
            f"dim mismatch: current={cur_dim}, target={target_dim}"
        )

    if framework_changed:
        risks.append(DryRunRisk(
            level="HIGH",
            category="framework_change",
            message=(
                f"Bundle framework changed from {cur_fw!r} to {target_fw!r}. "
                "Centroids will need to be recomputed against the new "
                "embedder. Run efe_calibrate_thresholds post-deploy."
            ),
        ))
        actions.append(
            "Plan to run efe_calibrate_thresholds for every catalog "
            "after the deployment."
        )

    if model_name_changed:
        risks.append(DryRunRisk(
            level="MEDIUM",
            category="model_name_change",
            message=(
                f"Model name changed from {cur_model!r} to {target_model!r}. "
                "Embedding distribution likely shifts ; expect centroid drift."
            ),
        ))

    target_dev_unsigned = bool(target_manifest.get("is_dev_unsigned"))
    if target_dev_unsigned:
        risks.append(DryRunRisk(
            level="BLOCKER",
            category="unsigned_bundle",
            message=(
                "Target bundle is marked is_dev_unsigned=true. "
                "Production deploy is blocked until a signed release exists."
            ),
        ))
        blocking.append("target bundle is is_dev_unsigned=true")

    # Audit fix M18-D49-A7 (HIGH) : tri-state signature handling.
    # Pre-fix, `verified=False` from inspect_bundle could mean ANY of :
    # (a) actual cryptographic failure, (b) trust store unavailable,
    # (c) cryptography lib missing, (d) no signature artifact present.
    # Only (a) is a BLOCKER ; (b), (c), (d) are MEDIUM "unknown" risks.
    if target_signature_verified is True:
        # OK — no signature risk.
        pass
    elif target_signature_unverifiable:
        risks.append(DryRunRisk(
            level="MEDIUM",
            category="signature_unverifiable",
            message=(
                "Target signature verification could not be performed "
                "(trust store unavailable, cryptography extra missing, "
                "or no signature artifact). Run "
                "efe_verify_bundle_signature manually after configuring "
                "EFE_OCI_PUBLIC_KEY{,_FILE}."
            ),
        ))
        actions.append(
            "Configure EFE_OCI_PUBLIC_KEY{,_FILE} and re-run "
            "efe_verify_bundle_signature for the target bundle."
        )
    elif target_signature_verified is False:
        # Cryptographic failure — actual bad signature.
        risks.append(DryRunRisk(
            level="BLOCKER",
            category="signature_unverified",
            message=(
                "Target bundle Ed25519 signature did NOT verify against "
                "the trusted public key. Refusing to plan a deploy."
            ),
        ))
        blocking.append("target signature unverified")
    else:
        # target_signature_verified is None → unknown
        risks.append(DryRunRisk(
            level="MEDIUM",
            category="signature_unknown",
            message=(
                "Target signature verification status is unknown. Run "
                "efe_verify_bundle_signature manually before deploying."
            ),
        ))
        actions.append(
            "Run efe_verify_bundle_signature for the target bundle "
            "before any apply."
        )

    if not manifest_changed:
        actions.append(
            "Manifest sha256 is identical to current ; the deploy is a "
            "no-op. Skip the helm upgrade."
        )
    else:
        actions.insert(
            0,
            "1. Run efe_verify_bundle_signature for the target bundle.",
        )
        actions.insert(
            1,
            "2. Run efe_check_bundle_compatibility against the live server.",
        )
        actions.append(
            "Generate Helm values via efe_generate_helm_values + apply "
            "manually with `helm upgrade -f values-mcp.yaml`."
        )
        actions.append(
            "Post-deploy : efe_get_deployment_status to confirm + "
            "efe_audit_health_check deep:true."
        )

    return DeploymentDiff(
        current_slug=cur_slug,
        current_version=cur_ver,
        current_manifest_sha256=cur_sha,
        target_slug=str(target_slug),
        target_version=str(target_ver),
        target_manifest_sha256=target_sha,
        manifest_changed=manifest_changed,
        model_name_changed=model_name_changed,
        dim_changed=dim_changed,
        framework_changed=framework_changed,
        expected_replica_restarts=expected_replica_restarts,
        expected_downtime_seconds_p99=expected_downtime_seconds_p99,
        risks=risks,
        blocking_issues=blocking,
        recommended_actions=actions,
    )


def estimate_blast_radius(*, namespace: str, environment: str) -> str:
    return f"single-namespace {namespace} ({environment})"
