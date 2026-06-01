# EFE Mode 1 — Service Level Objectives

**Owner:** EFE Platform team · **On-call:** `#efe-oncall` · **Docs:** `LETTER_E_ARCHITECTURE.md` §12.

This document is the **public SLO contract** between the EFE Platform
team (provider) and the consuming product teams (customers). It defines
what the service promises, how we measure it, and what happens when the
promise is broken.

---

## 1. Scope

This SLO covers the **data plane** (`efe.mode1.v1.Mode1Service/Classify`)
of the EFE production deployment, measured at the ingress of the
`efe` Kubernetes namespace.

The **control plane** (`efe.admin.v1.AdminService`) has its own,
less stringent SLO (best-effort, not formally tracked) because it is
operated by the Platform team only.

---

## 2. Service Level Indicators (SLI)

All SLIs are computed from Prometheus metrics emitted by `efe-server`
pods. They are **actionable**: each can drive an alert, each can be
traced back to a dashboard panel.

| SLI                       | Definition                                                          | Prometheus query (5-min rate)                                                                                                                                    |
|---------------------------|---------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Availability**          | `1 − (errors / total)`                                              | `1 - (sum(rate(efe_classify_errors_total[5m])) / sum(rate(efe_classify_requests_total[5m])))`                                                                    |
| **Latency p95**           | p95 of classify duration                                            | `histogram_quantile(0.95, sum(rate(efe_classify_duration_seconds_bucket[5m])) by (le))`                                                                          |
| **Latency p99**           | p99 of classify duration                                            | `histogram_quantile(0.99, sum(rate(efe_classify_duration_seconds_bucket[5m])) by (le))`                                                                          |
| **Error rate**            | non-OK gRPC codes over total                                        | `sum(rate(efe_classify_errors_total[5m])) / sum(rate(efe_classify_requests_total[5m]))`                                                                          |
| **Quota fairness**        | 429/RESOURCE_EXHAUSTED rate per tenant                              | `sum by (tenant) (rate(efe_classify_errors_total{grpc_code="RESOURCE_EXHAUSTED"}[5m]))`                                                                          |
| **Freshness**             | max age of loaded catalogs on any pod                               | `max(time() - efe_catalog_loaded_timestamp_seconds)`                                                                                                             |

The **error** set excludes `INVALID_ARGUMENT` and `PERMISSION_DENIED`
(these are *client bugs*, not service failures). It includes
`UNAVAILABLE`, `DEADLINE_EXCEEDED`, `INTERNAL`, `UNKNOWN`,
`RESOURCE_EXHAUSTED` above an allowed baseline.

---

## 3. Service Level Objectives (SLO)

| Objective           | Target (MVP) | Target (V1.1) | Measurement window |
|---------------------|--------------|---------------|--------------------|
| Availability        | **99.5 %**   | 99.9 %        | rolling 30 days    |
| Latency p95         | **< 2 s**    | < 1.5 s       | rolling 28 days    |
| Latency p99         | **< 3 s**    | < 2.5 s       | rolling 28 days    |
| Error rate          | **< 0.5 %**  | < 0.1 %       | rolling 30 days    |
| Catalog freshness   | **< 10 min** | < 2 min       | real-time          |

### Error budget (30 days)

| Target        | Budget (unavailability) |
|---------------|-------------------------|
| 99.5 %        | 3 h 36 min              |
| 99.9 % (V1.1) | 43 min 12 s             |

---

## 4. SLO exclusions

The following events **do not** consume error budget:

- Scheduled maintenance announced ≥ 7 days in advance.
- Upstream cluster outages (Kubernetes control plane, network).
- Client-side bugs producing `INVALID_ARGUMENT` or `PERMISSION_DENIED`.
- Tenant quota violations (`RESOURCE_EXHAUSTED` on the *offending* tenant
  only; other tenants' errors DO count).

---

## 5. Error budget policy

The **burn rate** is the multiple of the sustainable error rate we are
currently observing. A burn rate of *1×* exhausts the monthly budget
exactly at the end of the 30-day window; higher multiples exhaust it
faster. The canonical Google SRE alerts (fast / slow burn) trigger on
short + long windows simultaneously to catch both sudden outages and
slow degradations.

| Burn rate (evaluation window) | Budget burned per hour | Action                                                |
|-------------------------------|------------------------|-------------------------------------------------------|
| **≥ 14.4 × (5 m + 1 h windows)** | **~2 % / hour**     | **Auto-rollback** canary. **P1 incident**, page on-call. |
| ≥ 6 × (30 m + 6 h windows)    | ~0.83 % / hour         | P2 incident, acknowledge within 30 min.              |
| ≥ 3 × (2 h + 24 h windows)    | ~0.42 % / hour         | Slack warning in `#efe-alerts`, no page.              |
| 50 % budget consumed (30 d)   | —                      | Feature-freeze on non-critical releases.             |
| 100 % budget consumed (30 d)  | —                      | **Full freeze** except security patches; postmortem. |

**Worked example:** at 14.4× fast burn, each hour of elapsed time
consumes ≈ 2 % of the monthly budget. Three uninterrupted hours at
this rate is a P1 pager situation because prolonged burn at this rate
exhausts the entire budget in ~2 days.

### Freeze policy in detail

- **Feature freeze:** new protocol fields, new tenants, new catalogs blocked.
- **Full freeze:** only security CVEs and SLO-restoring fixes may ship.
- **Release cadence:** paused weekly deploy until error budget recovers.
- **Exit condition:** error budget back above 70 % on 7-day trailing window.

---

## 6. Measurement and reporting

- **Dashboards:** Grafana "EFE — Error Budget" panel (public to all
  consumer teams) and "EFE — Overview".
- **Weekly report:** auto-generated summary posted every Monday 09:00
  UTC in `#efe-slo-weekly`.
- **Monthly review:** EFE platform lead + one representative per
  consumer team, 30-min slot. Agenda: incidents, error budget trend,
  capacity forecast.

---

## 7. Incident escalation

| Severity | Paging              | Response SLA  | Resolution SLA |
|----------|---------------------|---------------|----------------|
| P1       | PagerDuty + Slack   | 5 min         | 60 min         |
| P2       | Slack `@oncall`     | 30 min        | 4 h            |
| P3       | Slack ticket        | 2 business hours | 3 business days |

See `deploy/docs/runbook.md` for step-by-step diagnostics.

---

## 8. Change management

- SLO changes **tighten** only with 30-day advance notice.
- SLO changes **loosen** only on postmortem finding + platform-team
  signoff + consumer-team notification.
- This document is versioned in Git; every change is a reviewable PR.

---

## 9. Exit / sunset

If the EFE service is to be retired or superseded:

1. 90-day written notice to all consumer teams.
2. Migration guide published (`deploy/docs/MIGRATION_D_TO_E.md`
   + any new target).
3. Final 30 days: read-only / classify-only mode.
4. Service shutdown + archive of audit logs for 1 year.

---

**Last reviewed:** 2026-04-19 · **Next review:** 2026-07-19.
