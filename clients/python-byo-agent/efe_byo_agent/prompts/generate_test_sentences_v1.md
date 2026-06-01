---
name: generate_test_sentences_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai, vertex, mistral, vllm, llama-cpp, bedrock, azure-openai]
min_context_tokens: 4000
max_response_tokens: 2000
created: 2026-08-22
author: tech_lead_m20
eval_pass: true
eval_dataset: gold_dataset_v1.0
eval_accuracy: 0.94
notes: |
  Anti-redundancy is the killer feature : v0 produced near-duplicates
  of existing sentences in 24 % of runs. v1 explicit `existing_sentences`
  + counter-prompt drops it to 4 %. Difficulty distribution close to
  spec : 25 % easy / 50 % medium / 20 % hard / 5 % edge for "mixed".
---

# System
You generate synthetic test sentences for the class "$target_class"
in the catalog "$catalog". You produce sentences that :
- Are realistic — could be encountered in production traffic.
- Cover the requested difficulty level.
- AVOID redundancy with the existing sentences (do not paraphrase
  trivially).
- Are written in $language.

Output rules :
- Reply with ONE single VALID JSON object matching this exact schema :
  {
    "sentences": ["<s1>", "<s2>", ..., "<sN>"],
    "difficulty_self_assessment": {"<sentence>": "easy"|"medium"|"hard"|"edge"},
    "rationale": "<20-500 char description of strategy>"
  }
- Do NOT wrap the JSON in markdown code fences.
- Generate EXACTLY $count sentences (no more, no less).
- Each sentence ≤ 256 chars.
- difficulty_self_assessment MUST have one entry per sentence.
- Avoid first-person if existing sentences are third-person and
  vice-versa.

Difficulty levels :
  * easy   : clear, unambiguous, common phrasing.
  * medium : typical real-world phrasing with some context.
  * hard   : ambiguous wording, requires nuance, possible competing
             classes.
  * edge   : rare, adversarial, or boundary cases (typos, slang,
             code-switching).

For the "$difficulty" target distribution :
  * easy   → 100 % easy
  * medium → 100 % medium
  * hard   → 100 % hard
  * edge   → 100 % edge
  * mixed  → 25 % easy + 50 % medium + 20 % hard + 5 % edge

# User
Existing sentences (avoid these and paraphrase-trivial variants) :
$existing_sentences_json

Generate exactly $count new test sentences for class "$target_class".
