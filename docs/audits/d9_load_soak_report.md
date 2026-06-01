# Sprint D9 — Load & soak audit report (skeleton)

> **Statut** : skeleton, à compléter dès la première mesure live
> reproductible sur un cluster post-merge D9 stable. Suit
> `d6_accuracy_validation.md` et `d8_decoder_comparison.md` dans la
> série des rapports d'audit chiffrés.

---

## 1. Contexte

D9 livre l'industrialisation Docker + Helm + load/soak (cf. spec
`docs/bench/d9_docker_helm_load_architecture.md` + supplement
`d9_deep_analysis_supplement.md`). Cette section vide se remplit
quand :

1. Le binaire `efe-server` v0.1.9 est buildé et déployé via Helm
   sur un cluster minikube (CI ou local).
2. Les 4 scénarios ghz (`smoke` / `sustained` / `stress` / `spike`)
   sont exécutés.
3. Le soak 24 h tourne au moins une fois.

> **À la première mesure live** : remplacer ce skeleton par les
> chiffres réels et les commentaires d'analyse (cf. `d8_decoder_comparison.md`
> pour le format).

---

## 2. Mesure live D9.1 — Docker image

| Aspect | Cible | Mesuré | Statut |
|--------|------:|-------:|:------:|
| Taille image (linux/amd64, compressée) | < 50 MiB | TBD | ⏸ |
| Démarrage `gpc_health_probe SERVING` | < 10 s | TBD | ⏸ |
| Distroless symbols compatibility | 100 % | TBD (script `check_distroless_symbols.py`) | ⏸ |
| Cosign verify | OK | TBD | ⏸ |
| Trivy CRITICAL/HIGH | 0 | TBD | ⏸ |

---

## 3. Mesure live D9.2 — Helm chart

| Vérif | Statut |
|-------|:------:|
| `helm lint deploy/helm/efe/` | ⏸ |
| `helm template` rend ≥ 11 manifestes | ⏸ |
| `helm install` minikube k8s 1.27 | ⏸ |
| `helm test efe -n efe` (grpc-health) | ⏸ |
| `helm test efe -n efe` (semantic, opt-in) | ⏸ |
| `helm uninstall` clean | ⏸ |

---

## 4. Mesure live D9.3 — ghz scenarios (résultats vivants à remplir)

### 4.1 Scénario `smoke`

| Métrique | Mesuré |
|----------|-------:|
| count | TBD |
| p50_ms | TBD |
| p99_ms | TBD |
| error_ratio | TBD |
| throughput_rps | TBD |

### 4.2 Scénario `sustained` (5 min, c=4)

| Métrique | Mesuré | SLO |
|----------|-------:|-----|
| count | TBD | — |
| p50_ms | TBD | — |
| p99_ms | TBD | `D9-LOAD-1` ≤ 5 000 ms |
| error_ratio | TBD | `D9-LOAD-2` ≤ 1 % |
| throughput_rps | TBD | — (informational) |

### 4.3 Scénario `stress` (2 min, c=16)

| Métrique | Mesuré | SLO |
|----------|-------:|-----|
| throughput_rps | TBD | `D9-LOAD-3` ≥ 5 RPS |
| error_ratio | TBD | — |

### 4.4 Scénario `spike` (90 s, 1→16 step)

| Métrique | Mesuré | SLO |
|----------|-------:|-----|
| recovery_seconds | TBD | `D9-LOAD-4` ≤ 30 s |

---

## 5. Mesure live D9.4 — Soak 24 h

```
Loaded N samples → after warmup (t≥1800s) → after smoothing (window=10) → fit OLS

β̂₁ (slope)        : TBD B/s
SE(β̂₁)            : TBD B/s
t-statistic       : TBD
threshold         : 870 B/s
status            : TBD
```

| SLO | Mesuré | Cible | Statut |
|-----|-------:|------:|:------:|
| `D9-SOAK-1` | TBD B/s | ≤ 870 | ⏸ |

---

## 6. Anti-régression — toutes les SLOs antérieures

À remplir post-mesure : aucune des 16 SLOs antérieures (D6 / D7 / D8) ne doit régresser.

---

## 7. Décisions de gel D9

À la freeze de la baseline `v0.1.9` :

```
Image                     : ghcr.io/efe-project/efe@sha256:TBD
Image tag                 : v0.1.9
Helm chart                : oci://ghcr.io/efe-project/charts/efe:0.1.9
Distroless base           : gcr.io/distroless/cc-debian12:nonroot
SOURCE_DATE_EPOCH         : (post-merge commit time)
ghz version               : 0.117.0
```

Carry-over D8 (winner) :

```
EFE_FUSION_BETA0          = 0.45
EFE_FUSION_LAMBDA_BETA    = 0.05
EFE_FUSION_ALPHA0         = 0.30
HYBRID_AGREE_MIN_CONF     = 0.45
HYBRID_SWARM_OVERRIDE_CONF= 0.85
TEMPLATE_REGISTRY_KIND    = Default
```

D9 nouveau :

```
service.headless          = false        # opt-in (pilot recommended : true)
tenant.id                 = "default"
tests.semanticSmoke       = false        # opt-in
EFE_IDEMPOTENCY_TTL_MS    = (env)        # 0 in load tests, default in prod
```

---

## 8. Next

Une fois ce rapport rempli :

1. Geler `bench_baselines/v0.1.9/baseline.json` avec les chiffres mesurés.
2. PR séparée pour flip `bench-load.continue-on-error` → `false`.
3. Annoncer `v0.1.9` (tag + GitHub Release).
4. Mettre à jour `BENCHMARKS.md` § 11 avec les valeurs finales (en gardant l'invariant : tableau au-dessus de la baseline JSON, jamais l'inverse).

— **fin du skeleton** —
