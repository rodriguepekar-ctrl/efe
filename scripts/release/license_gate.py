#!/usr/bin/env python3
"""
Sprint **M13 D27** — License gate (pre-publish CI check).

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §12.

Reads a bundle's `manifest.json`, extracts `license.spdx_id`, and refuses
to publish if the license is FORBIDDEN or RESTRICTED-without-ack.

Three categories :
  * ALLOWLIST   — Apache-2.0, MIT, BSD-3-Clause, … : always pass.
  * RESTRICTED  — OpenRAIL-M/S, CC-BY-NC, CC-BY-SA : require ack via
                  `EFE_BUNDLE_ACK_NONREDIST=true` env var.
  * FORBIDDEN   — GPL-3.0, AGPL-3.0, Proprietary, UNKNOWN : always fail.

Exit codes :
  * 0  — license is in ALLOWLIST or RESTRICTED + ack
  * 1  — license is FORBIDDEN
  * 2  — license is RESTRICTED but no ack
  * 3  — manifest is missing the license field or malformed
  * 4  — usage error

Pin tested by `test_d26_license_gate_*` + `test_m13_license_gate_*`.

Usage :
    python3 scripts/release/license_gate.py \\
        --manifest bundles/<slug>/manifest.json
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
#  License classification
# ---------------------------------------------------------------------------

ALLOWLIST: frozenset[str] = frozenset({
    "Apache-2.0",
    "Apache-2.0-with-LLVM-exception",
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CC-BY-4.0",
    "CC0-1.0",
    "ISC",
    "Unlicense",
})

RESTRICTED: frozenset[str] = frozenset({
    # ML-specific licenses requiring redistribution ack
    "OpenRAIL-M",
    "OpenRAIL-S",
    "BigScience-BLOOM-RAIL-1.0",
    "Llama-2-Community-License",
    # NC / SA require redistribution ack
    "CC-BY-NC-4.0",
    "CC-BY-NC-SA-4.0",
    "CC-BY-SA-4.0",
})

FORBIDDEN: frozenset[str] = frozenset({
    # Copyleft beyond LGPL — incompatible with EFE Apache-2.0 distribution
    "GPL-2.0-only",
    "GPL-2.0-or-later",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "AGPL-3.0-only",
    "AGPL-3.0-or-later",
    # Vendor lock-in
    "Proprietary",
    "Commercial",
    "Custom",
    # Unknown / undeclared
    "UNKNOWN",
    "NOASSERTION",
})


# ---------------------------------------------------------------------------
#  Gate
# ---------------------------------------------------------------------------


class LicenseGateFailure(Exception):
    """Raised when the gate refuses the manifest's license."""
    def __init__(self, exit_code: int, message: str) -> None:
        super().__init__(message)
        self.exit_code = exit_code


def gate_license(manifest: dict, *, ack_nonredist: bool = False) -> str:
    """Validate a manifest's license against M13 policy.

    Returns the license SPDX id on success.

    Raises :
      * LicenseGateFailure(exit_code=1) if FORBIDDEN
      * LicenseGateFailure(exit_code=2) if RESTRICTED + no ack
      * LicenseGateFailure(exit_code=3) if license field missing/malformed
    """
    license_block = manifest.get("license")
    if not isinstance(license_block, dict):
        raise LicenseGateFailure(
            3, "manifest.license is missing or not an object"
        )
    spdx = license_block.get("spdx_id")
    if not isinstance(spdx, str) or not spdx:
        raise LicenseGateFailure(
            3, "manifest.license.spdx_id is missing or empty"
        )

    if spdx in FORBIDDEN:
        raise LicenseGateFailure(
            1,
            f"License {spdx!r} is FORBIDDEN by M13 policy. "
            f"Refusing to publish bundle.",
        )

    if spdx in RESTRICTED:
        if not ack_nonredist:
            raise LicenseGateFailure(
                2,
                f"License {spdx!r} is RESTRICTED. Operator must "
                f"acknowledge via EFE_BUNDLE_ACK_NONREDIST=true "
                f"to publish.",
            )
        return spdx

    if spdx in ALLOWLIST:
        return spdx

    raise LicenseGateFailure(
        1,
        f"License {spdx!r} is NOT in M13 ALLOWLIST. "
        f"Add it explicitly to license_gate.ALLOWLIST or "
        f"use a forbidden+ack workflow if intentional.",
    )


# ---------------------------------------------------------------------------
#  Driver
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else None)
    p.add_argument("--manifest", type=Path, required=True,
                   help="Path to the bundle's manifest.json")
    p.add_argument("--ack-nonredist", action="store_true",
                   help="Acknowledge a RESTRICTED license. Equivalent to "
                        "setting EFE_BUNDLE_ACK_NONREDIST=true.")
    args = p.parse_args(argv)

    if not args.manifest.exists():
        print(f"::error::Manifest not found : {args.manifest}", file=sys.stderr)
        return 4

    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        print(f"::error::Manifest is not valid JSON : {e}", file=sys.stderr)
        return 4

    ack = args.ack_nonredist or (
        os.environ.get("EFE_BUNDLE_ACK_NONREDIST", "").lower() == "true"
    )

    try:
        spdx = gate_license(manifest, ack_nonredist=ack)
    except LicenseGateFailure as e:
        print(f"::error::license-gate REFUSED — {e}", file=sys.stderr)
        return e.exit_code

    print(f"LICENSE_GATE_OK spdx_id={spdx}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
