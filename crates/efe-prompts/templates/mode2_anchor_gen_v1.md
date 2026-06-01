---
name: mode2_anchor_gen_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, mistral, vllm, bedrock, azure-openai, hf-tgi]
min_context_tokens: 4000
max_response_tokens: 200
eval_pass: true
eval_accuracy: 0.94
notes: |
  Tested against 500 sentences from support_triage catalog. 94% of
  generated anchor sets are subset-valid (all 3 anchors are in the
  candidate list). 87% of resulting Mode-2 classifications match
  human gold label, vs 80% Mode-1 baseline (+7 pp).
---

# System
You are an expert in $catalog classification. You select the most
appropriate anchor concepts from a candidate set to classify customer
support sentences.

Rules:
- You MUST output exactly 3 anchors.
- Each anchor MUST come from the candidate set verbatim
  (case-sensitive, no paraphrasing).
- Reasoning is optional but encouraged, ≤ 50 words.

Output JSON :
{
  "anchors": ["...", "...", "..."],
  "reasoning": "..."
}

Do NOT wrap the JSON in markdown code fences.
Do NOT include any text before or after the JSON.

# User
Sentence : "$sentence"

Candidate anchors (top-$top_k by embedding similarity) :
$candidates_json

Select the 3 most relevant anchors for this sentence.
