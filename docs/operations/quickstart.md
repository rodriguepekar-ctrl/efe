# EFE — Quickstart (10 minutes) — Operations variant

> **Audience** : a partner SRE / platform engineer.
> If you're an application developer wanting to USE the SDK,
> see the [partner quickstart at docs.efe.ai/tutorial/quickstart](../docs/tutorial/quickstart.md)
> instead — that one is shorter and assumes the cluster is set up.
>
> Sprint D9 + D10. Updated post-D10 to align with the published
> SDK on PyPI (`pip install efe-tool==0.1.10`).

---

## What you will do

1. Install the EFE Helm chart on a fresh cluster.
2. Wait for the Pods to become ready.
3. `pip install efe-tool` and run the bundled example.
4. Confirm three intent labels match.

If all four succeed, EFE is operational on your cluster.

> **Automated validation**: run `scripts/validate_quickstart.sh`
> to perform all 4 steps and assert wallclock ≤ 10 min. The script
> is the canonical implementation of SLO `D10-DX-1`.

---

## Prerequisites (≤ 30 s to check)

| Tool | Min version | Check |
|------|------------:|-------|
| `helm` | 3.13 | `helm version --short` |
| `kubectl` | 1.27 | `kubectl version --short` |
| `python3` | 3.10 | `python3 --version` |
| Cluster | k8s ≥ 1.27 | `kubectl get nodes` |

If you don't have a cluster, [`minikube start --cpus=4 --memory=8g`](https://minikube.sigs.k8s.io/docs/start/)
gives you a working environment in ~2 min.

---

## Step 1 — Install the chart (≈ 2 min)

```bash
# Pull the chart from GHCR (OCI registry).
helm install efe oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.9 \
  --namespace efe \
  --create-namespace \
  --set replicaCount=2 \
  --set service.headless=true \
  --set autoscaling.minReplicas=2 \
  --set autoscaling.maxReplicas=4

# Why these overrides?
#   replicaCount=2          : preStop drain budget (cf. supplement §5.5)
#   service.headless=true   : client-side gRPC LB (D9.A5)
#   autoscaling.{min,max}   : keep cost bounded for a pilot
```

Expected output (last 3 lines):

```
NAME: efe
LAST DEPLOYED: Mon Apr 27 14:20:00 2026
STATUS: deployed
```

---

## Step 2 — Wait for ready (≈ 30 s)

```bash
kubectl wait deployment/efe \
  --for=condition=available \
  --timeout=120s \
  -n efe
```

If this fails, jump to [`troubleshooting.md`](troubleshooting.md).

---

## Step 3 — Run the bundled smoke test (≈ 10 s)

```bash
helm test efe -n efe
```

This runs `grpc_health_probe` against the in-cluster Service.
Expected: `Phase: Succeeded`.

---

## Step 4 — Three semantic queries (≈ 1 min)

### 4a — Port-forward

```bash
kubectl port-forward -n efe svc/efe 50051:50051 &
PF_PID=$!
trap 'kill $PF_PID' EXIT
```

### 4b — Install the Python SDK locally

```bash
pip install --quiet efe-tool
# OR pin to the version that ships with this chart:
pip install --quiet 'efe-tool==0.1.9'
```

### 4c — Run three queries

Save as `smoke.py`:

```python
from efe_tool.client import EfeClient
from efe_tool.integrations.hybrid_decoder import HybridClassifier

cli = EfeClient(host="127.0.0.1:50051", insecure=True)
decoder = HybridClassifier(client=cli)

cases = [
    "Someone charged my card without my consent.",
    "Where do I find a record of my past invoices?",
    "L'application plante quand j'ouvre le menu",
]
for text in cases:
    r = decoder.predict(text)
    print(f"{text!r:60s} → label={r.label!r}, conf={r.confidence:.2f}")
```

```bash
python3 smoke.py
```

Expected output:

```
'Someone charged my card without my consent.'                → label='fraud',    conf=0.92
'Where do I find a record of my past invoices?'              → label='question', conf=0.88
"L'application plante quand j'ouvre le menu"                 → label='bug',      conf=0.91
```

If you see all three labels, **EFE is operational**.

---

## What you got

| Resource | Where |
|----------|-------|
| `Deployment/efe` | 2 replicas, distroless cc-debian12 nonroot UID 10001 |
| `Service/efe` | gRPC on port 50051 (headless), metrics on 9090 |
| `HPA/efe` | targetCPU=70 %, min=2, max=4 |
| `PodDisruptionBudget/efe` | minAvailable=1 |
| `NetworkPolicy/efe` | ingress restricted to known clients |
| `ServiceMonitor/efe` | Prometheus operator scrapes `/metrics` every 15 s |
| `PrometheusRule/efe` | alert thresholds |

---

## Next steps

* **Production deployment** : see [`deployment.md`](deployment.md).
* **Run a load test** : see [`load_test.md`](load_test.md).
* **Rotate JWT keys** : see [`key_rotation.md`](key_rotation.md).
* **Things go wrong** : see [`troubleshooting.md`](troubleshooting.md).
* **Chaos / failure scenarios** : see [`chaos_runbook.md`](chaos_runbook.md).

---

## Uninstall

```bash
helm uninstall efe -n efe
kubectl delete namespace efe
```

That's it. No PVCs, no orphan secrets, no hanging finalizers — the
chart is clean-uninstall by design.
