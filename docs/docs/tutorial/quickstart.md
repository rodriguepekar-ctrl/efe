# Quickstart

> **10 minutes from zero to first classification.**
> Audience: a partner engineer with Python 3.10+, helm, kubectl,
> and a k8s cluster (minikube works).

---

## What you will achieve

By the end of this page you will:

1. Have an `efe-server` running in your cluster.
2. Have `efe-tool` installed in a Python venv.
3. Have classified five mixed-language support tickets.

If anything goes wrong, jump to
[troubleshooting](../../operations/troubleshooting.md).

---

## Pre-flight (30 s)

```bash
python3 --version    # ≥ 3.10
helm version --short # ≥ 3.13
kubectl get nodes    # at least one node Ready
```

---

## Step 1 — install EFE (2 min)

```bash
helm install efe oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.10 \
  --namespace efe --create-namespace \
  --set replicaCount=2 \
  --set service.headless=true \
  --set autoscaling.minReplicas=2

kubectl wait deploy/efe -n efe \
  --for=condition=available --timeout=120s
```

The `--set service.headless=true` enables client-side gRPC
load-balancing across replicas — required as soon as
`replicaCount > 1`. See
[Operations / Deployment](../../operations/deployment.md) for the
production-grade values file.

---

## Step 2 — install the SDK (30 s)

```bash
pip install efe-tool==0.1.10
```

---

## Step 3 — port-forward (10 s)

```bash
kubectl port-forward -n efe svc/efe 50051:50051 &
```

(Background process; remember to `kill %1` when done.)

---

## Step 4 — first classification (30 s)

The reference snippet is `examples/01_quickstart_classify.py` in
the repo. The imports:

```python
--8<-- "examples/01_quickstart_classify.py:imports"
```

The classification loop:

```python
--8<-- "examples/01_quickstart_classify.py:classify-loop"
```

Run it:

```bash
python examples/01_quickstart_classify.py
```

Expected output (substrings — actual confidence varies slightly
run-to-run due to fusion stochasticity):

```
Someone charged my card without my consent.            → fraud      (0.92)
Where do I find a record of my past invoices?          → question   (0.88)
L'application plante quand j'ouvre le menu paramètres. → bug        (0.91)
How do I reset my two-factor authentication?           → question   (0.86)
I was charged twice for the same order.                → billing    (0.93)
```

If you see five labels printed, **EFE is running on your cluster
and you've made your first classification.** 🎉

---

## What just happened?

* The SDK loaded `catalogs/support_triage.yaml` and embedded its
  anchor prompts via the local sentence-transformers model.
* `client.classify(text, catalog)` sent an `ExecuteRequest` over
  gRPC.
* The server ran a 4–7 round Mode-1 swarm fusion on the embedded
  text against your catalog's anchor cone.
* The result projected onto the cone — the nearest anchor's label
  + confidence is what you see.

Read [Concepts](concepts.md) for the design rationale (why
anchors, what is "ephemeral fluid swarm", hybrid Plan B).

---

## Next steps

* **[Your first classifier](your-first-classifier.md)** — define
  your own intents instead of `support_triage`.
* **[How-to: custom catalog](../howto/custom-catalog.md)** —
  the canonical YAML schema.
* **[How-to: production wiring](../howto/retries.md)** — JWT,
  retries, structured logging.
* **[Reference: API](../reference/api.md)** — full SDK API.

---

## If something went wrong

| Symptom | Likely cause | See |
|---------|--------------|-----|
| `helm install` fails with "already exists" | leftover from a previous run | `helm uninstall efe -n efe` first |
| `kubectl wait` times out at 120 s | image pull blocked | [troubleshooting A1](../../operations/troubleshooting.md#a1-helm-install-succeeds-but-pods-stuck-containercreating) |
| `pip install efe-tool` fails | corp proxy blocking PyPI | [How-to: install](../howto/install.md) |
| `connection refused` on port 50051 | port-forward died | re-run step 3 |
| Empty output / `UNAVAILABLE` | server not ready, retry once | [troubleshooting A3](../../operations/troubleshooting.md#a3-helm-test-efe-fails-with-connection-timeout) |

Stuck? File a bug at
[github.com/efe-ai/efe/issues](https://github.com/efe-ai/efe/issues)
with `kubectl describe pod -n efe -l app.kubernetes.io/name=efe`
output.
