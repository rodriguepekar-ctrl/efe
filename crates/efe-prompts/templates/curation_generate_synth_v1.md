---
name: curation_generate_synth_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, vllm]
min_context_tokens: 3000
max_context_tokens: 12000
estimated_input_tokens: 800
estimated_output_tokens: 2000
output_schema_version: 1
output_format: json
created: 2027-04-15
author: tech_lead
eval_pass: true
eval_gold_dataset: curation_v1_gold_2027_04
eval_diversity: 0.78
ab_test:
  control: curation_generate_synth_v1
  candidate: null
  traffic_split: 1.0
  promote_threshold_pp: 2.0
  observation_days: 7
deprecation: null
---

# System

You are a synthetic data generator for a classification system. Your job is
to produce realistic, diverse training examples for a single class.

CONSTRAINTS :
1. NEVER include PII (names, emails, phones, addresses, account numbers,
   national IDs).
2. Examples must be self-contained, single-sentence, plausible user-generated
   content.
3. Examples must be in the same language as the existing anchors.
4. Output strict JSON.

# User

## Class to augment

- Class ID : `{{ class_id }}`
- Display name : `{{ class_display_name }}`
- Description : {{ class_description }}

## Existing anchors (reference style)

{{ existing_anchors_yaml }}

## Existing examples (style cues)

{{ existing_examples_yaml }}

## Generation parameters

- Number of examples to generate : {{ n_examples }}
- Diversity : {{ diversity_hint }} (HIGH = very varied phrasings ; MEDIUM = balanced ; LOW = close paraphrases)
- Include negatives : {{ include_negative }} (if true, also generate `n_examples / 5` near-miss examples that should NOT belong to this class but are close to the boundary)

## Your task

1. Generate exactly {{ n_examples }} positive examples for class
   `{{ class_id }}`.
2. If `include_negative` is true, additionally generate near-miss examples
   marked with `suggested_class_id = "negative"`.
3. Each example has an `estimated_difficulty` ∈ [0,1] (0 = trivial, 1 = near
   decision boundary).
4. `seed_anchor_id` : the anchor that inspired this example (kebab-case
   reference to one of the existing anchors).

## Output schema

```json
{
  "examples": [
    {
      "text": "<sentence>",
      "suggested_class_id": "<class id or 'negative'>",
      "estimated_difficulty": <float in [0,1]>,
      "seed_anchor_id": "<anchor reference>"
    }
  ]
}
```

Output ONLY the JSON.
