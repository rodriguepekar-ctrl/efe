"""Rollback runbook generator (M18 D49).

Produces a structured plan + textual command list for an operator to
copy-paste into their terminal. NEVER executes anything ; the MCP
server is text-only on the deploy side (M18 §2.1).

Design choices :
  - Pre-checks always include audit chain integrity verification
    (D14-bis carry-over) to surface pre-existing tamper signals
    before the rollback masks them.
  - Commands are emitted as Markdown-style code lines so the
    operator's terminal copy paste preserves spacing.
  - We never assume cluster credentials ; the operator runs commands
    under their own kubectl context.
"""
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class RollbackPlan:
    from_version: str
    to_version: str
    from_manifest_sha256: str | None
    to_manifest_sha256: str | None
    pre_checks: list[dict]
    rollback_commands: list[str]
    post_checks: list[str]
    estimated_duration_seconds: int
    blast_radius: str
    reason: str
    environment: str


def generate_plan(
    *,
    from_version: str,
    to_version: str,
    environment: str = "production",
    namespace: str = "efe-prod",
    release_name: str = "efe",
    reason: str = "",
    from_manifest_sha256: str | None = None,
    to_manifest_sha256: str | None = None,
    helm_history_lookup: bool = True,
) -> RollbackPlan:
    """Produce a step-by-step rollback runbook.

    `to_version` may be the literal string `"previous"` to indicate
    "the version immediately before from_version". The operator
    resolves the actual revision number via `helm history` (we surface
    the lookup as a pre-check).
    """
    pre_checks: list[dict] = [
        {
            "name": "verify_target_bundle_signature",
            "description": (
                "Run efe_verify_bundle_signature for the target version "
                "before rolling back ; do not trust an unsigned bundle "
                "even if it was once production."
            ),
            "command_hint": "(automatic) call efe_verify_bundle_signature",
        },
        {
            "name": "verify_audit_chain_intact",
            "description": (
                "Run efe_audit_health_check deep:true to ensure the D14-bis "
                "HMAC chain is intact BEFORE the rollback ; a broken chain "
                "may signal a pre-existing tamper that the rollback would "
                "obscure."
            ),
            "command_hint": "(automatic) call efe_audit_health_check deep:true",
        },
        {
            "name": "snapshot_current_state",
            "description": (
                "Capture cluster state BEFORE rollback for forensic record."
            ),
            "command_hint": (
                f"kubectl get all -n {namespace} -o yaml > /tmp/efe-pre-rollback.yaml"
            ),
        },
    ]

    if helm_history_lookup and to_version == "previous":
        rev_lookup_steps = [
            "# 1. Find the helm revision corresponding to the previous version",
            f"helm history {release_name} -n {namespace}",
            "",
            "# 2. Pick the most recent revision BEFORE the current one and",
            "#    rollback. Replace <REV> with the integer value.",
            f"helm rollback {release_name} <REV> -n {namespace}",
        ]
    else:
        rev_lookup_steps = [
            "# 1. Find the helm revision number for the target version",
            f"helm history {release_name} -n {namespace}",
            "",
            f"# 2. Pick the revision matching {to_version!r}, replace <REV> :",
            f"helm rollback {release_name} <REV> -n {namespace}",
        ]

    rollback_commands = [
        "# === Pre-flight (run automatic checks above first) ===",
        "",
        *rev_lookup_steps,
        "",
        "# 3. Wait for rollout completion + capture status",
        f"kubectl rollout status statefulset/{release_name} "
        f"-n {namespace} --timeout=10m",
        "",
        "# 4. Confirm the target version is now active",
        f"kubectl get pods -n {namespace} -l app.kubernetes.io/name={release_name}",
    ]

    post_checks = [
        "Run efe_get_deployment_status — confirm bundle.version equals the target",
        "Run efe_audit_health_check deep:true — confirm chain intact",
        (
            "For each catalog with reported drift > 0.05 in dry_run output, "
            "run efe_calibrate_thresholds (M17) to re-tune"
        ),
        "Open a post-mortem stub : reason captured below for traceability",
    ]

    blast_radius = f"single-namespace {namespace} ({environment})"

    return RollbackPlan(
        from_version=from_version,
        to_version=to_version,
        from_manifest_sha256=from_manifest_sha256,
        to_manifest_sha256=to_manifest_sha256,
        pre_checks=pre_checks,
        rollback_commands=rollback_commands,
        post_checks=post_checks,
        estimated_duration_seconds=300,        # 5 min default
        blast_radius=blast_radius,
        reason=reason,
        environment=environment,
    )
