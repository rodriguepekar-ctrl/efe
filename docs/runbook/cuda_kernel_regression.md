# Runbook — CUDA Kernel Regression

> **Sévérité** : critical (P0) si SLO-CUDA-FUSION-ROUND > 12 ms ;
> warning (P1) si entre 10 et 12 ms.
> **Trigger** : alerte `GPU_FUSION_ROUND_SLOW` ou
> `GPU_FUSION_TOTAL_SLOW` ou `GPU_KERNEL_FAILURE_BURST`.
> **Sprint d'origine** : Φ-D D31.
> **Référence architecture** : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md`
> § 6 + § 17.

---

## TL;DR

Un kernel CUDA a régressé en perf ou en stabilité. Sans action,
SLO-C-1 (200 ms p99 GPU) sera violé dans l'heure. 3 causes
typiques :

1. **Deploy récent** : nouveau code introduit un bug perf.
2. **Tile size mauvaise** : env vars `EFE_RUNTIME_GPU_*` invalides
   (= post `efe-server tune`).
3. **Driver/hardware drift** : changement silencieux de la stack.

---

## 1. Triage (≤ 5 min)

### 1.1 Identifier le kernel coupable

```bash
# Top 3 kernels les plus lents
$ curl -s 'http://prometheus:9090/api/v1/query?query=topk(3, efe:gpu_kernel_p99_seconds)' | jq

# Comparaison vs baseline j-1
$ curl -s 'http://prometheus:9090/api/v1/query_range?query=efe:gpu_kernel_p99_seconds&start='$(date -d '24h ago' +%s)'&end='$(date +%s)'&step=300' | jq
```

Output type :
```
{kernel="fused_softmax_entropy", p99=15ms}    ← SLOW (cible 3ms)
{kernel="weighted_mean", p99=3ms}              OK
{kernel="cone_project", p99=3ms}               OK
```

### 1.2 Confirmer le scénario

```bash
# Récent deploy ?
$ kubectl rollout history deployment/efe-server -n efe | head -5

# Tile sizes actifs ?
$ kubectl exec -n efe efe-server-0 -- env | grep EFE_RUNTIME_GPU_

# Driver / kernels SHA actuel ?
$ kubectl exec -n efe efe-server-0 -- /efe/bin/efe-server fingerprint | jq .gpu
```

---

## 2. Mitigation scénario 1 — Bad deploy

### 2.1 Rollback

```bash
# Identifier la dernière révision GREEN
$ kubectl rollout history deployment/efe-server -n efe

# Rollback
$ kubectl rollout undo deployment/efe-server -n efe
$ kubectl rollout status deployment/efe-server -n efe --timeout=120s
```

SLO doit reviendr vert dans 5 min post-rollback.

### 2.2 Investigation post-rollback

```bash
# Identifier le commit coupable
$ git log --since='2 hours ago' -- crates/efe-kernels/
```

Tag le commit comme `bad-kernel-regression` ; ouvrir Jira ticket
investigation perf.

---

## 3. Mitigation scénario 2 — Bad tile sizes

### 3.1 Re-tune

```bash
# Sur 1 replica isolé
$ kubectl exec -n efe efe-server-0 -- /efe/bin/efe-server tune \
    --gpu-detect-auto \
    --benchmark-iterations 1000 \
    > /tmp/new-tune.env

# Apply via configmap
$ kubectl create configmap -n efe efe-gpu-tune --from-file=/tmp/new-tune.env --dry-run=client -o yaml | kubectl apply -f -
$ kubectl rollout restart deployment/efe-server -n efe
```

### 3.2 Validation

Dashboard `efe-gpu-kernels` panel #5 (p99 par kernel) doit montrer
tous les 12 kernels sous leur cible canonique.

---

## 4. Mitigation scénario 3 — Driver/hardware drift

Le `fingerprint` rapporté ne matche pas le baseline signé.

```bash
$ ./target/release/efe-attest verify-fingerprint \
    --baseline bench_baselines/v0.6.0/cuda_slos.yaml \
    --fingerprint <(kubectl exec -n efe efe-server-0 -- /efe/bin/efe-server fingerprint)

❌ FINGERPRINT MISMATCH
  expected: driver=535.104.05 cuda=12.4
  observed: driver=540.x cuda=12.5
```

Action : voir `docs/runbook/gpu_driver_upgrade.md` pour la
procédure rolling restart + signed baseline.

---

## 5. Validation post-mitigation

Quel que soit le scénario, vérifier :

- [ ] `GPU_FUSION_ROUND_SLOW` alerte resolved
- [ ] `efe:gpu_fusion_round_p99_seconds * 1000` < 10 ms sur 15 min
- [ ] Aucun nouveau `GPU_KERNEL_FAILURE_BURST`
- [ ] p99 latence Mode-1 (= dashboard P1 panel #1) < 200 ms
- [ ] Status page mis à jour

---

## 6. Post-incident

1. **RCA** : poster sur status.efe.ai dans les 4 h
2. **Post-mortem interne** : template P2, focusé sur :
   - Pourquoi le bench CI n'a pas catché la régression ?
   - Le numerical equivalence harness aurait-il détecté ?
   - Améliorations gating CI suggérées.
3. **Update bench baseline** si la régression était un faux positif
   (= GPU plus rapide qu'avant, cibles sous-estimées).
4. **Chaos test** : ajouter scénario "inject 5× latence kernel X"
   pour valider le rollback automatique.

---

## 7. Escalade

| Durée incident | Décision |
|----------------|----------|
| < 15 min | SRE on-call (= rollback typique) |
| 15-60 min | Engager tech lead Φ-D + CUDA expert |
| > 60 min | Activer cellule de crise ; rollback bundle entier |

---

## 8. Liens utiles

- Architecture : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 6 + § 17
- Tune CLI : `efe-server tune --help`
- Numerical equiv harness : `crates/efe-kernels/tests/numerical_equivalence.rs`
- NSight profiles : https://efe-bench-store.s3.amazonaws.com/nsight/
