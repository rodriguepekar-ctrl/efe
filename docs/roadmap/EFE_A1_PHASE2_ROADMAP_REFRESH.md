# EFE-A1 Phase 2 — Roadmap refresh (post PR #24)

**Status** : LIVE
**Date** : 2026-05-16 (post 10-pass audit of PR #24)
**Owner** : `@efe-ai/engineering-leadership`
**Companion docs** :
- Lessons : [`docs/lessons/pr24-lessons-learned.md`](../lessons/pr24-lessons-learned.md)
- Soak report : [`docs/reports/soak-report-2026-05-16.md`](../reports/soak-report-2026-05-16.md)
- Master runbook : [`docs/runbooks/bench-publish-incident-response.md`](../runbooks/bench-publish-incident-response.md)
- Quarterly drill : [`docs/drills/drill-quarterly-revocation.md`](../drills/drill-quarterly-revocation.md)
- Defacement drill : [`docs/drills/drill-defacement-response.md`](../drills/drill-defacement-response.md)

---

## 0. Why a refresh now

PR #24 finished a 10-pass chirurgical audit on 2026-05-16, identifying
**67 defects / coverage gaps** and pinning **6 known residual gaps**
explicitly carried into Phase 3.

The original Phase 2 roadmap predates this audit and underestimated
both the audit cost and the residual-gap surface.  This refresh :

1. Marks PR #24 as **shipped** (it is) — but with audit findings.
2. Pulls the 6 known residual gaps onto the Phase 3 G-stream.
3. Reprioritises Phase 3 around the audit's empirical defect
   distribution (D-Z / D-AQ / D-BE class issues weight heavier than
   we'd planned for).
4. Adds an "audit budget" line item — every future PR gets a
   30-50 % audit overhead written into its estimate up front.
5. Re-times Phase 3 (was Q3-2026 start ; now Q4-2026 start to
   absorb the residual-gap work).

---

## 1. Phase 2 status (where we are)

### 1.1 Shipped PRs

| PR | Title | Status | Audit verdict |
|---|---|---|---|
| #16 | Workspace foundations | ✅ shipped | clean (pre-audit-process) |
| #17 | Loadgen v0 | ✅ shipped | clean |
| #18 | Cluster-provisioner v0 (inline) | ✅ shipped | superseded by #23 |
| #19 | Scenario YAML schema | ✅ shipped | clean |
| #20 | Loadgen profile YAML | ✅ shipped | clean |
| #21 | Public-bench harness (PR #21 core) | ✅ shipped | clean (10 scenarios + 5 metrics) |
| #22 | SLO portal (parallel chain) | ✅ shipped | regex asymmetry surfaced by PR #24 audit (intentional, documented) |
| #23 | Cluster-provisioner extraction | ✅ shipped | byte-compat shim verified pass 10 D-BF |
| **#24** | **Publish bench results (public bench portal)** | ✅ shipped | **67 defects fixed across 10-pass audit ; 6 residual gaps tracked** |

### 1.2 Phase 2 frozen surface

STABILITY_CONTRACT v1.6 — frozen for ≥ 6 chart-minor versions :
- 11 SVG badges (10 scenarios + overall)
- 5 JSON API endpoints (`/api/{current,history,scenarios,baselines,version}.json`)
- 10 scenario page slugs (D-U fix : enforced via `SCENARIO_PAGE_FILENAMES`)
- Quarterly evidence path `/quarterly/Q{1..4}-{YYYY}.{json,pdf,pdf.bundle}`
- CLI surface : 7 `publish` flags + `publish-quarterly` + `evidence verify`
  subcommands (per § 24.5)
- Sparkline dimensions 200×40
- Shared cosign keyless OIDC identity regex (cross-chain via PR #24
  bench verifier — D-BE fix made this match the doc)
- gh-pages branch protection (force-push disabled, signed commits,
  CODEOWNERS, 2 approvals, linear history) (D-BK invariant test)

The frozen surface is now codified by **48 stability + invariant
tests** (was 9 in pass 1, +39 across the audit).

### 1.3 Phase 2 unfrozen (still mutable)

- The aggregator implementation (private, can refactor)
- The trend / sparkline data-source plumbing (Phase-3 G.10)
- The mdbook source `bench/publish/src/` (themed without breaking the
  frozen badge filenames + JSON endpoints)
- All test code (modify freely)

---

## 2. Phase 3 — refreshed plan

### 2.1 G-stream (gap closure from PR #24 audit)

These 6 items were pinned as known gaps in PR #24 and MUST be closed
in early Phase 3.  The pinned tests (D-AZ et al.) will fail when each
fix lands, making progress visible.

| ID | Title | Owner candidate | Estimate | Test that pins the gap |
|---|---|---|---|---|
| **G.10** | History ingestion : populate `/api/history.json` from a 30-day side store | release-eng | 5 weeks | `current_json_ignores_unknown_field_ad16` (D-W) |
| **G.11** | Tolerance_used population : aggregator re-runs compare against baseline | bench-platform | 3 weeks | `d_az_tolerance_used_currently_always_empty_pinned` |
| **G.12** | Offline Rekor inclusion proof verifier | security-eng | 4 weeks | (new — Phase-3 test) |
| **G.13** | Unicode-aware PDF (Type0 CIDFont) | report-eng | 6 weeks | `d_al_escape_strips_non_ascii_to_question_mark` |
| **G.14** | `recovery_time_s` populated by loadgen extension | loadgen-team | 4 weeks | `d_ba_recovery_time_s_field_present_but_unpopulated_in_aggregator` |
| **G.15** | Post-mortem template enforced by CI | release-eng | 1 week | (new — workflow YAML test) |

Total G-stream effort : **23 weeks** of focused work, split across
4 sub-teams.  Critical path : G.10 (history ingestion) blocks the
real 30-day sparkline shown in `/badges/trend-*.svg`.

### 2.2 H-stream (new Phase 3 capabilities)

| ID | Title | Rationale | Estimate |
|---|---|---|---|
| **H.1** | PR-comment generator polishes (multi-PR aggregation) | Currently one comment per PR ; PMs want a rollup | 3 weeks |
| **H.2** | Custom theme support for the bench mdbook | Operators want brand colours without breaking CSP | 4 weeks |
| **H.3** | Per-customer signed evidence packs | Enterprise feature : a customer can request a private-signed bundle | 8 weeks |
| **H.4** | Soak duration > 2 h (12 h overnight + 7 d week-long) | Longer soaks for confidence intervals | 2 weeks (scenario YAML) + ops time |
| **H.5** | Chaos faults catalogue expansion (5 → 12) | More variety in S04-S07 | 6 weeks |

H.4 + the existing 10-scenario surface combined : Phase 3 can ship
a chart-minor 0.7.0 with a stronger evidence binder.

### 2.3 I-stream (process)

Insights from the 10-pass audit codified as required-process steps :

| ID | Title | Trigger |
|---|---|---|
| **I.1** | Pre-merge audit prompt mandatory ; output is a `docs/lessons/PRN-lessons.md` file | Every PR ≥ 1000 LOC |
| **I.2** | Single-source-of-truth tables for parallel lists | Every new const list of paths / filenames / flags |
| **I.3** | Workflow YAML structural tests mandatory | Every new `.github/workflows/*.yml` |
| **I.4** | Doc/code reconciliation CI step | Every PR touching `docs/STABILITY_CONTRACT.md` |
| **I.5** | Drill cadence enforced : quarterly defacement, annual revocation | Calendar-based, owned by release-eng |
| **I.6** | "Audit budget" line item in PR estimates | Every PR estimate now includes 30-50 % overhead for self-audit |

I-stream is zero-LOC engineering ; it changes how we WORK, not what
we build.  But it's the largest predicted ROI from the audit retro :
the same 67 defects do not recur in PR #25.

### 2.4 J-stream (deferred to Phase 4)

Items considered for Phase 3 but moved to Phase 4 after the audit
re-estimate :

- **Multi-region active-active publish** (current : single region)
- **Per-customer SLA dashboards** (current : per-chart-minor)
- **Anomaly detection on history.json metric drift** (current : human review)
- **Self-service evidence-pack download UX** (current : auditor email)

These are real value but require G.10 (history) to land first.

---

## 3. Timeline

### 3.1 Original Phase 2 → Phase 3 cut

```
Phase 2 :  Q3-2025 ──── PR #16..#23 ──── Q1-2026
Phase 3 :  Q2-2026 (start) ──── stuff TBD
```

### 3.2 Refreshed

```
Phase 2 :  Q3-2025 ──── PR #16..#24 + 10-pass audit ──── Q2-2026 mid
Phase 3 :  Q4-2026 start (3 month slip)
           ├─ G-stream (gap closure)         Q4-2026 → Q1-2027
           ├─ H-stream (new capabilities)    parallel
           └─ I-stream (process)             active now
Phase 4 :  Q3-2027 (multi-region etc.)
```

The 3-month slip is because :
1. PR #24 audit + lessons capture took 6 weeks vs planned 2 weeks.
2. G-stream (residual gap closure) is critical-path for Phase 3
   user-visible features ; better to slip than to ship more partial.
3. I-stream changes (audit-mandatory, single-source-of-truth) require
   onboarding the whole team before the next big PR.

---

## 4. Risk register update

### 4.1 Top 10 risks (revised post-audit)

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | "Done" self-assessment fails again on a future PR | High (history) | High | I.1 mandatory audit prompt |
| 2 | Schema-version drift between PRs | Medium | Medium | I.2 + meta-test enforcer |
| 3 | gh-pages defacement | Low | Very high | Defacement drill quarterly + master runbook |
| 4 | Cosign sigstore outage during quarterly publish | Medium | Medium | Master runbook § 4.2 + workflow retry on next cron |
| 5 | Rekor unreachable at verify time | Medium | Low | G.12 offline verifier |
| 6 | mdbook hangs (network theme fetch) | Low | Low | D-AY 60s timeout fixed in audit |
| 7 | Page slug regression breaks click-through | Low (post D-U) | High | `SCENARIO_PAGE_FILENAMES` SOT |
| 8 | Auditor sees mojibake `Â§` in PDF | Low (post D-AL) | Medium | ASCII-only body + G.13 Unicode fix |
| 9 | Concurrent quarterly publish corruption | Very low (post D-Z + D-BJ) | High | UUID temp suffix + concurrency group |
| 10 | New maintainer ships PR without audit | Medium | Medium | I.1 + I.6 onboarding |

### 4.2 Removed from prior register (mitigated)

- "Aggregator never populates current.json" — fixed pass 3 D-A/B/C
- "PDF won't verify due to xref" — fixed pass 8 D-AQ
- "Cross-chain verifier broken" — fixed pass 10 D-BE
- "Cosign-uninvokable silent pass" — fixed pass 4 D-M

### 4.3 New risks identified by audit

- **Documentation drift > code drift** (12 of 67 defects).  Mitigation : I.4.
- **Coverage bias toward error paths** (16 of 67).  Mitigation : I.1 forces
  happy-path tests too.
- **Single-source-of-truth duplication** (D-AS ratchet drift, D-U slug
  drift).  Mitigation : I.2 mandates SOT for parallel lists.

---

## 5. Budget & resourcing

### 5.1 Headcount needed for Phase 3 (refreshed)

| Team | FTE | Primary G-stream items | Secondary |
|---|---|---|---|
| release-eng | 2.0 | G.10, G.15 | Drills, runbooks |
| bench-platform | 1.5 | G.11, H.1, H.2 | — |
| security-eng | 1.0 | G.12 | I.5 enforcement |
| report-eng | 1.5 | G.13, H.3 | — |
| loadgen-team | 1.0 | G.14, H.4, H.5 | — |
| **Total** | **7.0 FTE** | — | — |

Previously planned 5.0 FTE for Phase 3 ; the +2 FTE is the audit
budget for G-stream + the I-stream onboarding cost.

### 5.2 Audit overhead

I.6 mandates that every PR estimate include a 30-50 % audit overhead.
For PR #24 retrospective sizing :

- Implementation : 4 weeks
- Audit (10 passes) : 6 weeks (150 % overhead — outlier driven by
  it being the first audit-as-explicit-process PR)

Expected steady-state overhead : 30 % (G-stream PRs in Phase 3 will
land with 30 % audit budget built in).

### 5.3 Calendar overhead

- Annual quarterly revocation drill : 90 min × 4 participants = 6 h
- Quarterly defacement drill : 60 min × 3 participants × 4 = 12 h
- Master runbook review : 1 h × 4 quarterly = 4 h
- Lessons-learned doc per PR ≥ 1000 LOC : ~3-5 h
- Total : ~30 h / year of recurring release-eng calendar time.

---

## 6. Decision log (for Phase 3 planning sign-off)

| Decision | Rationale | Owner | Date |
|---|---|---|---|
| Slip Phase 3 start by 3 months | Audit work + residual gaps require it | engineering-leadership | 2026-05-16 |
| Mandate I.1 audit prompt for PR ≥ 1000 LOC | 67-defect signal from PR #24 | engineering-leadership | 2026-05-16 |
| Add 2 FTE to Phase 3 plan | G-stream + I-stream load | engineering-leadership | 2026-05-16 |
| Defer multi-region active-active to Phase 4 | Depends on G.10 history | engineering-leadership | 2026-05-16 |
| Adopt quarterly drill cadence | Defacement-response muscle memory | release-eng | 2026-05-16 |
| Adopt annual revocation drill cadence | SOC 2 audit alignment | release-eng + compliance | 2026-05-16 |
| Standardise `docs/lessons/` directory | Per-PR retrospectives | release-eng | 2026-05-16 |
| Standardise `docs/postmortems/_template.md` (Phase 3 G.15) | CI-enforced post-mortem shape | release-eng | 2026-05-16 |
| **PR #25 merged with 8 audit defects (5.6× below 67 budget)** | I-stream validated empirically ; STABILITY v1.7 § 25 frozen | release-eng + engineering-leadership | 2026-05-16 |
| **Branch protection : require `pr25 process checks pass`** | Operationalises I.1+I.2+I.6 enforcement at merge time | release-eng + SRE | 2026-05-16 |
| **Maintain 30 % audit-budget target through PR #26-#28** | PR #25 came in at 40 % ; need ≥ 3 data points before recalibrating | release-eng | 2026-05-16 |

---

## 7. Success criteria for Phase 3 (chart-minor 0.7.0)

Phase 3 ships when ALL of these hold :

- [ ] G.10–G.15 complete (all 6 pinned tests updated by their fixes).
- [x] **PR #25 (the first Phase-3 PR) lands with < 20 audit defects**
  **(test of whether I-stream worked). ✅ PASS — 8 defects, 5.6×**
  **better than the 67 budget ceiling.  Verdict : I-stream works.**
- [ ] Quarterly drills run 4× across Phase 3 with verdict pass on all.
- [ ] Revocation drill runs 1× with verdict pass.
- [x] **STABILITY_CONTRACT v1.7 cuts cleanly with no behaviour breakage**
  **for existing v1.6 consumers ✅ (forward-compat tests + v1.7 stability**
  **suite passing, 440/0).**
- [ ] Master runbook updated with at least one real (non-drill)
  incident learnings.
- [ ] 0 P0 incidents on the production bench-publish surface during
  Phase 3 (audit confidence indicator).

**Updated 2026-05-16 post-PR #25 merge** : 2 of 7 success criteria
already met.  Remaining 5 are operational (drills + real incidents
+ G-stream G.10..G.15).

If the PR #25 audit defect count is ≥ 67, we will pause Phase 3 and
re-examine I-stream — the audit-process change didn't take.  **Not
triggered : PR #25 landed at 8 defects.**

---

## 8. Open questions for engineering-leadership review

1. **Audit prompt enforcement** : how do we operationally make I.1
   not optional?  Proposal : PR template's checklist + reviewer-blocking
   label.  Alternative : a separate `audit-passed` status check.

2. **Funding G.13 (Type0 PDF)** : the Unicode-aware PDF is high-effort
   (6 weeks) for a "remove ? in body" outcome.  Is the audit-trail
   improvement enough?  Or do we live with the limitation for another
   chart-minor?

3. **Defacement drill against prod** : the current drill uses a
   staging branch.  Should we run it against PROD once / year (with
   a planned customer-visible window)?  Higher fidelity vs reputational
   risk trade-off.

4. **Lessons-learned visibility** : should `docs/lessons/` be public
   in the repo (current proposal), or restricted to internal docs?
   Current docs are public ; the 67-defect honest accounting may be
   read by competitors.  Counter : the honesty is also a marketing
   moment for trust-conscious customers.

5. **Audit budget granularity** : 30 % for steady-state is the
   estimate.  Should we measure per-PR audit ratios for the first
   3 Phase-3 PRs and re-calibrate?

These are flagged for the Phase 3 kickoff meeting (2026-09-15).

---

## 9. References & history

- Phase 1 retrospective : `docs/lessons/phase1-retrospective.md` (not
  yet authored — Phase 1 predated the audit-process change)
- PR #24 audit lessons : `docs/lessons/pr24-lessons-learned.md`
- Soak report : `docs/reports/soak-report-2026-05-16.md`
- STABILITY_CONTRACT history : `docs/STABILITY_CONTRACT.md` § "Audit trail addendum"

Last edits :
- 2026-05-16 : initial refresh post PR #24 audit.
- (Next : 2026-09-15 at Phase 3 kickoff.)
