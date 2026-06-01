# EFE — SLO & Error Budget

> Version : 1.0 (MVP). Ces SLO sont **contractuels** avec l'équipe
> produit et servent de signal pour geler les features en cas de
> dépassement.

## 1. SLI / SLO en vigueur

| SLI                       | Formule PromQL                                                                                            | SLO cible MVP | Fenêtre |
|---------------------------|-----------------------------------------------------------------------------------------------------------|---------------|---------|
| Disponibilité             | `1 - (sum(rate(efe_grpc_requests_total{code!="OK"}[5m])) / sum(rate(efe_grpc_requests_total[5m])))`       | ≥ 99,5 %      | 30 j    |
| Latence p95 (Execute)     | `histogram_quantile(0.95, sum(rate(efe_grpc_request_duration_seconds_bucket{method="/efe.v1.SwarmService/Execute"}[5m])) by (le))` | ≤ 900 ms      | 30 j    |
| Taux de convergence       | `sum(rate(efe_swarm_duration_seconds_count{verdict="converged"}[1h])) / sum(rate(efe_swarm_duration_seconds_count[1h]))` | ≥ 92 %        | 30 j    |
| Fuite PII                 | événement manuel / fuzz test                                                                              | **= 0**       | toujours |
| Pertes d'audit            | `increase(kubelet_volume_stats_write_errors_total{persistentvolumeclaim=~"efe-.*-audit"}[7d])`            | = 0           | 7 j     |

## 2. Error budget

SLO disponibilité de 99,5 % sur 30 jours ⇒ **3,6 h d'indisponibilité
tolérées** par période glissante.

### Politique de consommation

| Consommation du budget (sur 30 j) | Action                                  |
|-----------------------------------|-----------------------------------------|
| < 50 %                            | Normal — vélocité feature standard      |
| 50 % – 100 %                      | Gel partiel : pas de nouvelle feature ; bug-fixes uniquement |
| ≥ 100 %                           | Gel total + post-mortem obligatoire + priorisation remédiation |

### Calcul PromQL

```
(
  1 -
  sum_over_time(
    (1 - (
      sum(rate(efe_grpc_requests_total{code!="OK"}[1h]))
      /
      sum(rate(efe_grpc_requests_total[1h]))
    ))[30d:1h]
  ) / 720    # 30 j × 24 h
)
```

## 3. Règles d'alerte associées

Voir [`deploy/helm/efe/templates/prometheusrule.yaml`](../deploy/helm/efe/templates/prometheusrule.yaml) pour la définition exacte. Résumé :

| Alerte                   | Seuil (prod) | Sévérité | Runbook                       |
|--------------------------|--------------|----------|-------------------------------|
| `EFEHighErrorRate`       | 0,5 % / 10 min | warning  | `#high-error-rate`          |
| `EFEHighLatencyP95`      | 0,9 s / 10 min | warning  | `#high-latency`             |
| `EFEPodRestartLoop`      | 0,1 r/s / 5 min | critical | `#pod-restart-loop`         |
| `EFEConfidenceDrop`      | p50 < 0,85    | warning  | `#confidence-drop`          |
| `EFERateLimitStorm`      | 50 drops/s    | warning  | `#rate-limit-storm`         |
| `EFEPolicyRejectionSpike`| 10 rej/s      | warning  | `#policy-rejection-spike`   |
| `EFEAuditBackpressure`   | PVC < 10 % libre | warning  | `#audit-backpressure`       |

## 4. SLO bench (Sprint D1, v0.1.1)

À côté des SLO **service-level** ci-dessus (mesurés sur la prod via
Prometheus), Sprint D1 introduit des SLO **bench-level** mesurés à
chaque PR / push sur `main` par `.github/workflows/bench.yml`. Ils
gardent la signature de performance du chemin critique et bloquent
toute régression > 10 % p95 (cf. § 5).

| ID         | Nom                                  | Cible (M3 Pro, naïf) | Bench key                         |
|------------|--------------------------------------|---------------------:|-----------------------------------|
| `SLO-RD1-1` | Latence p95 small (réel)            | ≤ 400 ms            | `real_e2e_latency_naive::small`   |
| `SLO-RD1-2` | Latence p95 medium (réel)           | ≤ 700 ms            | `real_e2e_latency_naive::medium`  |
| `SLO-RD1-3` | Latence p95 large (réel)            | ≤ 700 ms            | `real_e2e_latency_naive::large`   |
| `SLO-RD1-1B` | Latence p95 small (réel, BLAS)     | ≤ 400 ms            | `real_e2e_latency_blas::small`    |
| `SLO-RD1-2B` | Latence p95 medium (réel, BLAS)    | ≤ 500 ms            | `real_e2e_latency_blas::medium`   |
| `SLO-RD1-3B` | Latence p95 large (réel, BLAS)     | ≤ 500 ms            | `real_e2e_latency_blas::large`    |
| `SLO-RD1-T1` | Throughput 8c-small (réel, naïf)   | ≤ 700 ms / batch    | `real_e2e_throughput_naive::c8`   |

> **Note de calibration (audit Sprint D5+)** : les targets initiaux
> v0.1.1 (5 / 50 / 250 ms) supposaient un single-round happy path.
> En réalité les fixtures v2 convergent en 8 multi-rounds, mesurés
> à ~170 / ~340 / ~340 ms sur M3 Pro. Targets recalibrés à ~×2 la
> mesure ; cohérent avec D4 (où D4 = D1 chain + ~600 µs RPC overhead).

Sur runners CI ubuntu-latest (~1.5× plus lents que la référence
M3 Pro), `bench.yml` applique automatiquement un coefficient `1.5`
aux cibles. Voir
[`docs/bench/d1_real_strategies_architecture.md`](bench/d1_real_strategies_architecture.md)
pour la calibration détaillée.

### Anti-régression

Avant que les SLO bench ne soient évalués, les **tests d'intégration**
`crates/efe-server/tests/bench_realism_check.rs` doivent passer. Ils
prouvent que le bench mesure le vrai chemin critique :

* `rounds_executed >= 2` (consensus n'a pas court-circuité).
* `output = Raw(centroid_dim_latent)` non-nul.
* 20 requêtes successives — confidence stable (pas de pollution
  inter-requête).
* Fixtures v2 : norme d'embedding ∈ ]0.5, 10.0[.

Ces tests roulent dans `cargo test -p efe-server` standard ; ils ne
nécessitent aucune feature spéciale.

## 5. SLO bench infra mémoire (Sprint D2, v0.1.2)

Sprint D2 ajoute 10 SLO bench-level pour la couche
`efe_core::memory::{MemoryPool, Arena, HostBackend}`. Mesurés à
chaque PR / push sur `main` par `.github/workflows/bench.yml` ; ils
gardent la **borne inférieure** de toute requête EFE.

### 5.1 Pool acquire

| ID | Nom | Cible (M3 Pro) | Bench key |
|----|-----|---------------:|-----------|
| `SLO-D2-1` | Pool acquire warm | ≤ 1 µs | `pool_acquire_warm::default` |
| `SLO-D2-2` | Pool acquire cold (1 MiB) | ≤ 1 ms | `pool_acquire_cold::1MiB` |
| `SLO-D2-3` | Pool concurrent (8c) | ≤ 50 µs | `pool_concurrent_acquire::c8` |

### 5.2 Arena allocation

| ID | Nom | Cible | Bench key |
|----|-----|------:|-----------|
| `SLO-D2-4` | Alloc f32(1) | ≤ 200 ns | `arena_alloc_zeroed_typed::f32/1` |
| `SLO-D2-5` | Alloc f32(1k) | ≤ 2 µs | `arena_alloc_zeroed_typed::f32/1024` |
| `SLO-D2-6` | Alloc f32(1M) | ≤ 2 ms | `arena_alloc_zeroed_typed::f32/1000000` |
| `SLO-D2-7` | Arena reset | ≤ 50 ns | `arena_reset::default` |

### 5.3 Pattern réel (Mvp swarm)

| ID | Nom | Cible | Bench key |
|----|-----|------:|-----------|
| `SLO-D2-8` | Pattern small | ≤ 5 µs | `realistic_swarm_alloc_pattern::small` |
| `SLO-D2-9` | Pattern medium | ≤ 30 µs | `realistic_swarm_alloc_pattern::medium` |
| `SLO-D2-10` | Pattern large | ≤ 150 µs | `realistic_swarm_alloc_pattern::large` |

### Anti-régression

Avant que les SLO D2 ne soient évalués, les **4 tests** dans
`crates/efe-core/tests/pool_realism_check.rs` doivent passer (loop
1 000 cycles sans fuite, cursor exact post-pattern, saturation
8 tasks/2 slots ≤ 2 allocations, drop pool libère tout backend).

Calibration : Apple M3 Pro / 11 cores / no-BLAS / 1 MiB bench arena.
CI ubuntu-latest applique le coefficient ×1.5 automatiquement.

## 6. SLO bench compute pipeline (Sprint D3, v0.1.3)

Sprint D3 ajoute 6 SLO bench-level pour la **chaîne compute** que
`MvpCpuFusion::run_round_sync` exécute. Mesurés à chaque PR / push
sur `main` par `.github/workflows/bench.yml`. Ils gardent le coût
arithmétique pur, hors orchestration (D1) et hors mémoire (D2).

### 6.1 Cibles naïve (default build)

| ID | Nom | Cible (M3 Pro) | Bench key |
|----|-----|---------------:|-----------|
| `SLO-D3-1` | Chain small (naïve) | ≤ 200 µs | `compute_pipeline_chain_naive::small` |
| `SLO-D3-2` | Chain medium (naïve) | ≤ 500 µs | `compute_pipeline_chain_naive::medium` |
| `SLO-D3-3` | Chain large (naïve) | ≤ 20 ms | `compute_pipeline_chain_naive::large` |

### 6.2 Cibles BLAS (`--features accelerate` / `--features openblas`)

| ID | Nom | Cible (M3 Pro) | Bench key |
|----|-----|---------------:|-----------|
| `SLO-D3-1B` | Chain small (BLAS) | ≤ 200 µs | `compute_pipeline_chain_blas::small` |
| `SLO-D3-2B` | Chain medium (BLAS) | ≤ 250 µs | `compute_pipeline_chain_blas::medium` |
| `SLO-D3-3B` | Chain large (BLAS) | ≤ 5 ms | `compute_pipeline_chain_blas::large` |

**Note** : sur la fixture *small* (M·K·N = 8 192), le seuil
de dispatch BLAS (`32 768`) n'est pas franchi → `_blas` et `_naive`
mesurent la même chose. La cible BLAS small est égale à la naïve par
construction.

### Per-phase et throughput

Les groupes `compute_pipeline_per_phase` et
`compute_pipeline_throughput` sont mesurés et publiés mais **ne
gate pas la PR** — leur valeur est diagnostique (attribution per-op
+ courbe de scaling). Ils valident l'**hypothèse d'additivité**
chain ≈ Σ phases (post-merge sur M3 Pro : 95-99 %).

### Anti-régression

Avant que les SLO D3 ne soient évalués, les **3 tests** dans
`crates/efe-core/tests/compute_pipeline_realism.rs` doivent passer
(outputs finis, shapes correctes, attention rows sum to 1).

Calibration : Apple M3 Pro / 11 cores / no-BLAS / v2 fixture shapes.
CI ubuntu-latest applique le coefficient ×1.5 automatiquement.

## 7. SLO bench gRPC server stack (Sprint D4, v0.1.4)

Sprint D4 ajoute 4 SLO bench-level pour la **pile gRPC** d'efe-server
(loopback TCP + tonic codec + interceptors + bridge + backend).
Mesurés à chaque PR / push sur `main` par
`.github/workflows/bench.yml`. Ils gardent le coût RPC qu'un client
externe paie réellement.

### 7.1 Cibles full stack (auth disabled, ratelimit ON, idempotency ON)

| ID | Nom | Cible (M3 Pro) | Bench key |
|----|-----|---------------:|-----------|
| `SLO-D4-1` | RPC full small | ≤ 500 ms | `server_rpc_full_stack::small` |
| `SLO-D4-2` | RPC full medium | ≤ 800 ms | `server_rpc_full_stack::medium` |
| `SLO-D4-3` | RPC full large | ≤ 800 ms | `server_rpc_full_stack::large` |
| `SLO-D4-T1` | RPC concurrent 8c-small (minimal stack) | ≤ 800 ms / batch | `server_rpc_concurrent::c8` |

**Lecture** : les cibles sont à ×2-3 de la mesure M3 Pro. Marge
généreuse parce que (a) la variance loopback est ~10 %, (b) une
régression réelle (interceptor cassé, tonic bump non compatible)
explose facilement ces caps.

### 7.2 Pas de SLO sur minimal_stack ni codec_only

Ces deux groupes sont mesurés mais **non gatés** :

* `server_rpc_minimal_stack` sert de baseline pour calculer
  `Δ_interceptors = full − minimal`. La métrique est publiée dans
  `BENCHMARKS.md` § 3 quater.
* `server_rpc_codec_only` est un micro-bench du codec proto (pas
  de transport, pas de serveur). Documenté pour donner la borne
  inférieure du chemin RPC.

### Anti-régression

Avant que les SLO D4 ne soient évalués, les **3 tests** dans
`crates/efe-server/tests/server_rpc_realism_check.rs` doivent
passer (RPC OK sur les 3 fixtures, idempotency cache hit < 1/5
du miss, concurrent < 1/2 du serial).

Calibration : Apple M3 Pro / 11 cores / loopback TCP / no-TLS /
no-auth / v2 fixture shapes. CI ubuntu-latest applique le
coefficient ×1.5 automatiquement.

## 8. SLO meta-swarm (PR #19 — chart 0.6.0+)

PR #19 introduit 4 SLO dédiés au plan **L2 meta-swarm** (controller +
participants). Distinct des SLO L1 EFE-SwarmService listés au § 1, ils
couvrent le chemin `Iterate` end-to-end + le pipeline OTel + le reload
de manifeste. Targets committed dans STABILITY_CONTRACT v1.1 § 9.5 ;
relâcher une target est un contract break.

| SLO id | Target | Fenêtre | Numérateur PromQL | Dénominateur PromQL |
|---|---|---|---|---|
| `iterate-availability`         | **≥ 99,9 %**         | 30 j | `sum(rate(efe_meta_swarm_iterate_total{result="ok"}[30d]))` | `sum(rate(efe_meta_swarm_iterate_total[30d]))` |
| `iterate-latency-p95`          | **95 % ≤ 1 s**       | 30 j | `sum(rate(efe_meta_swarm_iterate_duration_seconds_bucket{le="1.0"}[30d]))` | `sum(rate(efe_meta_swarm_iterate_duration_seconds_count[30d]))` |
| `manifest-reload-latency-p95`  | **95 % ≤ 500 ms**    | 30 j | `sum(rate(efe_meta_swarm_manifest_reload_duration_seconds_bucket{le="0.5"}[30d]))` | `sum(rate(efe_meta_swarm_manifest_reload_duration_seconds_count[30d]))` |
| `otel-drop-rate`               | **≤ 1 % dropped**    | 30 j | `sum(rate(efe_meta_swarm_otel_exported_spans_total[30d]))` | `sum(rate(efe_meta_swarm_otel_exported_spans_total[30d])) + sum(rate(efe_meta_swarm_otel_dropped_spans_total[30d]))` |

### Burn-rate alerts (Google SRE 2-window pattern)

Le SLO `iterate-availability` est gardé par deux alertes burn-rate
(architecture PR #19 § 30/31) :

| Alerte | Multiplicateur | Fenêtre courte | Fenêtre longue | Exhaustion budget |
|---|---|---|---|---|
| `SLOBudgetBurnFast14x` (paging) | 14,4× | 5 m | 1 h | ~ 2 jours |
| `SLOBudgetBurnSlow6x` (ticket)  | 6×    | 1 h | 6 h | ~ 5 jours |

Les deux fenêtres doivent **simultanément** dépasser le seuil pour que
l'alerte se déclenche — suppression des blips ponctuels.

### Politique de consommation budget meta-swarm

| Budget consommé / 30 j | Action |
|---|---|
| < 50 % | Vélocité produit normale |
| 50 – 100 % | Gel feature ; bug-fixes uniquement ; review SRE hebdomadaire |
| ≥ 100 % | Gel total + postmortem obligatoire + actions correctives prioritaires |

### Dashboards SLO

Les SLO meta-swarm sont surface visibles sur :
- **D1 Overview** P5 (gauge budget restant) + P9/P10 (burn-rate timeseries)
- **D2 Iterate** P1 (latence par manifest_version pour SLO 2)
- **D4 Reload** P5 (p95 reload pour SLO 3)
- **D5 OTel** P3 (drop ratio pour SLO 4)

Les runbooks `docs/runbooks/RB_PR19_MSW{9,10}.md` détaillent la
diagnostique + mitigation du burn.

## 9. Révision

Les SLO sont revus trimestriellement lors de la revue plateforme. La
proposition d'évolution passe par une RFC dans le repo sous `docs/rfcs/`.
Toute proposition de **relâchement** (baisse de target) d'un SLO listé
dans STABILITY_CONTRACT v1.1 § 9.5 est un *contract break* — chart major
bump + 6-minor-version deprecation notice obligatoires.
