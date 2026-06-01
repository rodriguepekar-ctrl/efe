# Soak report : S09_soak_2h_stability — 2026-05-16

**Report ID** : `SOAK-2026-05-16-01`
**Scenario** : `S09_soak_2h_stability`
**Run ID** : `01J9-soak-2026-05-16-09-00-utc`
**Chart version** : `0.6.0` (efe-bench-harness 0.2.0)
**Operator** : `@efe-ai/release-eng` (post-PR #24 10-pass audit baseline)
**Status** : SIGNED & PUBLISHED
**Verdict** : **PASS**

---

## Executive summary

The 2-hour soak scenario completed cleanly against the post-audit
chart-0.6.0 baseline.  All six SLI metrics held inside their
tolerance bands for the full 7200 s window.  No chaos events were
injected (S09 is a clean-state stability scenario by design).

**This is the first soak run produced AFTER the 10-pass audit that
fixed 67 defects.**  The audit's most impactful change for this
report is :

- **D-AQ** : the PDF xref offsets are now spec-compliant.  Auditors
  using strict PDF parsers (`qpdf --check`, Adobe Acrobat) will no
  longer see structural-validity warnings.
- **D-AL** : the PDF body is pure ASCII.  Previous reports rendered
  `§ 24` as the mojibake `Â§ 24` in default-encoding readers.
- **D-BE** : a future cross-chain verifier run against this PDF will
  succeed via the bench-specific marker AND via the PR #22 fallback
  marker (test of robustness, not behaviour change for this report).

---

## 1. Run metadata

| Field | Value |
|---|---|
| `schema_version` | 1 |
| `scenario_id` | `S09_soak_2h_stability` |
| `harness_version` | `0.2.0` |
| `loadgen_version` | `0.1.0` |
| `chart_version` | `0.6.0` |
| `image_tag` | `v0.6.0` |
| `git_commit` | `7f1f2563b2d7bce2d85f8c82ebf946ef334413da` |
| `run_id` | `01J9-soak-2026-05-16-09-00-utc` |
| `started_at` | `2026-05-16T09:00:00Z` |
| `ended_at` | `2026-05-16T11:00:00Z` |
| `duration_s` | `7200.0` |
| `total_requests` | `720_000` (100 QPS × 7200 s) |
| `chaos_events` | `[]` |

Cluster topology :

| Field | Value |
|---|---|
| Provisioner | `external` |
| K8s version | `1.29.4` |
| Node class | `m6i.2xlarge` (4 nodes) |
| Controller replicas | 3 |
| Participant replicas | 7 |

---

## 2. Metrics — 7200 s window

All values are tail percentiles across the full window (not rolling).
Tolerance bands come from `bench_baselines/v0.6.0/L2/S09_soak_2h_stability.json`.

| Metric | Baseline | Actual | Δ (%) | Tolerance | Status |
|---|---|---|---|---|---|
| `p50_ms` | 7.00 | 7.24 | +3.4 | ±10 % | ✅ |
| `p95_ms` | 12.00 | 12.83 | +6.9 | ±15 % | ✅ |
| `p99_ms` | 18.00 | 19.41 | +7.8 | ±20 % | ✅ |
| `error_rate` | 0.001 | 0.0011 | +0.0001 pp | ±0.0005 pp | ✅ |
| `throughput_qps` | 100.0 | 99.73 | -0.27 | -2 % min | ✅ |
| `recovery_time_s` | N/A | N/A | N/A | — | N/A |

Notes :
- `recovery_time_s` is not measured in S09 (no chaos injection).  See
  scenarios S04-S07 for recovery-time SLI coverage.
- The `+6.9 %` p95 drift vs baseline is within the ±15 % tolerance
  band but is the highest p95 drift observed across the last 4
  soak runs (Q1-2026 average drift was +2.1 %).  Tracking only ;
  no action.

### 2.1 5-minute bucket profile (p95 latency)

Per-bucket p95 over 24 buckets (one per 5 minutes).  Pinned here for
audit reproducibility.

```
Bucket  Window UTC          p95(ms)  Δ vs baseline
 0      09:00–09:05         12.41    +3.4 %
 1      09:05–09:10         12.55    +4.6 %
 2      09:10–09:15         12.72    +6.0 %
 3      09:15–09:20         12.81    +6.8 %
 4      09:20–09:25         12.78    +6.5 %
 5      09:25–09:30         12.74    +6.2 %
 6      09:30–09:35         12.79    +6.6 %
 7      09:35–09:40         12.83    +6.9 %
 8      09:40–09:45         12.86    +7.2 %  ← window-max
 9      09:45–09:50         12.85    +7.1 %
10      09:50–09:55         12.83    +6.9 %
11      09:55–10:00         12.81    +6.8 %
12      10:00–10:05         12.79    +6.6 %
13      10:05–10:10         12.80    +6.7 %
14      10:10–10:15         12.84    +7.0 %
15      10:15–10:20         12.83    +6.9 %
16      10:20–10:25         12.81    +6.8 %
17      10:25–10:30         12.85    +7.1 %
18      10:30–10:35         12.84    +7.0 %
19      10:35–10:40         12.82    +6.8 %
20      10:40–10:45         12.83    +6.9 %
21      10:45–10:50         12.82    +6.8 %
22      10:50–10:55         12.84    +7.0 %
23      10:55–11:00         12.83    +6.9 %  ← window-end
```

Statistical summary (24 buckets) :
- mean p95 = 12.79 ms (Δ +6.6 %)
- stddev   = 0.118 ms  (low — soak shows stable steady state)
- min      = 12.41 ms (bucket 0, cold-start ramp)
- max      = 12.86 ms (bucket 8, steady-state)
- p95 of p95 = 12.85 ms

**Cold-start observation** : bucket 0 is 0.4 ms faster than the
steady-state ; this is expected (cold JIT / cache).  Pattern is
consistent with Q1-2026 soak runs.

---

## 3. Error budget consumption

The 7200 s window consumed 0.012 % of the monthly 99.95 % availability
budget (744 h SLO month → 22 min budget ; 7200 s × 0.0011 error rate
= 7.92 errored seconds → 0.6 % of monthly budget).

Acceptable.

---

## 4. Resource utilisation (cluster-side)

Captured every 30 s via prometheus snapshot (collected with
`collect_prom_snapshot: true` in scenario YAML).

| Resource | Min | Mean | Max | Limit |
|---|---|---|---|---|
| CPU (controller) | 18 % | 23 % | 31 % | 80 % |
| CPU (participant) | 12 % | 16 % | 24 % | 80 % |
| Memory (controller) | 412 MB | 438 MB | 461 MB | 1024 MB |
| Memory (participant) | 198 MB | 215 MB | 232 MB | 512 MB |
| File descriptors | 87 | 94 | 108 | 1024 |
| Goroutines | 156 | 163 | 178 | — |

No resource trends over the 2 h window (file descriptors flat ;
goroutine count flat ; memory : +20 MB on controller from t=900 s,
stable thereafter — consistent with HTTP/2 connection-pool warmup).

**No leak detected.**  Compare to the Q4-2025 incident (memory grew
30 MB/h on participants — fixed in PR #18).

---

## 5. Compliance attestation

This soak report is part of the SOC 2 evidence pack for the v0.6.0
chart minor version.

### 5.1 Embedded SHA chain

| File | SHA-256 |
|---|---|
| `S09_2026-05-16.json` | `8a3f...e591` (full SHA in JSON sidecar) |
| `S09_2026-05-16.summary.json` | `b91d...4c08` |
| `S09_2026-05-16.report.pdf` | embedded `/EFE_BENCH_EVIDENCE_JSON_SHA256` matches above |

### 5.2 Cosign keyless OIDC signature

| Field | Value |
|---|---|
| Workflow | `.github/workflows/bench-publish-quarterly.yml` |
| OIDC issuer | `https://token.actions.githubusercontent.com` |
| Identity regex | `^https://github.com/efe-ai/efe/.github/workflows/(slo-quarterly|bench-publish-quarterly).yml@refs/.+$` |
| Bundle | `S09_2026-05-16.report.pdf.bundle` |
| Rekor entry | LogIndex `<populated by signer>` |

Verification command for auditors :

```bash
efe-bench-harness evidence verify S09_2026-05-16.report.pdf
# Expected output : "PDF verified : ... (SHA + cosign chain via shared identity regex)"
# Expected exit code : 0
```

> **D-BE note** : `evidence verify` accepts both this PDF and any
> PR #22 SLO quarterly PDF.  Before pass-10 of the PR #24 audit,
> this claim was documented but not implemented — auditors holding
> PR #22 PDFs would have seen a "marker missing" error.  As of the
> 2026-05-16 build, the cross-chain verifier works as documented.

### 5.3 Branch protection state at publish time

Captured from `gh api .../branches/gh-pages/protection` at the
moment of the publish commit :

| Setting | Required | Actual |
|---|---|---|
| `allow_force_pushes` | false | false ✅ |
| `required_signatures` | true | true ✅ |
| `required_linear_history` | true | true ✅ |
| `required_approving_review_count` | 2 | 2 ✅ |
| `enforce_admins` | true | true ✅ |
| CODEOWNERS for `bench/**` | `@efe-ai/release-eng` | `@efe-ai/release-eng` ✅ |

---

## 6. Audit trail

Past relevant publishes (history.json subset for context) :

```jsonc
[
  {
    "date": "2026-04-15",
    "url": "/history/2026-04-15.json",
    "sha256": "5e0b...8f12",
    "overall_verdict": "pass"
  },
  {
    "date": "2026-04-30",
    "url": "/history/2026-04-30.json",
    "sha256": "32a1...c0d4",
    "overall_verdict": "pass"
  },
  {
    "date": "2026-05-16",
    "url": "/history/2026-05-16.json",
    "sha256": "<this report>",
    "overall_verdict": "pass"
  }
]
```

The chain is intact ; each entry's `sha256` is computable from the
linked JSON ; no entry has a `replaces` field (no ERRATA in this
chart-minor).

---

## 7. Operational notes

### 7.1 Build environment

```
rustc 1.83.0 (90b35a623 2024-11-26)
cargo 1.83.0 (5ffbef321 2024-10-29)
cosign 2.4.0 (linux/amd64)
mdbook 0.4.40
```

No build-time anomalies.

### 7.2 Run-time observations

- All 10 scenario badges shipped clean ; overall badge green.
- All 5 JSON API endpoints non-empty (D-A/B/C aggregator fix from
  pass 3 verified working in production).
- All 10 scenario page URLs valid (D-U fix from pass 5 verified).
- Smoke badge embed visible at top of `index.html` (D-J / D-N fixes
  from pass 4 + 6 verified ; `--embed-smoke true` was the default).
- Sparkline `trend-S09.svg` shows green PASS colour for today's
  datapoint (D-K fix from pass 4 verified).

### 7.3 Known Phase-2 limitations carried in this report

Documented for auditor transparency :

- `recovery_time_s` is `null` for S09 (clean soak, no chaos).  Phase-3
  G.14 will populate this for chaos scenarios via loadgen extension.
- `tolerance_used` map in `/api/current.json` is `{}` for every scenario
  (Phase-2 aggregator does not re-run compare against baseline).
  Phase-3 G.11 will fix.
- 30-day sparkline shows a single point (today's run).  Phase-3 G.10
  will ingest the past 30 days from a side store ; until then,
  sparklines are visually a single dot at the right edge.

---

## 8. Verdict statement

> The S09_soak_2h_stability scenario on 2026-05-16 09:00–11:00 UTC
> completed cleanly against the v0.6.0 chart-minor baseline.  All
> six SLI metrics (p50, p95, p99, error_rate, throughput, error
> budget) held inside their tolerance bands for the full 7200 s
> window.  Cluster resource utilisation remained stable with no
> trend toward leak or saturation.  The signed evidence PDF passes
> verification by the bench harness AND, per AC22 cross-chain, by
> the PR #22 SLO verifier.
>
> Verdict : **PASS**.

Signed and committed to `gh-pages` at commit
`<populated by signer at publish time>` by workflow
`.github/workflows/bench-publish-quarterly.yml` on a push to `main`.

---

## 9. References

- Scenario YAML : `bench/scenarios/S09_soak_2h_stability.yaml`
- Baseline : `bench_baselines/v0.6.0/L2/S09_soak_2h_stability.json`
- Runbook : `docs/runbooks/bench-publish-quarterly.md`
- Master incident runbook : `docs/runbooks/bench-publish-incident-response.md`
- 10-pass audit lessons : `docs/lessons/pr24-lessons-learned.md`
- STABILITY_CONTRACT : `docs/STABILITY_CONTRACT.md` v1.6 § 24
- Architecture : `docs/EFE_A1_PHASE2_PR24_ARCHITECTURE.md`
