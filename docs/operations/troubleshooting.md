# EFE — Troubleshooting catalogue

> **Audience** : on-call SRE staring at red pods.
> **Sprint D9 deliverable**.
>
> Each entry: symptom → diagnostic command → most likely cause → fix.

---

## Section A — Install / startup

### A1. `helm install` succeeds but pods stuck `ContainerCreating`

```bash
kubectl describe pod -n efe -l app.kubernetes.io/name=efe \
    | grep -A3 -i events
```

| Event | Cause | Fix |
|-------|-------|-----|
| `Failed to pull image …` | Network egress to `ghcr.io` blocked | Whitelist `ghcr.io` egress, or mirror image internally and override `image.repository` |
| `secret … not found` | `secrets.existingSecretName` references missing Secret | Pre-create the Secret OR use `secrets.inline.enabled=true` (dev only) |
| `error validating "...": admission webhook "..."` | OPA Gatekeeper or PSA blocks `runAsUser: 10001` | Add a Constraint exemption or run as the cluster-default UID |

### A2. Pods running but `Readiness probe failed: connection refused`

```bash
kubectl logs -n efe -l app.kubernetes.io/name=efe --tail=50
```

| Log snippet | Cause | Fix |
|-------------|-------|-----|
| `failed to bind 0.0.0.0:50051: address in use` | Two containers on the same hostNetwork (rare on k8s ≥ 1.27) | Disable hostNetwork in deployment override |
| `permission denied: /var/lib/efe/audit/audit.jsonl` | `readOnlyRootFilesystem: true` + audit path on root volume | Mount an `emptyDir` at `/var/lib/efe/audit` (chart does this by default — verify it's not overridden) |
| `failed to load JWT keys: …` | Secret mounted at wrong path | Set `config.auth.secretsFile=/etc/efe/secrets/hs256` (chart default) |

### A3. `helm test efe` fails with `connection timeout`

```bash
kubectl logs -n efe efe-test-grpc-health
```

* If output is just `connection timeout`: pod-to-Service routing is
  broken. Check `kubectl get svc efe -n efe -o yaml | grep -A2 selector`
  — selector labels must match Pod labels. Common cause: stale
  selector after `--set tenant.id=…`.
* If output mentions `name resolution`: cluster DNS issue —
  `kubectl get pods -n kube-system | grep coredns`.

---

## Section B — Runtime errors

### B1. Sudden 401 storm

```bash
kubectl logs -n efe -l app.kubernetes.io/name=efe \
    | jq -r 'select(.level == "WARN") | .msg' | head
```

If you see `JWT verification failed: signature mismatch`:
1. Check key rotation status — see [`key_rotation.md`](key_rotation.md).
2. The chart supports multi-key validation ; both old and new
   keys should be in the Secret during rotation. Add the new key
   FIRST, deploy, then issue tokens with new key, then remove the
   old key.

### B2. Tail-latency spike (p99 > 5 s)

```bash
# Per-replica request rate
sum by (pod) (rate(grpc_server_handled_total[1m]))
```

| Pattern | Cause | Fix |
|---------|-------|-----|
| One pod 100 % CPU, rest idle | gRPC stuck on single connection (HTTP/2 multiplexing on ClusterIP) | Set `service.headless=true` (D9.A5). Then restart clients to re-resolve DNS |
| All replicas at request rate ceiling | Saturation — HPA hasn't scaled yet | Lower HPA `targetCPU` from 70 → 60 % OR raise `replicas` floor |
| Periodic spikes every N minutes | GC / cache invalidation | Inspect `process_resident_memory_bytes` ; if growing, see B3 |

### B3. Memory grows over hours / RSS slope > 870 B/s

```bash
# Check the soak job's leak detector report
kubectl logs -n efe job/efe-soak --tail=20
```

If `leak_detector` reports `FAIL`:
1. Enable `tokio-console` in non-prod ; capture for 1 h post-warmup.
2. Common culprits in EFE :
   - Cone registry cache without LRU eviction (M9 § 5).
   - Unbounded `mpsc` channel from telemetry sink.
   - Stuck `JoinHandle` waiting on a connection that never closes.
3. File a P2 issue; restart Pods every 24 h as mitigation
   (`kubectl rollout restart deploy/efe -n efe`).

### B4. `INVALID_ARGUMENT: dim_latent must be > 0`

The client is sending a malformed `SwarmSpec`. Mode-1 classifier
expects `dim_latent` ≥ 1 (typically 384 for the bundled
sentence-transformers embedder).

Check the SDK version on the client side — the mismatch usually
comes from `efe-tool` < 0.1.5 which had a bug constructing the
spec. Pin to `efe-tool>=0.1.9`.

---

## Section C — Helm / chart behaviour

### C1. `helm upgrade` failed: `cannot patch X with kind Y`

```
Error: UPGRADE FAILED: cannot patch "efe" with kind Service:
Service "efe" is invalid: spec.clusterIP: Invalid value: "": field is immutable
```

Cause: switching `service.headless` from `false` to `true` (or the
reverse) changes `clusterIP`, which is immutable.

Fix:
```bash
helm uninstall efe -n efe
helm install efe oci://ghcr.io/efe-project/charts/efe \
    --version 0.1.9 -n efe \
    --set service.headless=true ...
```

### C2. New tenant.id label not appearing on existing resources

`helm upgrade` patches existing resources but k8s preserves labels
absent from new templates only when the `last-applied-configuration`
annotation is in sync. If the labels don't appear:

```bash
# Force a re-apply.
kubectl rollout restart -n efe deploy/efe
# Or, full reset (downtime!) :
helm uninstall efe -n efe
helm install efe …
```

### C3. `helm test` semantic-smoke pod failed

```bash
kubectl logs -n efe efe-test-semantic
```

| Log | Cause | Fix |
|-----|-------|-----|
| `ImportError: efe_tool` | bench image missing the SDK | Set `tests.semanticSmoke.image` to a known-good build |
| `FAIL: <text> expected ~fraud, got billing` | Hybrid decoder mis-classified | Check D7 winner is gelled (env `EFE_FUSION_BETA0=0.45`) ; rare but possible after a rollback |

---

## Section D — Network / RBAC

### D1. NetworkPolicy blocking expected traffic

```bash
# Show the policy
kubectl get netpol -n efe efe -o yaml | yq .spec
```

Default policy allows ingress only from labeled namespaces.
If your client lives in `myapp-prod`, label that namespace:

```bash
kubectl label namespace myapp-prod app.kubernetes.io/managed-by=mycorp
```

(or set `networkPolicy.allowedNamespaces` to your namespace's labels)

### D2. ServiceMonitor not picked up by Prometheus

```bash
kubectl get servicemonitor -n efe
kubectl logs -n monitoring -l app=prometheus | grep efe
```

Prometheus only scrapes ServiceMonitors that match its
`serviceMonitorSelector`. If yours filters by `release: prom-stack`,
add `--set serviceMonitor.labels.release=prom-stack` to your Helm
install.

---

## Section E — Cluster-level

### E1. OOM-kill events on Pods

```bash
kubectl get events -n efe --field-selector reason=OOMKilling
```

Cause: real memory pressure OR a leak (see B3).

Mitigation:
1. Raise `resources.limits.memory` to 2 Gi.
2. If raise doesn't help, hunt the leak ; OOM-killer is the
   short-term safety valve, not the fix.

### E2. Cosign signature verification fails

```bash
cosign verify ghcr.io/efe-project/efe:v0.1.9 \
    --certificate-identity-regexp "https://github.com/efe-project/efe/.*" \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

If this fails:
* Image was re-tagged outside our release pipeline (supply-chain
  smell — investigate).
* Cosign was upgraded across a breaking change ; pin to a known-good
  cosign version.

### E3. Trivy scan flags a HIGH/CRITICAL CVE

The release workflow blocks merge on HIGH/CRITICAL. If a NEW CVE
gets backed into a base image at runtime (e.g. distroless rebuild
that pulls a newer libssl), the Trivy scan in the next release run
will catch it.

For an in-flight production cluster:
```bash
# Pin to a previous known-good image until you've reviewed.
helm upgrade efe oci://ghcr.io/efe-project/charts/efe \
    --version 0.1.9 \
    --reuse-values \
    --set image.digest=sha256:<previous-known-good>
```

---

## Section F — When all else fails

1. Capture diagnostics:
   ```bash
   kubectl get all,pdb,hpa,netpol,svc -n efe -o yaml > efe-state.yaml
   kubectl describe pod -n efe -l app.kubernetes.io/name=efe > efe-describe.txt
   kubectl logs -n efe -l app.kubernetes.io/name=efe --tail=200 > efe-logs.txt
   helm get all efe -n efe > efe-helm.yaml
   ```
2. File a bug : <https://github.com/efe-project/efe/issues/new>
3. Attach the four files above. Redact JWT secrets manually if any
   appear in `helm get values` output.
