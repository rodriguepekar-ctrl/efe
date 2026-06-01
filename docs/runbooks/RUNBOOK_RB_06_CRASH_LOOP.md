# RB-06 — efe-server CrashLoopBackOff

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P1** |
| MTTR cible | 30 min |
| Fréquence attendue | 1-2 / mois lors de releases |
| Last reviewed | 2026-05-09 |
| Owner primaire | SRE |
| Owner backup | Backend Lead |

## Symptômes

- Alerte Prometheus : `EfeServerCrashLoopBackOff`
- `kubectl get pods` : `1/1 Running` mais `RESTARTS` croissant
- p99 latency dégradée (HPA scale-up déclenché)

## Diagnostic

### 1. État pods

```bash
kubectl get pods -n efe -l app.kubernetes.io/name=efe-server -o wide
```

### 2. Logs du dernier crash

```bash
POD=$(kubectl get pods -n efe -l app.kubernetes.io/name=efe-server \
  --sort-by=.status.startTime -o name | head -1)
kubectl logs -n efe $POD --previous --tail=200
```

### 3. Events Kubernetes

```bash
kubectl describe pod -n efe $POD | tail -50
```

## Procédure

### Étape 1 : Stabiliser

#### A. CrashLoop récent (< 5 min) ET déploiement en cours

```bash
kubectl rollout undo -n efe deploy/efe-server
```

#### B. CrashLoop ancien

```bash
kubectl scale -n efe deploy/efe-server --replicas=0
sleep 30
kubectl scale -n efe deploy/efe-server --replicas=3
```

### Étape 2 : Diagnostiquer

| Symptôme | Cause probable | Solution |
|----------|----------------|----------|
| `OOMKilled` | Memory limit trop bas | Étape 3.A |
| `Liveness probe failed` | Probe timeout / startup lent | Étape 3.B |
| `Init container failed` | Migration DB ratée | Cf. RB-08 (Sprint C) |
| `ImagePullBackOff` | Image manquante / auth registry | Étape 3.C |
| `Config invalide` | ConfigMap mal formé | Étape 3.D |

### Étape 3 : Corriger

#### A. OOM

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --set efe-server.resources.limits.memory=8Gi \
  --reuse-values
```

#### B. Probe timeout

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --set efe-server.startupProbe.failureThreshold=60 \
  --reuse-values
```

#### C. ImagePullBackOff

Vérifier le pull secret :

```bash
kubectl get secret -n efe ghcr-pull-secret -o yaml
kubectl create secret docker-registry ghcr-pull-secret \
  --docker-server=ghcr.io \
  --docker-username=$GHCR_USER \
  --docker-password=$GHCR_TOKEN \
  -n efe \
  --dry-run=client -o yaml | kubectl apply -f -
```

#### D. Config invalide

```bash
kubectl get cm -n efe efe-server -o yaml
# inspect EFE_FEATURE_*_ENABLED values, fix manually if needed
```

### Étape 4 : Vérifier

```bash
kubectl wait -n efe pod -l app.kubernetes.io/name=efe-server \
  --for=condition=Ready --timeout=120s

efe-admin smoke-test acme
```

### Étape 5 : Communiquer

- Statuspage si downtime > 10 min
- Slack `#efe-incidents`

## Post-incident

- Si récurrent : tuner probes, ajouter chaos test régression

## Procédure de validation

Force OOM en staging :

```bash
kubectl set resources -n efe-staging deploy/efe-server \
  --limits=memory=64Mi
```

Doit déclencher CrashLoopBackOff puis l'alerte ≤ 10 min.

## Annexes

- Helm chart : `charts/efe-stack/charts/efe-server/templates/deployment.yaml`
- Dashboard Grafana : `EFE Overview` → panel « Pod restart rate »
- K8s probe docs : https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/
