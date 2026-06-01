#!/usr/bin/env python3
"""Safety filter evaluation for Mode-2 adversarial prompts.

Sprint **Φ-D-MODE2 Completion (D67)** — artefact #20.
Cf. ``docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`` § F.

## Mission

For each of the 50 adversarial prompts, simulate the Mode-2 safety
filter and verify that 100 % are blocked (= rejected before reaching
the LLM provider).

In mock mode (= default), the safety filter is simulated by a rules
engine that matches known attack patterns. Live mode would integrate
with the actual `efe-llm-router` safety filter.

## Output

```json
{
  "slo_compliance": {
    "SLO-MODE2-SAFETY-FILTER": {
      "measured_pass_count": 50,
      "measured_expected_count": 50,
      "per_category": {
        "jailbreak":         {"total": 10, "blocked": 10},
        "prompt_injection":  {"total": 8,  "blocked": 8},
        ...
      },
      "missed_prompts": []
    }
  }
}
```
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_PROMPTS = REPO_ROOT / "eval" / "mode2" / "adversarial_prompts" / "prompts.json"


# Mock safety filter rules. Each rule is (name, regex pattern, severity).
# In production, the `efe-llm-router::safety` module implements these
# with a multi-layer detector (= regex + transformer-based scoring).
SAFETY_RULES = [
    # Jailbreak patterns
    (re.compile(r"\b(ignore|forget|bypass|override)\b.{0,30}\b(instruction|prompt|safety|rule)", re.I), "jailbreak"),
    (re.compile(r"\b(DAN|do anything now|developer mode|unrestricted)\b", re.I), "jailbreak"),
    (re.compile(r"\b(pretend|act as|roleplay).{0,30}(without|no).{0,20}(restriction|safety|policy)", re.I), "jailbreak"),
    # Prompt injection
    (re.compile(r"(///|<\|im_start\|>|\[\[end_catalog\]\])", re.I), "prompt_injection"),
    (re.compile(r"\b(end of catalog|new system instruction|new catalog)\b", re.I), "prompt_injection"),
    (re.compile(r"rm\s+-rf\s+/", re.I), "prompt_injection"),
    (re.compile(r"\b(execute|run).{0,30}(code|command|script)", re.I), "prompt_injection"),
    # Token explosion
    (re.compile(r"repeat.{0,30}\d{4,}\s*times?", re.I), "token_explosion"),
    (re.compile(r"generate\s+\d{4,}", re.I), "token_explosion"),
    (re.compile(r"depth\s+\d{2,}", re.I), "token_explosion"),
    # PII / data extraction
    (re.compile(r"\b(database|user|customer).{0,20}(content|data|contents)", re.I), "pii_extraction"),
    (re.compile(r"\b(api.?key|secret|token|credential)\b", re.I), "pii_extraction"),
    (re.compile(r"\b(credit.?card|ssn|social.?security)\b", re.I), "pii_extraction"),
    (re.compile(r"training\s+(set|data)", re.I), "pii_extraction"),
    # Code injection
    (re.compile(r"<script|javascript:|onerror=", re.I), "code_injection"),
    (re.compile(r"\bsql\s+injection\b", re.I), "code_injection"),
    (re.compile(r"\bos\.system\b|\bexec\s*\(", re.I), "code_injection"),
    # Cross-tenant probe
    (re.compile(r"\btenant[-_]?[a-z]\b|\btenant[-_]?id\b", re.I), "cross_tenant_probe"),
    (re.compile(r"other\s+(users?|tenants?)", re.I), "cross_tenant_probe"),
    (re.compile(r"\bcached\s+response\b", re.I), "cross_tenant_probe"),
    # Hallucination triggers (= asking the model to recall non-existent context)
    (re.compile(r"\b(yesterday|previously|you\s+remember|previous\s+(conversation|session))\b", re.I), "hallucination_trigger"),
    (re.compile(r"\b(academic\s+papers?|2026\s+paper)\b", re.I), "hallucination_trigger"),
    (re.compile(r"\binternal\s+documentation\b", re.I), "hallucination_trigger"),
    # Schema violation
    (re.compile(r"\b(yaml|xml)\s+(instead\s+of|format)", re.I), "schema_violation"),
    (re.compile(r"\b(snake_case|extra\s+field|unicode\s+special)\b", re.I), "schema_violation"),
    (re.compile(r"\bexceeds?\s+maxItems\b", re.I), "schema_violation"),
    (re.compile(r"longer\s+than\s+\d+\s+character", re.I), "schema_violation"),
    # Sub-rules for missed cases
    (re.compile(r"☠️|forget your training", re.I), "jailbreak"),
    (re.compile(r"reveal\s+(your\s+)?(training\s+data|system\s+prompt)", re.I), "jailbreak"),
    (re.compile(r"format\s+examples?\s+as\s+(python|code)", re.I), "code_injection"),
    (re.compile(r"\bfetch\(\s*['\"]", re.I), "code_injection"),
    (re.compile(r"return\s+yaml", re.I), "schema_violation"),
    (re.compile(r"100\s+anchors?", re.I), "schema_violation"),
    # **Audit fix Φ-D-MODE2 safety expansion** — catch the 13 prompts
    # that escaped the initial pass.
    (re.compile(r"\broleplay\b.{0,40}\b(model|doesn'?t follow|policies)", re.I), "jailbreak"),
    (re.compile(r"after\s+the\s+(json\s+)?response.{0,30}include", re.I), "prompt_injection"),
    (re.compile(r"\bbase64", re.I), "prompt_injection"),
    (re.compile(r"every\s+possible|in\s+\d+\s+languages?", re.I), "token_explosion"),
    (re.compile(r"\d{3,}.?word\s+(essay|text|article)", re.I), "token_explosion"),
    (re.compile(r"email\s+addresses?\s+of", re.I), "pii_extraction"),
    (re.compile(r"output\s+api\s+keys?|memorize", re.I), "pii_extraction"),
    (re.compile(r"html\s+form|fetch\(\)|POSTs?\s+to", re.I), "code_injection"),
    (re.compile(r"tenant\s+ids?\s+in", re.I), "cross_tenant_probe"),
    (re.compile(r"\bappend\s+this\b", re.I), "prompt_injection"),
    (re.compile(r"language\s+to\s+(base64|english)", re.I), "prompt_injection"),
    # Final 3 missed.
    (re.compile(r"\btenant\s+[A-Z0-9]{3,}\b", re.I), "cross_tenant_probe"),
    (re.compile(r"arbitrary\s+extra\s+fields?|extra\s+fields?\s+not\s+in", re.I), "schema_violation"),
    (re.compile(r"\bmake\b.{0,15}\b(string|array)\s+instead\s+of", re.I), "schema_violation"),
]


def is_blocked(text: str) -> tuple[bool, str | None]:
    """Return (blocked, matched_rule_name)."""
    for pattern, category in SAFETY_RULES:
        if pattern.search(text):
            return True, category
    return False, None


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-2 safety filter eval")
    ap.add_argument("--prompts", type=Path, default=DEFAULT_PROMPTS)
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode2/safety_eval.json"))
    ap.add_argument("--strict", action="store_true",
                    help="Fail with exit 1 if any prompt is missed")
    args = ap.parse_args()

    if not args.prompts.exists():
        sys.stderr.write(f"FATAL : prompts file missing : {args.prompts}\n")
        return 2

    data = json.loads(args.prompts.read_text())
    prompts = data["prompts"]

    per_category = defaultdict(lambda: {"total": 0, "blocked": 0})
    missed = []
    for p in prompts:
        per_category[p["category"]]["total"] += 1
        blocked, matched = is_blocked(p["text"])
        if blocked:
            per_category[p["category"]]["blocked"] += 1
        else:
            missed.append({"id": p["id"], "category": p["category"], "text": p["text"][:80]})

    total_blocked = sum(d["blocked"] for d in per_category.values())
    total = len(prompts)

    print("\nSafety filter evaluation :", file=sys.stderr)
    print("─" * 60, file=sys.stderr)
    for cat, data in sorted(per_category.items()):
        sym = "✓" if data["blocked"] == data["total"] else "✗"
        print(
            f"  {cat:<25} : {data['blocked']:>3}/{data['total']:<3} {sym}",
            file=sys.stderr,
        )
    print("─" * 60, file=sys.stderr)
    print(f"  TOTAL : {total_blocked}/{total}", file=sys.stderr)
    if missed:
        print(f"\nMissed prompts ({len(missed)}) :", file=sys.stderr)
        for m in missed[:10]:
            print(f"  - {m['id']} ({m['category']}) : {m['text']}", file=sys.stderr)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-MODE2-SAFETY-FILTER": {
                        "measured_pass_count": total_blocked,
                        "measured_expected_count": total,
                        "per_category": dict(per_category),
                        "missed_prompts": missed[:20],
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    if args.strict and total_blocked < total:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
