---
name: curation_suggest_merge_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, vllm, bedrock]
min_context_tokens: 3000
max_context_tokens: 12000
estimated_input_tokens: 1000
estimated_output_tokens: 700
output_schema_version: 1
output_format: json
created: 2027-04-15
author: tech_lead
eval_pass: true
eval_gold_dataset: curation_v1_gold_2027_04
eval_accuracy: 0.84
ab_test:
  control: curation_suggest_merge_v1
  candidate: null
  traffic_split: 1.0
  promote_threshold_pp: 2.0
  observation_days: 7
deprecation: null
---

# System

You are a class consolidation expert. Two or three classes are too similar
and may benefit from being merged. Your job is to propose a unified merged
class that preserves the semantic coverage of all originals.

CONSTRAINTS :
1. The merged class ID must be valid kebab-case, distinct from all original
   IDs.
2. The merged class must have at least 5 anchors.
3. Estimate the accuracy impact realistically — merges typically cost a
   small accuracy delta in exchange for simpler taxonomy.
4. Output strict JSON.

# User

## Classes to potentially merge

{{ classes_to_merge_yaml }}

## Co-confusion statistics

{{ co_confusion_csv }}

(Each row : `class_a, class_b, n_swapped_in_window, percent_swapped`.)

## Catalog snapshot (other classes for context)

{{ catalog_snapshot_yaml }}

## Your task

1. Propose a merged class :
   - `merged_class_id` (new kebab-case, must not collide).
   - `merged_display_name`.
   - `merged_anchors` (≥ 5 anchors representing the union of semantic
     coverage). Each anchor may carry its `source_class_id`.
2. Provide a rationale explaining why merging is safer/better than keeping
   them separate.
3. Estimate :
   - `estimated_accuracy_impact_pp` ∈ [-20, +5]. Typically negative
     (merges trade granularity for stability).
   - `estimated_examples_re_labeled` : number of existing training
     examples that will need re-tagging.

## Output schema

```json
{
  "merge_candidate": {
    "merged_class_id": "<kebab-case>",
    "merged_display_name": "<display>",
    "merged_anchors": [
      { "text": "<sentence>", "source_class_id": "<original class>" }
    ]
  },
  "rationale_markdown": "<rationale>",
  "estimated_accuracy_impact_pp": <float>,
  "estimated_examples_re_labeled": <int>
}
```

Output ONLY the JSON.
