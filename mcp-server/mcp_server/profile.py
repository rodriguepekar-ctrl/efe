"""Profile gating (M17 D43).

The mcp-server can run in 4 profiles selected via `EFE_MCP_TOOL_PROFILE`
environment variable. Each profile gates the set of tools (and resources
and prompts) exposed to the LLM client.

  - **core** (default) — exactly the M16 6 tools. Bit-for-bit unchanged
    behaviour, no resources, no prompts, no sampling, no streaming.
  - **advanced** — M16 + 8 tools (reasoning + discovery non-sampling +
    diagnostic + export). Resources + prompts enabled.
  - **agent-native** — full M17 = M16 + 14 tools, sampling + streaming
    capabilities active.
  - **experimental** — reserved for M18+ ; identical to agent-native
    today, but allows future opt-in features without env churn.

Loading the profile is a one-shot, executed at server boot. The
resulting `Profile` enum is stored on the dispatcher and consulted at
every `tools/list`, `resources/list`, `prompts/list` call.
"""
from __future__ import annotations

import os
from enum import Enum

# ─────────────────────────────────────────────────────────────────────
# Profile enumeration + tool gating
# ─────────────────────────────────────────────────────────────────────

# Profile names are case-insensitive on input ; canonical form is lower.


class Profile(str, Enum):
    CORE = "core"
    ADVANCED = "advanced"
    AGENT_NATIVE = "agent-native"
    AGENT_NATIVE_DEPLOY = "agent-native-deploy"
    EXPERIMENTAL = "experimental"


# M16 tools — always exposed, regardless of profile.
M16_CORE_TOOLS: frozenset[str] = frozenset({
    "efe_classify",
    "efe_register_catalog",
    "efe_evolve_catalog",
    "efe_list_catalogs",
    "efe_get_audit_log",
    "efe_get_admin_quotas",
})

# M17 Phase A + B — 8 additional tools that don't need sampling.
# Available in "advanced" and above.
M17_ADVANCED_TOOLS: frozenset[str] = frozenset({
    "efe_explain_classification",
    "efe_compare_classes",
    "efe_score_distribution",
    "efe_find_similar_examples",
    "efe_diagnose_catalog",
    "efe_detect_drift",
    "efe_audit_health_check",
    "efe_export_catalog",
})

# M17 Phase C + D — 6 tools that need sampling and/or streaming.
# Available only in "agent-native" and "experimental".
M17_AGENT_NATIVE_TOOLS: frozenset[str] = frozenset({
    "efe_suggest_classes",
    "efe_propose_examples",
    "efe_curate_catalog",
    "efe_calibrate_thresholds",
    "efe_consolidate_versions",
    "efe_bulk_classify",
})

# M18 Phase A (D47) — 5 bundle-introspection tools.
M18_BUNDLE_INTROSPECTION_TOOLS: frozenset[str] = frozenset({
    "efe_list_available_bundles",
    "efe_inspect_bundle",
    "efe_verify_bundle_signature",
    "efe_diff_bundles",
    "efe_check_bundle_compatibility",
})

# M18 Phase B (D48) — SBOM tool.
M18_BUNDLE_SBOM_TOOLS: frozenset[str] = frozenset({
    "efe_get_sbom",
})

# M18 Phase C (D49) — deployment helpers (read-only / text-gen).
M18_DEPLOYMENT_HELPER_TOOLS: frozenset[str] = frozenset({
    "efe_get_deployment_status",
    "efe_get_deployment_history",
    "efe_dry_run_deploy",
    "efe_generate_helm_values",
    "efe_generate_rollback_plan",
})

# M18 Phase D (D50) — deploy write tools.
# Available ONLY in `agent-native-deploy` profile (opt-in for ops tier).
# Each tool requires `confirm: true` (M16 carry-over confirm pattern)
# AND audits via the standard mcp_call.<tool> event chain (D14-bis).
M18_DEPLOY_WRITE_TOOLS: frozenset[str] = frozenset({
    "efe_register_bundle",
    "efe_promote_bundle",
    "efe_request_deployment",
})

# M20 — LLM-helper tools (read-only, sampling-delegated).
# Available in `agent-native` and above. ALL gated additionally by
# the `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` env-var (default false).
# Even when the profile would expose them, the env-var gate filters
# them out at registry build time so default-OFF is preserved
# bit-for-bit (cf. roadmap § F.7 + M20 architecture § 15).
M20_LLM_HELPER_TOOLS: frozenset[str] = frozenset({
    "efe_summarize_classification",
    "efe_compare_bundles_quality",
    "efe_generate_test_sentences",
    "efe_analyze_confusion_matrix",
    "efe_draft_runbook",
})

# M21 — Mode-2 LLM-routed anchor classification tools.
# Available in `agent-native` and above (write tools in
# `agent-native-deploy` only). ALL gated by env-var
# `EFE_FEATURE_M21_MODE2_ENABLED` (default false), AND by
# tenant-level + catalog-level + request-level flags (7-condition AND
# cf. roadmap § F.7 + M21 architecture § 18).
M21_MODE2_TOOLS: frozenset[str] = frozenset({
    "efe_classify_mode2",        # read, agent-native
    "efe_get_llm_budget",        # read, core
    "efe_list_llm_providers",    # read, advanced
    "efe_invalidate_llm_cache",  # write+confirm, agent-native
})

# M21 write-only set (requires agent-native-deploy profile).
M21_MODE2_WRITE_TOOLS: frozenset[str] = frozenset({
    "efe_set_llm_budget",        # write+confirm, agent-native-deploy
})

# M22 — Mode-3 multi-turn iterative reasoning tools.
# Available in `agent-native` and above. ALL gated by env-var
# `EFE_FEATURE_M22_MODE3_ENABLED` (default false), AND requires M21
# active. Compliance gate (Postgres trigger) blocks misconfig at DB
# level (cf. M22 architecture § 17 + roadmap § F.7).
M22_MODE3_TOOLS: frozenset[str] = frozenset({
    "efe_classify_mode3",        # read, agent-native
    "efe_get_mode3_reviews",     # read, agent-native
    "efe_approve_mode3_review",  # write+confirm, agent-native
    "efe_reject_mode3_review",   # write+confirm, agent-native
    "efe_get_concept_stack",     # read, agent-native
})

# D8 (Sprint GLOSS) — admin self-service tools used by `efe-admin`
# CLI and the portal-admin Next.js app. ALL gated by env-var
# `EFE_FEATURE_D8_ADMIN_ENABLED` (default false). Read tools exposed
# in `agent-native` ; write tools restricted to `agent-native-deploy`.
# Cf. `docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md` § 10
# + audit pass GLOSS-A (extended from 5 to 16 tools to cover the full
# CLI surface : tenant CRUD, key CRUD, compliance verify, etc.).
D8_ADMIN_READ_TOOLS: frozenset[str] = frozenset({
    "efe_get_tenant_health",     # read, agent-native
    "efe_list_tenants",          # read, agent-native
    "efe_get_tenant",            # read, agent-native
    "efe_verify_compliance",     # read, agent-native
    "efe_list_api_keys",         # read, agent-native
    "efe_export_audit_log",      # read, agent-native
})

D8_ADMIN_WRITE_TOOLS: frozenset[str] = frozenset({
    "efe_create_tenant",         # write+confirm, agent-native-deploy
    "efe_update_tenant",         # write+confirm, agent-native-deploy
    "efe_suspend_tenant",        # write+confirm, agent-native-deploy
    "efe_delete_tenant",         # write+confirm, agent-native-deploy
    "efe_seed_catalog",          # write+confirm, agent-native-deploy
    "efe_attest_compliance",     # write+confirm, agent-native-deploy
    "efe_create_api_key",        # write+confirm, agent-native-deploy
    "efe_rotate_api_key",        # write+confirm, agent-native-deploy
    "efe_revoke_api_key",        # write+confirm, agent-native-deploy
    "efe_set_quotas",            # write+confirm, agent-native-deploy
})

D8_ADMIN_TOOLS: frozenset[str] = D8_ADMIN_READ_TOOLS | D8_ADMIN_WRITE_TOOLS

# Aggregate of all M18 read-only tools currently shipped.
M18_BUNDLE_READONLY_TOOLS: frozenset[str] = (
    M18_BUNDLE_INTROSPECTION_TOOLS
    | M18_BUNDLE_SBOM_TOOLS
    | M18_DEPLOYMENT_HELPER_TOOLS
)

# ─────────────────────────────────────────────────────────────────────
# M23 — taxonomy curation tools (sub-package).
# Gated by `EFE_FEATURE_M23_CURATION_ENABLED`.
# ─────────────────────────────────────────────────────────────────────
M23_CURATION_TOOLS: frozenset[str] = frozenset({
    "efe_audit_bundle_vs_gold",
    "efe_explain_misclassification",
    "efe_generate_synthetic_examples",
    "efe_suggest_class_merge",
    "efe_propose_taxonomy_split",
})

# ─────────────────────────────────────────────────────────────────────
# M24 — active learning tools (sub-package).
# Gated by `EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED`.
# ─────────────────────────────────────────────────────────────────────
M24_ACTIVE_LEARNING_TOOLS: frozenset[str] = frozenset({
    "efe_list_gold_label_queue",
    "efe_confirm_gold_label",
    "efe_trigger_bundle_retune",
    "efe_request_gdpr_erasure",
})

# ─────────────────────────────────────────────────────────────────────
# M25 — agent runtime tools (sub-package, EXPERIMENTAL).
# Gated by `EFE_FEATURE_AGENT_RUNTIME_ENABLED` + per-tenant DPIA.
# ─────────────────────────────────────────────────────────────────────
M25_AGENT_RUNTIME_TOOLS: frozenset[str] = frozenset({
    "efe_create_agent_run",
    "efe_get_agent_run",
    "efe_approve_agent_step",
    "efe_terminate_agent_run",
})


PROFILE_TOOL_GATE: dict[Profile, frozenset[str]] = {
    Profile.CORE: M16_CORE_TOOLS,
    Profile.ADVANCED: M16_CORE_TOOLS | M17_ADVANCED_TOOLS,
    Profile.AGENT_NATIVE: (
        M16_CORE_TOOLS
        | M17_ADVANCED_TOOLS
        | M17_AGENT_NATIVE_TOOLS
        | M18_BUNDLE_READONLY_TOOLS
        | M20_LLM_HELPER_TOOLS  # gated by env-var, filtered in registry
        | M21_MODE2_TOOLS       # gated by env-var, filtered in registry
        | M22_MODE3_TOOLS       # gated by env-var, filtered in registry
        | D8_ADMIN_READ_TOOLS   # gated by env-var, filtered in registry
        | M23_CURATION_TOOLS    # gated by EFE_FEATURE_M23_CURATION_ENABLED
        | M24_ACTIVE_LEARNING_TOOLS  # gated by EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED
        # M25 not in agent-native — operator-only ; require deploy profile.
    ),
    Profile.AGENT_NATIVE_DEPLOY: (
        M16_CORE_TOOLS
        | M17_ADVANCED_TOOLS
        | M17_AGENT_NATIVE_TOOLS
        | M18_BUNDLE_READONLY_TOOLS
        | M18_DEPLOY_WRITE_TOOLS
        | M20_LLM_HELPER_TOOLS
        | M21_MODE2_TOOLS
        | M21_MODE2_WRITE_TOOLS
        | M22_MODE3_TOOLS
        | D8_ADMIN_TOOLS        # full D8 set in deploy profile
        | M23_CURATION_TOOLS
        | M24_ACTIVE_LEARNING_TOOLS
        | M25_AGENT_RUNTIME_TOOLS    # EXPERIMENTAL — gated by EFE_FEATURE_AGENT_RUNTIME_ENABLED
    ),
    Profile.EXPERIMENTAL: (
        M16_CORE_TOOLS
        | M17_ADVANCED_TOOLS
        | M17_AGENT_NATIVE_TOOLS
        | M18_BUNDLE_READONLY_TOOLS
        | M18_DEPLOY_WRITE_TOOLS
        | M20_LLM_HELPER_TOOLS
        | M21_MODE2_TOOLS
        | M21_MODE2_WRITE_TOOLS
        | M22_MODE3_TOOLS
        | D8_ADMIN_TOOLS
        | M23_CURATION_TOOLS
        | M24_ACTIVE_LEARNING_TOOLS
        | M25_AGENT_RUNTIME_TOOLS
    ),
}


# ─────────────────────────────────────────────────────────────────────
# Feature flags (M20+)
# ─────────────────────────────────────────────────────────────────────


def feature_enabled(name: str, *, default: bool = False) -> bool:
    """Resolve a server-level feature flag from environment.

    Convention : `EFE_FEATURE_<NAME>` env-var with values
    `true`/`false`/`1`/`0`. Anything else → `default`.

    Used by tools.types.Tool.feature_flag and the registry filter at
    boot. Cf. roadmap § F.3.2 for the full M20-M25 flag inventory.
    """
    raw = os.environ.get(f"EFE_FEATURE_{name.upper()}", "").strip().lower()
    if raw in ("true", "1", "yes", "on"):
        return True
    if raw in ("false", "0", "no", "off", ""):
        return False
    return default


# Resources + prompts are gated on the same threshold (advanced+).
RESOURCES_ENABLED_FOR: frozenset[Profile] = frozenset({
    Profile.ADVANCED,
    Profile.AGENT_NATIVE,
    Profile.AGENT_NATIVE_DEPLOY,
    Profile.EXPERIMENTAL,
})
PROMPTS_ENABLED_FOR: frozenset[Profile] = frozenset({
    Profile.ADVANCED,
    Profile.AGENT_NATIVE,
    Profile.AGENT_NATIVE_DEPLOY,
    Profile.EXPERIMENTAL,
})
SAMPLING_ENABLED_FOR: frozenset[Profile] = frozenset({
    Profile.AGENT_NATIVE,
    Profile.AGENT_NATIVE_DEPLOY,
    Profile.EXPERIMENTAL,
})
STREAMING_ENABLED_FOR: frozenset[Profile] = frozenset({
    Profile.AGENT_NATIVE,
    Profile.AGENT_NATIVE_DEPLOY,
    Profile.EXPERIMENTAL,
})


# ─────────────────────────────────────────────────────────────────────
# Loading
# ─────────────────────────────────────────────────────────────────────


def load_profile_from_env(*, env_var: str = "EFE_MCP_TOOL_PROFILE") -> Profile:
    """Resolve the active profile from environment.

    Defaults to `Profile.CORE` to preserve M16 behaviour for legacy
    deployments. Unknown values raise so misconfiguration is loud.
    """
    raw = os.environ.get(env_var, "").strip().lower()
    if not raw:
        return Profile.CORE
    try:
        return Profile(raw)
    except ValueError:
        valid = ", ".join(sorted(p.value for p in Profile))
        raise RuntimeError(
            f"unknown {env_var}={raw!r} ; expected one of: {valid}"
        )


def tools_for(profile: Profile) -> frozenset[str]:
    return PROFILE_TOOL_GATE[profile]


def resources_enabled(profile: Profile) -> bool:
    return profile in RESOURCES_ENABLED_FOR


def prompts_enabled(profile: Profile) -> bool:
    return profile in PROMPTS_ENABLED_FOR


def sampling_enabled(profile: Profile) -> bool:
    return profile in SAMPLING_ENABLED_FOR


def streaming_enabled(profile: Profile) -> bool:
    return profile in STREAMING_ENABLED_FOR
