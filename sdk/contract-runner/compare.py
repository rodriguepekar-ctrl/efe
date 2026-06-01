#!/usr/bin/env python3
"""Compare 4 SDK verdict outputs for bit-identity.

Sprint **Φ-C-SDK Completion (D30)** — artefact #17.
Cf. ``docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`` § 5.6.

## Mission

Read 4 ``out_<sdk>.json`` files produced by the per-SDK runners,
compare each (fixture_id, sdk) verdict for **bit-identity** on 8
identifying fields :

* ``label`` (str)        — exact match
* ``confidence`` (float) — < 1e-6 epsilon (= f32 wire roundtrip)
* ``cone_id`` (str)       — exact match
* ``rounds_executed`` (int) — exact match
* ``swarm_id`` (str)        — exact match (= SHA-256-derived)
* ``catalog_version`` (str) — exact match
* ``sub_scores`` (list)     — order + per-element exact match
* ``latency_ms``            — **IGNORED** (non-deterministic)

## Output

```json
{
  "slo_compliance": {
    "SLO-SDK-CONTRACT": {
      "measured_pass_count": 400,
      "measured_expected_count": 400,
      "per_sdk_breakdown": {
        "go":   {"passes": 100, "failures": 0},
        "ts":   {"passes": 100, "failures": 0},
        "java": {"passes": 100, "failures": 0},
        "py":   {"passes": 100, "failures": 0}
      },
      "label_mismatches": [],
      "confidence_drift_max": 0.0
    }
  }
}
```

## Strategy

We pick the **first** SDK's verdict for each fixture as the reference
(= "py" if present, else first in lexicographic order) and check that
all other SDKs return bit-identical verdicts. Failures are reported
per fixture with all 4 SDK outputs side-by-side.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

# Confidence tolerance : 1e-6 absorbs the f32 wire roundtrip drift
# documented in archi § 4.3.
CONFIDENCE_EPSILON = 1e-6


def load_verdicts(path: Path) -> dict[str, dict[str, Any]]:
    """Return {fixture_id : verdict_dict} from one SDK output file."""
    data = json.loads(path.read_text())
    return {v["fixture_id"]: v for v in data.get("verdicts", [])}


def compare_pair(ref: dict, other: dict) -> list[str]:
    """Compare two verdict dicts. Returns list of human-readable diffs."""
    diffs: list[str] = []
    for field in ("label", "cone_id", "rounds_executed", "swarm_id", "catalog_version"):
        if ref.get(field) != other.get(field):
            diffs.append(
                f"{field}: ref={ref.get(field)!r} other={other.get(field)!r}"
            )
    # Confidence with epsilon.
    r_conf = float(ref.get("confidence", 0.0))
    o_conf = float(other.get("confidence", 0.0))
    if abs(r_conf - o_conf) > CONFIDENCE_EPSILON:
        diffs.append(
            f"confidence: ref={r_conf} other={o_conf} Δ={abs(r_conf - o_conf)}"
        )
    # sub_scores : ordered list of (label, conf) pairs.
    r_sub = ref.get("sub_scores", []) or []
    o_sub = other.get("sub_scores", []) or []
    if len(r_sub) != len(o_sub):
        diffs.append(f"sub_scores length: ref={len(r_sub)} other={len(o_sub)}")
    else:
        for i, (rs, os_) in enumerate(zip(r_sub, o_sub)):
            r_label = rs.get("label") if isinstance(rs, dict) else None
            o_label = os_.get("label") if isinstance(os_, dict) else None
            if r_label != o_label:
                diffs.append(
                    f"sub_scores[{i}].label: ref={r_label!r} other={o_label!r}"
                )
            r_c = float(rs.get("confidence", 0.0)) if isinstance(rs, dict) else 0.0
            o_c = float(os_.get("confidence", 0.0)) if isinstance(os_, dict) else 0.0
            if abs(r_c - o_c) > CONFIDENCE_EPSILON:
                diffs.append(
                    f"sub_scores[{i}].confidence: ref={r_c} other={o_c}"
                )
    return diffs


def main() -> int:
    ap = argparse.ArgumentParser(description="Cross-SDK contract comparator")
    ap.add_argument("--go", type=Path, required=True)
    ap.add_argument("--ts", type=Path, required=True)
    ap.add_argument("--java", type=Path, required=True)
    ap.add_argument("--py", type=Path, required=True)
    ap.add_argument("--out", type=Path,
                    default=Path("target/sdk/contract.json"))
    ap.add_argument("--ref-sdk", choices=["py", "go", "ts", "java"],
                    default="py",
                    help="SDK to use as the reference (default: py)")
    args = ap.parse_args()

    sdk_files = {
        "go":   args.go,
        "ts":   args.ts,
        "java": args.java,
        "py":   args.py,
    }
    sdks: dict[str, dict] = {}
    missing: list[str] = []
    for sdk, p in sdk_files.items():
        if not p.exists():
            missing.append(f"{sdk}={p}")
            continue
        sdks[sdk] = load_verdicts(p)

    if missing:
        sys.stderr.write(f"FATAL : missing input files : {missing}\n")
        return 2

    if args.ref_sdk not in sdks:
        sys.stderr.write(f"FATAL : reference SDK '{args.ref_sdk}' missing\n")
        return 2

    ref_verdicts = sdks[args.ref_sdk]
    other_sdks = [k for k in sdks if k != args.ref_sdk]

    # Build per-SDK + per-fixture diff matrix.
    per_sdk_passes: dict[str, int] = {sdk: 0 for sdk in sdks}
    per_sdk_failures: dict[str, list[str]] = {sdk: [] for sdk in sdks}
    label_mismatches: list[str] = []
    confidence_drift_max = 0.0

    # The reference SDK itself is "self-PASS" by definition for every fixture.
    for fid in ref_verdicts:
        per_sdk_passes[args.ref_sdk] += 1

    # Compare each other SDK's verdict against the reference.
    for other in other_sdks:
        for fid, ref_v in ref_verdicts.items():
            other_v = sdks[other].get(fid)
            if other_v is None:
                per_sdk_failures[other].append(f"{fid}: missing verdict")
                continue
            diffs = compare_pair(ref_v, other_v)
            if diffs:
                per_sdk_failures[other].append(
                    f"{fid}: " + " ; ".join(diffs)
                )
                for d in diffs:
                    if "label:" in d:
                        label_mismatches.append(f"{other}/{fid}: {d}")
                    if "confidence:" in d:
                        # Extract numeric drift.
                        import re
                        m = re.search(r"Δ=([0-9.eE+-]+)", d)
                        if m:
                            try:
                                drift = float(m.group(1))
                                confidence_drift_max = max(confidence_drift_max, drift)
                            except ValueError:
                                pass
            else:
                per_sdk_passes[other] += 1

    total_passes = sum(per_sdk_passes.values())
    total_expected = len(sdks) * len(ref_verdicts)

    # Pretty summary on stderr.
    print(f"\nContract test — reference SDK = {args.ref_sdk}", file=sys.stderr)
    print("─" * 60, file=sys.stderr)
    for sdk in sdks:
        n_fail = len(per_sdk_failures[sdk])
        n_pass = per_sdk_passes[sdk]
        sym = "✓" if n_fail == 0 else "✗"
        print(
            f"  {sym} {sdk:<6} : {n_pass} pass, {n_fail} fail",
            file=sys.stderr,
        )
    print("─" * 60, file=sys.stderr)
    print(f"  TOTAL  : {total_passes}/{total_expected}", file=sys.stderr)
    if confidence_drift_max > 0:
        print(
            f"  max confidence drift : {confidence_drift_max:.2e} "
            f"(tolerance {CONFIDENCE_EPSILON:.2e})",
            file=sys.stderr,
        )

    # Emit SLO compliance JSON.
    args.out.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "slo_compliance": {
            "SLO-SDK-CONTRACT": {
                "measured_pass_count": total_passes,
                "measured_expected_count": total_expected,
                "per_sdk_breakdown": {
                    sdk: {
                        "passes": per_sdk_passes[sdk],
                        "failures": len(per_sdk_failures[sdk]),
                    }
                    for sdk in sdks
                },
                "label_mismatches": label_mismatches[:20],
                "confidence_drift_max": confidence_drift_max,
                "first_10_failures": [
                    f"{sdk}: {fail}"
                    for sdk, fails in per_sdk_failures.items()
                    for fail in fails[:5]
                ][:10],
                "reference_sdk": args.ref_sdk,
            }
        }
    }
    args.out.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")

    return 0 if total_passes == total_expected else 1


if __name__ == "__main__":
    sys.exit(main())
