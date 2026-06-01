#!/usr/bin/env python3
"""SLO-FFI-WHEEL-SIZE — verify each wheel ≤ N MiB compressed.

Sprint **Φ-C-FFI Completion (D28)** — artefact #18.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 6 + § 16.

## Usage

```bash
python scripts/bench/verify_wheel_size.py --max-mib 8 crates/efe-ffi/dist/
# ou :
python scripts/bench/verify_wheel_size.py --max-mib 8 path/to/specific.whl
```

## Output

Stderr : human-readable summary table.
``target/ffi/wheel_size.json`` : SLO compliance JSON :

```json
{
  "slo_compliance": {
    "SLO-FFI-WHEEL-SIZE": {
      "measured_bytes": 7_340_032,
      "max_bytes": 8_388_608,
      "per_wheel": {
        "efe-0.6.0-cp310-abi3-manylinux_2_17_x86_64.whl": 7340032,
        "efe-0.6.0-cp310-abi3-macosx_11_0_arm64.whl":   6815744
      }
    }
  }
}
```

Note : ``measured_bytes`` = max sur l'ensemble des wheels (= le pire
cas qui doit toujours rester sous budget). ``check_slo.py`` lit
``measured_bytes`` pour comparator ``lte``.

## Exit codes

* 0 : toutes les wheels passent.
* 1 : ≥ 1 wheel dépasse le budget.
* 2 : aucune wheel trouvée (= probable bug pipeline).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser(description="Verify wheel size budget")
    ap.add_argument(
        "path",
        type=Path,
        help="Directory containing *.whl files, or a specific *.whl path",
    )
    ap.add_argument(
        "--max-mib",
        type=float,
        default=8.0,
        help="Per-wheel size budget in MiB (default 8.0)",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=Path("target/ffi/wheel_size.json"),
        help="Output JSON path",
    )
    args = ap.parse_args()

    if args.path.is_dir():
        wheels = sorted(args.path.glob("*.whl"))
    elif args.path.suffix == ".whl":
        wheels = [args.path]
    else:
        sys.stderr.write(f"FATAL : {args.path} is neither a dir nor a .whl\n")
        return 2

    if not wheels:
        sys.stderr.write(f"FATAL : no wheels found in {args.path}\n")
        return 2

    max_bytes = int(args.max_mib * 1024 * 1024)
    per_wheel: dict[str, int] = {}
    max_observed = 0
    failures: list[str] = []

    print(
        f"\n{'Wheel':<70}{'Size':>12}{'Status':>10}",
        file=sys.stderr,
    )
    print("─" * 92, file=sys.stderr)
    for whl in wheels:
        size = whl.stat().st_size
        per_wheel[whl.name] = size
        max_observed = max(max_observed, size)
        status = "OK" if size <= max_bytes else "FAIL"
        if status == "FAIL":
            failures.append(whl.name)
        print(
            f"{whl.name:<70}{size / 2**20:>10.2f} MiB{status:>10}",
            file=sys.stderr,
        )
    print("─" * 92, file=sys.stderr)
    print(
        f"max wheel : {max_observed / 2**20:.2f} MiB ; budget : {args.max_mib:.2f} MiB",
        file=sys.stderr,
    )

    # Emit SLO compliance JSON.
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-FFI-WHEEL-SIZE": {
                        "measured_bytes": max_observed,
                        "max_bytes": max_bytes,
                        "per_wheel": per_wheel,
                        "failures": failures,
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    if failures:
        sys.stderr.write(
            f"\nFAIL : {len(failures)} wheel(s) over budget :\n"
        )
        for f in failures:
            sys.stderr.write(f"  - {f}\n")
        return 1

    print("\nALL WHEELS PASS\n", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
