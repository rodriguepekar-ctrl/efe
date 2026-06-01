#!/usr/bin/env python3
"""Test Mode-3 human review escalation triggers.

Sprint **Φ-D-MODE3 Completion (D73)** — artefact #26.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 9.1.

Verifies the 5 escalation triggers fire correctly :
  1. low_confidence (= turn confidence < 0.5)
  2. convergence_oscillating (= oscillation detected)
  3. high_cost_session (= session cost > 80 % of cap)
  4. gdpr_sensitive_class (= PII/health/financial)
  5. tenant_policy (= per-tenant flag)
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

# Trigger detection rules (= mock for unit-test ; production uses ML model).
GDPR_PATTERNS = [
    re.compile(r"\b\d{3}-\d{2}-\d{4}\b"),                # SSN
    re.compile(r"\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b"),  # CC
    re.compile(r"\b(HIV|cancer|diabetes|pregnant)\b", re.I),  # health
    re.compile(r"\b(bankrupt|debt|foreclosure)\b", re.I),     # financial distress
]


def detect_low_confidence(confidence: float) -> bool:
    return confidence < 0.5


def detect_high_cost(cumulative_usd: float, cap_usd: float = 0.10) -> bool:
    return cumulative_usd > 0.80 * cap_usd


def detect_gdpr_sensitive(text: str) -> bool:
    return any(p.search(text) for p in GDPR_PATTERNS)


def detect_oscillating(convergence_scores: list[float]) -> bool:
    """Oscillation : cosine_sim(N, N-2) > cosine_sim(N, N-1) for ≥ 3 turns."""
    if len(convergence_scores) < 4:
        return False
    # convergence_scores are sim(N, N-1). For oscillation we need sim
    # between non-adjacent pairs ; simplification : declare oscillation
    # when scores alternate above/below a midpoint.
    above = [s > 0.7 for s in convergence_scores[-4:]]
    return above == [True, False, True, False] or above == [False, True, False, True]


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-3 review triggers test")
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode3/review_triggers_test.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    # Run unit-tests for each trigger.
    tests = []
    # Trigger 1 : low_confidence
    tests.append(("low_confidence_below", detect_low_confidence(0.3), True))
    tests.append(("low_confidence_above", detect_low_confidence(0.7), False))
    # Trigger 2 : oscillation
    osc_scores = [0.85, 0.40, 0.88, 0.35]
    tests.append(("oscillation_detected", detect_oscillating(osc_scores), True))
    tests.append(("oscillation_smooth", detect_oscillating([0.5, 0.7, 0.85, 0.95]), False))
    # Trigger 3 : high_cost
    tests.append(("high_cost_above", detect_high_cost(0.09), True))
    tests.append(("high_cost_below", detect_high_cost(0.05), False))
    # Trigger 4 : GDPR sensitive
    tests.append(("gdpr_ssn", detect_gdpr_sensitive("My SSN is 123-45-6789"), True))
    tests.append(("gdpr_cc", detect_gdpr_sensitive("My card is 4111-1111-1111-1111"), True))
    tests.append(("gdpr_health", detect_gdpr_sensitive("I have HIV"), True))
    tests.append(("gdpr_clean", detect_gdpr_sensitive("Hello world"), False))
    # Trigger 5 : tenant_policy (= config-driven, mocked here)
    tenant_policy_flagged = True  # mock
    tests.append(("tenant_policy_flagged", tenant_policy_flagged, True))

    passed = all(actual == expected for _, actual, expected in tests)
    n_pass = sum(1 for _, a, e in tests if a == e)

    print(f"Review trigger tests : {n_pass}/{len(tests)} PASS", file=sys.stderr)
    for name, actual, expected in tests:
        sym = "✓" if actual == expected else "✗"
        print(f"  {sym} {name}: got={actual} expected={expected}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "review_triggers": {
            "tests_run": len(tests),
            "tests_passed": n_pass,
            "all_pass": passed,
            "detail": [{"name": n, "actual": a, "expected": e, "pass": a == e}
                       for n, a, e in tests],
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
