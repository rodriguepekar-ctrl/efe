# EFE — JWT key rotation runbook

> **Sprint D9.A7 amendment** — closes HAZOP gap #11
> ("Helm upgrade with new secret doesn't reload pods").
>
> EFE supports zero-downtime HS256 key rotation via multi-key
> validation. The procedure below has been tested on minikube
> and on the production-clone staging cluster.

---

## Why multi-key

EFE accepts ANY JWT signed with ANY key currently in the Secret.
Tokens are validated against each key in turn ; the first matching
key wins. This means:

* Old tokens (signed with the previous key) keep working until
  they expire (TTL ≤ 1 h typically).
* New tokens (signed with the next key) work as soon as you
  publish them.
* No "flash cut" required.

---

## Two failure modes to avoid

1. **Replace-only rotation** : if you delete the old key BEFORE
   issuing tokens with the new one, every in-flight client gets
   401s until they re-fetch a new token from the IDP. Bad UX.
2. **Pods don't reload Secret content** : Kubernetes mounts a
   Secret as a *projected volume* and reflects updates within
   ~60 s by default — but `efe-server` reads the file at
   startup, not on every request. Without a reload signal, the
   Pod keeps using the old keys.

---

## Step 1 — Add the new key alongside the old

If you use **External Secrets Operator** (recommended):

```bash
# Edit the upstream secret store (AWS Secrets Manager / Vault).
# The Secret should now contain:
#
#   <one HS256 key per line, hex-encoded>
#   abc123…       # old, still valid
#   def456…       # new, just added
```

If you use a **manually-managed Secret**:

```bash
kubectl get secret efe-jwt-prod -n efe -o yaml > /tmp/secret.yaml
# Edit /tmp/secret.yaml ; append the new key under data.hs256
# (base64-encoded — use `echo -n 'newkey' | base64`).
kubectl apply -f /tmp/secret.yaml
```

---

## Step 2 — Force the Pods to reload

Two equally-valid mechanisms:

### Option A — `helm upgrade --reuse-values` with a checksum bump

The chart hashes the rendered Secret content into a Deployment
annotation. `helm upgrade` notices the hash change and triggers
a rollout — no new args required:

```bash
helm upgrade efe-prod oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.9 \
  --reuse-values \
  -n efe
kubectl rollout status -n efe deploy/efe-prod
```

### Option B — `kubectl rollout restart` (no Helm changes)

```bash
kubectl rollout restart -n efe deploy/efe-prod
kubectl rollout status -n efe deploy/efe-prod
```

Both options preserve the PDB invariant (`minAvailable: 2` of 3
replicas remain Ready throughout). With headless Service and
gRPC keepalive, no client request is dropped.

---

## Step 3 — Issue tokens with the new key

Update your IDP to sign with the new key. Existing tokens (signed
with the old key) keep working ; new tokens (signed with the new
key) start arriving and are validated successfully.

---

## Step 4 — Wait for old-token expiry

Default token TTL is 1 hour. Wait ≥ 1 h before proceeding to step 5.

If you have unusually long token TTL (multi-day), wait the longer
time, OR forcibly invalidate old tokens via your IDP's revocation
list. EFE itself does not maintain a revocation list ; trust is
delegated to the IDP.

---

## Step 5 — Remove the old key

```bash
# Edit the Secret to remove the old key.
# Then trigger a reload (option A or B from step 2).
```

After this, any token still signed with the old key gets 401.

---

## Step 6 — Verify

```bash
# 1. helm test should pass.
helm test efe-prod -n efe

# 2. No 401 spike in metrics.
# (Prometheus query)
sum(rate(grpc_server_handled_total{grpc_code="Unauthenticated"}[5m]))

# 3. Logs show the new key fingerprint in the boot message.
kubectl logs -n efe -l app.kubernetes.io/name=efe --tail=20 \
    | jq -r 'select(.msg | test("loaded.*HS256")) | .msg'
```

---

## Edge cases

### What if the new key has a different audience?

The `config.auth.jwtAudience` Helm value applies to ALL keys.
If you need to migrate audiences:

1. Add the new key (step 1).
2. `helm upgrade --set config.auth.jwtAudience=…` to broaden
   the accepted audience.
3. Issue tokens with the new audience.
4. After expiry, narrow the audience back via another upgrade
   and remove the old key.

### What if a Pod can't read the Secret?

Symptom: log line `failed to load JWT keys: permission denied`.
Cause: the Secret was patched in-place by `kubectl patch` and the
projected volume failed to refresh (rare but seen on EKS).
Fix: `kubectl rollout restart deploy/efe-prod -n efe`.

---

## See also

* [`troubleshooting.md`](troubleshooting.md) §B1 (sudden 401 storm)
* [`deployment.md`](deployment.md) §4 (initial JWT setup)
