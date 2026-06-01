# RB_PR17 — `MetricsCardinalityBreach` runbook

| Field | Value |
|---|---|
| **Audience** | Platform on-call + SRE |
| **Severity matrix** | SEV-2 by default ; SEV-1 if `/metrics` body trips the 4 MB cap |
| **Owns** | EFE platform observability area |
| **Reference** | `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 41 (Appendix G), § 18 |
| **Related** | `docs/STABILITY_CONTRACT.md`, `RB_PR16_TLS_MANUAL_ROTATION.md` |

---

## 1. When to use this runbook

Triggered by **any** of :

1. `efe_meta_swarm_cardinality_breach_total > 0` — the in-process
   circuit-breaker dropped at least one observation because the
   per-pod label cardinality ceiling (`1 000` active series) was
   hit.
2. `MetaSwarmMetricsBodyTruncated` alert — the `/metrics` exposition
   body exceeded the 4 MB cap and was truncated (the WARNING line
   `# WARNING : exposition truncated at 4 MB` appears).
3. Pod memory growth correlated with `/metrics` body size growth.

DO NOT use this for ordinary Prometheus scrape-timeout incidents
(`MetaSwarmMetricsScrapeTimeout`) — those are typically scrape
infrastructure issues, not cardinality.

## 2. Pre-flight (≤ 60 s)

```bash
# 2.1 Confirm the breach counter is non-zero and which metric.
kubectl exec -n efe deploy/tst-efe-meta-swarm-controller \
  -- curl -s http://localhost:9091/metrics \
  | grep "^efe_meta_swarm_cardinality_breach_total"
# Expected : at least one row with value > 0 and a `metric=` label
# pointing at the offending series.

# 2.2 Pod memory snapshot.
kubectl top pod -n efe -l app.kubernetes.io/part-of=efe-meta-swarm
# If RSS > 1.5 GB, suspect a leak ; full cardinality breach typically
# adds < 200 MB.

# 2.3 /metrics body size + ESTIMATED series count.
kubectl exec -n efe deploy/tst-efe-meta-swarm-controller \
  -- curl -s http://localhost:9091/metrics | wc -c
kubectl exec -n efe deploy/tst-efe-meta-swarm-controller \
  -- curl -s http://localhost:9091/metrics | grep -c "^efe_meta_swarm_"
# /metrics body size should be < 1 MB for a healthy pod.
```

## 3. Triage (≤ 5 min)

### 3.1 Identify the runaway label

```bash
# Filter the body for the offending metric (from § 2.1) and group
# the label cardinality.
METRIC="efe_meta_swarm_step_total"  # from § 2.1
kubectl exec -n efe deploy/tst-efe-meta-swarm-controller \
  -- curl -s http://localhost:9091/metrics \
  | grep "^${METRIC}{" \
  | wc -l
# This is the active series count for this metric.

# To find the runaway label specifically :
kubectl exec -n efe deploy/tst-efe-meta-swarm-controller \
  -- curl -s http://localhost:9091/metrics \
  | grep "^${METRIC}{" \
  | sed -E 's/.*\{([^}]+)\}.*/\1/' \
  | tr ',' '\n' \
  | sed -E 's/^([^=]+)=.*/\1/' \
  | sort -u
# Listed labels are the dimensions ; check each for unbounded values.
```

### 3.2 Common causes — checklist

  - [ ] **`participant_id`** : verify against manifest.  Schema gates
        DNS-1123 only — any non-conforming value is a red flag.  Run :
        ```bash
        kubectl get cm tst-efe-meta-swarm-manifest -o yaml \
          | yq '.data."manifest.yaml"' | grep "^  - id:"
        ```
  - [ ] **`failure_reason`** : MUST be one of the 8 closed-taxonomy
        values (timeout | dim_mismatch | safety_violation |
        participant_down | internal | deadline_exceeded | cancelled |
        other).  Any other value indicates a code regression — the
        binary's `FailureReason::from_server_error` mapper missed a
        `ServerError` variant.
  - [ ] **`manifest_version`** : during a rolling deploy, 2-3 values
        coexist briefly.  More than 3 distinct values = stuck deploy.
  - [ ] **Cardinality breach by a single metric exceeding 100 series**
        per dimension — suspect a code regression that turned a
        closed label into an open one.

### 3.3 Detect PII-redaction false negatives

```bash
# Search the rendered series for any label value containing a
# capital letter (the chart's runtime PII regex normally redacts
# these).  If found, the PII filter has a gap.
kubectl exec -n efe deploy/tst-efe-meta-swarm-controller \
  -- curl -s http://localhost:9091/metrics \
  | grep -oE '"[A-Z][^"]*"' \
  | sort -u
# Expected : nothing (all label values are lowercase per AD14).
```

## 4. Resolution

### 4.1 Runaway from a bad manifest

```bash
# Fix the manifest (e.g. lowercase the participant id).
kubectl edit cm tst-efe-meta-swarm-manifest -n efe
# Trigger reload via SIGHUP.
kubectl exec -n efe deploy/tst-efe-meta-swarm-controller \
  -- kill -HUP 1
# OR via gRPC :
grpcurl -d '{}' tst-efe-meta-swarm-controller.efe.svc:50080 \
  efe.meta_swarm.MetaSwarmService/ReloadConfig
```

The breach counter does **not** reset on reload — it's an u64
counter that only resets on pod restart.  Subsequent observations
will succeed if the new manifest's cardinality is below the ceiling.

### 4.2 Runaway from a code regression

If § 3.2 reveals a `failure_reason` value outside the closed
taxonomy, the binary needs a patch :

1. Locate the offending `with_label_values(&[..., "bogus_reason"])`
   call in the codebase.
2. Map it through `FailureReason::from_server_error` properly.
3. Cut a chart minor patch with the fix.

In the meantime, restart the affected pods to clear the in-process
guard state :

```bash
kubectl rollout restart deploy -n efe -l app.kubernetes.io/part-of=efe-meta-swarm
```

### 4.3 Body truncation (`MetaSwarmMetricsBodyTruncated`)

The 4 MB cap is intentional (T55 mitigation — bounded blast radius).
If the body actually exceeds 4 MB, the cardinality breach already
fired upstream.  Truncation is the secondary symptom, not the
cause.  Apply § 4.1 or § 4.2 to fix the underlying cardinality
issue ; the body will shrink below the cap once the pods restart
or the runaway label-value is purged.

## 5. Post-resolution checklist

  - [ ] `cardinality_breach_total` rate has been zero for ≥ 30 minutes.
  - [ ] `/metrics` body size is below 1 MB.
  - [ ] No `MetaSwarmMetricsBodyTruncated` alert in the last hour.
  - [ ] Pod memory RSS within ± 10 % of pre-incident baseline.
  - [ ] Postmortem stub filed (linked from incident channel).
  - [ ] If the cause was a code regression : patch PR cut + reviewers
        assigned.
  - [ ] Stability contract not violated (cf. `STABILITY_CONTRACT.md`).

## 6. Known caveats

  - **The CardinalityGuard is per-pod, not fleet-wide.**  A breach
    on pod A doesn't suppress observations on pod B.  Prometheus's
    aggregation handles this correctly, but operators looking at a
    single pod's `/metrics` may see different breach counts.
  - **The breach counter never decrements.**  It's an u64 counter,
    reset only on pod restart.  This is by design — operators
    should grep the counter VALUE, not its RATE-OF-CHANGE, to
    triage.
  - **PII redaction is per-call.**  An already-registered label-value
    that later gets PII-classified (e.g. via a schema gate change)
    will NOT be retroactively redacted — only future observations
    of new tuples go through the filter.  Restart the pod to clear.
