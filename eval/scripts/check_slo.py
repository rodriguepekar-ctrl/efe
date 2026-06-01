#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""SLO gate for the P1 surface (Sprint D19).

Reads a YAML baseline that declares the contractual SLOs (cf.
`bench_baselines/v0.2.0/p1_slos.yaml`) and a bench output JSON
(produced by `ghz --format=json` or by criterion via
`scripts/parse_criterion_report.py`), then verifies that each
ACTIVE SLO is satisfied.

## Exit codes

- `0` : all ACTIVE SLOs pass.
- `1` : at least one ACTIVE SLO fails.
- `2` : malformed input (YAML/JSON parse error, schema mismatch).
- `3` : host_fingerprint mismatch (= inopposable measurement).

## Flags

- `--baseline PATH` : YAML baseline (required).
- `--measure PATH` : bench output JSON (required).
- `--fingerprint SHA256` : current host fingerprint ; compared
  against `host_fingerprint_baseline` in the YAML. If mismatch and
  the SLO has `host_fingerprint_required: true`, the SLO is
  reported as `REFUSED_BY_FINGERPRINT_MISMATCH` and exit code = 3.
- `--skip-pending-gpu-runner` : skip SLOs whose `status =
  PENDING_GPU_RUNNER` (used by the CPU CI job).
- `--strict-skip` : if set, ANY skipped SLO causes exit code 1
  (used by the release-tag CI job which requires all SLOs ACTIVE).
- `--report PATH` : optional Markdown report path.
- `--json` : emit a machine-readable JSON summary on stdout.

## Bench output schemas supported

### ghz JSON (preferred for P1 sustained workload)

```json
{
  "count": 120000,
  "average": 0.034,
  "latencyDistribution": [
    {"percentage": 50, "latency": 0.018},
    {"percentage": 95, "latency": 0.07},
    {"percentage": 99, "latency": 0.187},
    {"percentage": 99.9, "latency": 0.312}
  ],
  ...
}
```

### Criterion report schema (legacy, fallback)

```json
{ "slo_compliance": { "SLO-X-Y": { "measured_ns": ..., "passed": ... } } }
```

If the measure file contains `slo_compliance`, we use the
pre-computed values ; otherwise we compute from
`latencyDistribution`.

## Architecture

Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 5.3.1.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


# ──────────────────────────────────────────────────────────────────
# Types
# ──────────────────────────────────────────────────────────────────


@dataclass
class SloDef:
    """One SLO declaration from the YAML baseline.

    **Audit fix Φ-C.A4** — accepte 3 fields supplémentaires
    ergonomiques côté YAML (= P2 + Bundle baselines les utilisent) :

    - `threshold_seconds` (float) : converti en nanos dans `__post_init__`.
    - `threshold_bytes` (int) : pour `SLO-BUNDLE-SIZE` / `SLO-BUNDLE-VRAM`.
    - `threshold_count` (int) : pour les SLO count-based (= drift, etc.).

    L'invariant : exactement UN field threshold actif après normalisation.
    """

    id: str
    description: str
    comparator: str               # "gte" | "lte" | "eq"
    status: str                   # "ACTIVE" | "PENDING_GPU_RUNNER" | "SKIPPED"
    threshold_ns: int | None = None
    threshold_ratio: float | None = None
    threshold_seconds: float | None = None    # ← audit fix Φ-C.A4
    threshold_bytes: int | None = None        # ← audit fix Φ-C.A4
    threshold_count: int | None = None        # ← audit fix Φ-C.A4
    threshold_bytes_per_hour: int | None = None  # ← audit fix Φ-D.A1 (= SOAK-24H drift)
    # **Sprint Φ-C-FFI** — pass-count SLO (= 100/100 fixtures passes,
    # k/N cross-platform verifications, etc.). Sans cette extension,
    # `ffi_slos.yaml` aurait 6 SLOs silencieusement skippés (= replay
    # exact du pattern Φ-D.A1 / Φ-C.A4). expected_count est mandatory
    # quand threshold_pass_count est set (= "k passes sur N attendus").
    threshold_pass_count: int | None = None   # ← Φ-C-FFI
    expected_count: int | None = None         # ← Φ-C-FFI
    metric: str | None = None
    quantile: float | None = None
    host_fingerprint_required: bool = False
    note: str | None = None
    thresholds: dict[str, float] | None = None       # multi-threshold (BURN)
    source_bench: str | None = None

    def __post_init__(self) -> None:
        # Normalise threshold_seconds → threshold_ns en interne pour
        # garder la logique d'évaluation existante intacte.
        if self.threshold_seconds is not None and self.threshold_ns is None:
            self.threshold_ns = int(round(self.threshold_seconds * 1e9))


@dataclass
class SloResult:
    """One SLO evaluation outcome."""

    slo: SloDef
    measured: float | None = None     # nanoseconds for time, ratio else
    passed: bool | None = None
    skipped: bool = False
    skip_reason: str | None = None

    @property
    def status_str(self) -> str:
        if self.skipped:
            return f"SKIPPED ({self.skip_reason})"
        if self.passed is True:
            return "PASS"
        if self.passed is False:
            return "FAIL"
        return "UNKNOWN"


@dataclass
class CheckReport:
    """Aggregate report."""

    fingerprint_match: bool = True
    fingerprint_baseline: str | None = None
    fingerprint_observed: str | None = None
    results: list[SloResult] = field(default_factory=list)

    @property
    def failures(self) -> list[SloResult]:
        return [r for r in self.results if r.passed is False and not r.skipped]

    @property
    def passes(self) -> list[SloResult]:
        return [r for r in self.results if r.passed is True]

    @property
    def skips(self) -> list[SloResult]:
        return [r for r in self.results if r.skipped]


# ──────────────────────────────────────────────────────────────────
# YAML parsing
# ──────────────────────────────────────────────────────────────────


def _require(d: dict, key: str, slo_id: str) -> Any:
    if key not in d:
        raise KeyError(f"SLO {slo_id} is missing required field `{key}`")
    return d[key]


def parse_baseline(path: Path) -> tuple[list[SloDef], str | None]:
    """Load + parse the YAML baseline. Returns (slos, fingerprint_baseline)."""
    try:
        import yaml          # type: ignore  # pyyaml is a workspace dep
    except ImportError:  # pragma: no cover
        sys.stderr.write(
            "FATAL: PyYAML missing. Install with `pip install pyyaml`.\n"
        )
        sys.exit(2)

    raw = yaml.safe_load(path.read_text())
    if not isinstance(raw, dict):
        sys.stderr.write(f"FATAL: {path} : top-level must be a mapping.\n")
        sys.exit(2)

    fingerprint = raw.get("host_fingerprint_baseline")
    # **Audit fix Φ-D.B3b** — toute valeur commençant par
    # `PLACEHOLDER_` est traitée comme un sentinel "pas encore
    # signé" → no fingerprint required. Sans cette tolérance,
    # tout nouveau baseline (= P1, P2, Bundle, CUDA) qui utilise
    # son propre placeholder distinct (e.g.
    # `PLACEHOLDER_compute_at_freeze_with_GpuFingerprint`) verrait
    # ses SLOs `host_fingerprint_required` faussement FAILed avec
    # `REFUSED_BY_FINGERPRINT_MISMATCH` → gates silencieusement
    # cassés en CI.
    if fingerprint is not None and fingerprint.startswith("PLACEHOLDER_"):
        fingerprint = None

    slos_raw = raw.get("slos") or []
    slos: list[SloDef] = []
    for item in slos_raw:
        sid = _require(item, "id", item.get("id", "<unknown>"))
        slos.append(
            SloDef(
                id=sid,
                description=_require(item, "description", sid),
                comparator=_require(item, "comparator", sid),
                status=item.get("status", "ACTIVE"),
                threshold_ns=item.get("threshold_ns"),
                threshold_ratio=item.get("threshold_ratio"),
                threshold_seconds=item.get("threshold_seconds"),     # ← audit fix Φ-C.A4
                threshold_bytes=item.get("threshold_bytes"),         # ← audit fix Φ-C.A4
                threshold_count=item.get("threshold_count"),         # ← audit fix Φ-C.A4
                threshold_bytes_per_hour=item.get("threshold_bytes_per_hour"),  # ← audit fix Φ-D.A1
                threshold_pass_count=item.get("threshold_pass_count"),  # ← Φ-C-FFI
                expected_count=item.get("expected_count"),              # ← Φ-C-FFI
                metric=item.get("metric"),
                quantile=item.get("quantile"),
                host_fingerprint_required=item.get(
                    "host_fingerprint_required", False
                ),
                note=item.get("note"),
                thresholds=item.get("thresholds"),
                source_bench=item.get("source_bench"),
            )
        )
    return slos, fingerprint


# ──────────────────────────────────────────────────────────────────
# Measure parsing (ghz JSON + criterion report)
# ──────────────────────────────────────────────────────────────────


def _ghz_quantile_seconds(measure: dict, q: float) -> float | None:
    """Extract a quantile from ghz `latencyDistribution`.

    `q` is a fraction in [0, 1]. ghz reports % as integer or float
    in the field `percentage`. Returns None if the quantile isn't
    present.
    """
    dist = measure.get("latencyDistribution") or []
    target_pct = q * 100.0
    for entry in dist:
        pct = float(entry.get("percentage", -1))
        if abs(pct - target_pct) < 1e-6:
            lat = entry.get("latency")
            if isinstance(lat, (int, float)):
                return float(lat)
            # ghz can emit a duration string like "12.3ms" — handle below
            return _parse_duration_seconds(str(lat))
    return None


_UNIT_TO_SECS = {
    "ns": 1e-9,
    "us": 1e-6,
    "µs": 1e-6,
    "ms": 1e-3,
    "s": 1.0,
}


def _parse_duration_seconds(s: str) -> float | None:
    """Parse a duration like '12.3ms' or '0.187' (=seconds)."""
    s = s.strip()
    if not s:
        return None
    # Try unit-stripped first
    for unit, mult in sorted(_UNIT_TO_SECS.items(), key=lambda kv: -len(kv[0])):
        if s.endswith(unit):
            try:
                return float(s[: -len(unit)]) * mult
            except ValueError:
                return None
    try:
        return float(s)
    except ValueError:
        return None


def _maybe_ns_from_seconds(s: float | None) -> int | None:
    if s is None:
        return None
    return int(round(s * 1e9))


# ──────────────────────────────────────────────────────────────────
# SLO evaluation
# ──────────────────────────────────────────────────────────────────


def evaluate_one(
    slo: SloDef,
    measure: dict,
    *,
    skip_pending_gpu_runner: bool,
) -> SloResult:
    """Compute pass/fail for a single SLO."""
    # 1. Status skip rules
    if slo.status == "SKIPPED":
        return SloResult(slo=slo, skipped=True, skip_reason="status=SKIPPED")
    if slo.status == "PENDING_GPU_RUNNER" and skip_pending_gpu_runner:
        return SloResult(
            slo=slo, skipped=True, skip_reason="status=PENDING_GPU_RUNNER"
        )

    # 2. Try criterion-style pre-computed `slo_compliance` first.
    comp = (measure.get("slo_compliance") or {}).get(slo.id)
    if comp is not None:
        m_ns = comp.get("measured_ns")
        passed = comp.get("passed")
        if m_ns is not None and passed is not None:
            return SloResult(slo=slo, measured=float(m_ns), passed=bool(passed))

    # 2-bis. **Sprint Φ-C-FFI** — pre-computed `measured_seconds` in
    # `slo_compliance[id]` (= what `test_cold_start.py` /
    # `test_numerical_equiv.py` / Criterion bridge emit). Without this
    # branch, a SLO with `threshold_seconds` AND no `quantile`/`metric`
    # would fall through to step 7 (= silently SKIPPED) → exact
    # replay of pattern Φ-D.A1.
    if slo.threshold_ns is not None and slo.quantile is None:
        comp_id = (measure.get("slo_compliance") or {}).get(slo.id, {})
        m_secs = comp_id.get("measured_seconds")
        if m_secs is not None:
            measured_ns = int(round(float(m_secs) * 1e9))
            passed = (
                (measured_ns <= slo.threshold_ns)
                if slo.comparator == "lte"
                else (measured_ns >= slo.threshold_ns)
            )
            return SloResult(slo=slo, measured=float(measured_ns), passed=passed)
        # `measured_ns` direct (rare ; emitted by some Rust bench
        # bridges that pre-convert).
        m_ns = comp_id.get("measured_ns")
        if m_ns is not None:
            passed = (
                (m_ns <= slo.threshold_ns)
                if slo.comparator == "lte"
                else (m_ns >= slo.threshold_ns)
            )
            return SloResult(slo=slo, measured=float(m_ns), passed=passed)

    # 3. Compute from ghz `latencyDistribution`.
    if slo.threshold_ns is not None and slo.quantile is not None:
        secs = _ghz_quantile_seconds(measure, slo.quantile)
        if secs is None:
            return SloResult(
                slo=slo,
                skipped=True,
                skip_reason=(
                    f"quantile {slo.quantile} absent du ghz "
                    "latencyDistribution"
                ),
            )
        measured_ns = _maybe_ns_from_seconds(secs)
        assert measured_ns is not None
        passed = (
            (measured_ns <= slo.threshold_ns)
            if slo.comparator == "lte"
            else (measured_ns >= slo.threshold_ns)
        )
        return SloResult(slo=slo, measured=float(measured_ns), passed=passed)

    # 4. Threshold-ratio (e.g. SLO-PC-REPRO).
    if slo.threshold_ratio is not None:
        # The bench must populate `slo_compliance[id].measured_ratio`
        # because the ratio depends on 2 runs ; ghz alone can't derive.
        m_ratio = (
            (measure.get("slo_compliance") or {})
            .get(slo.id, {})
            .get("measured_ratio")
        )
        if m_ratio is None:
            return SloResult(
                slo=slo,
                skipped=True,
                skip_reason=(
                    "no measured_ratio (needs 2-run REPRO test ; see "
                    "doc P1 § 5.2.4)"
                ),
            )
        passed = (
            (m_ratio <= slo.threshold_ratio)
            if slo.comparator == "lte"
            else (m_ratio >= slo.threshold_ratio)
        )
        return SloResult(slo=slo, measured=float(m_ratio), passed=passed)

    # 5. Threshold bytes (= SLO-BUNDLE-SIZE / SLO-BUNDLE-VRAM).
    # Audit fix Φ-C.A4 — le measure peut contenir `size_bytes`
    # directement ou via slo_compliance pre-computed.
    if slo.threshold_bytes is not None:
        # Cherche dans slo_compliance pre-computed
        m_bytes = (
            (measure.get("slo_compliance") or {})
            .get(slo.id, {})
            .get("measured_bytes")
        )
        if m_bytes is None:
            # Fallback : top-level `size_bytes` du bench output
            m_bytes = measure.get("size_bytes")
        if m_bytes is None:
            return SloResult(
                slo=slo,
                skipped=True,
                skip_reason="no `measured_bytes` or top-level `size_bytes` in bench output",
            )
        passed = (
            (m_bytes <= slo.threshold_bytes)
            if slo.comparator == "lte"
            else (m_bytes >= slo.threshold_bytes)
        )
        return SloResult(slo=slo, measured=float(m_bytes), passed=passed)

    # 6. Threshold count (= SLO-P2-DOCS-DRIFT et autres count-based).
    if slo.threshold_count is not None:
        m_count = (
            (measure.get("slo_compliance") or {})
            .get(slo.id, {})
            .get("measured_count")
        )
        if m_count is None:
            return SloResult(
                slo=slo,
                skipped=True,
                skip_reason="no `measured_count` in bench output",
            )
        if slo.comparator == "eq":
            passed = m_count == slo.threshold_count
        elif slo.comparator == "lte":
            passed = m_count <= slo.threshold_count
        else:
            passed = m_count >= slo.threshold_count
        return SloResult(slo=slo, measured=float(m_count), passed=passed)

    # 6-bis. Threshold bytes/hour (= SLO-CUDA-SOAK-24H drift rate).
    # Audit fix Φ-D.A1 — la mesure produit un slope OLS sur RSS bytes
    # vs time (= bytes / hour). Le bench expose `measured_bytes_per_hour`
    # dans slo_compliance[id]. Comparator typiquement `lte`.
    if slo.threshold_bytes_per_hour is not None:
        m_bph = (
            (measure.get("slo_compliance") or {})
            .get(slo.id, {})
            .get("measured_bytes_per_hour")
        )
        if m_bph is None:
            return SloResult(
                slo=slo,
                skipped=True,
                skip_reason="no `measured_bytes_per_hour` in bench output (run 24h soak first)",
            )
        passed = (
            (m_bph <= slo.threshold_bytes_per_hour)
            if slo.comparator == "lte"
            else (m_bph >= slo.threshold_bytes_per_hour)
        )
        return SloResult(slo=slo, measured=float(m_bph), passed=passed)

    # 6-ter. Threshold pass-count (= "k passes sur N attendus").
    # **Sprint Φ-C-FFI** — modèle pour SLO du genre
    # `SLO-FFI-EQUIVALENCE` (100/100 fixtures), `SLO-FFI-DETERMINISM`
    # (100/100 runs), `SLO-FFI-CROSS-PY` (4/4 Python versions).
    #
    # Le bench expose dans `slo_compliance[id]` deux champs :
    #   * `measured_pass_count` (int) : nombre de cas passants observés.
    #   * `measured_expected_count` (int, optional) : N attendu — si
    #     présent doit matcher `slo.expected_count` (= sanity check sur
    #     la taille de l'échantillon ; refuse silently a 50/100 si N
    #     a été tronqué à 50 par accident).
    #
    # Comparator `eq` (canonique) → measured == threshold AND, si
    # expected_count est set, observed N == expected N.
    if slo.threshold_pass_count is not None:
        compliance = (measure.get("slo_compliance") or {}).get(slo.id, {})
        m_pass = compliance.get("measured_pass_count")
        m_expected = compliance.get("measured_expected_count")
        if m_pass is None:
            return SloResult(
                slo=slo,
                skipped=True,
                skip_reason="no `measured_pass_count` in bench output",
            )
        # Defensive : if YAML declared expected_count, the bench must
        # have run that many cases — refuse a sample that was silently
        # truncated.
        if (
            slo.expected_count is not None
            and m_expected is not None
            and m_expected != slo.expected_count
        ):
            return SloResult(
                slo=slo,
                measured=float(m_pass),
                passed=False,
                skip_reason=(
                    f"sample size mismatch : expected {slo.expected_count}, "
                    f"got {m_expected}"
                ),
            )
        if slo.comparator == "eq":
            passed = m_pass == slo.threshold_pass_count
        elif slo.comparator == "lte":
            passed = m_pass <= slo.threshold_pass_count
        else:
            passed = m_pass >= slo.threshold_pass_count
        return SloResult(slo=slo, measured=float(m_pass), passed=passed)

    # 7. Multi-threshold (BURN rate). Considered ACTIVE but never
    # evaluated by check_slo.py (Prometheus alert handles live ; CI
    # checks for static baseline only).
    if slo.thresholds is not None:
        return SloResult(
            slo=slo,
            skipped=True,
            skip_reason="multi-threshold BURN — gated par Prometheus rules.p1, pas par CI",
        )

    return SloResult(
        slo=slo, skipped=True, skip_reason="no comparable metric in bench output"
    )


def check_fingerprint(
    baseline_fp: str | None, observed_fp: str | None
) -> bool:
    """Return True if fingerprints match (or no baseline to compare)."""
    if baseline_fp is None:
        return True  # pas de baseline signée ⇒ pas de gating
    if observed_fp is None:
        return False  # baseline existe mais runner n'a pas fourni → fail
    return observed_fp == baseline_fp


# ──────────────────────────────────────────────────────────────────
# Reporting
# ──────────────────────────────────────────────────────────────────


def render_markdown(report: CheckReport) -> str:
    lines = [
        "# P1 SLO Compliance Report",
        "",
    ]
    fp_status = "MATCH" if report.fingerprint_match else "MISMATCH"
    lines.extend(
        [
            f"**Host fingerprint** : `{fp_status}`",
            f"- baseline : `{report.fingerprint_baseline or 'n/a'}`",
            f"- observed : `{report.fingerprint_observed or 'n/a'}`",
            "",
            "| ID | Status | Measured | Threshold | Description |",
            "|----|:------:|---------:|----------:|-------------|",
        ]
    )
    for r in report.results:
        s = r.slo
        if r.measured is None:
            measured = "—"
        elif s.threshold_ns is not None:
            measured = f"{r.measured / 1e6:.2f} ms"
        elif s.threshold_ratio is not None:
            measured = f"{r.measured:.4f}"
        else:
            measured = f"{r.measured:.2f}"
        if s.threshold_ns is not None:
            threshold = f"{s.threshold_ns / 1e6:.2f} ms"
        elif s.threshold_ratio is not None:
            threshold = f"{s.threshold_ratio}"
        else:
            threshold = "—"
        emoji = (
            "✅" if r.passed is True
            else "❌" if r.passed is False
            else "⏸️"
        )
        # Suffixe le `description` avec le `skip_reason` quand
        # présent — c'est utile dans 2 cas :
        #   (a) status REFUSED_BY_FINGERPRINT_MISMATCH (passed=False)
        #   (b) skip_pending_gpu_runner (skipped=True, raison
        #       PENDING_GPU_RUNNER).
        desc = s.description
        if r.skip_reason:
            desc = f"{desc} _({r.skip_reason})_"
        lines.append(
            f"| `{s.id}` | {emoji} {r.status_str} | {measured} | {threshold} | {desc} |"
        )
    lines.extend(
        [
            "",
            f"**Total** : {len(report.passes)} PASS, "
            f"{len(report.failures)} FAIL, "
            f"{len(report.skips)} SKIP.",
        ]
    )
    return "\n".join(lines) + "\n"


def render_json(report: CheckReport) -> str:
    return json.dumps(
        {
            "fingerprint_match": report.fingerprint_match,
            "fingerprint_baseline": report.fingerprint_baseline,
            "fingerprint_observed": report.fingerprint_observed,
            "results": [
                {
                    "id": r.slo.id,
                    "description": r.slo.description,
                    "passed": r.passed,
                    "skipped": r.skipped,
                    "skip_reason": r.skip_reason,
                    "measured": r.measured,
                    "comparator": r.slo.comparator,
                    "threshold_ns": r.slo.threshold_ns,
                    "threshold_ratio": r.slo.threshold_ratio,
                }
                for r in report.results
            ],
            "summary": {
                "pass": len(report.passes),
                "fail": len(report.failures),
                "skip": len(report.skips),
            },
        },
        indent=2,
    )


# ──────────────────────────────────────────────────────────────────
# CLI
# ──────────────────────────────────────────────────────────────────


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--measure", type=Path, required=True)
    parser.add_argument("--fingerprint", type=str, default=None)
    parser.add_argument("--skip-pending-gpu-runner", action="store_true")
    parser.add_argument("--strict-skip", action="store_true")
    parser.add_argument("--report", type=Path, default=None)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    if not args.baseline.exists():
        sys.stderr.write(f"FATAL: baseline file {args.baseline} not found.\n")
        return 2
    if not args.measure.exists():
        sys.stderr.write(f"FATAL: measure file {args.measure} not found.\n")
        return 2

    slos, baseline_fp = parse_baseline(args.baseline)
    measure = json.loads(args.measure.read_text())

    fp_match = check_fingerprint(baseline_fp, args.fingerprint)

    report = CheckReport(
        fingerprint_match=fp_match,
        fingerprint_baseline=baseline_fp,
        fingerprint_observed=args.fingerprint,
    )

    for slo in slos:
        if slo.host_fingerprint_required and not fp_match:
            report.results.append(
                SloResult(
                    slo=slo,
                    skipped=False,
                    passed=False,
                    skip_reason="REFUSED_BY_FINGERPRINT_MISMATCH",
                )
            )
            continue
        report.results.append(
            evaluate_one(
                slo,
                measure,
                skip_pending_gpu_runner=args.skip_pending_gpu_runner,
            )
        )

    md = render_markdown(report)
    if args.report:
        args.report.write_text(md)
    if args.json:
        print(render_json(report))
    elif not args.quiet:
        sys.stdout.write(md)

    # Exit-code discipline
    if not fp_match:
        return 3
    if report.failures:
        return 1
    if args.strict_skip and report.skips:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
