#!/usr/bin/env python3
"""Freeze the FFI host attestation fingerprint.

Sprint **Φ-C-FFI Completion (D28)** — référencé par
``bench_baselines/v0.6.0/ffi_slos.yaml`` § 5.1.

## Mission

Calcule un **fingerprint déterministe** de l'environnement de build
de la wheel FFI :

* Version du toolchain Rust (= ``rustc --version``).
* Version de PyO3 (= ``cargo tree | grep pyo3``).
* Version de maturin (= ``maturin --version``).
* SHA-256 du code source ``efe-ffi/src/``.
* Hash de la wheel produite (si --wheel-path est donné).

Le fingerprint est utilisé comme ``host_fingerprint_baseline`` dans
``ffi_slos.yaml`` pour gating SLO opposable : seules les bench output
qui matchent ce fingerprint sont admises comme preuves contractuelles.

## Usage

```bash
# Génère le fingerprint courant (= post-build, pre-tag)
python scripts/bench/freeze_ffi_attestation.py --out target/ffi/attestation.json

# Override le SLO baseline avec ce fingerprint
python scripts/bench/freeze_ffi_attestation.py \\
    --out target/ffi/attestation.json \\
    --update-baseline bench_baselines/v0.6.0/ffi_slos.yaml
```

## Output

```json
{
  "fingerprint": "0a1b2c3d4e5f...",
  "components": {
    "rustc_version": "rustc 1.85.0 (...)",
    "pyo3_version": "0.21.0",
    "maturin_version": "1.7.4",
    "src_sha256": "abc123...",
    "wheel_sha256": "def456..."   // optional
  },
  "freeze_date": "2026-05-13T12:00:00Z"
}
```

## SLO usage

```yaml
# bench_baselines/v0.6.0/ffi_slos.yaml
host_fingerprint_baseline: "0a1b2c3d4e5f..."
```

Au runtime, ``check_slo.py --fingerprint $(python freeze_ffi_attestation.py --emit-fingerprint)``
compare ; si mismatch et ``host_fingerprint_required: true`` sur un
SLO, le check fail avec ``REFUSED_BY_FINGERPRINT_MISMATCH``.

## D25 status

Pré-D29 (= avant que le real wiring DT-F-WIRE-1 soit câblé), le
fingerprint inclut uniquement les composants stables. Le hash de la
wheel n'est pas inclus dans le baseline (= il varie par build). Le
fingerprint est donc opposable **à la chaîne de build**, pas à la
wheel binaire elle-même.

Φ-C-FFI freeze : `PLACEHOLDER_compute_at_freeze` sentinel (= traité
comme None par check_slo.py, cf. fix Φ-D.B3b).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


def safe_run(cmd: list[str]) -> str:
    """Run a command, return stdout. Return empty string on failure."""
    try:
        out = subprocess.check_output(
            cmd, stderr=subprocess.DEVNULL, timeout=30
        )
        return out.decode("utf-8", errors="replace").strip()
    except (subprocess.CalledProcessError, FileNotFoundError, subprocess.TimeoutExpired):
        return ""


def rustc_version() -> str:
    return safe_run(["rustc", "--version"])


def maturin_version() -> str:
    return safe_run(["maturin", "--version"])


def pyo3_version(crate_root: Path) -> str:
    """Extract `pyo3` version from `cargo tree`."""
    out = safe_run(
        ["cargo", "tree", "-p", "efe-ffi", "--depth", "2", "--prefix", "none"]
    )
    if not out:
        return ""
    for line in out.splitlines():
        line = line.strip()
        # Lines like "pyo3 v0.21.0"
        if line.startswith("pyo3 v"):
            return line.split("v", 1)[1].split()[0].strip()
    return ""


def src_sha256(src_dir: Path) -> str:
    """SHA-256 of all *.rs files in src_dir, hashed in sorted path order."""
    h = hashlib.sha256()
    for rs in sorted(src_dir.rglob("*.rs")):
        rel = rs.relative_to(src_dir)
        h.update(str(rel).encode("utf-8"))
        h.update(b"\0")
        h.update(rs.read_bytes())
    return h.hexdigest()


def file_sha256(path: Path) -> str:
    if not path.exists():
        return ""
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def compute_attestation(
    crate_root: Path, wheel_path: Path | None = None
) -> dict:
    components = {
        "rustc_version": rustc_version(),
        "pyo3_version": pyo3_version(crate_root),
        "maturin_version": maturin_version(),
        "src_sha256": src_sha256(crate_root / "src"),
    }
    if wheel_path is not None:
        components["wheel_sha256"] = file_sha256(wheel_path)
        components["wheel_path"] = str(wheel_path)

    # Compute a stable composite fingerprint :
    #   sha256(json_canonical(components_excl_wheel))
    canonical = {
        k: v for k, v in components.items()
        if k not in ("wheel_sha256", "wheel_path")
    }
    fingerprint = hashlib.sha256(
        json.dumps(canonical, sort_keys=True).encode("utf-8")
    ).hexdigest()

    return {
        "fingerprint": fingerprint,
        "components": components,
        "freeze_date": datetime.now(timezone.utc).isoformat(),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="Freeze FFI host attestation")
    ap.add_argument(
        "--crate-root",
        type=Path,
        default=Path(__file__).resolve().parents[2] / "crates" / "efe-ffi",
        help="efe-ffi crate root (default : derived from script location)",
    )
    ap.add_argument(
        "--wheel-path",
        type=Path,
        help="Optional wheel file to include in attestation",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=Path("target/ffi/attestation.json"),
        help="Output JSON path",
    )
    ap.add_argument(
        "--emit-fingerprint",
        action="store_true",
        help="Emit only the fingerprint to stdout (for shell pipelines)",
    )
    ap.add_argument(
        "--update-baseline",
        type=Path,
        help="Update host_fingerprint_baseline in the given SLO YAML file",
    )
    args = ap.parse_args()

    if not args.crate_root.exists():
        sys.stderr.write(f"FATAL : crate root {args.crate_root} does not exist\n")
        return 2

    attestation = compute_attestation(args.crate_root, args.wheel_path)

    if args.emit_fingerprint:
        # Shell-pipeline-friendly : print only the fingerprint, no JSON.
        print(attestation["fingerprint"])
        return 0

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(attestation, indent=2) + "\n")
    print(
        f"Wrote attestation → {args.out}\n"
        f"  fingerprint = {attestation['fingerprint']}\n"
        f"  rustc     = {attestation['components']['rustc_version']}\n"
        f"  pyo3      = {attestation['components']['pyo3_version']}\n"
        f"  maturin   = {attestation['components']['maturin_version']}\n"
        f"  src_sha256= {attestation['components']['src_sha256'][:16]}..."
    )

    if args.update_baseline:
        # Update the SLO baseline YAML with the new fingerprint.
        try:
            import yaml  # type: ignore
        except ImportError:
            sys.stderr.write(
                "WARN : pyyaml not installed ; skipping --update-baseline\n"
            )
            return 0
        path = args.update_baseline
        if not path.exists():
            sys.stderr.write(f"FATAL : baseline {path} does not exist\n")
            return 2
        data = yaml.safe_load(path.read_text())
        data["host_fingerprint_baseline"] = attestation["fingerprint"]
        path.write_text(yaml.safe_dump(data, sort_keys=False, allow_unicode=True))
        print(f"Updated baseline {path} with fingerprint {attestation['fingerprint']}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
