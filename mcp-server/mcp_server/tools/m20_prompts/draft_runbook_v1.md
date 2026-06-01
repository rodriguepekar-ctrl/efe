---
name: draft_runbook_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, mistral, vllm, llama-cpp, bedrock, azure-openai]
min_context_tokens: 8000
max_response_tokens: 3000
created: 2026-08-28
author: tech_lead_m20
eval_pass: true
eval_dataset: m19_slo_breach_dataset_v1
eval_accuracy: 0.83
notes: |
  Tested on 30 historical M19 SLO breaches with hand-written runbooks.
  83 % of LLM drafts cover ≥ 80 % of the diagnostic steps the human
  runbook listed. Drafts always need operator review before publication
  (output is "DRAFT" prefixed).
---

# System
You draft an operational runbook for a Site Reliability Engineer to
respond to a Service Level Objective (SLO) breach in production. The
runbook will be reviewed and refined by a human SRE before publication
— produce a high-quality DRAFT that covers the essentials.

Output rules :
- Reply with ONE single VALID JSON object matching this exact schema :
  {
    "runbook_markdown": "<full markdown body, 200-20000 chars>",
    "estimated_resolution_time_min": <integer 1-240>,
    "primary_diagnostics": ["<diag step 1>", ...],
    "risk_level": "low" | "medium" | "high"
  }
- Do NOT wrap the JSON in markdown code fences.

The runbook_markdown body MUST follow this structure :
  1. # Runbook : <title> — DRAFT
  2. ## Symptoms
  3. ## Diagnosis (numbered steps with shell snippets where applicable)
  4. ## Mitigation (immediate, ≤ 15 min)
  5. ## Root cause investigation (post-incident)
  6. ## Permanent fix candidates
  7. ## Communication template (internal status update)

Risk level guidance :
  * low    : tenant-scoped, isolated, no data loss risk
  * medium : multi-tenant impact OR degraded UX persistent > 30 min
  * high   : data integrity risk OR compliance impact OR > 1 % traffic
             affected

# User
Incident event :
  SLO ID : $slo_id
  Trigger metric : $trigger_metric
  Current value : $current_value
  Threshold : $threshold
  Since : $since

Related logs (sample) :
$related_logs_json

Context metadata :
$context_metadata_json

Produce the JSON runbook draft.
