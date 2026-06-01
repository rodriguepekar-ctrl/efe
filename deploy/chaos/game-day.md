# EFE Game Day Playbook

Quarterly chaos-engineering exercise on the **staging** environment
(or a pre-prod clone). The goal is to validate the runbook, catch
unknown-unknowns, and make sure on-call folks know where the buttons are.

> **Never run these drills against prod** unless explicitly approved
> and coordinated with all consumer teams.

---

## Prerequisites

- Chaos Mesh (or Litmus) installed in the cluster.
- At least 2 pods running in staging.
- `k6` binary available locally.
- Paged engineer + shadow engineer + facilitator (3 roles).
- War room Zoom + `#efe-gameday-YYYYQN` channel.

---

## Drill catalogue

### D1 — Random pod kill

**Goal:** verify HPA + PDB keep availability ≥ 99 % during a pod loss.

**Steps:**

1. Baseline: `kubectl -n efe-staging top pods`; note current replicas.
2. Apply Chaos Mesh `PodChaos` killing 1 pod every 60s for 10 min:

```yaml
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata: { name: efe-random-kill, namespace: efe-staging }
spec:
  action: pod-kill
  mode: one
  selector: { labelSelectors: { app: efe-server } }
  scheduler: { cron: "@every 60s" }
  duration: "10m"
```

3. Run k6 `classify.js` at 100 QPS in parallel.
4. Watch: error rate, p95 latency, replica count.

**Expected:**

- No error spike above 1 %.
- p95 stays below 2 s.
- PDB blocks simultaneous kill.

**Red flag:** error budget burns > 2 × normal.

---

### D2 — Catalog corruption injection

**Goal:** verify the server refuses to load a bad catalog and falls back
to previously cached one.

**Steps:**

1. Upload a `support_triage.yaml` with deliberate schema error via
   AdminService.
2. Observe that all pods refuse the new version (logs contain
   `catalog_validation_failed`).
3. Traffic continues to resolve via the previous in-memory catalog.
4. Remove the bad version via `AdminService.DeleteCatalog`.

**Expected:**

- Zero client-visible error.
- Audit log shows the rejected upload.

---

### D3 — Network partition pods ↔ S3

**Goal:** verify graceful degradation when the catalog store is
unreachable.

**Steps:**

1. Apply `NetworkChaos` blocking egress to S3 from `efe-staging` namespace:

```yaml
apiVersion: chaos-mesh.org/v1alpha1
kind: NetworkChaos
metadata: { name: efe-partition-s3, namespace: efe-staging }
spec:
  action: partition
  mode: all
  selector: { labelSelectors: { app: efe-server } }
  direction: to
  target: { mode: all, selector: { namespaces: [s3-gateway] } }
  duration: "20m"
```

2. Run classify at 50 QPS.

**Expected:**

- All classify calls succeed (in-memory cache).
- Alert `EfeS3CatalogStoreDegraded` fires within 2 min.
- Catalog freshness alert fires after 10 min.

**Red flag:** any pod crashes when S3 is unreachable.

---

### D4 — mTLS cert expiry

**Goal:** verify cert-manager auto-renewal, and that downstream clients
handle a cert rotation without dropped requests.

**Steps:**

1. Force-expire the cert:
   `kubectl -n efe-staging annotate certificate efe-mtls
    cert-manager.io/renew=true --overwrite`
2. Observe old cert in use until rotation completes (2-5 min).
3. Run k6 continuously through the event.

**Expected:**

- Zero dropped requests.
- New cert serial # visible in `kubectl describe certificate`.

---

### D5 — 10× traffic spike

**Goal:** verify HPA scale-up speed and rate-limit fairness.

**Steps:**

1. Run k6 baseline 50 QPS for 2 min.
2. Ramp to 500 QPS over 30 s, hold for 5 min, ramp down.
3. Watch HPA, replica count, latency.

**Expected:**

- HPA reaches maxReplicas within 2 min.
- p95 degrades to < 4 s during ramp (not < 2 s).
- No tenant saturates others (rate-limit fair).
- System recovers to baseline within 3 min after ramp-down.

---

## Debrief template

After each drill:

- **Duration of the drill:**
- **Time to first alert:**
- **Time to acknowledge:**
- **Time to remediation:**
- **Runbook gaps spotted:**
- **Action items (owner + date):**

File in `deploy/docs/gameday-reports/YYYYQN-drill-summary.md`.

---

## Opt-out clause

Any tenant with explicit off-hours SLA (team-fraud for example) can
veto a drill during their critical window. Coordinate ≥ 72 h in advance.
