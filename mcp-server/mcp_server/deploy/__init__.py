"""Deployment helpers (M18 D49).

Pure-logic helpers for the M18 D49 deployment-helper tools :
  - `helm_values.render(input)` : produce a Helm `values.yaml` body
    from a typed dataclass. Whitelist-only sections (TM-M18-E-02).
  - `rollback.generate_plan(...)` : produce a copy-paste rollback
    runbook (commands as text — never executed).
  - `dry_run.compute_diff(...)` : compute the semantic diff between
    a current deployment and a target bundle.

NEVER invokes subprocess / kubectl / helm / cosign. The MCP server
remains a pure-Python read+text-generation boundary (M18 §2.1).
"""

from .dry_run import (
    DeploymentDiff,
    DryRunRisk,
    compute_diff,
    estimate_blast_radius,
)
from .helm_values import (
    ALLOWED_HELM_SECTIONS,
    HelmValuesInput,
    render as render_helm_values,
)
from .rollback import RollbackPlan, generate_plan as generate_rollback_plan

__all__ = [
    # helm_values
    "HelmValuesInput",
    "render_helm_values",
    "ALLOWED_HELM_SECTIONS",
    # rollback
    "RollbackPlan",
    "generate_rollback_plan",
    # dry_run
    "DeploymentDiff",
    "DryRunRisk",
    "compute_diff",
    "estimate_blast_radius",
]
