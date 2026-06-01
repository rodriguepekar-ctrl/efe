"""Prompt template registry + safe variable substitution.

Templates use a tiny whitelisted syntax :
  {var}            simple substitution, required
  {var?}           simple substitution, optional (empty string if missing)
  {var?default}    optional with default value
  {#if var}…{/if}  conditional block (kept iff var was supplied)

No loops, no eval, no arbitrary expression — TM-M17-E-02 mitigation.
"""
from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class PromptArgument:
    name: str
    description: str
    required: bool = False

    def to_mcp_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "description": self.description,
            "required": self.required,
        }


@dataclass(frozen=True)
class PromptTemplate:
    name: str
    description: str
    arguments: tuple[PromptArgument, ...]
    body: str
    # Audit fix M18-FINAL-A3 (LOW) : `profile_tier` filters prompts
    # in `list_prompts()` so a prompt referencing tools the active
    # profile doesn't have is hidden from the LLM. Mid-workflow
    # tool-not-found errors are avoided.
    #
    # Tiers, in increasing capability :
    #   "advanced"            — visible in advanced+
    #   "agent-native"        — visible in agent-native+
    #   "agent-native-deploy" — visible only in deploy profile
    profile_tier: str = "advanced"

    def to_mcp_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "description": self.description,
            "arguments": [a.to_mcp_dict() for a in self.arguments],
        }


# ─────────────────────────────────────────────────────────────────────
# Template bodies (kept inline for portability — no FS lookup needed)
# ─────────────────────────────────────────────────────────────────────


_CLASSIFY_WITH_REASONING = """\
I want to classify the following text against the EFE catalog
**{catalog}** and understand why it lands where it does.

Text:
```
{text}
```

Please :
1. Call `efe_classify` with `catalog={catalog}` and `text={text}`.
2. Then call `efe_explain_classification` with the same args plus
   `predicted_class=<top1 from step 1>`.
3. Summarize the result in 3 bullets : (a) the predicted class,
   (b) the gap to runner-up, (c) any ambiguity signals or contributors
   worth flagging.
"""


_BOOTSTRAP_CATALOG = """\
I have a CSV file at `{csv_path}` containing one text per row.
{#if target_classes_hint}I'd like roughly {target_classes_hint} classes.{/if}

Please :
1. Read the CSV (you may use a file-reading tool of your choice).
2. Call `efe_suggest_classes` with the texts and `confirm:true`.
3. Show me the proposed classes ; let me approve or refine each one.
4. Once I approve, call `efe_register_catalog` with `confirm:true`.
5. Run `efe_diagnose_catalog` to confirm the catalog is healthy.
"""


_AUDIT_COMPLIANCE = """\
I need to verify the EFE audit log integrity for compliance purposes.

Please :
1. Call `efe_audit_health_check`{#if since} with `from_timestamp={since}`{/if}.
   Use `deep:true` for a full re-verification of the HMAC chain.
2. Call `efe_get_audit_log` with `limit:200` and report the action
   distribution (count by action type).
3. If `efe_audit_health_check` returns `ok:false`, print the
   `broken_at` event and recommend escalation per
   `docs/runbooks/m11_audit_chain_break.md`.
"""


_DRIFT_INVESTIGATION = """\
Class `{class}` in catalog `{catalog}` seems to be misclassifying.
Help me investigate :

1. Call `efe_diagnose_catalog` with `catalog={catalog}` and `depth:full`.
2. Call `efe_detect_drift` for `{catalog}` (latest vs prev version).
3. Call `efe_score_distribution` with `class_name:{class}` and
   `sample_count:200`.
4. Summarize : (a) overall health, (b) drift since last version,
   (c) recommended threshold, (d) ambiguity neighbors.
5. If overall_health < "fair", suggest concrete next actions
   (calibrate, evolve, etc.).
"""


_THRESHOLD_CALIBRATION = """\
Let's calibrate thresholds for catalog `{catalog}`.
Target precision : {target_precision?0.9}.

1. Call `efe_calibrate_thresholds` with `target_metric:precision` and
   `target_value:{target_precision?0.9}`.
2. Show me the per-class recommended thresholds with expected metrics.
3. For each class where `expected_recall < 0.6`, flag and suggest
   an alternative target_metric (e.g. `f1`).
4. When I approve, output the exact `efe_evolve_catalog` payload to
   apply the new thresholds (do NOT call evolve without my explicit
   confirmation).
"""


# ─────────────────────────────────────────────────────────────────────
# M18 D48 — bundle supply-chain prompts
# ─────────────────────────────────────────────────────────────────────


_VERIFY_BUNDLE_SUPPLY_CHAIN = """\
I want to fully audit the supply chain of EFE bundle
**{slug}:{version}**. Walk through this :

1. Call `efe_inspect_bundle` with `slug={slug}` and `version={version}`.
   Note the manifest sha256, signature status, SLSA level, and SBOM
   summary.
2. Call `efe_verify_bundle_signature` for the same bundle. The result
   MUST be `verified:true` AND `details.manifest_sha256_matches:true`
   for the bundle to be trusted.
3. Read resource `efe://bundles/{slug}/{version}/signature` to inspect
   the SLSA provenance attestation. Confirm :
     - `slsa_provenance.present` is true
     - `slsa_provenance.subject_digest_match` is true
     - `slsa_provenance.builder_trusted` is true
     - `slsa_provenance.level >= 2`
4. Call `efe_get_sbom slug={slug} version={version} filter_severity:HIGH`.
   List any HIGH or CRITICAL CVEs. If present, do NOT recommend deploy
   without remediation.
5. Summarise in 5 bullets : (a) signature verified, (b) SLSA level + builder,
   (c) high-severity CVEs, (d) overall recommendation
   (DEPLOY-READY | REVIEW-NEEDED | BLOCKED), (e) the next concrete
   action the operator should take.
"""


_AUDIT_BUNDLE_PROVENANCE = """\
Time for the bundle provenance audit.

Scope : `{slug?every bundle currently registered with the backend}`.
{#if version}Version filter : `{version}`.{/if}

For each bundle in scope :
1. `efe_verify_bundle_signature` — `verified:true` is required.
2. Read `efe://bundles/<slug>/<version>/signature` (or list each
   registered bundle via `efe://bundles` and iterate). Extract :
     - SLSA `level`
     - `builder` (URL)
     - `builder_trusted` (must be true)
     - `subject_digest_match` (must be true)
3. Confirm the `produced_by` annotation in the bundle manifest matches
   the `builder` in the SLSA attestation.
4. Output a markdown table with columns :
   `Bundle | Sig verified | SLSA level | Builder trusted | Subject match | Status`.
   Status = PASS only when all four checks are green.
5. Flag every row with Status != PASS for follow-up.
"""


_DEPLOY_NEW_BUNDLE = """\
I want to deploy bundle **{slug}:{version}** to the
{environment?production} environment. Walk me through the deploy
SAFELY. The MCP server NEVER executes kubectl/helm — I (the operator)
run the commands under my own credentials.

1. Pre-flight checks (must ALL pass) :
   a. `efe_check_bundle_compatibility slug={slug} version={version}` —
      MUST return `compatible:true`.
   b. `efe_verify_bundle_signature` — MUST return `verified:true`.
   c. Read `efe://bundles/<slug>/<version>/signature` — confirm SLSA
      `subject_digest_match:true` and `level >= 2`.
   d. `efe_dry_run_deploy slug={slug} version={version}` — review the
      diff and risks. Stop if `has_blockers:true`.

2. If pre-flight passes :
   a. `efe_generate_helm_values slug={slug} version={version}` —
      I'll save the `values_yaml` field to `values-mcp.yaml`.
   b. Output the EXACT commands the operator should run :
      ```
      helm upgrade efe ./charts/efe -f values-mcp.yaml --dry-run
      # review the kubectl diff manually
      helm upgrade efe ./charts/efe -f values-mcp.yaml
      kubectl rollout status statefulset/efe -n {environment?efe-prod} --timeout=10m
      ```
   c. Wait for me to confirm execution.

3. Post-deploy verification :
   a. `efe_get_deployment_status` — confirm `bundle.version == {version}`.
   b. `efe_audit_health_check deep:true` — confirm chain intact.
   c. For each catalog with reported drift > 0.05 in dry_run output,
      suggest `efe_calibrate_thresholds` (M17 tool).

If ANY pre-flight check fails or the dry_run reports a BLOCKER, STOP.
Do not proceed without explicit operator override.
"""


_ROLLBACK_TO_PREVIOUS_BUNDLE = """\
Emergency rollback requested for {environment?production}.
Reason : `{reason}`.

The MCP server NEVER runs helm/kubectl. I (the operator) execute the
commands ; you (the agent) plan + verify.

1. Capture state :
   a. `efe_get_deployment_history limit:5` — show me the last 5 deploys.
   b. `efe_get_deployment_status` — capture the current bundle as
      `from_version`.

2. Generate the runbook :
   `efe_generate_rollback_plan to_version:previous environment:{environment?production} reason:{reason}` —
   produce the runbook. Highlight the pre_checks in BOLD.

3. Wait for me to execute each step. After I confirm execution :

4. Post-rollback verification :
   a. `efe_get_deployment_status` — confirm `bundle.version` reverted.
   b. `efe_audit_health_check deep:true` — confirm chain intact.
   c. Read `efe://deployments/current` — confirm health is
      `healthy` or warn if degraded.

5. Open a post-mortem stub citing the reason : `{reason}`.
"""


_WEEKLY_BUNDLE_HEALTH_REPORT = """\
Generate the weekly bundle health report.

1. List bundles via `efe_list_available_bundles` (or read
   `efe://bundles` resource).
2. For each bundle, run `efe_inspect_bundle` and tabulate :
   - signature status (signed / verified)
   - SLSA level + builder_trusted
   - SBOM CVE counts by severity (LOW/MEDIUM/HIGH/CRITICAL)
   - manifest produced_at (age)
3. Read `efe://audit/recent?limit=200` to find the previous report
   (look for `mcp_call.weekly_bundle_health_report` events) and
   highlight what changed since :
     - new bundles registered
     - new HIGH/CRITICAL CVEs detected
     - signing status changes
     - SLSA level downgrades
4. Output a Markdown report with sections :
     ## Summary  ## New bundles  ## CVE deltas  ## Action items
   Ready to copy-paste in the team channel.
5. End with a one-line health score : ✅ green if 0 HIGH/CRITICAL CVEs
   AND all bundles are SLSA L≥2 ; ⚠️ yellow otherwise ; ❌ red if any
   bundle is unsigned or SLSA L0.
"""


PROMPT_DESCRIPTORS: tuple[PromptTemplate, ...] = (
    PromptTemplate(
        name="classify_with_reasoning",
        description="Classify a text against an EFE catalog and explain the result.",
        arguments=(
            PromptArgument("catalog", "EFE catalog name", required=True),
            PromptArgument("text", "Text to classify", required=True),
        ),
        body=_CLASSIFY_WITH_REASONING,
    ),
    PromptTemplate(
        name="bootstrap_catalog_from_csv",
        description="Walk a user from a raw CSV of texts to a registered EFE catalog.",
        arguments=(
            PromptArgument("csv_path", "Path to the CSV file", required=True),
            PromptArgument("target_classes_hint", "Hint for target class count", required=False),
        ),
        body=_BOOTSTRAP_CATALOG,
    ),
    PromptTemplate(
        name="audit_compliance_check",
        description="Quarterly compliance walkthrough of EFE audit logs.",
        arguments=(
            PromptArgument("tenant", "Tenant ID (informational)", required=False),
            PromptArgument("since", "ISO8601 timestamp lower bound", required=False),
        ),
        body=_AUDIT_COMPLIANCE,
    ),
    PromptTemplate(
        name="drift_investigation",
        description="Investigate why a class suddenly underperforms.",
        arguments=(
            PromptArgument("catalog", "EFE catalog name", required=True),
            PromptArgument("class", "Class name to investigate", required=True),
        ),
        body=_DRIFT_INVESTIGATION,
    ),
    PromptTemplate(
        name="threshold_calibration_session",
        description="Guided per-class threshold tuning.",
        arguments=(
            PromptArgument("catalog", "EFE catalog name", required=True),
            PromptArgument("target_precision", "Target precision in [0,1]", required=False),
        ),
        body=_THRESHOLD_CALIBRATION,
    ),
    # M18 D48 — bundle supply-chain prompts
    PromptTemplate(
        name="verify_bundle_supply_chain",
        description=(
            "Full supply-chain audit of a single bundle : signature + "
            "SLSA provenance + SBOM CVEs + recommendation."
        ),
        arguments=(
            PromptArgument("slug", "Bundle slug (kebab-case)", required=True),
            PromptArgument("version", "Bundle version (vX.Y.Z)", required=True),
        ),
        body=_VERIFY_BUNDLE_SUPPLY_CHAIN,
    ),
    PromptTemplate(
        name="audit_bundle_provenance",
        description=(
            "SLSA + signature ceremony : verify provenance attestations "
            "for one or all registered bundles."
        ),
        arguments=(
            PromptArgument("slug", "Optional bundle slug", required=False),
            PromptArgument("version", "Optional bundle version", required=False),
        ),
        body=_AUDIT_BUNDLE_PROVENANCE,
    ),
    PromptTemplate(
        name="weekly_bundle_health_report",
        description=(
            "Weekly periodic compliance + CVE delta report across all "
            "registered bundles."
        ),
        arguments=(),
        body=_WEEKLY_BUNDLE_HEALTH_REPORT,
    ),
    # M18 D49 — deployment workflow prompts
    PromptTemplate(
        name="deploy_new_bundle",
        description=(
            "Guided new-bundle deployment workflow : pre-flight, dry-run, "
            "Helm-values generation, post-deploy verification. The MCP "
            "server NEVER runs kubectl/helm ; the operator does. "
            "Profile: agent-native-deploy only."
        ),
        arguments=(
            PromptArgument("slug", "Bundle slug (kebab-case)", required=True),
            PromptArgument("version", "Bundle version (vX.Y.Z)", required=True),
            PromptArgument(
                "environment", "Target environment (default: production)",
                required=False,
            ),
        ),
        body=_DEPLOY_NEW_BUNDLE,
        # M18-FINAL-A3 fix : gated to agent-native-deploy because the
        # workflow references write tools (efe_request_deployment etc.)
        # that don't exist in agent-native.
        profile_tier="agent-native-deploy",
    ),
    PromptTemplate(
        name="rollback_to_previous_bundle",
        description=(
            "Emergency rollback walkthrough : capture state, generate "
            "rollback runbook, post-rollback verification. Commands "
            "only ; never executes."
        ),
        arguments=(
            PromptArgument("reason", "Why we are rolling back", required=True),
            PromptArgument(
                "environment", "Target environment (default: production)",
                required=False,
            ),
        ),
        body=_ROLLBACK_TO_PREVIOUS_BUNDLE,
    ),
)


_BY_NAME: dict[str, PromptTemplate] = {p.name: p for p in PROMPT_DESCRIPTORS}


# ─────────────────────────────────────────────────────────────────────
# Substitution
# ─────────────────────────────────────────────────────────────────────


_REQUIRED_RE = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)\}")
_OPTIONAL_RE = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)\?([^{}]*)\}")
_IF_RE = re.compile(
    r"\{#if\s+([A-Za-z_][A-Za-z0-9_]*)\}(.*?)\{/if\}",
    re.DOTALL,
)


def _escape(value: str) -> str:
    """Escape characters that could be interpreted as template syntax.

    We strip braces from user-supplied values to prevent injection of
    new template directives (TM-M17-E-02).
    """
    return value.replace("{", "").replace("}", "")


def render_prompt(name: str, args: dict[str, Any] | None = None) -> str:
    template = _BY_NAME.get(name)
    if template is None:
        raise KeyError(f"unknown prompt: {name!r}")
    args = dict(args or {})
    # Validate required args
    for a in template.arguments:
        if a.required and not args.get(a.name):
            raise ValueError(f"prompt {name!r} requires argument {a.name!r}")

    body = template.body

    # 1. Conditional blocks first (before substitution so they only see
    #    the original `args` set — not substituted text).
    def _cond(match: re.Match[str]) -> str:
        var = match.group(1)
        block = match.group(2)
        if var in args and args[var] not in (None, ""):
            return block
        return ""

    body = _IF_RE.sub(_cond, body)

    # 2. Optional `{var?default}` substitutions
    def _opt(match: re.Match[str]) -> str:
        var = match.group(1)
        default = match.group(2)
        v = args.get(var)
        if v is None or v == "":
            return _escape(default)
        return _escape(str(v))

    body = _OPTIONAL_RE.sub(_opt, body)

    # 3. Required `{var}` substitutions
    def _req(match: re.Match[str]) -> str:
        var = match.group(1)
        if var not in args or args[var] in (None, ""):
            raise ValueError(f"prompt {name!r} missing required {var!r} at render")
        return _escape(str(args[var]))

    body = _REQUIRED_RE.sub(_req, body)

    return body


def get_prompt(name: str) -> PromptTemplate | None:
    return _BY_NAME.get(name)
