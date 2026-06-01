# EFE On-Call Runbook

**Version:** 1.0.0 · **Last drill:** 2026-04-19 · **Pager:** PagerDuty service `efe-platform`.

> **If you are paged at 3 a.m.**: read **§0 first**, then jump to the
> alert in the table of contents.

---

## 0. Emergency contacts

- On-call engineer: PagerDuty escalation policy `efe-oncall`.
- Backup: `#efe-oncall` Slack channel (mention `@efe-lead`).
- Break-glass root access: AWS account `efe-prod-break-glass`, 2FA via
  the platform Bitwarden vault (ask team lead).
- War room: Zoom link in `#efe-incidents` channel topic.

---

## 0.1 Triage checklist (first 90 seconds)

1. **Acknowledge the page** in PagerDuty (stops re-pages).
2. Open Grafana dashboard **"EFE — Overview"** — is it actually burning
   error budget right now, or a spike that already recovered?
3. Check `#efe-alerts` — previous related fires?
4. Post in `#efe-incidents`:
   `Ack'd <alert-name>. Investigating. Will update in 10 min.`

If in doubt about severity, **page P1**: a false P1 is cheaper than a
real one missed.

---

## 1. Alert index

| Alert                          | Section |
|--------------------------------|---------|
| `EfeErrorRateHigh`             | §2      |
| `EfeLatencyP95High`            | §3      |
| `EfeLatencyP99High`            | §4      |
| `EfePodRestartLoop`            | §5      |
| `EfePodNotReady`               | §6      |
| `EfeTenantQuotaBreach`         | §7      |
| `EfeCatalogStaleOnFleet`       | §8      |
| `EfeCertMTLSExpiringSoon`      | §9      |
| `EfeAdminAuditLogStalled`      | §10     |
| `EfeS3CatalogStoreDegraded`    | §11     |
| `EfeMemoryPressureHigh`        | §12     |
| `EfeCPUThrottlingHigh`         | §13     |

---

## 2. `EfeErrorRateHigh` — error rate > 1 % for 5 min

**Severity:** P1 (critical, burns error budget fast).

**Likely causes (order of probability):**

1. A bad release just rolled out.
2. A corrupted catalog was registered.
3. Upstream dependency (S3, catalog store) is degraded.
4. gRPC server panic loop on a subset of pods.

**Diagnostics (2 min):**

```bash
# Which pods / tenants are affected?
kubectl -n efe top pods --sort-by=memory
kubectl -n efe get pods -o wide
kubectl -n efe logs -l app=efe-server --tail=200 | rg -i 'error|panic'

# Which version is running?
kubectl -n efe describe deployment efe-server | grep Image

# Any canary recently?
kubectl -n efe rollout history deployment/efe-server | tail -5

# Per-tenant breakdown (Grafana "EFE — Per-Tenant")
# or PromQL direct:
sum by (tenant) (rate(efe_classify_errors_total[5m]))
```

**Remediation tree:**

- **Just deployed?** → `kubectl -n efe rollout undo deployment/efe-server`.
  Confirmation: error rate drops within 2 min.
- **Specific tenant explodes?** → check their catalog via
  `grpcurl … AdminService/GetCatalog`. If fingerprint mismatch:
  `AdminService/DeleteCatalog @latest` + re-register last known good
  version.
- **S3 degraded?** → pods serve from in-memory cache (OK for 10 min).
  Coordinate with cloud team.
- **gRPC panic?** → collect pod log, `kubectl delete pod` the panicking
  one to force recreate. File ticket with stack trace.

**Escalation:** 15 min without root cause → page `@efe-lead`.

**Postmortem:** required within 48 h if duration > 10 min.

---

## 3. `EfeLatencyP95High` — p95 > 2 s for 10 min

**Severity:** P2.

**Likely causes:**

1. Traffic spike — HPA catching up.
2. Slow swarm convergence (noisy text, high `max_rounds`).
3. Noisy neighbor: one tenant hammering.
4. Memory pressure → swapping / slow allocations.

**Diagnostics:**

```promql
# Latency split per catalog
histogram_quantile(0.95,
  sum by (catalog, le) (rate(efe_classify_duration_seconds_bucket[5m])))

# Per tenant QPS
sum by (tenant) (rate(efe_classify_requests_total[5m]))

# HPA status
kubectl -n efe describe hpa efe-server | tail -30

# Pod CPU usage
kubectl -n efe top pods
```

**Remediation:**

- **QPS spike?** Ensure HPA ramps up; manually `kubectl scale
  deployment/efe-server --replicas=N` if HPA is lagging.
- **One tenant eating everything?** Check if their quota is
  appropriate; tighten in `tenants.yaml` via `AdminService.UpdateTenant`
  (V1.1). MVP workaround: `kubectl -n efe apply -f patched-rate-limit.yaml`.
- **Specific catalog slow?** Inspect its canonical sentences count.
  Rollback to previous version if just updated.

**Escalation:** 30 min unresolved or p95 > 4 s → P1.

---

## 4. `EfeLatencyP99High` — p99 > 3 s for 10 min

See §3 — same diagnostics, slightly less urgent. If p95 is fine but p99
is high, investigate tail latency: inspect trace samples in Jaeger for
slow spans, look for long `swarm_consensus` spans.

---

## 5. `EfePodRestartLoop` — restarts > 0.3 / 10 min

**Severity:** P2.

**Likely causes:**

1. OOMKill (memory limit too low).
2. Liveness probe flapping (server slow to warm up after GC).
3. Panic in the gRPC handler.

**Diagnostics:**

```bash
kubectl -n efe get events --sort-by=.lastTimestamp | rg efe-server
kubectl -n efe describe pod <offending-pod> | rg -A5 'Last State'
kubectl -n efe logs <offending-pod> --previous
```

**Remediation:**

- **OOM?** Bump `resources.limits.memory` (Helm value, re-apply).
  Investigate memory leak as follow-up.
- **Liveness flap?** Raise `initialDelaySeconds` or `failureThreshold`.
- **Panic?** Collect trace; file Rust crash bug.

---

## 6. `EfePodNotReady` — a pod stuck `Pending`/`CrashLoop`

**Diagnostics:**

```bash
kubectl -n efe describe pod <name>
# Look for: ImagePullBackOff, FailedScheduling, ContainerCannotRun
```

**Common causes:**

- Node resources exhausted → scale cluster.
- Image pull error → check registry auth.
- Config volume missing → kubectl describe shows "MountVolume.SetUp failed".

---

## 7. `EfeTenantQuotaBreach` — tenant hit rate-limit repeatedly

**Severity:** P3 (info, no budget burn — working as designed).

**What to do:**

- Notify tenant via Slack/email: rate-limit breached, current quota X QPS.
- If legit scaling need: negotiate quota bump via `#efe-capacity`.
- Update `tenants.yaml` + deploy.

**Do NOT** auto-bump without platform team approval.

---

## 8. `EfeCatalogStaleOnFleet` — catalog age > 10 min on any pod

**Likely causes:**

1. S3 catalog store down.
2. Pod lost its Redis pub/sub connection (if used for notify).
3. Pod in an odd state (fork issue, OOM post-restart).

**Diagnostics:**

```bash
# Per-pod catalog timestamp
for pod in $(kubectl -n efe get pods -l app=efe-server -o name); do
  kubectl -n efe exec "$pod" -- grpcurl -plaintext localhost:50052 \
    efe.admin.v1.AdminService/ListCatalogs
done
```

**Remediation:**

- Trigger force-reload: `grpcurl … AdminService/ReloadCatalogs`.
- Restart the offending pod: `kubectl -n efe delete pod <name>`.

---

## 9. `EfeCertMTLSExpiringSoon` — cert expires in < 48 h

cert-manager should auto-renew. If it hasn't:

```bash
kubectl -n efe get certificate
kubectl -n efe describe certificate efe-mtls
kubectl -n efe logs -l app=cert-manager -n cert-manager --tail=50
```

Force renewal: `kubectl -n efe annotate certificate efe-mtls cert-manager.io/renew=true --overwrite`.

---

## 10. `EfeAdminAuditLogStalled` — no admin log entries in 24 h

Usually benign (nobody registered a catalog), but verify the audit
sink (DB or PVC) is healthy:

```bash
kubectl -n efe exec -it <pod> -- ls -la /var/lib/efe/audit/
```

---

## 11. `EfeS3CatalogStoreDegraded` — S3 4xx/5xx from pods

Pods serve from in-memory cache. Budget: ~1 h before stale warnings.

Coordinate with cloud team. If prolonged, switch to the fallback PVC:
set `EFE_CATALOG_STORE=/var/lib/efe/catalogs-fallback` via ConfigMap
and rolling restart.

---

## 12. `EfeMemoryPressureHigh` — pod memory > 90 %

Embedder + caches. If chronic: reduce `cache_size` via Helm values, or
lower `maxReplicas` (memory = replicas × embedder). Acute: rolling restart.

---

## 13. `EfeCPUThrottlingHigh` — `container_cpu_cfs_throttled_seconds_total` spiking

Bump `resources.limits.cpu`. Also verify HPA is using CPU utilisation
target correctly (target too high = late scale-up).

---

## 14. Game day playbook summary

See `deploy/chaos/game-day.md` for the full exercise script. Quarterly
drills cover:

- D1: Random pod kill (chaos-mesh `podchaos`).
- D2: Catalog corruption injection.
- D3: Network partition between pods and S3.
- D4: mTLS cert artificial expiry.
- D5: 10× traffic spike (k6 stress).

After each drill: update this runbook with lessons learned.

---

## 15. Postmortem template

For any user-visible incident > 10 min, file a postmortem within 48 h:

```
Title:
When:
Duration:
Severity:
Tenants affected:

Summary:

Timeline (UTC):
  T+00  :
  T+05  :
  T+15  :

Root cause:

Contributing factors:

What went well:

What went badly:

Action items (with owners + dates):
  [ ] ...
  [ ] ...
```

Share in `#efe-postmortems`, blameless tone, focus on systemic fixes.

---

**If this runbook is wrong, fix it in the same PR as the incident
remediation.** A stale runbook is worse than none.
