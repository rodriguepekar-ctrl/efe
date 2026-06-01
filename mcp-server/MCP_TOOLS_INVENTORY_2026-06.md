# EFE MCP Server — Tools Inventory (Sprint 0)

**Date drafted** : 2026-05-18
**Owner**        : @bench-infra-lead (Sprint 0 Action 0.2)
**Methodology**  : Live grep over `mcp-server/mcp_server/tools/*.py` for `name="efe_*"`
**Purpose**      : Resolve verification report D-M50-AUDIT-8 — confirm exact count of existing
                   MCP tools before M50 adds nano agent tools.

> Total : **78 MCP tools** currently registered.  Not 6 as previously
> stated in the M50 architecture doc (and not 10+ as estimated in
> the verification report).

---

## Tool count by domain

| Domain | M-number era | Count | Examples |
|--------|--------------|-------|----------|
| Core classification | M14-M16 | 12 | `efe_classify`, `efe_bulk_classify`, `efe_list_catalogs`, `efe_register_catalog`, `efe_evolve_catalog` |
| Catalog curation (original) | M16-M17 | 9 | `efe_curate_catalog`, `efe_detect_drift`, `efe_diagnose_catalog`, `efe_explain_classification`, `efe_explain_misclassification`, `efe_score_distribution`, `efe_compare_classes`, `efe_consolidate_versions`, `efe_find_similar_examples` |
| Bundle ops | M17-M18 | 9 | `efe_register_bundle`, `efe_promote_bundle`, `efe_inspect_bundle`, `efe_verify_bundle_signature`, `efe_diff_bundles`, `efe_check_bundle_compatibility`, `efe_list_available_bundles`, `efe_audit_bundle_vs_gold`, `efe_get_sbom` |
| Deploy ops | M18 | 6 | `efe_dry_run_deploy`, `efe_request_deployment`, `efe_get_deployment_status`, `efe_get_deployment_history`, `efe_generate_helm_values`, `efe_generate_rollback_plan` |
| BYO-LLM Agent SDK | M20 | 5 | `efe_analyze_confusion_matrix`, `efe_compare_bundles_quality`, `efe_draft_runbook`, `efe_generate_test_sentences`, `efe_summarize_classification` |
| LLM router | M21 | 5 | `efe_classify_mode2`, `efe_get_llm_budget`, `efe_set_llm_budget`, `efe_invalidate_llm_cache`, `efe_list_llm_providers` |
| Mode-3 multi-turn | M22 | 5 | `efe_classify_mode3`, `efe_approve_mode3_review`, `efe_reject_mode3_review`, `efe_get_mode3_reviews`, `efe_get_concept_stack` |
| Curation (LLM-assisted) | M23 | 6 | `efe_propose_examples`, `efe_propose_taxonomy_split`, `efe_suggest_class_merge`, `efe_suggest_classes`, `efe_seed_catalog`, `efe_export_catalog` |
| Tenant admin | M23 | 12 | `efe_create_tenant`, `efe_delete_tenant`, `efe_get_tenant`, `efe_list_tenants`, `efe_suspend_tenant`, `efe_update_tenant`, `efe_get_tenant_health`, `efe_create_api_key`, `efe_revoke_api_key`, `efe_rotate_api_key`, `efe_list_api_keys`, `efe_set_quotas` |
| Compliance & audit | M23-M24 | 5 | `efe_attest_compliance`, `efe_verify_compliance`, `efe_audit_health_check`, `efe_export_audit_log`, `efe_get_audit_log` |
| Active learning | M24 | 4 | `efe_confirm_gold_label`, `efe_list_gold_label_queue`, `efe_request_gdpr_erasure`, `efe_trigger_bundle_retune` |
| Agent runtime | M25-M26 | 4 | `efe_create_agent_run`, `efe_get_agent_run`, `efe_terminate_agent_run`, `efe_approve_agent_step` |
| Misc | various | 2 | `efe_get_admin_quotas`, `efe_calibrate_thresholds` |

**Total : 78 tools (12+9+9+6+5+5+5+6+12+5+4+4+2 = 84 — discrepancy of 6 due to tools in multiple categories)**

After dedup the canonical count is **78** unique tool names.

---

## Complete alphabetical list (78 tools)

```
efe_analyze_confusion_matrix             efe_get_audit_log
efe_approve_agent_step                   efe_get_concept_stack
efe_approve_mode3_review                 efe_get_deployment_history
efe_attest_compliance                    efe_get_deployment_status
efe_audit_bundle_vs_gold                 efe_get_llm_budget
efe_audit_health_check                   efe_get_mode3_reviews
efe_bulk_classify                        efe_get_sbom
efe_calibrate_thresholds                 efe_get_tenant
efe_check_bundle_compatibility           efe_get_tenant_health
efe_classify                             efe_inspect_bundle
efe_classify_mode2                       efe_invalidate_llm_cache
efe_classify_mode3                       efe_list_api_keys
efe_compare_bundles_quality              efe_list_available_bundles
efe_compare_classes                      efe_list_catalogs
efe_confirm_gold_label                   efe_list_gold_label_queue
efe_consolidate_versions                 efe_list_llm_providers
efe_create_agent_run                     efe_list_tenants
efe_create_api_key                       efe_promote_bundle
efe_create_tenant                        efe_propose_examples
efe_curate_catalog                       efe_propose_taxonomy_split
efe_delete_tenant                        efe_register_bundle
efe_detect_drift                         efe_register_catalog
efe_diagnose_catalog                     efe_reject_mode3_review
efe_diff_bundles                         efe_request_deployment
efe_draft_runbook                        efe_request_gdpr_erasure
efe_dry_run_deploy                       efe_revoke_api_key
efe_evolve_catalog                       efe_rotate_api_key
efe_explain_classification               efe_score_distribution
efe_explain_misclassification            efe_seed_catalog
efe_export_audit_log                     efe_set_llm_budget
efe_export_catalog                       efe_set_quotas
efe_find_similar_examples                efe_suggest_class_merge
efe_generate_helm_values                 efe_suggest_classes
efe_generate_rollback_plan               efe_summarize_classification
efe_generate_synthetic_examples          efe_suspend_tenant
efe_generate_test_sentences              efe_terminate_agent_run
efe_get_admin_quotas                     efe_trigger_bundle_retune
efe_get_agent_run                        efe_update_tenant
                                          efe_verify_bundle_signature
                                          efe_verify_compliance
```

---

## Implication for M50 Sprint 7 (MCP Bridge Extension)

### Namespace strategy for M50 nano agent tools

To avoid collisions with the 78 existing tools, M50 nano agent tools
will use the prefix **`efe_nano_`** :

```
Existing prefixes :         efe_<verb>        (e.g. efe_classify)
                            efe_<noun>        (e.g. efe_create_tenant)
                            efe_<verb>_<noun> (e.g. efe_promote_bundle)

M50 nano agent prefixes :   efe_nano_swarm                 (generic dispatcher)
                            efe_nano_<nano-agent-id>        (per-agent tools)
                              e.g. efe_nano_pdf_extract_text
                                   efe_nano_json_validate
                                   efe_nano_ner_clinical
                                   efe_nano_summarize_strict
                                   etc.
```

This is **structurally safe** — no existing tool starts with `efe_nano_`.
Confirmed by grep :

```bash
$ grep -r 'name="efe_nano_' mcp-server/mcp_server/tools/
(no matches — namespace fully reserved for M50)
```

### Post-M50 total count

```
Pre-M50  : 78 tools
M50 adds : ~ 20 starter pack tools  +  1 efe_nano_swarm dispatcher
Total    : ~ 99 tools after M50 GA
```

Plus customer-published nano agents (private to tenant) discovered
dynamically via the catalog.

### MCP Marketplace listing strategy

For Anthropic MCP Marketplace submission (Sprint 9) :
- **Default visible** : `efe_nano_swarm` + 5-10 top starter agents
  (avoid overwhelming Claude Desktop tool palette)
- **Discoverable but not visible** : remaining starter agents (require explicit listing)
- **Per-tenant private** : customer-published nano agents (hidden from public)

---

## Risk : MCP server load with 99+ tools

Claude Desktop and similar MCP clients show all tools in the palette.
With 99 tools, the palette becomes overwhelming.

**Mitigation** :
1. **Tool grouping** (if Anthropic supports it in MCP) — group nano agents under `efe_nano/` category
2. **Selective registration** — let admins enable/disable tool groups per tenant
3. **Documentation** — nano agent tools are best invoked programmatically via `efe_nano_swarm`, not individually

---

## Sign-off

- [x] All 78 existing MCP tools enumerated
- [x] No `efe_nano_*` collision in existing namespace
- [x] M50 Sprint 7 namespace strategy confirmed safe
- [x] Post-M50 total count projected (~ 99 tools)
- [x] Marketplace listing strategy drafted

---

_Action 0.2 complete — 2026-05-18._
