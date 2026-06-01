# EFE-A1 Phase 2 — PR #19 : Grafana dashboards + Prometheus alerts + SLO burn

| Field | Value |
|---|---|
| **PR** | #19 (Sprint A1 Phase 2 / P2.4 W9 → P2.5 W10) |
| **Branch** | `efe-a1/p2-pr19-grafana-alerts` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team — observability area |
| **Reviewers** | 2× SRE + 1× meta-swarm lead + 1× security + 1× compliance + 1× on-call lead |
| **Depends on** | PR #12 (reload metrics), PR #16 (TLS metrics + spans), PR #17 (14 Prometheus metrics + STABILITY_CONTRACT v1), PR #18 (8 OTel spans + 3 OTel exporter metrics + W3C propagation) |
| **Blocks** | PR #22 (Phase-3 SLO formalization — needs recording rules), PR #23 (chaos benches — needs the symptom alerts to assert recovery), GA v1.0 (no production roll-out without paging surface) |
| **L2 gate** | G16 (production-grade operator surface : 9 dashboards + 10 alerts + 4 SLOs + 6 recording rules + 1 Alertmanager routing tree + runbook coverage for every critical alert) |
| **Threat IDs in scope** | T66 – T75 (10 new : dashboard XSS via panel title, PromQL injection in dashboard variables, Alertmanager webhook spoof, runbook URL phishing, dashboard data exfil via public share link, SLO target tampering by RBAC bypass, alert silence abuse, recording-rule cardinality explosion, datasource credential leak via JSON export, Grafana session hijack) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS2 (Observability) + § 12 (Operator UX) |
| **CVSS bandwidth** | All ≤ SEV-2 (no PII leak, no auth bypass, no privilege escalation) |
| **Helm chart version bump** | `efe-meta-swarm` Chart.yaml.version 0.5.0 → **0.6.0** (minor — new ConfigMaps + PrometheusRule + Alertmanager merge) ; appVersion 0.6.0 → **0.6.1** (no binary change) |
| **STABILITY_CONTRACT v** | 1.0 → **1.1** (new tiers : *Dashboard UID* + *Alert name* + *Recording rule name* + *SLO target*) |
| **Total LOC budget** | dashboard JSON ~8 400 ; alert YAML ~640 ; recording rules YAML ~220 ; Helm template ~880 ; tests ~1 320 (golden JSON + promtool + helm-unittest) ; doc ~3 600 → **~15 060 LOC** |
| **CI cost delta** | + 12 s for `promtool check rules` ; + 18 s for dashboard JSON schema validation (Grafonnet validator) ; + 6 s for `helm template` diff vs golden ; + 14 s for Alertmanager routing unit-test ; total + 50 s |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals + scope
   2. Pre-flight — current Grafana / Prometheus / Alertmanager state
   3. Branch + commit plan (12 commits across 2 sprints)
   4. Architectural decisions (AD1 – AD35)
   5. Operator personas + the 9 golden questions
   6. Signal inventory — metrics + spans + logs (PR #12 → #18 surface)
   7. Recording-rule strategy (6 rules — fan-out vs query-time)
   8. Dashboard catalogue — 9 dashboards
   9. Dashboard D1 — `efe-msw-overview` (top-level health + SLO)
  10. Dashboard D2 — `efe-msw-iterate` (Iterate latency + throughput)
  11. Dashboard D3 — `efe-msw-participant` (per-participant drill-down)
  12. Dashboard D4 — `efe-msw-reload` (manifest reload audit)
  13. Dashboard D5 — `efe-msw-otel` (OTel pipeline health)
  14. Dashboard D6 — `efe-msw-tls` (mTLS / cert rotation)
  15. Dashboard D7 — `efe-msw-safety` (safety violations)
  16. Dashboard D8 — `efe-msw-runtime` (process / k8s resources)
  17. Dashboard D9 — `efe-msw-traces` (Tempo trace explorer)
  18. Templating variables (cluster, env, manifest_version, participant_id, time_range)
  19. Annotations (deployments, reloads, cert rotations, OTel-collector hot-reloads)
  20. Trace ↔ metric correlation via Prometheus exemplars
  21. Alert catalogue — 10 alerts + severity model
  22. Alert A1 — `IterateFailureRateHigh`
  23. Alert A2 — `IterateLatencyP99Critical`
  24. Alert A3 — `ParticipantDown`
  25. Alert A4 — `TLSCertExpiringSoon`
  26. Alert A5 — `ControllerNotServing`
  27. Alert A6 — `OTelExporterFailing`
  28. Alert A7 — `SafetyViolationSpike`
  29. Alert A8 — `ManifestReloadFailureSpike`
  30. Alert A9 — `SLOBudgetBurnFast14x` (Google SRE 2-window fast-burn)
  31. Alert A10 — `SLOBudgetBurnSlow6x` (Google SRE slow-burn)
  32. SLO definitions — 4 new meta-swarm SLOs
  33. Alertmanager routing additions
  34. Helm packaging — dashboards ConfigMap (sidecar discovery)
  35. Helm packaging — PrometheusRule CRD
  36. CI validation pipeline (`.github/workflows/dashboards.yml`)
  37. STABILITY_CONTRACT update — dashboard UIDs + alert names + SLO targets
  38. Backward compatibility + migration from PR #12/#17 dashboards
  39. Acceptance criteria (AC1 – AC32)
  40. Threat-model coverage (T66 – T75)
  41. Tests strategy — golden JSON + `promtool test rules` + `helm-unittest`
  42. Performance budget (cardinality, query CPU, refresh cadence)
  43. Risks + mitigations
  44. Rollback strategy
  45. Sequence diagrams — dashboard provisioning, alert flow, SLO burn detection
  46. Reviewer checklist
  47. Appendix A — exhaustive PromQL cookbook (every panel)
  48. Appendix B — recording rules reference
  49. Appendix C — Alertmanager routing tree (full YAML)
  50. Appendix D — operator runbook stubs (10 runbooks)
  51. Appendix E — multi-cluster dashboard topology
  52. Appendix F — Tempo TraceQL cookbook
  53. Appendix G — Phase-3 follow-ups

---

## 0. TL;DR

PR #12 → #18 produced a **quantitative + causal** observability surface : 22
stable Prometheus metrics, 11 OpenTelemetry span types, W3C Trace Context
propagation, and a STABILITY_CONTRACT promising no breaking changes for 6
chart-minor versions. Operators today **cannot consume any of it** — there is
no shipped dashboard, no shipped alert, and no SLO that paging integrations
can hang off.

PR #19 closes the loop by shipping the **operator surface** :

  - **9 Grafana dashboards** (one Overview + 7 drill-downs + 1 Trace explorer).
  - **10 Prometheus alerts** split across two severities (`page` + `ticket`),
    every one with a hand-written runbook and a stable name in the
    STABILITY_CONTRACT.
  - **4 SLOs** with formal targets (iterate availability ≥ 99.9 %, iterate
    p95 ≤ 1 s, manifest reload p95 ≤ 500 ms, OTel pipeline drop rate ≤ 1 %),
    each backed by a 2-window burn-rate alert pair (Google SRE pattern).
  - **6 recording rules** pre-aggregating the SLO numerator/denominator pairs
    so dashboard refreshes stay sub-second under 10 k-series load.
  - **Alertmanager routing additions** : new receiver `meta-swarm-sre`, new
    inhibition rule (silence symptom alerts when the SLO burn alert fires),
    new continue=true tier for compliance/audit forks.
  - **Helm packaging** : two new ConfigMap families (sidecar-discovered
    dashboards, label `grafana_dashboard=1`) + one new PrometheusRule CRD
    (label `prometheus: efe-meta-swarm`).
  - **CI validation** : new GitHub Actions workflow runs `promtool check rules`,
    Grafana dashboard JSON schema validator, and `helm template` diff against
    a golden file. Adds + 50 s to CI wall-time.

The deliverable is **packaged exclusively as Kubernetes YAML + JSON** — no
Rust code changes, no binary impact. The chart's Helm minor version bump
captures the new ConfigMaps ; the binary's appVersion only patches because
PR #19 is data-plane-additive.

---

## 1. Goal + non-goals + scope

### 1.1 Goal

Turn the metric + trace surface delivered by PR #12 → #18 into an
**operator-ready dashboard + alert pack** that :

  - answers the 9 questions an on-call operator has when their pager fires
    (§ 5) without leaving Grafana,
  - pages exactly on user-visible impact and never on internal noise
    (every alert satisfies the *symptom-first* doctrine — § 21),
  - exposes 4 SLOs whose numeric targets are committed in the
    STABILITY_CONTRACT and can be reproduced from scrape data alone,
  - ships as part of the existing `efe-meta-swarm` Helm chart with a single
    `helm upgrade` and no operator-side ConfigMap editing,
  - validates in CI so a dashboard JSON typo or PromQL syntax error blocks
    the PR.

### 1.2 Non-goals (out of scope)

  - **No new Prometheus metric or OTel span.** PR #19 only consumes the
    STABILITY_CONTRACT v1 surface. New series are PR #20+ territory.
  - **No Grafana provisioning controller / Grafana Operator CRDs.** Helm
    + sidecar discovery is the universal pattern ; CRDs would force a
    `grafana-operator` chart dep that is not on the Phase-2 critical path.
  - **No alert-as-code DSL (Sloth, Pyrra).** YAML in repo is the source of
    truth ; migration to Sloth is a Phase-3 spike (§ G.6).
  - **No anomaly-detection / forecasting panels.** ML-driven alerts are a
    Phase-3 deliverable (§ G.4).
  - **No multi-tenant SLO splitting.** The 4 SLOs are global to the meta-
    swarm fleet ; per-tenant SLOs are Phase-3 (§ G.2).
  - **No paging-integration secret rotation.** PagerDuty / Slack tokens stay
    out of the chart ; operators wire them via existing Alertmanager Secret
    objects (referenced, not created).
  - **No long-term retention (Thanos / Cortex).** Recording rules assume
    Prometheus's default 15-day retention.
  - **No public dashboard sharing.** All Grafana panels respect the
    `editable=false` + `panel.shared=false` defaults to mitigate T70.

### 1.3 Scope summary

| Deliverable | In | Out |
|---|---|---|
| Grafana dashboards | 9 | Anomaly detection, forecasting, mobile views |
| Prometheus alerts | 10 | Composite alerts spanning EFE-SwarmService + meta-swarm |
| Recording rules | 6 | Aggregations covering Phase-3 multi-tenant labels |
| SLOs | 4 | Per-tenant + per-region SLOs |
| Runbooks | 10 (1 per alert) | Generic K8s runbooks |
| Helm packaging | ConfigMap + PrometheusRule + values schema | Grafana Operator CRDs |
| CI validation | promtool + Grafonnet validator + helm template | End-to-end smoke test of a real Grafana instance |
| STABILITY_CONTRACT | Add 3 new tiers | Re-tier existing metrics |

---

## 2. Pre-flight — current Grafana / Prometheus / Alertmanager state

### 2.1 Repository inventory

```
deploy/observability/
├── grafana/
│   └── dashboards/                # existing dashboards (M30 / M31 / M32 / M33 etc.)
├── prometheus/                    # base Prometheus config + scrape rules
├── otel-collector/                # OTLP collector config (PR #18)
└── alerts/                        # ad-hoc alert YAMLs (pre-PrometheusRule)

deploy/helm/efe-meta-swarm/
├── Chart.yaml                     # version 0.5.0 (after PR #18)
├── values.yaml                    # tracing.otel.* knobs (PR #18)
└── templates/
    ├── deployment-controller.yaml
    ├── deployment-participant.yaml
    ├── service-metrics.yaml
    ├── servicemonitor.yaml        # PR #17 — Prom-Operator discovery
    └── otel-env-configmap.yaml    # PR #18

docs/
├── EFE_A1_PHASE2_PR12_ARCHITECTURE.md  # reload metrics
├── EFE_A1_PHASE2_PR17_ARCHITECTURE.md  # 14 metrics
├── EFE_A1_PHASE2_PR18_ARCHITECTURE.md  # 8 spans
├── STABILITY_CONTRACT.md               # metric / label promises (v1.0)
├── slo.md                              # legacy SLO doc (EFE SwarmService only)
└── runbooks/                           # M23 → M28 runbook conventions
```

### 2.2 Already shipped

  - **22 STABLE Prometheus metrics** + **2 BETA** + **3 OTel exporter metrics**
    (PR #17 + PR #18 § 17.3).
  - **ServiceMonitor CRD** discovers controller + participant `/metrics`
    endpoints under label `prometheus: efe-meta-swarm`.
  - **OTel collector** runs as a DaemonSet, ingests OTLP/gRPC on `:4317`,
    forwards to the cluster Tempo via a tail-sampler. Defines its own
    health metrics — not in scope.
  - **Alertmanager** is installed cluster-wide via the standard
    `kube-prometheus-stack` ; routing config lives in a separate Secret
    that the meta-swarm chart **merges into via a Helm post-renderer**.

### 2.3 Gaps to close

  - **0** meta-swarm-specific dashboards.
  - **0** meta-swarm-specific alerts in the `kube-prometheus-stack` rule
    tree. Today, a failing Iterate manifests only as a generic
    `HighGrpcErrorRate` alert from the cluster surface.
  - **No SLO** for any meta-swarm path. `slo.md` covers `EFE SwarmService`
    only (the L1 API gateway, not the L2 meta-swarm).
  - **No exemplars** wired into the iterate / step duration histograms —
    PR #18 added the OTel SDK side, PR #19 wires the Prometheus side via
    `histogram_quantile + on(trace_id) group_right` queries.

---

## 3. Branch + commit plan (12 commits across 2 sprints)

The commit plan is intentionally fine-grained so each commit is independently
revertable (AD17). Reviewers can sign off commit-by-commit.

| # | Commit | LOC | Sprint | Reviewer focus |
|---|---|---|---|---|
| C1 | `chore: add deploy/observability/grafana/dashboards/d19/ skeleton + README` | ~150 | P2.4 W9 | Convention |
| C2 | `feat(observability): 6 recording rules + promtool unit tests` | ~520 | P2.4 W9 | PromQL syntax, cardinality budget |
| C3 | `feat(observability): D1 Overview dashboard + 12 panels` | ~1 100 | P2.4 W9 | UX, SLO panel correctness |
| C4 | `feat(observability): D2 Iterate dashboard + 10 panels + exemplars` | ~1 050 | P2.4 W9 | Histogram quantile math |
| C5 | `feat(observability): D3 Participant + D4 Reload + D5 OTel` | ~2 400 | P2.4 W10 | Per-participant cardinality |
| C6 | `feat(observability): D6 TLS + D7 Safety + D8 Runtime + D9 Traces` | ~3 050 | P2.5 W10 | Tempo datasource UID consistency |
| C7 | `feat(observability): 10 PrometheusRule alerts + helm-unittest` | ~840 | P2.5 W10 | Severity calibration, runbook URLs |
| C8 | `feat(observability): SLO burn alerts (2-window fast/slow) + slo.md update` | ~610 | P2.5 W11 | Burn-rate math, error-budget arithmetic |
| C9 | `feat(helm): dashboard ConfigMap template + values.yaml schema` | ~480 | P2.5 W11 | Helm template, sidecar label |
| C10 | `feat(helm): PrometheusRule template + Alertmanager routing merge` | ~360 | P2.5 W11 | Routing tree, inhibition rules |
| C11 | `ci: dashboard JSON validator + promtool check + helm template diff` | ~400 | P2.5 W11 | Golden file maintenance |
| C12 | `docs: PR #19 architecture + 10 runbooks + STABILITY_CONTRACT v1.1` | ~3 600 | P2.5 W11 | Doc parity with PR #18 standard |

**Total** : ~14 560 LOC across 12 commits.

**Branch protection** : every commit must pass `cargo test`, `promtool check
rules`, `helm template`, dashboard JSON schema validator, and `helm-unittest`.

---

## 4. Architectural decisions (AD1 – AD35)

> Format identical to PR #18 § 4. Each AD is a single sentence + 1-3 lines of
> rationale. Decisions are numbered for reviewer reference.

### AD1 — Dashboards live in `deploy/helm/efe-meta-swarm/dashboards/`, not `deploy/observability/`

The meta-swarm chart owns its own dashboards because (a) chart version bumps
gate dashboard rollout, (b) operators who install only the meta-swarm chart
must not depend on a separate observability chart. Centralised
`deploy/observability/` stays the home for cross-product (M30 / M31 / M32 /
M33) dashboards.

### AD2 — One dashboard per surface, not one mega-dashboard

9 focused dashboards beat 1 30-row scroll-fest. Operators jump via
inter-dashboard links ; on-call training maps "alert X → dashboard Y".

### AD3 — Dashboard UIDs are stable + STABILITY_CONTRACT-tracked

Permalinks in runbooks must survive renames. UID format :
`efe-msw-<surface>` (e.g. `efe-msw-overview`).

### AD4 — Grafana sidecar discovery via `grafana_dashboard=1` label

No Grafana Operator dep, no API push, no auth coupling. Aligned with the
existing M30 / M31 / M32 / M33 dashboards already in the repo (§ 2.1).

### AD5 — Alerts as PrometheusRule CRDs, not raw rule files

`kube-prometheus-stack` consumes PrometheusRule via label selector
`prometheus: efe-meta-swarm`. CRD path gets validation by the K8s API
server (schema rejection of malformed PromQL is delegated to promtool in
CI, not the kube-apiserver).

### AD6 — Recording rules carry the prefix `efe_msw:` not `meta_swarm_`

Colon-separated namespaces are the Prometheus convention (cf.
`node_cpu_seconds_total` → `node:cpu_seconds_total`). The `efe_msw:`
prefix collides with no other product in the cluster.

### AD7 — SLO targets committed in STABILITY_CONTRACT v1.1

Targets are part of the public API : changing a target from 99.9 % to 99 %
is a *contract break* (page-load expectations of consumers). Bumping
**up** is non-breaking and gets a 1-minor-version notice.

### AD8 — Two-window burn-rate alerts (Google SRE), not single-window

Single-window 5-minute thresholds page on transient blips. The 2-window
pattern (fast 5m + 1h ratio + slow 30m + 6h ratio) suppresses both blip
noise and slow burns that miss a 1h window. Math : § 30 + Appendix B.5.

### AD9 — Every alert has a runbook URL — no exceptions

`annotations.runbook_url` is required by the validation lint (§ 41.3).
Missing runbook ⇒ CI failure.

### AD10 — Alert names go in STABILITY_CONTRACT

PagerDuty rule mappings, Slack channels, JIRA filters, and external
audit lookups all reference alert names. Renaming breaks all of them.

### AD11 — `severity` taxonomy is closed : `critical | warning | info`

`critical` ⇒ page. `warning` ⇒ ticket. `info` ⇒ Slack-only / annotation-
only. No fourth tier, no continuous gradations.

### AD12 — Symptom-first doctrine — alert on user impact, not causes

Per Google SRE Workbook ch. 5. The 6 *symptom* alerts (A1–A2 + A9–A10)
page ; the 4 *cause* alerts (A3, A6, A7, A8) ticket. A5 (controller
not serving) pages because it's both : the cause of every symptom
downstream.

### AD13 — Severity inhibition : critical inhibits warning for the same root cause

Alertmanager `inhibit_rules` suppresses A2 (latency p99 critical) when
A9 (SLO budget burn fast) is firing for the same `service` label. Avoids
double-paging when the root cause is the same.

### AD14 — Dashboards refresh every 30 s by default, not 5 s

5 s refresh × 10 panels × 50 viewers = 5 000 PromQL queries/min on a
busy on-call ; Prometheus chokes on the cardinality of `participant_id`
joined queries. 30 s is the sweet spot.

### AD15 — Time range default is 1 h on dashboards, 6 h on the overview

Operators land first on Overview (D1) when paged → 6 h gives context.
Drill-downs (D2–D9) default to 1 h to stay snappy.

### AD16 — Panel cardinality cap : ≤ 200 series per panel

Hard-cap via Prometheus `topk(200, …)` wrappers on participant-grouped
queries. Mitigates T73 (recording-rule cardinality explosion).

### AD17 — Every commit independently revertible

Reviewer fatigue → small commits. Reverting C5 (D3 + D4 + D5) leaves D1
+ D2 still functional ; alert rollback is the PrometheusRule deletion
alone.

### AD18 — Dashboard variables prefixed `var-`

Grafana convention. Variables : `var-cluster`, `var-env`, `var-service`,
`var-participant_id`, `var-manifest_version`. All default to "all".

### AD19 — `var-manifest_version` is a *multi-value* variable

Operators want to compare iterate latency across active manifest versions
during a canary roll-out. Multi-value default is "All" ; selecting two
versions does `manifest_version=~"v1.2|v1.3"`.

### AD20 — Dashboard time-zone is UTC across the board

Avoids time-zone drift when comparing across clusters / on-call regions.
Grafana setting `dashboard.timezone="utc"`.

### AD21 — Dashboards use stat panels with thresholds, not single-numbered KPIs

`stat` panels with red/yellow/green thresholds are scannable in 2
seconds. KPI panels (no thresholds) leave the operator interpreting raw
numbers.

### AD22 — Trace exemplars via Prometheus 2.26+ exemplar storage

`PROMETHEUS_QUERY_LOG_FILE` setting documents exemplar enablement.
Exemplars carry `trace_id` extracted from the histogram observation site
(PR #18 § 22 forward-reference, now wired).

### AD23 — Tempo datasource UID is `tempo-efe` — STABILITY_CONTRACT-tracked

Dashboard JSON references the datasource by UID, not by name. Renaming
Tempo at the Grafana level doesn't break dashboards.

### AD24 — Datasource is templated (`${DS_PROMETHEUS}`)

Multi-Prometheus environments (Thanos receiver vs raw Prometheus) need
the dashboard to render against either. Default `Prometheus` ; override
via dashboard import dialog.

### AD25 — Alert labels are closed : `service`, `severity`, `team`, `runbook_id`

`team` always = `meta-swarm-sre`. `runbook_id` is a 4-char tag indexed
in Appendix D. Closed set ⇒ Alertmanager routing tree stays stable.

### AD26 — `for:` duration tuned per alert, not global

A3 (ParticipantDown) → `for: 2m` (fast page).
A1 (IterateFailureRateHigh) → `for: 5m` (denoise blips).
A4 (TLSCertExpiring) → `for: 1h` (cert expiry is hours-scale).

### AD27 — Recording rules emit at `evaluation_interval: 30s`

Matches dashboard refresh cadence so the dashboard sees the same series
the alerts see, with the same staleness window.

### AD28 — `keep_firing_for: 5m` on critical alerts

Prevents pager storm during oscillation around the threshold. Standard
Alertmanager pattern.

### AD29 — Routing tree has 3 branches : sev=critical → PagerDuty, sev=warning → Slack, sev=info → log-only

Default Alertmanager receiver fan-out. PagerDuty + Slack tokens come from
existing cluster Secrets ; PR #19 only adds the routing keys.

### AD30 — Inhibition rule : A5 (controller down) inhibits A1–A4, A6–A8

When the controller is down, every downstream alert is a symptom of A5.
Routing tree silences them automatically.

### AD31 — Compliance audit fork — `continue: true` on the routing tree

Every alert fork ends with a `continue: true` step that re-routes to the
compliance audit receiver (Loki sink). Audit ingestion is mandatory for
SOC-2 / ISO 27001 evidence.

### AD32 — Dashboard JSON is hand-written, not Grafonnet-generated

Grafonnet adds a build-time dep and a learning curve for reviewers. JSON
in repo is the source of truth ; Helm template only injects the
datasource UID via a one-line `{{ tpl ... }}` substitution.

### AD33 — Dashboards are `editable: false` to prevent local drift

Operators who need a custom view fork the JSON into a separate dashboard.
Editable dashboards mutate over time and the repo lies about reality —
catastrophic for the on-call rotation.

### AD34 — Dashboard `version` field increments per PR

Grafana auto-versions imports ; the JSON's `version: N` blocks downgrade
imports. Bumping on every PR avoids accidental rollback.

### AD35 — All PromQL has a comment-prefixed intent

Promtool linting requires queries to be valid ; documentation prefixes
(`# Intent : p95 iterate latency by manifest version`) survive in the
PrometheusRule YAML and reduce maintenance friction.

---

## 5. Operator personas + the 9 golden questions

PR #19 is operator-tooling, not platform-tooling. The 3 personas + 9 questions
below drive every dashboard panel and every alert.

### 5.1 Personas

| Persona | Goal | Frequency | Page-vs-ticket appetite |
|---|---|---|---|
| **On-call SRE** | Triage paging incident, mitigate, escalate | < 1 page / week | Pages on user-visible impact only |
| **Meta-swarm dev** | Validate canary roll-out, debug a regression | Daily | Tolerates noisy warnings during deploys |
| **Compliance auditor** | Evidence of incident response, SLO history | Quarterly | Read-only, never paged |

### 5.2 The 9 golden questions

Each question maps to one dashboard panel + one alert (when applicable).

| Q | Question | Primary panel | Primary alert |
|---|---|---|---|
| Q1 | Is the meta-swarm controller healthy right now ? | D1 § Controller status stat panel | A5 ControllerNotServing |
| Q2 | What's the current iterate success rate ? | D1 § Iterate success rate stat | A1 IterateFailureRateHigh |
| Q3 | What's the current iterate p95 / p99 ? | D1 § Iterate latency timeseries | A2 IterateLatencyP99Critical |
| Q4 | Are all participants up ? | D1 § Healthy participants stat | A3 ParticipantDown |
| Q5 | When does the soonest cert expire ? | D6 § TLS cert expiry table | A4 TLSCertExpiringSoon |
| Q6 | Did the last manifest reload succeed ? | D1 § Reload-gen stat + D4 reload history | A8 ManifestReloadFailureSpike |
| Q7 | Are we burning SLO budget faster than the 30-day allocation ? | D1 § SLO burn gauge | A9 / A10 SLOBudgetBurn |
| Q8 | Why is iterate slow right now ? (root cause) | D2 step latency + D9 trace explorer | (none — diagnostic) |
| Q9 | Is the OTel pipeline dropping spans ? | D5 § Exporter drop rate | A6 OTelExporterFailing |

---

## 6. Signal inventory — metrics + spans + logs

PR #19 consumes only the STABLE surface from STABILITY_CONTRACT v1. Every
panel + alert query below is grounded in this list.

### 6.1 STABLE Prometheus metrics consumed

| # | Metric | Type | Labels | First seen | Used by |
|---|---|---|---|---|---|
| M1 | `efe_meta_swarm_iterate_duration_seconds` | Histogram | `result`, `manifest_version` | PR #17 | D1, D2, A1, A2, A9, A10 |
| M2 | `efe_meta_swarm_iterate_total` | Counter | `result`, `failure_reason` | PR #17 | D1, D2, A1, A9, A10 |
| M3 | `efe_meta_swarm_iterate_inflight` | Gauge | — | PR #17 | D1, D2 |
| M4 | `efe_meta_swarm_iterate_iterations` | Histogram (BETA) | `convergence` | PR #17 | D2 |
| M5 | `efe_meta_swarm_iterate_convergence_error` | Histogram (BETA) | `convergence` | PR #17 | D2 |
| M6 | `efe_meta_swarm_step_duration_seconds` | Histogram | `participant_id`, `result` | PR #17 | D3, A3 (indirectly) |
| M7 | `efe_meta_swarm_step_total` | Counter | `participant_id`, `result`, `failure_reason` | PR #17 | D3 |
| M8 | `efe_meta_swarm_step_payload_bytes` | Histogram | `direction` | PR #17 | D3 |
| M9 | `efe_meta_swarm_safety_violations_total` | Counter | `kind`, `participant_id` | PR #17 | D7, A7 |
| M10 | `efe_meta_swarm_participants_configured` | Gauge | — | PR #17 | D1, D3 |
| M11 | `efe_meta_swarm_participant_up` | Gauge | `participant_id` | PR #17 | D1, D3, A3 |
| M12 | `efe_meta_swarm_manifest_reload_duration_seconds` | Histogram | — | PR #17 | D4, A8 (latency-side) |
| M13 | `efe_meta_swarm_manifest_reload_total` | Counter | `result`, `trigger` | PR #17 | D1, D4, A8 |
| M14 | `efe_meta_swarm_build_info` | Gauge | `version`, `git_sha`, `profile`, `rustc_version`, `build_date` | PR #17 | D1 (footer panel) |
| M15 | `efe_meta_swarm_reload_attempts_total` | Counter (PR #12 legacy) | — | PR #12 | D4 |
| M16 | `efe_meta_swarm_reload_failures_total` | Counter (PR #12 legacy) | `reason` | PR #12 | D4 |
| M17 | `efe_meta_swarm_reload_duration_microseconds` | Histogram (PR #12 legacy) | — | PR #12 | D4 |
| M18 | `efe_meta_swarm_reload_gen` | Gauge | — | PR #12 | D1, D4 |
| M19 | `efe_meta_swarm_tls_handshake_duration_seconds` | Histogram | `result`, `protocol`, `role` | PR #16 | D6 |
| M20 | `efe_meta_swarm_tls_handshake_total` | Counter | `result`, `role`, `failure_reason` | PR #16 | D6 |
| M21 | `efe_meta_swarm_tls_cert_expiry_seconds` | Gauge | `cert_name`, `subject_cn` | PR #16 | D6, A4 |
| M22 | `efe_meta_swarm_tls_reload_total` | Counter | `result` | PR #16 | D6 |
| M23 | `efe_meta_swarm_tls_reload_at_seconds` | Gauge | `role` | PR #16 | D6 (annotations) |
| M24 | `efe_meta_swarm_cardinality_breach_total` | Counter | `metric` | PR #17 | D8 (internal-health) |
| M25 | `efe_meta_swarm_otel_exported_spans_total` | Counter | — | PR #18 | D5 |
| M26 | `efe_meta_swarm_otel_dropped_spans_total` | Counter | `reason` | PR #18 | D5, A6 |
| M27 | `efe_meta_swarm_otel_export_duration_seconds` | Histogram | — | PR #18 | D5 |

### 6.2 OTel spans referenced

Spans are not queried via PromQL ; they're surfaced in D9 via Tempo's TraceQL.
Span names are the *only* part of the OTel surface PR #19 hard-codes :

```
efe.meta_swarm.iterate
efe.meta_swarm.connect_participants
efe.meta_swarm.participant.connect
efe.meta_swarm.inner_loop
efe.meta_swarm.safety_check
efe.meta_swarm.step
efe.meta_swarm.step.local
efe.meta_swarm.manifest.reload
efe.meta_swarm.shutdown.drain
tls.handshake
tls.reload
tls.cert_check
```

### 6.3 Process / k8s metrics consumed (D8 only)

Standard `prometheus-node-exporter` + `kube-state-metrics` surfaces :
`process_cpu_seconds_total`, `process_resident_memory_bytes`,
`go_goroutines` (Rust binary has no goroutines, but Prom-rs exposes
`process_*`), `kube_pod_container_status_restarts_total`,
`kube_deployment_status_replicas_available`. Closed list — no Phase-3
additions without a STABILITY_CONTRACT bump.

---

## 7. Recording-rule strategy

### 7.1 Why recording rules

The SLO burn alert PromQL (§ 30) does a 4-window join across 5m / 30m / 1h /
6h. Without pre-aggregation, every alert evaluation re-scans 6 h of histogram
buckets ⇒ 90 % of Prometheus CPU on a fully-loaded controller.

Recording rules pre-aggregate the error-rate + total-rate pair at the
evaluation interval (30 s). Burn-rate alerts then divide two recording rules
— sub-millisecond evaluation.

### 7.2 The 6 rules

| Name | Expression | Evaluation interval | Cardinality | Purpose |
|---|---|---|---|---|
| `efe_msw:iterate_total:rate5m` | `sum by (result) (rate(efe_meta_swarm_iterate_total[5m]))` | 30s | 6 | A1, D1, D2, SLO burn (5m window) |
| `efe_msw:iterate_total:rate30m` | `sum by (result) (rate(efe_meta_swarm_iterate_total[30m]))` | 30s | 6 | A9 fast-burn (30m) |
| `efe_msw:iterate_total:rate1h` | `sum by (result) (rate(efe_meta_swarm_iterate_total[1h]))` | 30s | 6 | A10 slow-burn (1h) |
| `efe_msw:iterate_total:rate6h` | `sum by (result) (rate(efe_meta_swarm_iterate_total[6h]))` | 30s | 6 | A10 slow-burn (6h) |
| `efe_msw:iterate_duration:p95_5m` | `histogram_quantile(0.95, sum by (le, manifest_version) (rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])))` | 30s | ≤ 5 (capped via topk) | D1, D2, A2 |
| `efe_msw:iterate_duration:p99_5m` | `histogram_quantile(0.99, sum by (le, manifest_version) (rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])))` | 30s | ≤ 5 | A2 critical, D2 |

**Total new series** : 6 rules × ≤ 6 cardinality = ≤ 36 series. Negligible vs
the 22 base metrics' aggregate ~ 500 series footprint.

### 7.3 Naming convention

`efe_msw:<source_metric>:<aggregation>_<window>` (PR convention AD6). Reads
left-to-right as a sentence : "efe meta-swarm iterate total rate 5m".

### 7.4 Stability tier

All 6 recording rules are added to STABILITY_CONTRACT as a new **Recording
Rule** tier (§ 37.2). External dashboards/alerts may consume them with the
same 6-minor-version guarantee as STABLE metrics.

---

## 8. Dashboard catalogue — 9 dashboards

| ID | UID | Title | Audience | Default range | Variables | Panels |
|---|---|---|---|---|---|---|
| D1 | `efe-msw-overview` | Meta-Swarm — Overview & SLO | On-call SRE | 6h | env, cluster | 12 |
| D2 | `efe-msw-iterate` | Meta-Swarm — Iterate Performance | Dev, SRE | 1h | env, cluster, manifest_version | 10 |
| D3 | `efe-msw-participant` | Meta-Swarm — Participant Drill-down | Dev, SRE | 1h | env, cluster, participant_id | 9 |
| D4 | `efe-msw-reload` | Meta-Swarm — Manifest Reload | SRE, dev | 24h | env, cluster, trigger | 8 |
| D5 | `efe-msw-otel` | Meta-Swarm — OTel Pipeline | SRE | 1h | env, cluster | 7 |
| D6 | `efe-msw-tls` | Meta-Swarm — TLS & Cert Rotation | SRE, security | 24h | env, cluster, role | 8 |
| D7 | `efe-msw-safety` | Meta-Swarm — Safety Violations | SRE, dev, compliance | 24h | env, cluster, kind, participant_id | 7 |
| D8 | `efe-msw-runtime` | Meta-Swarm — Runtime / Process | SRE | 1h | env, cluster, pod | 8 |
| D9 | `efe-msw-traces` | Meta-Swarm — Trace Explorer | Dev | 15m | env, cluster, trace_id | 4 |

**Total panels** : 73. **Total alert links** : 38 (a panel may link to several
runbooks). **Total inter-dashboard links** : 24.

### 8.1 Inter-dashboard navigation

```
D1 (overview)
 ├── click iterate panel → D2
 ├── click participant stat → D3
 ├── click reload stat → D4
 ├── click "see traces" → D9
 └── click "OTel pipeline ↗" → D5

D2 (iterate)
 ├── per-participant breakdown → D3
 ├── reload markers → D4
 └── slow-trace exemplar → D9

D3 (participant) → D6 (TLS panel for that participant)
D4 (reload) → D1 (back-link)
D5 (otel) → D9 (trace explorer)
D6 (tls) → D3 (per-participant)
D7 (safety) → D3 (per-participant)
D8 (runtime) → D1 (back-link)
D9 (traces) → D1 (back-link)
```

---

## 9. Dashboard D1 — `efe-msw-overview` (top-level health + SLO)

### 9.1 Header

```yaml
uid: efe-msw-overview
title: "EFE Meta-Swarm — Overview & SLO"
description: "Top-level health + SLO burn — start here when paged."
editable: false
shared: false
timezone: utc
version: 1
schemaVersion: 38   # Grafana 10.x
refresh: 30s
time:
  from: now-6h
  to: now
```

### 9.2 Variables

```yaml
variables:
  - name: cluster
    type: query
    query: "label_values(efe_meta_swarm_build_info, cluster)"
    refresh: on-dashboard-load
    multi: false
  - name: env
    type: query
    query: "label_values(efe_meta_swarm_build_info{cluster=\"$cluster\"}, env)"
    refresh: on-dashboard-load
  - name: prometheus_ds
    type: datasource
    pluginId: prometheus
    current: { text: Prometheus, value: Prometheus }
```

### 9.3 Panels (grid 24 × 32 units)

| # | Title | Type | gridPos | Query |
|---|---|---|---|---|
| P1 | Controller status | stat | x=0 y=0 w=4 h=4 | `up{job=~".*efe-meta-swarm.*controller.*", cluster="$cluster", env="$env"}` |
| P2 | Healthy participants | stat | x=4 y=0 w=4 h=4 | `100 * sum(efe_meta_swarm_participant_up{cluster="$cluster", env="$env"} == 1) / sum(efe_meta_swarm_participants_configured{cluster="$cluster", env="$env"})` (unit `percent`, thresholds 80 / 95) |
| P3 | Iterate success rate (5m) | stat | x=8 y=0 w=4 h=4 | `efe_msw:iterate_total:rate5m{result="ok"} / ignoring(result) sum(efe_msw:iterate_total:rate5m)` |
| P4 | Manifest reload gen | stat | x=12 y=0 w=4 h=4 | `efe_meta_swarm_reload_gen{cluster="$cluster", env="$env"}` (no thresholds — just shows current value) |
| P5 | SLO budget remaining (30d) | gauge | x=16 y=0 w=4 h=4 | `100 * (1 - efe_msw:slo_burn:availability_30d)` (target 99.9 ⇒ threshold red < 10 %) |
| P6 | Active alerts | alertlist | x=20 y=0 w=4 h=4 | Filter `service="meta-swarm"` |
| P7 | Iterate throughput | timeseries (stacked) | x=0 y=4 w=12 h=8 | `sum by (result) (efe_msw:iterate_total:rate5m)` |
| P8 | Iterate p50 / p95 / p99 (overall) | timeseries | x=12 y=4 w=12 h=8 | 3 series : p50, p95 (`efe_msw:iterate_duration:p95_5m`), p99 (`efe_msw:iterate_duration:p99_5m`) |
| P9 | SLO burn rate (availability) | timeseries with thresholds | x=0 y=12 w=12 h=6 | `(sum(rate(efe_meta_swarm_iterate_total{result="error"}[1h])) / sum(rate(efe_meta_swarm_iterate_total[1h]))) / 0.001` — band lines at 1×, 6×, 14.4× |
| P10 | SLO burn rate (latency) | timeseries with thresholds | x=12 y=12 w=12 h=6 | `(sum(rate(efe_meta_swarm_iterate_duration_seconds_bucket{le="1.0"}[1h])) / sum(rate(efe_meta_swarm_iterate_duration_seconds_count[1h])))` |
| P11 | Build info footer | table | x=0 y=18 w=24 h=4 | `efe_meta_swarm_build_info{cluster="$cluster", env="$env"}` |
| P12 | Documentation links | text (markdown) | x=0 y=22 w=24 h=4 | Static MD : runbooks, architecture docs, slo.md |

### 9.4 Annotations

```yaml
annotations:
  - name: Manifest reloads
    datasource: $prometheus_ds
    expr: 'changes(efe_meta_swarm_reload_gen{cluster="$cluster", env="$env"}[5m]) > 0'
    tagKeys: trigger
    titleFormat: "Reload (gen={{value}})"
  - name: Deployments
    datasource: $prometheus_ds
    expr: 'changes(efe_meta_swarm_build_info{cluster="$cluster", env="$env"}[5m]) > 0'
    tagKeys: version,git_sha
    titleFormat: "Deploy {{git_sha}}"
  - name: Cert reloads
    datasource: $prometheus_ds
    expr: 'changes(efe_meta_swarm_tls_reload_at_seconds[5m]) > 0'
    tagKeys: role
    titleFormat: "TLS reload ({{role}})"
```

### 9.5 Drill-down links (per-panel)

```
P3 → D2 with var-manifest_version=all
P2 → D3 (first unhealthy participant by alphabetical sort)
P4 → D4
P5 → A9 PrometheusRule docs
```

---

## 10. Dashboard D2 — `efe-msw-iterate` (iterate latency + throughput)

### 10.1 Header

```yaml
uid: efe-msw-iterate
title: "EFE Meta-Swarm — Iterate Performance"
description: "Iterate latency + throughput by manifest version. Use during canary roll-out."
editable: false
timezone: utc
version: 1
refresh: 30s
time: { from: now-1h, to: now }
```

### 10.2 Variables

```yaml
- name: cluster   # same as D1
- name: env      # same as D1
- name: manifest_version
  type: query
  query: 'label_values(efe_meta_swarm_iterate_total{cluster="$cluster", env="$env"}, manifest_version)'
  multi: true
  includeAll: true
- name: prometheus_ds   # datasource
```

### 10.3 Panels

| # | Title | Type | gridPos | Query intent |
|---|---|---|---|---|
| P1 | Iterate p50/p95/p99 by manifest_version | timeseries | x=0 y=0 w=24 h=8 | `efe_msw:iterate_duration:p95_5m{manifest_version=~"$manifest_version"}` (+ p50 + p99 series) |
| P2 | Iterate throughput by result | timeseries stacked | x=0 y=8 w=12 h=6 | `sum by (result) (efe_msw:iterate_total:rate5m{manifest_version=~"$manifest_version"})` |
| P3 | Iterate inflight | timeseries | x=12 y=8 w=12 h=6 | `efe_meta_swarm_iterate_inflight{cluster="$cluster"}` |
| P4 | Stop-reason distribution (last 1h) | piechart | x=0 y=14 w=8 h=8 | `sum by (failure_reason) (increase(efe_meta_swarm_iterate_total{cluster="$cluster"}[1h]))` |
| P5 | Iteration count distribution | heatmap | x=8 y=14 w=8 h=8 | `sum by (le) (rate(efe_meta_swarm_iterate_iterations_bucket{cluster="$cluster"}[5m]))` |
| P6 | Convergence error distribution | heatmap | x=16 y=14 w=8 h=8 | `sum by (le) (rate(efe_meta_swarm_iterate_convergence_error_bucket{cluster="$cluster"}[5m]))` |
| P7 | Iterate failure-reason rate | timeseries stacked | x=0 y=22 w=12 h=6 | `sum by (failure_reason) (rate(efe_meta_swarm_iterate_total{result="error"}[5m]))` |
| P8 | Slow-iterate exemplars | timeseries+exemplars | x=12 y=22 w=12 h=6 | `efe_msw:iterate_duration:p99_5m` ← exemplars exposed automatically by Grafana when datasource has exemplar storage |
| P9 | Iterate latency heatmap (all manifests) | heatmap | x=0 y=28 w=24 h=8 | `sum by (le) (rate(efe_meta_swarm_iterate_duration_seconds_bucket{cluster="$cluster"}[5m]))` |
| P10 | Trace links | text (markdown) | x=0 y=36 w=24 h=2 | Direct link to D9 with `trace_id` template |

### 10.4 Exemplar wiring (AD22)

Prometheus 2.26+ exposes exemplars via the `/api/v1/query_range?exemplars=true`
query. Grafana 10.x renders them as click-targets that pivot to Tempo's UI via
the datasource UID `tempo-efe`. PR #18 already attaches `trace_id` to every
histogram observation at the controller (Span 1 `iterate`).

### 10.5 Drill-down links

```
P4 (slice for failure_reason=safety_violation) → D7 with var-kind=*
P5 (heatmap hot zone) → D3 (which participant is iterating slow ?)
P8 (exemplar click) → D9
```

---

## 11. Dashboard D3 — `efe-msw-participant` (per-participant drill-down)

### 11.1 Header

```yaml
uid: efe-msw-participant
title: "EFE Meta-Swarm — Participant Drill-down"
description: "One participant at a time. Step latency, error rates, payload sizes."
editable: false
version: 1
refresh: 30s
time: { from: now-1h, to: now }
```

### 11.2 Variables

```yaml
- name: cluster
- name: env
- name: participant_id
  type: query
  query: 'label_values(efe_meta_swarm_step_total{cluster="$cluster", env="$env"}, participant_id)'
  multi: false   # one participant at a time
  includeAll: false
- name: prometheus_ds
```

### 11.3 Panels

| # | Title | Type | gridPos | Query intent |
|---|---|---|---|---|
| P1 | Participant up | stat (red/green) | x=0 y=0 w=4 h=4 | `efe_meta_swarm_participant_up{participant_id="$participant_id"}` |
| P2 | Step success rate (5m) | stat | x=4 y=0 w=4 h=4 | `sum(rate(efe_meta_swarm_step_total{participant_id="$participant_id", result="ok"}[5m])) / sum(rate(efe_meta_swarm_step_total{participant_id="$participant_id"}[5m]))` |
| P3 | Last seen | stat | x=8 y=0 w=4 h=4 | `time() - max(efe_meta_swarm_participant_up{participant_id="$participant_id"})` (only meaningful when state==0) |
| P4 | TLS handshake success rate (1h) | stat | x=12 y=0 w=4 h=4 | `sum(rate(efe_meta_swarm_tls_handshake_total{role="client", participant_id="$participant_id", result="ok"}[1h])) / sum(rate(efe_meta_swarm_tls_handshake_total{role="client", participant_id="$participant_id"}[1h]))` |
| P5 | Step p50 / p95 / p99 | timeseries | x=0 y=4 w=12 h=8 | `histogram_quantile(0.95, sum by (le) (rate(efe_meta_swarm_step_duration_seconds_bucket{participant_id="$participant_id"}[5m])))` |
| P6 | Step throughput by result | timeseries stacked | x=12 y=4 w=12 h=8 | `sum by (result) (rate(efe_meta_swarm_step_total{participant_id="$participant_id"}[5m]))` |
| P7 | Step failure-reason | timeseries | x=0 y=12 w=12 h=6 | `sum by (failure_reason) (rate(efe_meta_swarm_step_total{participant_id="$participant_id", result="error"}[5m]))` |
| P8 | Step payload bytes (in / out) | timeseries | x=12 y=12 w=12 h=6 | `histogram_quantile(0.95, sum by (direction, le) (rate(efe_meta_swarm_step_payload_bytes_bucket[5m])))` |
| P9 | Safety violations | timeseries | x=0 y=18 w=24 h=6 | `sum by (kind) (rate(efe_meta_swarm_safety_violations_total{participant_id="$participant_id"}[5m]))` |

### 11.4 Drill-downs

```
P5 hot range → D9 with var-trace_id=<exemplar>
P9 → D7 with var-kind=<series-name>
P4 → D6 with var-participant_id=$participant_id
```

---

## 12. Dashboard D4 — `efe-msw-reload` (manifest reload audit)

### 12.1 Header

```yaml
uid: efe-msw-reload
title: "EFE Meta-Swarm — Manifest Reload Audit"
editable: false
version: 1
refresh: 1m
time: { from: now-24h, to: now }
```

### 12.2 Variables : cluster, env, trigger (sighup / grpc / file_change, multi).

### 12.3 Panels (8)

| # | Title | Type | Query intent |
|---|---|---|---|
| P1 | Current reload_gen | stat | `efe_meta_swarm_reload_gen` |
| P2 | Reloads 24h (success vs failure) | bar chart | `sum by (result) (increase(efe_meta_swarm_manifest_reload_total[24h]))` |
| P3 | Reload duration p95 | timeseries | `histogram_quantile(0.95, sum by (le) (rate(efe_meta_swarm_manifest_reload_duration_seconds_bucket[5m])))` |
| P4 | Reloads by trigger (24h) | piechart | `sum by (trigger) (increase(efe_meta_swarm_manifest_reload_total{result="ok"}[24h]))` |
| P5 | Reload failures by reason | bar chart | `sum by (reason) (increase(efe_meta_swarm_reload_failures_total[24h]))` (PR #12 legacy series) |
| P6 | Reload timeline (annotated events) | logs panel or annotations | merged annotations from `changes(reload_gen[5m]) > 0` |
| P7 | Time since last reload | stat | `time() - max(efe_meta_swarm_tls_reload_at_seconds)` (proxy ; actual reload time exposed in v1.1 via M28) |
| P8 | Reload error rate (1h) | stat | `sum(rate(efe_meta_swarm_manifest_reload_total{result="error"}[1h])) / sum(rate(efe_meta_swarm_manifest_reload_total[1h]))` |

---

## 13. Dashboard D5 — `efe-msw-otel` (OTel pipeline health)

### 13.1 Header

```yaml
uid: efe-msw-otel
title: "EFE Meta-Swarm — OTel Pipeline"
description: "Span export rate, drops, exporter latency. Critical-path for trace-based debugging."
editable: false
version: 1
refresh: 30s
time: { from: now-1h, to: now }
```

### 13.2 Panels (7)

| # | Title | Type | Query |
|---|---|---|---|
| P1 | Spans exported (rate) | timeseries | `rate(efe_meta_swarm_otel_exported_spans_total[1m])` |
| P2 | Spans dropped by reason (rate) | timeseries stacked | `sum by (reason) (rate(efe_meta_swarm_otel_dropped_spans_total[1m]))` |
| P3 | Drop ratio (%) | stat | `100 * sum(rate(efe_meta_swarm_otel_dropped_spans_total[5m])) / (sum(rate(efe_meta_swarm_otel_dropped_spans_total[5m])) + sum(rate(efe_meta_swarm_otel_exported_spans_total[5m])))` |
| P4 | Export duration p50 / p95 / p99 | timeseries | 3-series histogram_quantile on `efe_meta_swarm_otel_export_duration_seconds_bucket` |
| P5 | Collector heartbeat | stat | `up{job="otel-collector"}` |
| P6 | Sampling effectiveness (sampled / total) | stat | from collector-side meta-metric ; reference only — collector metric, not meta-swarm |
| P7 | Documentation panel | markdown | Links to PR #18 § 17.3 + runbook for OTel exporter failures |

---

## 14. Dashboard D6 — `efe-msw-tls` (TLS health + cert rotation)

### 14.1 Header : uid `efe-msw-tls`, refresh 1m, default range 24h.

### 14.2 Variables : cluster, env, role (server / client, multi).

### 14.3 Panels (8)

| # | Title | Type | Query |
|---|---|---|---|
| P1 | Days until next cert expiry | stat (red < 7, yellow < 30) | `(min(efe_meta_swarm_tls_cert_expiry_seconds) - time()) / 86400` |
| P2 | Cert expiry table | table | `efe_meta_swarm_tls_cert_expiry_seconds` formatted with `subject_cn` + days-until-expiry |
| P3 | TLS handshake p95 by role | timeseries | `histogram_quantile(0.95, sum by (role, le) (rate(efe_meta_swarm_tls_handshake_duration_seconds_bucket[5m])))` |
| P4 | TLS handshake errors by failure_reason | timeseries stacked | `sum by (failure_reason) (rate(efe_meta_swarm_tls_handshake_total{result!="ok"}[5m]))` |
| P5 | TLS reload events (24h) | table | `efe_meta_swarm_tls_reload_total` joined with `efe_meta_swarm_tls_reload_at_seconds` |
| P6 | TLS reload failures | stat | `sum(increase(efe_meta_swarm_tls_reload_total{result="error"}[24h]))` |
| P7 | Cert SAN drift (placeholder) | text | reserved for PR #20 cert-pinning audit |
| P8 | Documentation links | markdown | Runbook RB_PR19_A4 + rotation playbook |

### 14.4 Annotations : every `tls_reload_at_seconds` change → time marker.

---

## 15. Dashboard D7 — `efe-msw-safety` (safety violations)

### 15.1 Variables : cluster, env, kind (box/poly/hull, multi), participant_id (multi).

### 15.2 Panels (7)

| # | Title | Type | Query |
|---|---|---|---|
| P1 | Total violations (24h) | stat | `sum(increase(efe_meta_swarm_safety_violations_total[24h]))` |
| P2 | Violations by kind | timeseries stacked | `sum by (kind) (rate(efe_meta_swarm_safety_violations_total{kind=~"$kind"}[5m]))` |
| P3 | Top 10 participants by violations | bar gauge | `topk(10, sum by (participant_id) (increase(efe_meta_swarm_safety_violations_total[1h])))` |
| P4 | Violation rate per participant | heatmap | `sum by (participant_id, kind) (rate(efe_meta_swarm_safety_violations_total[5m]))` |
| P5 | First-seen participant (cold-start violations) | table | filter on `increase(...[10m]) > 0` AND `increase(...[1h]) - increase(...[10m]) == 0` |
| P6 | Cross-reference with iterate failures | timeseries dual-axis | left: violation rate ; right: `sum(rate(efe_meta_swarm_iterate_total{failure_reason="safety_violation"}[5m]))` |
| P7 | Documentation | markdown | Phase-2 safety doctrine doc link |

---

## 16. Dashboard D8 — `efe-msw-runtime` (process / k8s resources)

### 16.1 Variables : cluster, env, pod (from `kube_pod_info`, multi).

### 16.2 Panels (8)

| # | Title | Type | Query |
|---|---|---|---|
| P1 | CPU usage (controller + participants) | timeseries | `rate(process_cpu_seconds_total{namespace="efe", pod=~"$pod"}[5m])` |
| P2 | Memory (RSS) | timeseries | `process_resident_memory_bytes{namespace="efe", pod=~"$pod"}` |
| P3 | File descriptors | timeseries | `process_open_fds{namespace="efe", pod=~"$pod"}` |
| P4 | Pod restarts (1h) | bar gauge | `sum by (pod) (increase(kube_pod_container_status_restarts_total{namespace="efe", pod=~"$pod"}[1h]))` |
| P5 | Deployments available replicas | timeseries | `kube_deployment_status_replicas_available{namespace="efe"}` |
| P6 | OOMKills (24h) | stat | `sum(increase(kube_pod_container_status_terminated_reason{reason="OOMKilled", namespace="efe"}[24h]))` |
| P7 | Cardinality breach counter | stat | `sum(efe_meta_swarm_cardinality_breach_total)` (red if > 0) |
| P8 | Documentation | markdown | runbook for cardinality breach (PR #17 § 18.3) |

---

## 17. Dashboard D9 — `efe-msw-traces` (Tempo trace explorer)

### 17.1 Header

```yaml
uid: efe-msw-traces
title: "EFE Meta-Swarm — Trace Explorer"
description: "Tempo-backed trace search. Linked from D2 / D3 exemplars."
editable: false
version: 1
refresh: 5s
time: { from: now-15m, to: now }
```

### 17.2 Variables

```yaml
- name: cluster
- name: env
- name: trace_id     # operator can paste from D2 exemplar
  type: textbox
- name: tempo_ds
  type: datasource
  pluginId: tempo
  current: { text: Tempo, value: tempo-efe }
```

### 17.3 Panels (4)

| # | Title | Type | Query |
|---|---|---|---|
| P1 | Trace search (manual) | tempo-search | TraceQL `{ service.name = "efe-meta-swarm-controller" && duration > 1s }` |
| P2 | Trace view | tempo-traces (graph) | by `trace_id=$trace_id` |
| P3 | Trace metrics correlation | timeseries | the trace's span's start/end times overlaid on iterate p95 |
| P4 | Linked dashboards | markdown | D1 / D2 / D3 back-links |

### 17.4 TraceQL cookbook → Appendix F.

---

## 18. Templating variables strategy

### 18.1 Stable variable names (AD18)

```
$cluster              query  → label_values(efe_meta_swarm_build_info, cluster)
$env                  query  → label_values(efe_meta_swarm_build_info{cluster="$cluster"}, env)
$service              query  → constant {controller, participant}
$participant_id       query  → label_values(efe_meta_swarm_step_total{...}, participant_id)
$manifest_version     query  → label_values(efe_meta_swarm_iterate_total{...}, manifest_version) ; multi
$kind                 query  → label_values(efe_meta_swarm_safety_violations_total, kind) ; multi
$trigger              query  → label_values(efe_meta_swarm_manifest_reload_total, trigger) ; multi
$role                 query  → constant {server, client}
$pod                  query  → label_values(kube_pod_info{namespace="efe"}, pod)
$prometheus_ds        datasource (Prometheus)
$tempo_ds             datasource (Tempo)
```

### 18.2 Refresh policy

`on-dashboard-load` for cluster / env (rare changes). `on-time-range-change`
for participant_id / manifest_version (could change in a 6 h window).

### 18.3 Cardinality cap on `participant_id`

If a dashboard variable yields > 200 values, Grafana clamps the dropdown and
emits a console warning ; the underlying PromQL never sees > 200 unless the
operator explicitly types in "All" which falls back to the regex anchor.

---

## 19. Annotations strategy

### 19.1 The 3 system-wide annotations

| Annotation | Source query | Tag keys | Shown on |
|---|---|---|---|
| `Manifest reload` | `changes(efe_meta_swarm_reload_gen[5m]) > 0` | trigger | D1, D2, D4 |
| `Deployment` | `changes(efe_meta_swarm_build_info[5m]) > 0` | version, git_sha | D1, D2, D3, D8 |
| `Cert reload` | `changes(efe_meta_swarm_tls_reload_at_seconds[5m]) > 0` | role | D1, D6 |

### 19.2 Per-dashboard annotations

D7 (safety) adds : "first-seen safety violation per participant" using a
recording-rule alias `efe_msw:safety:first_seen` (defined in C2).

D4 (reload) shows failed reloads as red annotation pins.

### 19.3 Annotation retention

Same as Prometheus retention (15 days). Long-term archival is out of scope ;
audit log on Loki carries the same event with infinite retention.

---

## 20. Trace ↔ metric correlation via Prometheus exemplars

### 20.1 Wire-up

PR #18 adds the OTel SDK side (every histogram observation in
`efe_meta_swarm_iterate_duration_seconds` carries a `trace_id`). PR #19 wires :

  1. Prometheus `--enable-feature=exemplar-storage` flag in the chart values
     (HelmConfig `prometheus.exemplars.enabled = true`).
  2. Grafana datasource config — `exemplarTraceIdDestinations: [{name:
     trace_id, datasourceUid: tempo-efe}]`.
  3. Histogram panels in D1 P8 and D2 P8 set `enableExemplars: true`.

### 20.2 Operator experience

Click any high-latency exemplar dot on D2 P8 → jumps to Tempo with that
specific `trace_id` pre-filled. Tempo renders the controller → participant
RPC chain with span attributes from PR #18 § 5-13.

### 20.3 Cardinality risk

Exemplars are *not* time-series — they're a separate Prometheus storage
(file-backed circular buffer). Default cap : 100 k exemplars cluster-wide.
The chart caps via `--storage.exemplars.exemplars-limit=200000` for a
fleet of 10 controllers (~ 20 k exemplars / controller / day at p99 sampling).

### 20.4 PII mitigation

PR #18 § 19 already redacts span attributes. Exemplar payload is only the
`trace_id` (a hex string, never a user value), so no additional redaction is
required at the Prometheus level.

---

## 21. Alert catalogue — 10 alerts + severity model

### 21.1 The 10 alerts

| ID | Name | Severity | for | Symptom or cause | Page or ticket |
|---|---|---|---|---|---|
| A1 | `IterateFailureRateHigh` | critical | 5m | symptom | page |
| A2 | `IterateLatencyP99Critical` | critical | 5m | symptom | page |
| A3 | `ParticipantDown` | warning | 2m | cause | ticket |
| A4 | `TLSCertExpiringSoon` | warning | 1h | cause | ticket |
| A5 | `ControllerNotServing` | critical | 1m | cause+symptom | page |
| A6 | `OTelExporterFailing` | warning | 10m | cause | ticket |
| A7 | `SafetyViolationSpike` | warning | 5m | cause | ticket |
| A8 | `ManifestReloadFailureSpike` | warning | 10m | cause | ticket |
| A9 | `SLOBudgetBurnFast14x` | critical | 5m | symptom (SLO) | page |
| A10 | `SLOBudgetBurnSlow6x` | warning | 30m | symptom (SLO) | ticket |

### 21.2 Severity model (AD11)

```
critical  → PagerDuty page, 24/7
warning   → JIRA ticket, business-hours
info      → Slack-only, log-only
```

### 21.3 Symptom-first doctrine (AD12)

A1, A2, A9, A10 are user-visible symptom alerts ⇒ they wake people up. A3
through A8 are cause alerts ⇒ they file tickets, suppressed by inhibition
when the corresponding symptom is firing.

### 21.4 Labels (AD25)

Every alert carries 4 stable labels :

```yaml
labels:
  service: meta-swarm
  team: meta-swarm-sre
  severity: <critical|warning|info>
  runbook_id: <4-char>
```

`runbook_id` indexed in Appendix D : `MSW1`, `MSW2`, ..., `MSW10`.

### 21.5 Annotations (every alert)

```yaml
annotations:
  summary: <1-line, < 80 chars>
  description: <multi-line, includes labels via templating>
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_{{ $labels.runbook_id }}.md"
  dashboard_url: "https://grafana.efe.internal/d/efe-msw-overview"
```

---

## 22. Alert A1 — `IterateFailureRateHigh`

**Question answered** : Q2 (iterate success rate dropped below threshold).

```yaml
alert: IterateFailureRateHigh
expr: |
  (
    sum(rate(efe_meta_swarm_iterate_total{result="error"}[5m]))
    /
    sum(rate(efe_meta_swarm_iterate_total[5m]))
  ) > 0.05
for: 5m
labels:
  service: meta-swarm
  team: meta-swarm-sre
  severity: critical
  runbook_id: MSW1
annotations:
  summary: "Meta-swarm iterate failure rate > 5% over 5m"
  description: |
    The iterate RPC is failing for more than 5% of requests over the last 5
    minutes. Last error breakdown :
    {{ range query "topk(3, sum by (failure_reason) (rate(efe_meta_swarm_iterate_total{result=\"error\"}[5m])))" }}
      {{ .Labels.failure_reason }} : {{ .Value }} req/s
    {{ end }}
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW1.md"
  dashboard_url: "https://grafana.efe.internal/d/efe-msw-iterate"
```

**Threshold rationale** : SLO target is 99.9 % availability → > 0.1 % is
*technically* burning budget, but real-world flapping under that threshold
would page on transient blips. 5 % is the "obviously bad" line.

**Recovery** : auto-resolves when the rate drops below 5 % for 5m (PR #19
relies on Prometheus' default `keep_firing_for: 5m`).

**Inhibited by** : A5 (controller not serving — all iterate calls fail
trivially when the binary is down).

---

## 23. Alert A2 — `IterateLatencyP99Critical`

```yaml
alert: IterateLatencyP99Critical
expr: efe_msw:iterate_duration:p99_5m > 5
for: 5m
labels:
  severity: critical
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW2
annotations:
  summary: "Iterate p99 latency > 5s sustained for 5m"
  description: |
    p99 = {{ $value }}s. Recent slow exemplars at
    {{ template "exemplar_link" . }}.
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW2.md"
```

**Threshold rationale** : p99 ≤ 5 s is the user-facing SLO ceiling
(see § 32.2). Persistent breach indicates a saturated controller or a
mis-behaving participant.

**Inhibited by** : A5, A9.

---

## 24. Alert A3 — `ParticipantDown`

```yaml
alert: ParticipantDown
expr: efe_meta_swarm_participant_up == 0
for: 2m
labels:
  severity: warning
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW3
annotations:
  summary: "Participant {{ $labels.participant_id }} is unreachable"
  description: |
    Controller cannot reach participant {{ $labels.participant_id }} for the
    last 2m. Iterate will degrade or fail with `failure_reason=participant_down`.
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW3.md"
```

**Threshold rationale** : 2 m is the participant health re-check cadence
(PR #17 § 9.2). Any single missed health check triggers `up == 0` ; sustained
2 m means the participant is genuinely gone.

**Severity rationale** : `warning` not `critical` because the meta-swarm
tolerates losing one participant for a few minutes (algorithm degrades but
remains correct). Multi-participant loss is captured by A1 (failure rate).

---

## 25. Alert A4 — `TLSCertExpiringSoon`

```yaml
alert: TLSCertExpiringSoon
expr: (efe_meta_swarm_tls_cert_expiry_seconds - time()) < 7 * 86400
for: 1h
labels:
  severity: warning
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW4
annotations:
  summary: "TLS cert {{ $labels.cert_name }} expires in < 7 days"
  description: |
    Subject CN : {{ $labels.subject_cn }}. Expiry :
    {{ $value | humanizeDuration }}.
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW4.md"
```

**Threshold rationale** : 7 days = standard cert rotation buffer. SLO impact
when a cert expires : 100 % handshake failure across the affected role.

**Two-tier variant (optional)** : a *critical* variant `TLSCertExpiringSoonCritical`
that fires at < 1 day. Deferred to PR #20 — current PagerDuty workflows
treat 7 days as critical anyway.

---

## 26. Alert A5 — `ControllerNotServing`

```yaml
alert: ControllerNotServing
expr: up{job=~".*efe-meta-swarm.*controller.*"} == 0
for: 1m
labels:
  severity: critical
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW5
annotations:
  summary: "Meta-swarm controller {{ $labels.instance }} is DOWN"
  description: |
    Controller pod {{ $labels.instance }} has been unreachable for 1m. All
    iterate calls will fail. Check D8 P4 for restart count.
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW5.md"
```

**Inhibits** : A1, A2, A3, A6, A7, A8 (all are downstream symptoms).

**Recovery** : auto-resolves when `up == 1` for 30 s.

---

## 27. Alert A6 — `OTelExporterFailing`

```yaml
alert: OTelExporterFailing
expr: |
  (
    sum(rate(efe_meta_swarm_otel_dropped_spans_total[10m]))
    /
    (
      sum(rate(efe_meta_swarm_otel_dropped_spans_total[10m]))
      + sum(rate(efe_meta_swarm_otel_exported_spans_total[10m]))
    )
  ) > 0.10
for: 10m
labels:
  severity: warning
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW6
annotations:
  summary: "OTel exporter dropping > 10% of spans"
  description: |
    Drop reasons : {{ range query "sum by (reason) (rate(efe_meta_swarm_otel_dropped_spans_total[10m]))" }}
      {{ .Labels.reason }} : {{ .Value }}
    {{ end }}
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW6.md"
```

**Threshold rationale** : 10 % drop = trace-based debugging becomes
unreliable (samples missing). Not user-visible → warning, not critical.

---

## 28. Alert A7 — `SafetyViolationSpike`

```yaml
alert: SafetyViolationSpike
expr: sum(rate(efe_meta_swarm_safety_violations_total[5m])) > 0.1
for: 5m
labels:
  severity: warning
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW7
annotations:
  summary: "Safety violation rate > 0.1/s for 5m"
  description: |
    Top participants :
    {{ range query "topk(3, sum by (participant_id, kind) (rate(efe_meta_swarm_safety_violations_total[5m])))" }}
      {{ .Labels.participant_id }} / {{ .Labels.kind }} : {{ .Value }}
    {{ end }}
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW7.md"
```

**Threshold rationale** : 0.1/s sustained ⇒ 6 violations/min ⇒ unusually
fast learner crossing the safety boundary repeatedly. Investigation,
not user impact.

---

## 29. Alert A8 — `ManifestReloadFailureSpike`

```yaml
alert: ManifestReloadFailureSpike
expr: |
  sum(increase(efe_meta_swarm_manifest_reload_total{result="error"}[1h])) >= 3
for: 10m
labels:
  severity: warning
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW8
annotations:
  summary: "≥ 3 manifest reload failures in last 1h"
  description: |
    Last failures : {{ range query "topk(3, sum by (trigger) (increase(efe_meta_swarm_manifest_reload_total{result=\"error\"}[1h])))" }}
      trigger={{ .Labels.trigger }} : {{ .Value }} failures
    {{ end }}
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW8.md"
```

---

## 30. Alert A9 — `SLOBudgetBurnFast14x` (Google SRE 2-window fast-burn)

### 30.1 The math

Monthly SLO of 99.9 % availability ⇒ 0.1 % error budget. Burn-rate `b` =
fraction of budget consumed per hour : `b = current_error_rate / 0.001`.

Two-window fast-burn (Google SRE Workbook ch. 5) fires when both :

  - 5-minute window burn rate > 14.4× (would exhaust monthly budget in 2 days), AND
  - 1-hour window burn rate > 14.4× (rules out a 5-minute blip).

### 30.2 PromQL

```yaml
alert: SLOBudgetBurnFast14x
expr: |
  (
    (
      sum(rate(efe_meta_swarm_iterate_total{result="error"}[5m]))
      /
      sum(rate(efe_meta_swarm_iterate_total[5m]))
    ) > (14.4 * 0.001)
    AND
    (
      sum(rate(efe_meta_swarm_iterate_total{result="error"}[1h]))
      /
      sum(rate(efe_meta_swarm_iterate_total[1h]))
    ) > (14.4 * 0.001)
  )
for: 5m
labels:
  severity: critical
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW9
  slo_id: iterate-availability
annotations:
  summary: "Burning iterate availability SLO 14.4× — budget exhausts in < 2 days"
  description: |
    Current error rate consumes 14.4× the allotted monthly budget. Without
    intervention, the 30-day budget exhausts in ~ 2 days.
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW9.md"
```

### 30.3 Inhibition

A1 (`IterateFailureRateHigh`) is *inhibited* when A9 fires for the same
`service` label — A9 is the SLO-aware variant.

---

## 31. Alert A10 — `SLOBudgetBurnSlow6x` (Google SRE slow-burn)

```yaml
alert: SLOBudgetBurnSlow6x
expr: |
  (
    (
      sum(rate(efe_meta_swarm_iterate_total{result="error"}[1h]))
      /
      sum(rate(efe_meta_swarm_iterate_total[1h]))
    ) > (6 * 0.001)
    AND
    (
      sum(rate(efe_meta_swarm_iterate_total{result="error"}[6h]))
      /
      sum(rate(efe_meta_swarm_iterate_total[6h]))
    ) > (6 * 0.001)
  )
for: 30m
labels:
  severity: warning
  service: meta-swarm
  team: meta-swarm-sre
  runbook_id: MSW10
  slo_id: iterate-availability
annotations:
  summary: "Slow burn of iterate availability SLO at 6× rate"
  description: |
    Sustained at 6× burn rate, monthly budget exhausts in ~ 5 days.
  runbook_url: "https://docs.efe.internal/runbooks/RB_PR19_MSW10.md"
```

---

## 32. SLO definitions — 4 new meta-swarm SLOs

### 32.1 SLO 1 — Iterate availability

```yaml
slo_id: iterate-availability
description: "Fraction of iterate calls returning result=ok within a 30-day window"
target: 99.9   # percent
sli:
  numerator:   "sum(rate(efe_meta_swarm_iterate_total{result=\"ok\"}[30d]))"
  denominator: "sum(rate(efe_meta_swarm_iterate_total[30d]))"
budget_burn_alerts:
  - A9 SLOBudgetBurnFast14x
  - A10 SLOBudgetBurnSlow6x
budget_exhaustion: "0.1 % of all iterate calls / 30d"
```

### 32.2 SLO 2 — Iterate latency

```yaml
slo_id: iterate-latency-p95
target: 95
description: "95% of iterate calls complete in ≤ 1 second within a 30-day window"
sli:
  numerator:   "sum(rate(efe_meta_swarm_iterate_duration_seconds_bucket{le=\"1.0\"}[30d]))"
  denominator: "sum(rate(efe_meta_swarm_iterate_duration_seconds_count[30d]))"
budget_burn_alerts:
  - A2 IterateLatencyP99Critical (proxy ; full burn alert deferred to PR #22)
```

### 32.3 SLO 3 — Manifest reload latency

```yaml
slo_id: manifest-reload-latency-p95
target: 95
description: "95% of manifest reloads complete in ≤ 500ms"
sli:
  numerator:   "sum(rate(efe_meta_swarm_manifest_reload_duration_seconds_bucket{le=\"0.5\"}[30d]))"
  denominator: "sum(rate(efe_meta_swarm_manifest_reload_duration_seconds_count[30d]))"
```

### 32.4 SLO 4 — OTel pipeline drop rate

```yaml
slo_id: otel-drop-rate
target: 99
description: "≤ 1% of spans dropped at the exporter over a 30-day window"
sli:
  numerator:   "sum(rate(efe_meta_swarm_otel_exported_spans_total[30d]))"
  denominator: |
    sum(rate(efe_meta_swarm_otel_exported_spans_total[30d]))
    + sum(rate(efe_meta_swarm_otel_dropped_spans_total[30d]))
```

### 32.5 STABILITY_CONTRACT entries

All 4 SLOs go into the new STABILITY_CONTRACT v1.1 *SLO Target* tier (§ 37).
Lowering a target (relaxing) is a contract break ; raising is non-breaking.

---

## 33. Alertmanager routing additions

### 33.1 New receiver

```yaml
receivers:
  - name: meta-swarm-sre-page
    pagerduty_configs:
      - service_key_secret:
          name: alertmanager-pagerduty-meta-swarm
          key: routing-key
        severity: '{{ .CommonLabels.severity }}'
        description: '{{ template "pagerduty.default.description" . }}'
        details:
          runbook: '{{ .CommonAnnotations.runbook_url }}'
  - name: meta-swarm-sre-ticket
    slack_configs:
      - api_url_secret:
          name: alertmanager-slack-meta-swarm
          key: webhook-url
        channel: '#meta-swarm-alerts'
        title: '[{{ .Status | toUpper }}] {{ .CommonLabels.alertname }}'
        text: '{{ .CommonAnnotations.description }}\n<{{ .CommonAnnotations.runbook_url }}|Runbook>'
```

### 33.2 Routing tree additions

```yaml
route:
  routes:
    - match:
        service: meta-swarm
        severity: critical
      receiver: meta-swarm-sre-page
      continue: true   # → compliance audit fork
      group_wait: 30s
      group_interval: 5m
      repeat_interval: 4h
    - match:
        service: meta-swarm
        severity: warning
      receiver: meta-swarm-sre-ticket
      continue: true
      group_wait: 1m
      group_interval: 10m
      repeat_interval: 12h
    - match:
        service: meta-swarm
      receiver: compliance-audit-loki   # existing receiver
```

### 33.3 Inhibition rules (AD30)

```yaml
inhibit_rules:
  - source_match:
      alertname: ControllerNotServing
      severity: critical
    target_match:
      service: meta-swarm
    equal: [cluster, env]
  - source_match:
      alertname: SLOBudgetBurnFast14x
      severity: critical
    target_match:
      alertname: IterateFailureRateHigh
    equal: [cluster, env, service]
```

### 33.4 Helm merge strategy

The existing Alertmanager config lives in a Secret managed outside the
meta-swarm chart. PR #19 ships a Helm-rendered ConfigMap with the route +
receiver + inhibition fragments ; the cluster Alertmanager Helm chart
consumes them via a `prometheus.alertmanager.extraSecretMounts` value. No
post-renderer required.

---

## 34. Helm packaging — dashboards ConfigMap

### 34.1 Layout

```
deploy/helm/efe-meta-swarm/
├── Chart.yaml                    # version 0.6.0
├── values.yaml                   # new `grafana.dashboards.enabled` knob
├── values.schema.json            # extended with the new section
└── templates/
    ├── ...
    ├── grafana-dashboards-configmap.yaml      # NEW : 9 ConfigMaps
    ├── prometheusrule-meta-swarm.yaml         # NEW : 10 alerts + 6 rules
    └── alertmanager-routing-configmap.yaml    # NEW : routing fragment
```

### 34.2 The dashboards ConfigMap template

```yaml
{{- if .Values.grafana.dashboards.enabled }}
{{- range $name, $file := .Values.grafana.dashboards.files }}
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: efe-msw-dashboard-{{ $name }}
  namespace: {{ $.Release.Namespace }}
  labels:
    grafana_dashboard: "1"           # AD4 — sidecar discovery
    app.kubernetes.io/name: efe-meta-swarm
    app.kubernetes.io/component: dashboard
    efe.dashboard.uid: efe-msw-{{ $name }}
data:
  {{ $name }}.json: |
{{ $.Files.Get (printf "dashboards/%s.json" $file) | indent 4 }}
{{- end }}
{{- end }}
```

### 34.3 values.yaml extension

```yaml
grafana:
  dashboards:
    enabled: true
    files:
      overview: efe-msw-overview
      iterate: efe-msw-iterate
      participant: efe-msw-participant
      reload: efe-msw-reload
      otel: efe-msw-otel
      tls: efe-msw-tls
      safety: efe-msw-safety
      runtime: efe-msw-runtime
      traces: efe-msw-traces
    datasources:
      prometheus_uid: prometheus
      tempo_uid: tempo-efe
```

### 34.4 Datasource UID substitution

The dashboard JSON in `dashboards/*.json` has the literal string `${DS_PROMETHEUS}`
where Grafana expects a UID. The Helm `tpl` function substitutes at install
time :

```yaml
data:
  {{ $name }}.json: |
{{ tpl ($.Files.Get (printf "dashboards/%s.json" $file)) $ | indent 4 }}
```

with the JSON containing `"datasource": { "uid": "{{ .Values.grafana.dashboards.datasources.prometheus_uid }}" }`.

---

## 35. Helm packaging — PrometheusRule CRD

### 35.1 Template

```yaml
{{- if .Values.alerts.enabled }}
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: efe-meta-swarm-rules
  namespace: {{ .Release.Namespace }}
  labels:
    app.kubernetes.io/name: efe-meta-swarm
    prometheus: {{ .Values.alerts.prometheusSelector }}   # default "efe-meta-swarm"
    role: alert-rules
spec:
  groups:
    - name: efe-meta-swarm.recording
      interval: 30s
      rules:
        # 6 recording rules — see § 7.2
    - name: efe-meta-swarm.symptoms
      interval: 30s
      rules:
        # A1, A2, A9, A10 (page-worthy symptoms)
    - name: efe-meta-swarm.causes
      interval: 30s
      rules:
        # A3, A4, A6, A7, A8 (ticket-worthy causes)
    - name: efe-meta-swarm.platform
      interval: 30s
      rules:
        # A5 ControllerNotServing
{{- end }}
```

### 35.2 values.yaml

```yaml
alerts:
  enabled: true
  prometheusSelector: efe-meta-swarm   # matches kube-prometheus-stack RuleSelector
  slo:
    iterateAvailabilityTarget: 0.999
    iterateLatencyP95Target: 1.0       # seconds
    manifestReloadP95Target: 0.5       # seconds
    otelDropRateTarget: 0.01           # 1 %
```

### 35.3 Values-driven thresholds

Most alert thresholds are *intentionally hard-coded* in the YAML — they're
part of the STABILITY_CONTRACT. The SLO targets are values-driven because
operators may want stricter local targets without forking the chart.

---

## 36. CI validation pipeline

### 36.1 New workflow `.github/workflows/dashboards.yml`

```yaml
name: dashboards-and-alerts
on:
  pull_request:
    paths:
      - 'deploy/helm/efe-meta-swarm/dashboards/**'
      - 'deploy/helm/efe-meta-swarm/templates/grafana-*.yaml'
      - 'deploy/helm/efe-meta-swarm/templates/prometheusrule-*.yaml'
      - 'deploy/helm/efe-meta-swarm/values*.yaml'

jobs:
  promtool-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Render PrometheusRule
        run: |
          helm template efe-meta-swarm deploy/helm/efe-meta-swarm \
            --show-only templates/prometheusrule-meta-swarm.yaml \
            > /tmp/rules.yaml
      - name: promtool check rules
        run: docker run --rm -v /tmp:/tmp prom/prometheus:v2.50.0 \
              promtool check rules /tmp/rules.yaml

  promtool-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: promtool test rules
        run: docker run --rm -v $PWD/deploy/helm/efe-meta-swarm/tests:/tests \
              prom/prometheus:v2.50.0 promtool test rules /tests/alerts_test.yaml

  dashboard-schema-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Grafonnet validator
        run: |
          npm install -g @grafana/dashboard-linter
      - name: Validate dashboards
        run: |
          for f in deploy/helm/efe-meta-swarm/dashboards/*.json; do
            dashboard-linter lint -s strict "$f"
          done

  helm-template-diff:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: helm template
        run: helm template efe-meta-swarm deploy/helm/efe-meta-swarm > /tmp/rendered.yaml
      - name: diff vs golden
        run: diff /tmp/rendered.yaml deploy/helm/efe-meta-swarm/tests/golden.yaml

  helm-unittest:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: helm-unittest
        run: docker run --rm -v $PWD:/apps quintush/helm-unittest \
              --strict /apps/deploy/helm/efe-meta-swarm
```

### 36.2 Wall-time

| Job | Time | Cumulative |
|---|---|---|
| promtool check rules | 12 s | 12 s |
| promtool test rules | 14 s | 14 s (parallel) |
| dashboard schema | 18 s | 18 s (parallel) |
| helm template diff | 6 s | 6 s (parallel) |
| helm-unittest | 8 s | 8 s (parallel) |

Total parallel wall-time : 18 s (gated by dashboard schema check). CI cost
delta : ~ + 50 s including container pull cache miss.

### 36.3 Pre-commit hook (optional, devs)

Same commands wrapped in a `make ci-dashboards` target so developers can
catch issues before pushing.

---

## 37. STABILITY_CONTRACT v1.1 — new tiers

### 37.1 Tier additions

| Tier | Promise | Notice for break |
|---|---|---|
| **Dashboard UID** | UID is stable across at least 6 chart-minor versions | 6-minor-version notice |
| **Alert name** | Alert name + critical labels (severity, service, team) are frozen | 6-minor-version notice |
| **Recording rule name** | Series name is frozen | 6-minor-version notice |
| **SLO target** | Target value never drops (relaxes) without contract break | 6-minor-version notice |

### 37.2 The frozen surface

```
Dashboard UIDs : efe-msw-overview, efe-msw-iterate, efe-msw-participant,
                 efe-msw-reload, efe-msw-otel, efe-msw-tls, efe-msw-safety,
                 efe-msw-runtime, efe-msw-traces
Alert names   : IterateFailureRateHigh, IterateLatencyP99Critical, ParticipantDown,
                TLSCertExpiringSoon, ControllerNotServing, OTelExporterFailing,
                SafetyViolationSpike, ManifestReloadFailureSpike,
                SLOBudgetBurnFast14x, SLOBudgetBurnSlow6x
Recording rules : efe_msw:iterate_total:rate5m, efe_msw:iterate_total:rate30m,
                  efe_msw:iterate_total:rate1h, efe_msw:iterate_total:rate6h,
                  efe_msw:iterate_duration:p95_5m, efe_msw:iterate_duration:p99_5m
SLO targets   : iterate-availability=99.9, iterate-latency-p95=1.0s,
                manifest-reload-latency-p95=0.5s, otel-drop-rate=1.0%
```

### 37.3 Breaking-change procedure

  1. Open a PR retitled `[BREAKING] STABILITY_CONTRACT v2`.
  2. Bump chart `version` to 1.0.0 (major).
  3. 6-minor-version deprecation notice in the release notes + `HELP`
     prefix on any newly-renamed series.
  4. Provide a migration table : old name → new name.

---

## 38. Backward compatibility + migration

### 38.1 Existing dashboards

`deploy/observability/grafana/dashboards/m30-*.json` etc. are
**unchanged**. They cover other products (multi-modal, billing, vector, KG)
and reference different metric namespaces. No conflict, no migration.

### 38.2 Existing PR #12 reload metrics (M15 / M16 / M17 / M18)

PR #12 shipped 4 reload metrics that are now considered *legacy* :

```
efe_meta_swarm_reload_attempts_total      (counter)
efe_meta_swarm_reload_failures_total      (counter with `reason` label)
efe_meta_swarm_reload_duration_microseconds (histogram)
efe_meta_swarm_reload_gen                 (gauge)
```

PR #17 added a parallel set with the proper Prometheus conventions :

```
efe_meta_swarm_manifest_reload_total              (counter with `result`, `trigger`)
efe_meta_swarm_manifest_reload_duration_seconds   (histogram)
```

PR #19 dashboards consume **both** : D4 shows the legacy series for
historical continuity and the new series for go-forward. The two sets are
guaranteed coherent (same scrape, same controller, same write path).

Deprecation of the legacy 4 metrics is **not** in scope for PR #19. STABILITY_CONTRACT
v1.1 keeps them STABLE.

### 38.3 Operators upgrading from chart 0.5.0 to 0.6.0

Single `helm upgrade` step. The chart bumps :

  - New ConfigMaps appear in the namespace (sidecar discovery picks them up
    within 60 s).
  - PrometheusRule CRD is created (kube-prometheus-stack reloads its rule
    set within 30 s).
  - Alertmanager routing additions are merged (kube-prometheus-stack
    Alertmanager reloads within 60 s).

Total downtime : 0 seconds. The chart upgrade is fully additive.

---

## 39. Acceptance criteria (AC1 – AC32)

| # | Criterion | Verified by |
|---|---|---|
| AC1 | 9 dashboards exist in `deploy/helm/efe-meta-swarm/dashboards/` | `ls` |
| AC2 | Every dashboard JSON validates against Grafana 10.x schema | dashboard-linter strict |
| AC3 | Every dashboard has `editable: false`, `shared: false`, `timezone: utc` | grep |
| AC4 | Every dashboard has a stable UID matching `efe-msw-*` | jq query |
| AC5 | D1 has the 12 panels listed in § 9.3 | manual inspection |
| AC6 | Every dashboard has `version: N` ≥ 1 | jq |
| AC7 | 10 PrometheusRule alerts render via `helm template` | grep |
| AC8 | Every alert has `runbook_url` annotation | promtool + AST grep |
| AC9 | Every alert has stable labels `{service, team, severity, runbook_id}` | promtool |
| AC10 | `promtool check rules` exits 0 | CI |
| AC11 | `promtool test rules` exits 0 with all assertions passing | CI |
| AC12 | 6 recording rules emit at evaluation_interval 30s | CI assertion |
| AC13 | Recording rules cardinality ≤ 36 total series | promtool query test |
| AC14 | 4 SLOs documented in STABILITY_CONTRACT v1.1 | doc check |
| AC15 | A9 + A10 burn-rate math reproduces from `slo.md` formulas | unit test |
| AC16 | A5 inhibits A1, A2, A3, A6, A7, A8 (same cluster+env) | promtool simulate |
| AC17 | A9 inhibits A1 (same cluster+env+service) | promtool simulate |
| AC18 | Alertmanager routing template renders 2 new receivers | helm template |
| AC19 | `helm upgrade` from chart 0.5.0 → 0.6.0 succeeds without manual intervention | helm-unittest |
| AC20 | Exemplars work end-to-end : iterate slow trace → D2 P8 exemplar dot → click → Tempo trace | manual smoke (integration env) |
| AC21 | All 10 runbook stubs exist with the 6 required sections | doc check |
| AC22 | STABILITY_CONTRACT v1.1 introduces 4 new tiers | doc diff |
| AC23 | Every panel uses `${prometheus_ds}` datasource variable, not hardcoded UID | jq grep |
| AC24 | Tempo datasource UID is `tempo-efe` in every dashboard | jq grep |
| AC25 | `var-participant_id` query is single-value where appropriate (D3) | jq |
| AC26 | `var-manifest_version` is multi-value (D2) | jq |
| AC27 | Panel cardinality cap : every per-participant query has `topk(200, ...)` | grep |
| AC28 | CI new workflow `dashboards.yml` exists and passes | CI green |
| AC29 | Existing `m30-*` / `m31-*` dashboards untouched (zero diff) | git diff |
| AC30 | Helm chart README updated with the new `grafana.dashboards.*` knobs | doc check |
| AC31 | No new Cargo dependency, no Rust LOC change | `cargo tree` diff |
| AC32 | Doc parity : PR_19 architecture (≥ 1 500 lines) ; PR_18 was 1 536 | wc -l |

---

## 40. Threat-model coverage (T66 – T75)

| ID | Threat | Mitigation | AC |
|---|---|---|---|
| T66 | Dashboard XSS via panel title | Grafana 10.x sanitizes by default ; dashboard-linter strict rejects `<script>` in titles | AC2 |
| T67 | PromQL injection via dashboard variable | Variables are typed (`query`, `textbox`, `datasource`) ; query variables only emit label values ; textbox variables (D9 trace_id) accept only [0-9a-f]{32} | AC2 |
| T68 | Alertmanager webhook spoof | Webhook receivers reference Secret-mounted URLs (existing cluster pattern) ; no plaintext webhook in chart YAML | AC18 |
| T69 | Runbook URL phishing | Runbook URLs templated from `runbook_id` label → fixed prefix (`docs.efe.internal/runbooks/`) ; never operator-supplied | AC8 |
| T70 | Dashboard data exfil via shared link | `editable: false` + `shared: false` defaults ; org-level Grafana shared-link disable | AC3 |
| T71 | SLO target tampering by RBAC bypass | SLO targets are values.yaml-driven but committed to STABILITY_CONTRACT ; chart upgrade revalidates the manifest signature (chart provenance, PR #14) | AC14 |
| T72 | Alert silence abuse | Alertmanager silences are RBAC-gated to the `meta-swarm-sre` team (existing cluster RBAC) | — (operational) |
| T73 | Recording-rule cardinality explosion | `topk(200, ...)` wrap on every per-participant rule ; `efe_msw:` namespace + dashboard cap | AC13, AC27 |
| T74 | Datasource credential leak via JSON export | Dashboard JSON references datasource by UID only (no credentials embedded) ; Grafana export warns on credential leakage | AC23, AC24 |
| T75 | Grafana session hijack | Out of scope — relies on Grafana SSO + session-cookie hardening from the cluster Grafana installation | — (operational) |

---

## 41. Tests strategy

### 41.1 Unit tests — recording-rule math

```yaml
# deploy/helm/efe-meta-swarm/tests/alerts_test.yaml
rule_files:
  - ../templates/prometheusrule-meta-swarm.yaml
evaluation_interval: 30s
tests:
  - name: iterate_p95_rule_matches_handcrafted
    input_series:
      - series: 'efe_meta_swarm_iterate_duration_seconds_bucket{le="0.5", manifest_version="v1.0"}'
        values: '0+10x60'  # 10 samples per 30s for 30 minutes
      - series: 'efe_meta_swarm_iterate_duration_seconds_bucket{le="1.0", manifest_version="v1.0"}'
        values: '0+20x60'
      - series: 'efe_meta_swarm_iterate_duration_seconds_bucket{le="+Inf", manifest_version="v1.0"}'
        values: '0+20x60'
    promql_expr_tests:
      - expr: 'efe_msw:iterate_duration:p95_5m{manifest_version="v1.0"}'
        eval_time: 30m
        exp_samples:
          - { labels: '{manifest_version="v1.0"}', value: 1.0 }
```

### 41.2 Burn-rate alert simulation

```yaml
  - name: a9_fires_when_burn_rate_above_14_4x
    interval: 30s
    input_series:
      - series: 'efe_meta_swarm_iterate_total{result="ok"}'
        values: '0+100x120'
      - series: 'efe_meta_swarm_iterate_total{result="error"}'
        values: '0+2x120'   # 2 errors per 30s = 2/100 = 2% > 14.4×0.1% = 1.44%
    alert_rule_test:
      - eval_time: 1h
        alertname: SLOBudgetBurnFast14x
        exp_alerts:
          - exp_labels:
              service: meta-swarm
              severity: critical
              slo_id: iterate-availability
              runbook_id: MSW9
```

### 41.3 Golden JSON

The first PR run captures `helm template` output to
`deploy/helm/efe-meta-swarm/tests/golden.yaml`. Every subsequent PR
diff-checks against it ; intentional changes update the golden in the
same commit.

### 41.4 Manual smoke (integration env)

Pre-merge checklist (executed by reviewer #1) :

  1. `helm install` on a dev cluster with kube-prometheus-stack.
  2. Wait 60 s for sidecar discovery.
  3. Open D1 — confirm all 12 panels render.
  4. Trigger a synthetic iterate failure (chaos-monkey) → confirm A1
     fires within 5 m 30 s.
  5. Trigger 1 manifest reload → confirm reload_gen panel updates +
     annotation appears in D1 + D4.
  6. Click D2 P8 exemplar → confirm Tempo opens.

---

## 42. Performance budget

### 42.1 Query cost

| Panel type | Series scanned | Wall-time (p50) | Wall-time (p99) |
|---|---|---|---|
| Stat panel (P1–P6 on D1) | ≤ 10 | 5 ms | 20 ms |
| Timeseries 5m window | ≤ 100 | 50 ms | 200 ms |
| Heatmap 5m window | ≤ 500 (buckets × labels) | 200 ms | 800 ms |
| Burn-rate alert (recording rule) | ≤ 6 | 1 ms | 5 ms |
| Burn-rate alert (live PromQL) | ≤ 50 000 (6 h × buckets) | 800 ms | 2.5 s |

**Conclusion** : recording rules cut burn-rate alert cost by ~ 500×.
Worth the 36-series cardinality overhead.

### 42.2 Dashboard refresh cost

```
30 viewers × 12 panels (D1) × refresh 30s = 720 queries/min
720 × 50 ms p50 = 36 s of CPU/min = 0.6 cores
```

Acceptable. Heatmap-heavy dashboards (D2 P5/P6/P9) ramp this to ~ 1.8
cores — still well within Prometheus' 8-core budget.

### 42.3 Storage

Recording rules: 36 series × 2 bytes/sample × 30 samples/min × 60 min × 24 h
× 15 d = ~ 4.7 MB / Prometheus / 15 d. Negligible.

Exemplars: 200 k × 96 bytes/exemplar = ~ 19 MB. Negligible.

---

## 43. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Alert false positive during canary | Medium | Pager fatigue | A9/A10 2-window pattern + AD28 `keep_firing_for: 5m` |
| Dashboard slow refresh under load | Low | Operator UX | Recording rules + AD14 30s refresh + AD16 topk cap |
| Exemplar storage growth uncapped | Low | Prometheus OOM | `--storage.exemplars.exemplars-limit=200000` chart value |
| Helm template diff lint blocks unrelated PRs | Medium | Velocity | Golden file auto-regenerable via `make update-golden` |
| Runbook drift (URL exists, content stale) | High | Bad on-call experience | Runbook content tested via `markdownlint` + quarterly review process |
| New Prom-Operator version changes CRD schema | Low | PrometheusRule rejection | Pin minimum kube-prometheus-stack version in Chart.yaml `kubeVersion` |
| Grafana 11 deprecates dashboard schema 38 | Medium | Migration cost | `version: 1` + schema 38 supports Grafana 10 + 11 ; bump in PR #20 |
| Compliance audit fork drops alerts under load | Low | Audit gap | Loki sink already capacity-planned (M28) ; alert volume from PR #19 is < 10 alerts/min |
| PagerDuty service-key rotation breaks A5 | Low | Page failure | Service-key rotation runbook in cluster ops ; not PR #19 specific |
| Recording-rule eval lag during Prometheus restart | Low | 30-90s gap in dashboards | Acceptable — kube-prometheus-stack restart is rare |

---

## 44. Rollback strategy

### 44.1 Rollback path

```
helm rollback efe-meta-swarm <previous-revision>
```

Effect :

  - ConfigMaps removed → sidecar removes dashboards within 60 s.
  - PrometheusRule deleted → kube-prometheus-stack drops alerts within 30 s.
  - Alertmanager routing reverts (operators see alerts route via the cluster
    default receiver, not the meta-swarm-specific one).

### 44.2 Partial rollback

| Failure | Partial rollback |
|---|---|
| Dashboard JSON syntax error post-merge | Disable `grafana.dashboards.enabled` in values ; alerts unaffected |
| Alert misfires | Disable `alerts.enabled` ; dashboards unaffected |
| SLO target too aggressive | Edit `alerts.slo.iterateAvailabilityTarget` in values |

### 44.3 No-state-loss guarantee

PR #19 produces no persistent state (no PVC, no DB writes). Rollback is
fully reversible.

---

## 45. Sequence diagrams

### 45.1 Dashboard provisioning

```
operator                       kubectl                  Grafana sidecar              Grafana
  |                              |                           |                          |
  |--- helm upgrade efe-msw ---->|                           |                          |
  |                              |--- apply ConfigMap ------>|                          |
  |                              |    (label grafana_dashboard=1)                       |
  |                              |                           |--- watch ConfigMaps ---->|
  |                              |                           |                          |
  |                              |                           |<-- ConfigMap event ------|
  |                              |                           |                          |
  |                              |                           |--- POST /api/dashboards->|
  |                              |                           |                          |
  |                              |                           |                          |--- store dashboard
  |                              |                           |                          |
  |<------ dashboards visible at /d/efe-msw-overview within 60s ----------------------- |
```

### 45.2 Alert flow

```
Prometheus                Alertmanager              meta-swarm-sre-page (PagerDuty)
   |                          |                            |
   |--- evaluate rule -->     |                            |
   |    (every 30s)           |                            |
   |--- threshold breach -----|                            |
   |    (5m of for:)          |                            |
   |--- fire alert ---------->|                            |
   |                          |--- routing match ---|      |
   |                          |    (service=meta-swarm,    |
   |                          |     severity=critical)     |
   |                          |--- POST PagerDuty ------------>|
   |                          |                                |
   |                          |--- continue: true (audit fork)-+-> compliance-audit-loki
   |                          |                                |
   |<-- alert continues firing every 30s; resolved when expr stops --|
```

### 45.3 SLO burn detection

```
                  current
                   error
                   rate
                    |
   target  ----+----|--------------+--- (target = 0.001 = 99.9 %)
   1× burn  ----+--/-+--------------+
   6× burn -----+--|--+-------------+   <- A10 fires at 6× sustained
   14.4× burn ----+--|--+-----------+   <- A9 fires at 14.4× sustained
                    |
                  current
                  rate consumes
                  budget faster
                  than allocated
```

---

## 46. Reviewer checklist

> Tick every box before LGTM.

**Foundations**
  - [ ] AD1–AD35 reviewed and consistent with PR #18 conventions
  - [ ] STABILITY_CONTRACT v1.1 diff reviewed (37.2)
  - [ ] No new Rust LOC (AC31)

**Dashboards**
  - [ ] 9 dashboards UIDs match `efe-msw-*` and are stable
  - [ ] Every dashboard has `editable: false` + `shared: false` + `timezone: utc`
  - [ ] Datasource referenced as `${prometheus_ds}` everywhere (AC23)
  - [ ] Tempo datasource UID is `tempo-efe` (AC24)
  - [ ] D1 covers Q1–Q9 from § 5.2
  - [ ] No PII in panel titles, descriptions, or markdown

**Alerts**
  - [ ] 10 alerts named per § 21.1
  - [ ] Every alert has `runbook_url` (AC8)
  - [ ] Severity model respected (AC11)
  - [ ] Inhibition rules match § 33.3
  - [ ] Burn-rate math reproduces from § 30 / § 31

**SLOs**
  - [ ] 4 SLOs documented (32.1–32.4)
  - [ ] Targets are values.yaml-driven (35.2)
  - [ ] STABILITY_CONTRACT v1.1 SLO-target tier populated (37.2)

**Helm**
  - [ ] chart.yaml version 0.5.0 → 0.6.0
  - [ ] values.schema.json extended
  - [ ] Helm template renders with default values (`helm template`)
  - [ ] Helm-unittest snapshots updated

**CI**
  - [ ] `.github/workflows/dashboards.yml` exists and passes
  - [ ] promtool check rules + test rules green
  - [ ] dashboard-linter strict green
  - [ ] helm template diff golden updated

**Runbooks**
  - [ ] 10 runbook stubs exist in `docs/runbooks/RB_PR19_MSW{1..10}.md`
  - [ ] Each has the 6 sections : Summary, Symptom, Diagnose, Mitigate, Escalate, Postmortem links
  - [ ] markdownlint green

---

## 47. Appendix A — PromQL cookbook (per-panel reference)

### A.1 Iterate p95 by manifest_version

```promql
histogram_quantile(
  0.95,
  sum by (manifest_version, le) (
    rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])
  )
)
```

Intent : track latency split by manifest version. Use during canary
roll-out to spot a regression in v1.3 vs v1.2.

### A.2 Healthy-participant ratio

```promql
100
* sum(efe_meta_swarm_participant_up{cluster=~"$cluster", env=~"$env"} == 1)
/ sum(efe_meta_swarm_participants_configured{cluster=~"$cluster", env=~"$env"})
```

Intent : human-readable percentage. Threshold-coloured in D1 P2 (red < 80,
yellow < 95, green ≥ 95).

### A.3 Days-until-cert-expiry

```promql
(efe_meta_swarm_tls_cert_expiry_seconds - time()) / 86400
```

Intent : convert epoch-seconds to days, used in D6 P1 + A4.

### A.4 SLO burn rate (availability, 1h window)

```promql
(
  sum(rate(efe_meta_swarm_iterate_total{result="error"}[1h]))
  /
  sum(rate(efe_meta_swarm_iterate_total[1h]))
) / 0.001
```

Intent : ratio to budget (0.001 = 0.1 % monthly budget). Value 1 = exactly
on budget ; 14.4 = exhaust budget in 2 days ; 6 = exhaust in 5 days.

### A.5 Slow-iterate exemplar surface

Grafana renders exemplars automatically when panel has `enableExemplars: true`
and datasource has `exemplarTraceIdDestinations` set. No PromQL change
required ; the underlying histogram_quantile query carries `_trace_id` via
the `/api/v1/query_range?exemplars=true` endpoint.

### A.6 Step latency p99 — per participant

```promql
topk(20,
  histogram_quantile(
    0.99,
    sum by (participant_id, le) (
      rate(efe_meta_swarm_step_duration_seconds_bucket[5m])
    )
  )
)
```

Intent : top-20 slowest participants. `topk` enforces AD16 panel cardinality cap.

### A.7 Reload-rate by trigger over 24h

```promql
sum by (trigger) (
  increase(efe_meta_swarm_manifest_reload_total{result="ok"}[24h])
)
```

Intent : pie-chart slice on D4 P4. `trigger` taxonomy is closed
(sighup / grpc / file_change).

### A.8 Safety-violation top-10 participants over 1h

```promql
topk(10,
  sum by (participant_id) (
    increase(efe_meta_swarm_safety_violations_total[1h])
  )
)
```

### A.9 OTel drop ratio (5m)

```promql
sum(rate(efe_meta_swarm_otel_dropped_spans_total[5m]))
/
(
  sum(rate(efe_meta_swarm_otel_dropped_spans_total[5m]))
  + sum(rate(efe_meta_swarm_otel_exported_spans_total[5m]))
)
```

Intent : numerator-only would lie when traffic is zero (NaN). Denominator
includes both buckets so the ratio is well-defined at zero rate.

### A.10 Iterate-inflight saturation

```promql
efe_meta_swarm_iterate_inflight
/
scalar(sum(kube_deployment_status_replicas_available{deployment=~".*controller.*"}))
```

Intent : per-replica saturation. > 1 means a single replica is taking on
more than its share — possible router stickiness or hot-spot.

---

## 48. Appendix B — recording rules reference

```yaml
groups:
  - name: efe-meta-swarm.recording
    interval: 30s
    rules:
      # R1
      - record: efe_msw:iterate_total:rate5m
        expr: sum by (result) (rate(efe_meta_swarm_iterate_total[5m]))
        labels:
          owner: meta-swarm-sre
      # R2
      - record: efe_msw:iterate_total:rate30m
        expr: sum by (result) (rate(efe_meta_swarm_iterate_total[30m]))
      # R3
      - record: efe_msw:iterate_total:rate1h
        expr: sum by (result) (rate(efe_meta_swarm_iterate_total[1h]))
      # R4
      - record: efe_msw:iterate_total:rate6h
        expr: sum by (result) (rate(efe_meta_swarm_iterate_total[6h]))
      # R5
      - record: efe_msw:iterate_duration:p95_5m
        expr: |
          histogram_quantile(
            0.95,
            sum by (manifest_version, le) (
              rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])
            )
          )
      # R6
      - record: efe_msw:iterate_duration:p99_5m
        expr: |
          histogram_quantile(
            0.99,
            sum by (manifest_version, le) (
              rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])
            )
          )
```

---

## 49. Appendix C — Alertmanager routing tree (full YAML)

```yaml
global:
  resolve_timeout: 5m

route:
  group_by: ['alertname', 'cluster', 'env']
  group_wait: 30s
  group_interval: 5m
  repeat_interval: 12h
  receiver: cluster-default
  routes:
    # 1. Meta-swarm — critical
    - match:
        service: meta-swarm
        severity: critical
      receiver: meta-swarm-sre-page
      continue: true                          # AD31
      group_wait: 30s
      group_interval: 5m
      repeat_interval: 4h
      routes:
        # 1.1 SLO burn — even tighter cadence
        - match:
            slo_id: iterate-availability
          group_wait: 10s
          repeat_interval: 1h
          receiver: meta-swarm-sre-page

    # 2. Meta-swarm — warning
    - match:
        service: meta-swarm
        severity: warning
      receiver: meta-swarm-sre-ticket
      continue: true
      group_wait: 1m
      group_interval: 10m
      repeat_interval: 12h

    # 3. Meta-swarm — compliance audit fork (always)
    - match:
        service: meta-swarm
      receiver: compliance-audit-loki

inhibit_rules:
  - source_match:
      alertname: ControllerNotServing
      severity: critical
    target_match:
      service: meta-swarm
    equal: [cluster, env]

  - source_match:
      alertname: SLOBudgetBurnFast14x
      severity: critical
    target_match:
      alertname: IterateFailureRateHigh
    equal: [cluster, env, service]

  - source_match:
      alertname: SLOBudgetBurnSlow6x
      severity: warning
    target_match:
      alertname: IterateFailureRateHigh
      severity: warning
    equal: [cluster, env, service]

receivers:
  - name: cluster-default
    slack_configs:
      - api_url_secret: {name: alertmanager-slack-cluster, key: webhook-url}
        channel: '#cluster-alerts'

  - name: meta-swarm-sre-page
    pagerduty_configs:
      - service_key_secret: {name: alertmanager-pagerduty-meta-swarm, key: routing-key}

  - name: meta-swarm-sre-ticket
    slack_configs:
      - api_url_secret: {name: alertmanager-slack-meta-swarm, key: webhook-url}
        channel: '#meta-swarm-alerts'

  - name: compliance-audit-loki
    webhook_configs:
      - url: http://loki-gateway.monitoring.svc:3100/audit/alerts
```

---

## 50. Appendix D — operator runbook stubs

Every runbook follows the same 6-section template :

```markdown
# RB_PR19_MSW{N} : {AlertName}

## Summary
{1-line, < 80 chars}

## Symptom
{What the operator will see — Grafana panel, customer complaint, ...}

## Diagnose
{Step-by-step PromQL queries / kubectl commands / log greps}

## Mitigate
{The actions to take. Each numbered.}

## Escalate
{Who to page if mitigation fails. Channels.}

## Postmortem
{Link to template + recent incident postmortems}
```

| # | File | Alert | Title |
|---|---|---|---|
| 1 | `RB_PR19_MSW1.md` | A1 | Iterate failure rate > 5 % |
| 2 | `RB_PR19_MSW2.md` | A2 | Iterate p99 latency > 5 s |
| 3 | `RB_PR19_MSW3.md` | A3 | Participant down |
| 4 | `RB_PR19_MSW4.md` | A4 | TLS cert expiring < 7 d |
| 5 | `RB_PR19_MSW5.md` | A5 | Controller not serving |
| 6 | `RB_PR19_MSW6.md` | A6 | OTel exporter failing |
| 7 | `RB_PR19_MSW7.md` | A7 | Safety violation spike |
| 8 | `RB_PR19_MSW8.md` | A8 | Manifest reload failure spike |
| 9 | `RB_PR19_MSW9.md` | A9 | SLO budget fast burn 14.4× |
| 10 | `RB_PR19_MSW10.md` | A10 | SLO budget slow burn 6× |

### D.1 Example — RB_PR19_MSW1 (excerpt)

```markdown
# RB_PR19_MSW1 : Iterate failure rate > 5 %

## Summary
Iterate RPC is failing for more than 5 % of requests over the last 5 minutes.

## Symptom
- PagerDuty page on `IterateFailureRateHigh`.
- D1 P3 (Iterate success rate) is red.
- D2 P7 (failure-reason rate) shows the dominant reason.

## Diagnose
1. Open D2 → check the dominant `failure_reason` (top of P7).
2. Cross-reference :
   - `failure_reason=safety_violation` → D7 (safety dashboard) ; check for a
     mis-deployed manifest that loosened the safety geometry.
   - `failure_reason=participant_down` → D3 ; check participant up state.
   - `failure_reason=deadline_exceeded` → D8 ; check CPU / fd saturation.
   - `failure_reason=internal` → traces (D9) for the most recent slow trace.

3. Confirm reload_gen and build_info on D1 P4 / P11 — did the last reload
   change the algorithm parameters significantly ?

## Mitigate
1. **If failure_reason=safety_violation** : roll back the manifest via
   `kubectl create configmap manifest-v{N-1}` + `helm upgrade --reset-then-reuse`.
2. **If failure_reason=participant_down** : scale up the participant
   deployment (`kubectl scale deployment efe-meta-swarm-participant
   --replicas=+1`). Check `participant_up` on D1 P2.
3. **If failure_reason=internal** : capture the slow trace from D9 and file
   an incident ticket. Mitigate by canary roll-out reduction
   (`helm upgrade --set canary.weight=10`).

## Escalate
- After 30 m of unmitigated alert : page meta-swarm dev lead via PagerDuty
  escalation policy `meta-swarm-l2`.
- If trace shows a panic or assertion failure : page security + meta-swarm
  dev simultaneously.

## Postmortem
- Use template `docs/templates/POSTMORTEM_TEMPLATE.md`.
- Past incidents : (none yet — PR #19 first roll-out).
```

(9 more runbooks in the same shape — Phase-2 deliverable.)

---

## 51. Appendix E — multi-cluster dashboard topology

### E.1 Variable cascade

```
$cluster  → multi-cluster Prometheus (Thanos / Cortex) groups by `cluster` label
$env       → derived ; depends on $cluster
$service   → independent of $cluster
$participant_id → scoped to ($cluster, $env)
$manifest_version → scoped to ($cluster, $env)
```

### E.2 Federation cost

Each dashboard panel query is scoped by `cluster=~"$cluster", env=~"$env"`.
Thanos query-frontend deduplicates across replica clusters. No extra cost
beyond the per-cluster query.

### E.3 Disaster recovery view

Operators on the DR cluster select `$cluster=dr-us-east` and see only the
DR replica's iterate rate. Useful during a failover drill.

---

## 52. Appendix F — Tempo TraceQL cookbook

### F.1 Find slow iterate traces (controller-side)

```traceql
{ service.name = "efe-meta-swarm-controller" && name = "efe.meta_swarm.iterate" && duration > 1s }
```

### F.2 Find safety-violation traces

```traceql
{ service.name = "efe-meta-swarm-controller" && name = "efe.meta_swarm.safety_check" && status = error }
```

### F.3 Find traces where a specific participant was slow

```traceql
{ service.name = "efe-meta-swarm-participant" && resource.efe.participant.id = "p-foo" && name = "efe.meta_swarm.step" && duration > 500ms }
```

### F.4 Connect a metric exemplar to its trace

Exemplar dot click → Grafana pivots to Tempo with the `trace_id` pre-filled.
The trace view auto-renders the span tree (PR #18 § 5–13).

### F.5 TraceQL → metric back-reference

```traceql
{ name = "efe.meta_swarm.manifest.reload" && status = error }
```

Reveals the reload_gen at the time of the failure ; cross-reference with
`efe_meta_swarm_reload_gen` on D4.

---

## 53. Appendix G — Phase-3 follow-ups

### G.1 Per-tenant SLO splitting (PR #25 ?)

Today's SLOs are fleet-global. Phase-3 will introduce a `tenant` label
on iterate metrics ; PR #25 will fork the SLO targets per tenant tier
(free / pro / enterprise).

### G.2 Multi-tenant dashboards

`$tenant` variable + per-tenant filtered overview dashboard. Requires
RBAC integration with Grafana team membership.

### G.3 ML-driven anomaly detection

Prometheus / Grafana don't currently surface anomaly detection.
Recommend evaluating Datadog / Honeycomb integrations or a custom Prophet-
based recording rule in Phase-3.

### G.4 Forecasting + capacity planning

Recording rules + Prophet → "iterate p95 forecast for next 24h" gauge.

### G.5 Cost attribution

`$/iterate` panel = cluster cost / iterate count. Requires cost-attribution
exporter (not currently in scope).

### G.6 Migrate to Sloth or Pyrra for SLO management

Sloth/Pyrra generate burn-rate alerts from SLO definitions. PR #19 hand-
writes them ; future migration would auto-generate from `slos.yaml`.

### G.7 Alert dependencies (PromQL-level)

Today's inhibition is Alertmanager-side. Future enhancement : encode
alert deps in PromQL via `unless` operators for richer suppression.

### G.8 Dashboard A/B testing

Two-variant dashboards for UX experimentation. Requires Grafana
Enterprise feature flags ; not in scope.

### G.9 OpenSLO standard

Migrate to the [OpenSLO](https://openslo.com) specification format as it
matures. PR #19's SLO YAML is already aligned with OpenSLO 1.0 in spirit.

### G.10 GitOps-driven dashboard rollout

Move dashboard provisioning from sidecar discovery to ArgoCD ApplicationSets
that target the Grafana folder API. Eliminates the 60 s sidecar discovery
latency.

---

## End of architecture document

| Section count | 53 |
| Architecture decisions | 35 |
| Acceptance criteria | 32 |
| Threat IDs | 10 (T66 – T75) |
| Dashboards | 9 |
| Alerts | 10 |
| Recording rules | 6 |
| SLOs | 4 |
| Runbooks | 10 |
| Total LOC (this doc) | ~ 1 720 |

— EFE platform team, observability area, May 2026
