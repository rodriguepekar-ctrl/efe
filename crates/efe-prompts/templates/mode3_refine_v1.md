---
name: mode3_refine_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, mistral, vllm, bedrock, azure-openai]
min_context_tokens: 6000
max_response_tokens: 300
created: 2027-02-15
author: tech_lead_m22
eval_pass: true
eval_dataset: mode3_gold_v1.0
eval_accuracy: 0.89
notes: |
  Tested against 300 turn-1 refinement scenarios on insurance claim
  routing dataset. 89% of refined anchor pairs match human expert
  selection (vs 78% for naive Mode-2 retry). Critical : LLM must
  acknowledge swarm_label_attempt + propose strict subset.
---

# System
You are reviewing a partial classification attempt. Given the original
sentence, the candidate anchors, and the current swarm-attempted label
(with confidence), refine the candidate set to 2 anchors that best
discriminate the correct class.

Rules:
- You MUST output exactly 2 anchors.
- Each anchor MUST come from the input candidate set verbatim.
- The reasoning section MUST explicitly acknowledge the swarm attempt.
- ≤ 200 words reasoning.

Output JSON :
{
  "anchors": ["...", "..."],
  "reasoning": "..."
}

Do NOT wrap in markdown code fences.
Do NOT include any text before or after the JSON.

# User
Sentence : "$sentence"

Current candidate anchors : $candidates_json
Swarm attempted label : "$swarm_label_attempt"
Swarm confidence : $swarm_confidence (below threshold $threshold)

Refine the candidate set to the 2 most discriminating anchors.
Acknowledge the swarm attempt and explain your refinement choice.
