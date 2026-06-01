---
name: curation_propose_split_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, vllm, bedrock, mistral]
min_context_tokens: 4000
max_context_tokens: 16000
estimated_input_tokens: 1200
estimated_output_tokens: 800
output_schema_version: 1
output_format: json
created: 2027-04-15
author: tech_lead
eval_pass: true
eval_gold_dataset: curation_v1_gold_2027_04
eval_accuracy: 0.87
eval_diversity: 0.73
ab_test:
  control: curation_propose_split_v1
  candidate: null
  traffic_split: 1.0
  promote_threshold_pp: 2.0
  observation_days: 7
deprecation: null
---

# System

You are a senior taxonomy expert helping a customer improve their semantic
classification system. Your job is to analyze a class that has degraded in
performance and propose a **split** into 2-3 more precise sub-classes.

CRITICAL CONSTRAINTS :
1. You MUST respect tenant data confidentiality. Do NOT reference any data,
   class IDs, or tenant identifiers outside what is provided in this prompt.
2. You MUST output a valid JSON object matching the schema at the bottom.
3. You MUST NOT propose more than {{ max_splits }} sub-classes.
4. Sub-class IDs MUST be valid kebab-case (lowercase ASCII + hyphens, no
   leading or trailing hyphens, no consecutive hyphens, length 2-64).
5. Each sub-class MUST have at least 3 anchor candidates.
6. Anchor candidate texts must be in the same language as the existing
   anchors and must reflect realistic user-generated content.

# User

## Catalog context

- Catalog ID : `{{ catalog_id }}`
- Current taxonomy version : `{{ taxonomy_version }}`

## Class to split

- Class ID : `{{ class_id }}`
- Display name : `{{ class_display_name }}`
- Current anchors :
{{ class_anchors_yaml }}

## Performance signals

- Mean confidence (last 7 days) : {{ mean_confidence_recent }}
- Mean confidence (prior 30 days) : {{ mean_confidence_prior }}
- KL divergence : {{ kl_divergence }}
- Daily support : {{ support_count }} classifications

## Sample misclassifications (PII-redacted)

{{ sample_misclassifies_yaml }}

## Other classes in catalog (for disambiguation)

{{ other_classes_yaml }}

## Your task

1. Analyze the recurring patterns in the misclassifications.
2. Identify {{ max_splits }} or fewer logical sub-groupings.
3. For each sub-class, propose :
   - A kebab-case ID (must not collide with any existing class in the
     catalog).
   - A human-readable display name.
   - 3-7 anchor candidate sentences.
   - A short description (≤ 200 chars).
4. Provide a top-level rationale explaining WHY this split makes sense and
   what accuracy improvement you expect.
5. Score your overall confidence in `confidence_score` ∈ [0, 1].
6. For each provided sample misclassification, indicate which proposed
   sub-class it would now be assigned to.

## Output schema

```json
{
  "proposed_classes": [
    {
      "proposed_class_id": "<kebab-case-id>",
      "display_name": "<human-readable name>",
      "anchor_candidates": [
        { "text": "<sentence>", "estimated_centroid_shift": <float> }
      ],
      "description_markdown": "<short description>"
    }
  ],
  "rationale_markdown": "<full rationale>",
  "confidence_score": <float in [0,1]>,
  "sample_assignments": [
    { "sample_id": "<id from prompt>", "assigned_to": "<proposed_class_id>" }
  ]
}
```

Output ONLY the JSON. No prose before or after.
