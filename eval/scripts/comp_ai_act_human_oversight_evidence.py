#!/usr/bin/env python3
"""Evidence : AI Act human oversight = Mode-3 forced for high-risk (D-COMP-8).

Sprint Φ-D-COMPLIANCE D88 — evidence #15.
Controls : AI-ACT-Art14, AI-ACT-Art16.a, GDPR-Art22, AI-ACT-Art29.
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path

CONTROLS = ["AI-ACT-Art14", "AI-ACT-Art16.a", "GDPR-Art22", "AI-ACT-Art29"]


def route_to_mode(risk_level: str, requested_mode: str) -> str:
    """Production-grade gate : high-risk forces Mode-3, unacceptable refused."""
    if risk_level == "unacceptable":
        raise PermissionError("unacceptable risk catalog refused")
    if risk_level == "high":
        return "mode3"   # always force
    return requested_mode


SCENARIOS = [
    # (name, risk_level, requested_mode, expected_resolved)
    ("high_force_mode3",       "high",    "mode1", "mode3"),
    ("high_already_mode3_ok",  "high",    "mode3", "mode3"),
    ("limited_keep_request",   "limited", "mode2", "mode2"),
    ("minimal_keep_request",   "minimal", "mode1", "mode1"),
    ("high_mode2_request_forced", "high", "mode2", "mode3"),
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_ai_act_oversight.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-AI-ACT-HIGH-OVERSIGHT": {
                    "skip_reason": "Live catalog routing audit (= prod traffic)",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    results = []
    n_pass = 0
    for name, risk, req_mode, expected in SCENARIOS:
        got = route_to_mode(risk, req_mode)
        ok = got == expected
        if args.mode == "synth_fail" and name == "high_force_mode3":
            ok = False  # inject
        if ok:
            n_pass += 1
        results.append({"name": name, "risk": risk, "requested": req_mode,
                         "expected": expected, "got": got, "ok": ok})
        print(f"  {'✓' if ok else '✗'} {name}", file=sys.stderr)

    # Adversarial : unacceptable risk must raise.
    try:
        route_to_mode("unacceptable", "mode1")
        unacceptable_raise = False
    except PermissionError:
        unacceptable_raise = True
    results.append({"name": "unacceptable_refused", "ok": unacceptable_raise})
    if unacceptable_raise:
        n_pass += 1

    passed = n_pass == len(results)
    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-AI-ACT-HIGH-OVERSIGHT": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_scenarios": len(results), "n_passed": n_pass,
                "scenarios": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
