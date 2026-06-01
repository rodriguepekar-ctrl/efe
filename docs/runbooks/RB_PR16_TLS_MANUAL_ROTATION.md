# RB_PR16 — Manual TLS cert rotation (meta-swarm)

| Field | Value |
|---|---|
| **Audience** | Platform on-call ; security incident responder |
| **Severity matrix** | SEV-2 by default ; SEV-1 if key exposure suspected |
| **Owns** | Platform team + security team co-own |
| **Reference** | `docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md` § 38 (Appendix B) |
| **Related runbooks** | `RUNBOOK_RB_10_TLS_CERT_EXPIRY.md`, `d22_mtls_cert_renewal_failed.md` |

---

## 1. When to use this runbook

Trigger any of :

1. **`MetaSwarmTlsCertExpiringSoon`** alert fires (cert < 7 days from
   expiry — cert-manager should have rotated by now).
2. **Suspected private-key compromise** (Secret exposure, audit log
   anomaly, attestation report flag).
3. **CA key rotation** orchestrated by the security team.
4. **Manual SAN change** (adding `tls.additionalSans` mid-flight).
5. The chaos drill `D-PR16-ROTATION-NO-DROP` quarterly rehearsal.

DO NOT use this runbook for the routine cert-manager 23-day rotation
— that's automatic.

## 2. Pre-flight (≤ 2 min)

```bash
# 2.1 Confirm cert-manager is healthy.
kubectl get pods -n cert-manager
# Expected : cert-manager + cert-manager-cainjector + cert-manager-webhook all `Running`.

# 2.2 List the EFE-meta-swarm Certificates + their `Ready` condition.
kubectl get certificate -n efe -l app.kubernetes.io/part-of=efe-meta-swarm
# Expected : 4 rows (1 controller + 3 participants), Ready=True for all.

# 2.3 Snapshot the current `notAfter` per cert.
for c in $(kubectl get cert -n efe -l app.kubernetes.io/part-of=efe-meta-swarm -o name) ; do
  echo "$c : $(kubectl get $c -n efe -o jsonpath='{.status.notAfter}')"
done
# Note these timestamps ; you'll diff against them after rotation.

# 2.4 Snapshot the in-pod cert hash (so we can later prove the
# inotify watcher actually reloaded).
for pod in $(kubectl get pod -n efe -l app.kubernetes.io/part-of=efe-meta-swarm -o name) ; do
  echo "$pod : $(kubectl exec -n efe $pod -- sha256sum /etc/efe-meta-swarm/tls/tls.crt | awk '{print $1}')"
done
```

If any of these fail, **STOP** and escalate to the security incident
channel ; do NOT trigger a rotation against a broken cert-manager.

## 3. Trigger rotation (≤ 30 s)

### 3.1 Routine renewal (no compromise)

```bash
# For one cert :
kubectl cert-manager renew tst-efe-meta-swarm-controller-tls -n efe

# For all 4 certs :
kubectl cert-manager renew \
  -n efe \
  $(kubectl get cert -n efe -l app.kubernetes.io/part-of=efe-meta-swarm -o name | tr '\n' ' ')
```

### 3.2 Emergency rotation (suspected key compromise)

This forces cert-manager to issue **fresh keys** AND new certs :

```bash
kubectl annotate certificate tst-efe-meta-swarm-controller-tls \
  cert-manager.io/issue-temporary-certificate="true" \
  --overwrite -n efe

# Repeat per participant cert.  Loop :
for c in $(kubectl get cert -n efe -l app.kubernetes.io/part-of=efe-meta-swarm -o name) ; do
  kubectl annotate $c -n efe cert-manager.io/issue-temporary-certificate=true --overwrite
done
```

### 3.3 SAN change (adding `tls.additionalSans`)

Edit the values overlay :

```yaml
tls:
  additionalSans:
    - controller.efe.public.example.com    # NEW
```

Then `helm upgrade tst ./charts/efe-stack -n efe -f values-prod.yaml`.
The Cert resources update in place ; cert-manager re-issues with the
new SAN list.  The runtime pods reload via inotify (no restart).

## 4. Watch the reload land (≤ 90 s)

```bash
# 4.1 Watch cert-manager issue new certs.
kubectl get certificate -n efe -w -l app.kubernetes.io/part-of=efe-meta-swarm
# Wait until `Ready=True` for all 4.

# 4.2 Confirm the in-pod cert hash CHANGED (the inotify watcher
# observed the new Secret).
for pod in $(kubectl get pod -n efe -l app.kubernetes.io/part-of=efe-meta-swarm -o name) ; do
  echo "$pod : $(kubectl exec -n efe $pod -- sha256sum /etc/efe-meta-swarm/tls/tls.crt | awk '{print $1}')"
done
# Diff vs § 2.4 ; every hash must differ.

# 4.3 Confirm the binary logged the reload event.
kubectl logs -n efe deploy/tst-efe-meta-swarm-controller --tail=200 \
  | grep -E "tls\.reload|cert reload completed"
# Expected : `INFO tls.reload : elapsed_ms=… tls.reload OK`.

# 4.4 Confirm the Prometheus reload counter incremented.
curl -s http://prometheus.efe.svc:9090/api/v1/query \
  --data-urlencode 'query=sum(increase(efe_meta_swarm_tls_reload_total{result="ok"}[10m]))'
# Expected : ≥ 4 (one per Cert).
```

## 5. Verify zero connection drop (≤ 60 s)

```bash
# 5.1 In-flight handshake failure rate must remain < 1 %.
curl -s http://prometheus.efe.svc:9090/api/v1/query \
  --data-urlencode 'query=sum(rate(efe_meta_swarm_tls_handshake_total{result="fail"}[5m])) by (role) / sum(rate(efe_meta_swarm_tls_handshake_total[5m])) by (role)'
# Each role : value < 0.01.

# 5.2 Iterate p99 latency must NOT have spiked during the rotation
# window.
curl -s http://prometheus.efe.svc:9090/api/v1/query \
  --data-urlencode 'query=histogram_quantile(0.99, sum by (le) (rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])))'
# Compare to baseline ; must be within ±10 %.

# 5.3 No alert fired during rotation.
kubectl logs -n monitoring deploy/alertmanager --tail=200 \
  | grep -E "MetaSwarmTls"
# Expected : empty.
```

## 6. Audit chain entry (when audit infra lands)

Future PR (audit chain hookup) — for now this is informational :

```bash
# When the audit chain is in place (post-PR #20), every handshake
# generates an audit_events row :
psql -h efe-pg.efe.svc -U efe -d efe -c "
  SELECT occurred_at, event_type, peer_subject, result, protocol_version
  FROM audit_events
  WHERE event_type = 'tls_handshake'
    AND occurred_at > NOW() - INTERVAL '30 minutes'
  ORDER BY occurred_at DESC
  LIMIT 20;
"
# Expected : ≥ 4 rows with event_type='tls_reload' for the rotation
# itself + a normal trickle of `tls_handshake` rows.
```

## 7. Rollback (if rotation went wrong)

The inotify-based reload is **fail-closed** : a malformed cert leaves
the OLD config in memory and increments `efe_meta_swarm_tls_reload_total{result="fail"}`.
The pod keeps serving on the old cert.  If the old cert has already
expired by the time the new one is malformed, you have an outage —
mitigation :

```bash
# 7.1 Disable TLS temporarily (5-minute SLA on the security review).
helm upgrade tst ./charts/efe-stack -n efe -f values-prod.yaml \
  --set efe-meta-swarm.tls.enabled=false
# Pods restart in plain-text mode.

# 7.2 Restore from the last-known-good cert Secret backup.  PR #16
# does NOT (yet) ship a backup ring for Secrets ; manual `kubectl
# get secret <name> -o yaml > backup.yaml` BEFORE the rotation is the
# escape hatch.  Phase-3 ships an automated ring (D-PR16-SECRET-BACKUP).

# 7.3 Re-enable TLS once the cert situation is sane.
helm upgrade tst ./charts/efe-stack -n efe -f values-prod.yaml \
  --set efe-meta-swarm.tls.enabled=true
```

## 8. Post-rotation checklist

  - [ ] All 4 Certificates show `Ready=True`.
  - [ ] Every Pod's in-pod cert hash changed.
  - [ ] No `MetaSwarmTls*` alert fired during the rotation window.
  - [ ] No spike in `efe_meta_swarm_tls_handshake_total{result="fail"}`.
  - [ ] No spike in `efe_meta_swarm_iterate_duration_seconds` p99.
  - [ ] Audit chain entry recorded (once audit infra lands).
  - [ ] Customer-facing SLA dashboard shows no incident in the window.
  - [ ] Postmortem stub filed if the rotation was triggered by an alert
    (not for a scheduled drill).

## 9. Known issues + caveats

  * **macOS dev clusters** (kind / k3d on macOS) : `notify`'s
    FSEvents backend may emit duplicate events ; the chart's
    `WATCHER_DEBOUNCE_MS=500` window collapses them.  If you see
    elevated `efe_meta_swarm_tls_reload_total` counts, that's the
    debounce being too tight on a slow filesystem — not a bug.
  * **TLS 1.2 opt-down** : if `tls.minProtocolVersion="1.2"` is set,
    the rotation still works but `MetaSwarmTlsHandshakeFailureRate`
    threshold may need adjustment (legacy peers churn more).
  * **Cross-namespace ClusterIssuer** : when `tls.certManager.
    clusterIssuer=true`, the rotation cascades to every namespace
    using the issuer ; coordinate with other tenants.
