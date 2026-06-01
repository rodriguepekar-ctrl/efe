# Explanation — observability architecture (D13)

> Sprint **D13** (v0.1.13). The "why" behind the observability
> stack. For the "how" and "what", see the matching how-to and
> reference docs.

---

## The premise

EFE is a multi-tenant SaaS substrate. Once 5+ partners are using
it concurrently, the SRE-on-call rotation must answer five
questions in under five minutes :

1. Le service est-il en bonne santé en ce moment ?
2. Quel tenant pose problème ?
3. Quelle version de catalogue chaque tenant utilise ?
4. Combien chaque tenant nous coûte ?
5. Quelqu'un attaque-t-il le système ?

Without observability, the SRE writes ad-hoc PromQL during the
incident — MTTR explodes from minutes to hours. D13 packages
the answers as 6 persona-driven dashboards, 10 runbook-linked
alerts, structured logs, and traceable spans.

---

## Why open-source stack only

* **Portable** : every partner can self-host. No vendor lock-in.
* **Cost-aware** : at 1000+ tenants, Datadog / Newrelic are
  3-10× more expensive than Mimir / Loki / Tempo on object
  storage.
* **PromQL / LogQL standard** : transferable between deployments.
* **Auditable** : the partner can inspect the full pipeline.

The trade-off : the partner runs 4 services (Prometheus,
Grafana, optionally Loki, optionally Tempo). The community
operationality is high — these are stable, idempotent images.

---

## Why dashboards as JSON, not Grafonnet

JSON is :

* **Diffable** : reviewer can see exactly which panel changed
  in a PR.
* **Stable** : Grafana JSON v9-v10 is backwards compatible.
* **Tested** : `jsonschema -i panels.json schema.json` in CI.

Grafonnet (jsonnet) was rejected because :

* Adds a build dep (jsonnet binary).
* Harder to read for non-Grafana experts.
* The panel count (6 dashboards × 10-15 panels = ~ 80) is small
  enough that hand-written JSON is maintainable.

---

## Why ConfigMaps + sidecar discovery

Three options were considered :

| Option | Verdict |
|--------|---------|
| Push via Grafana API at deploy time | Stateful, hard to roll back, requires API token |
| Mount JSON as volume on Grafana pod | Tightly couples to one Grafana instance |
| **ConfigMaps with `grafana_dashboard=1` label, sidecar polling** | **Picked** : declarative, rollback = `helm rollback` |

The sidecar polls every 30 s ; new dashboards land within a
minute of `helm upgrade`. Removed dashboards disappear from
Grafana automatically.

Trade-off : Grafana must run with `kiwigrid/k8s-sidecar` (the
de-facto standard, but not built into vanilla Grafana
installations). The how-to documents the prereq.

---

## The cardinality strategy

D13 spends a lot of effort on cardinality budgeting because
multi-tenant Prometheus deployments universally bite themselves
on this :

* **Bounded labels everywhere** : `tenant`, `method`, `code`,
  `class`, `op_tag` are all explicitly enumerated.
* **No `request_id`, no `text`, no `email`** : these stay in
  trace spans and structured logs.
* **Recording rules** for top-N : `efe:top20_tenants_by_rps:5m`
  is a *constant* 20 series regardless of tenant count.
* **Hash-bucket** for the long tail : `tenant_bucket = hashmod
  100` lets us see the long tail anonymously.
* **Alert** when budget is exceeded :
  `D13ObsCardinalityBomb` fires at 50 000 series.

The expected steady-state cardinality at 100 tenants is
~ 18 000 series (cf. architecture § 5.4) — well within the
panic threshold.

---

## Multi-tenant data plane

There are three layers of separation in the observability stack :

1. **Folder** (per-product) : all EFE dashboards in folder
   `EFE`.
2. **Org** (per-customer) : Grafana 10+ orgs, one per tenant.
3. **Datasource RBAC** (per-team) : the Prometheus datasource
   threads `X-Scope-OrgID` to Mimir / Cortex, which filters
   server-side.

A partner SRE has a Viewer role in their tenant's org. The
`$tenant` variable defaults to their tenant id and (optionally)
is hardcoded so they can't even attempt cross-tenant queries.

The host SaaS operator has a separate "EFE-SaaS-Admin" org
with cross-tenant visibility — never exposed to clients.

---

## Why OpenTelemetry on-by-default (server side)

Traces have one job : pivot from a slow request in a dashboard
to the actual span that took the time.

Without traces, "p99 = 800 ms" is unknowable. With traces +
exemplars, the SRE clicks the slow bucket → traceID → Tempo
waterfall → identifies that `embedder.encode` took 600 ms
on this trace → root cause known in 90 seconds.

The cost is minimal because :

* Sampling at 5 % keeps ingest volume low.
* Errors and slow requests are always sampled (where they
  matter most).
* Spans live on cheap object storage (Tempo).

The client SDK (`efe-tool`) is opt-in via the `[otel]` extra,
so client-side overhead is paid only by users who explicitly
want it.

---

## Why structured logs in 5 streams

The naïve "everything is one log line" model breaks at scale :

* PromQL on free-text doesn't work.
* PII can leak silently.
* Different consumers need different volumes (security wants
  audit retained 7 years ; SRE wants request retained 14
  days).

Splitting into 5 streams (`request`, `audit`, `classify`,
`evolution`, `security`) gives :

* Per-stream retention via Loki labels.
* Schema-driven validation (JSON-Schema in CI).
* Independent indexing and access control.

The streams use a common envelope (`schema_version`, `stream`,
`timestamp`, `tenant_id`, `request_id`, `trace_id`,
`span_id`, …) so cross-stream correlation works seamlessly :
"give me all logs for trace_id X" pulls request + classify +
audit + security in one query.

---

## Decision : runbook URL is mandatory on every alert

Every alert in `prometheusrule-d13.yaml` carries a
`runbook_url` annotation. CI fails the build if any alert
lacks this annotation. Why ?

* An alert without a runbook is a 3 AM page that nobody can
  action.
* The runbook is the *checked-in* knowledge of how to
  diagnose, mitigate, and resolve. It outlives the on-call
  rotation.
* Alert hygiene — the act of writing a runbook is the act of
  proving the alert is reasonable.

The ten alerts have ten runbooks. Each runbook has a fixed
template (alert summary, first 5 minutes, common root causes,
mitigation, resolution, escalation, postmortem). Writing a new
alert = writing a new runbook = 30 minutes.

---

## Failure modes designed against

10 HAZOP vectors are documented in the architecture (§ 15).
The most likely in production :

1. **Cardinality bomb via attacker-controlled label** —
   defended by ServiceMonitor allowlist.
2. **Cross-tenant data leak via shared Grafana org** —
   defended by Mimir multi-tenancy.
3. **Trace data exposing PII** — defended by span attribute
   whitelist (no `text`, no `email`).
4. **Logs leaking JWT tokens** — defended by promtail
   redaction stage (`Bearer.*` → `***REDACTED***`).
5. **Alert flapping** — defended by `for: ≥ 5m` minimum
   enforced by CI test.

The defense-in-depth philosophy is : every mitigation has a CI
test that prevents regression.

---

## Roadmap

Things D13 *doesn't* solve :

* **Auto-scaling** based on observability signals — D15+.
* **Pricing-aware cost dashboards** — D15+ (the SaaS operator's
  product layer).
* **HMAC-signed audit logs** — D14 (when key management lands).
* **Postgres-backed catalog** — D14.
* **APM-style code-level profiling** — D15+.

D13 establishes the *substrate* on which these will land.
