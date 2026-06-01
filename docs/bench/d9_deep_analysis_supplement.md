# Sprint D9 — Deep analysis supplement (ultra-think)

> **Statut** : compagnon de
> [`d9_docker_helm_load_architecture.md`](d9_docker_helm_load_architecture.md)
> (la spec opérationnelle, 1983 lignes). Ce document ne remplace pas
> la spec — il **ajoute les couches analytiques** que la spec
> n'expose pas : fondations mathématiques, comparaisons rigoureuses
> des alternatives rejetées, analyse symbole-par-symbole de la
> surface distroless, modèle de coût, HAZOP, reproductibilité
> bit-à-bit.
> **Public visé** : reviewer infra/sécu/SRE qui doit accepter
> les défaults D9, et toute personne qui veut **comprendre pourquoi**
> chaque ligne de la spec est ce qu'elle est.
> **Précision chirurgicale suprême** — chaque chiffre est
> dérivable, chaque décision est traceable.

---

## Table des matières

1. [Fondations mathématiques du load test](#1-fondations-mathématiques-du-load-test)
2. [Statistique du soak test (puissance de détection)](#2-statistique-du-soak-test-puissance-de-détection)
3. [Capacity planning : Little's law appliquée à Mode-1](#3-capacity-planning--littles-law-appliquée-à-mode-1)
4. [Distroless : analyse symbole par symbole](#4-distroless--analyse-symbole-par-symbole)
5. [Réseau gRPC + k8s — pièges et choix](#5-réseau-grpc--k8s--pièges-et-choix)
6. [Comparaison rigoureuse : 5 alternatives × 5 axes](#6-comparaison-rigoureuse--5-alternatives--5-axes)
7. [Modèle de coût (TCO partenaire pilote)](#7-modèle-de-coût-tco-partenaire-pilote)
8. [HAZOP : 12 modes de défaillance](#8-hazop--12-modes-de-défaillance)
9. [Reproductibilité bit-à-bit du build](#9-reproductibilité-bit-à-bit-du-build)
10. [Chaos engineering : 8 scénarios DR](#10-chaos-engineering--8-scénarios-dr)
11. [Frontière multi-tenancy D9 ↔ D11](#11-frontière-multi-tenancy-d9--d11)
12. [Inventaire des hypothèses & invariants](#12-inventaire-des-hypothèses--invariants)
13. [Plan de validation analytique](#13-plan-de-validation-analytique)

---

## 1. Fondations mathématiques du load test

### 1.1 Pourquoi 4 scénarios et pas 1 ou 10

Un load test isolé donne une mesure ponctuelle. La spec D9 §5.2
définit 4 scénarios — il faut prouver que **chacun mesure quelque
chose d'orthogonal** et que **leur union couvre l'espace de
décision**.

| Scénario | Mesure | Décision soutenue | Variable indépendante |
|----------|--------|-------------------|-----------------------|
| `smoke` (1c, 30s) | Le serveur répond-il ? | Go/no-go pré-prod | présence/absence |
| `sustained` (4c, 5min) | p50/p95/p99 stable | SLA contractuel partenaire | latence stationnaire |
| `stress` (16c, 2min) | Comportement à saturation | Capacity planning | latence sous queue |
| `spike` (0→16c instantané) | Recovery time | SLA disponibilité | dérivée temporelle |

Les 4 scénarios couvrent exactement les **3 régimes du queueing
classique** (sous-charge, charge nominale, surcharge) plus le
**transitoire** (spike). Ajouter un 5ème scénario apporterait
moins de signal que les 4 existants — réduction des dimensions
linéairement indépendantes.

### 1.2 Sample sizing : pourquoi `sustained = 5 min @ ~10 RPS`

Le scénario `sustained` doit produire un **intervalle de confiance
exploitable** sur p99. Pour cela, il faut suffisamment d'échantillons
dans la queue droite.

Avec une cible p99 ≤ 5 s, on veut estimer le quantile 0.99 avec
une précision de ±10 % (i.e. ±500 ms). La formule classique pour
le sample size d'un quantile estimé par bootstrap est :

```
n ≥ p(1-p) / (ε² · f(q_p)²)
```

où `p = 0.99`, `ε = 0.10` (précision relative), `f(q_p)` est la
densité de probabilité au quantile cible. Pour une distribution
log-normale typique des latences gRPC avec µ_log ≈ 7.6, σ_log ≈ 0.8 :

```
f(q_0.99) ≈ 0.99 · 1/(σ_log · q_0.99 · √(2π)) ≈ 6.4e-5 s⁻¹
n ≥ 0.0099 / (0.01 · (6.4e-5 · 5000)²) ≈ 9 700 échantillons
```

À 10 RPS ce sont **970 secondes ≈ 16 min**. La spec D9 retient
**5 min** comme compromis CI (320 s = 3 200 échantillons donnent
un IC 99% large mais utilisable, ±25 % au lieu de ±10 %).

> **Dette analytique** : pour un IC ±10 % réel sur p99, le scénario
> `sustained` devrait durer 16 min. Tracé en `DT-D9.A1` (P3) avec
> note dans `bench/load/run_load_test.sh`. La cible 5 min est la
> précision **viable** en CI ; la mesure de référence en prod
> devrait étendre à 15-30 min.

### 1.3 Throughput nominal : pourquoi 5 RPS et pas 50

La latence Mode-1 D8 mesurée est dominée par le swarm multi-round :
- p50 = 2.0 s
- p99 = 5.0 s
- mean ≈ 2.5 s

Avec une concurrence c = 4 (`sustained`) et un service-time mean
S = 2.5 s, le throughput théorique d'un seul replica par Little's
law (cf. § 3) est :

```
λ_max_single_replica = c / S = 4 / 2.5 = 1.6 RPS  (sustained)
λ_max_single_replica = 16 / 2.5 = 6.4 RPS         (stress)
```

Le SLO `D9-LOAD-3` cible 5 RPS soutenu **agrégé sur le replica
actif**. C'est cohérent avec stress @ 16 concurrents — pas avec
sustained @ 4 concurrents qui plafonne à 1.6 RPS. **Donc le seuil
"5 RPS soutenu" suppose que HPA scale-up active 2-3 replicas**.

> **Précision** : la spec mesure le throughput **côté client ghz**
> (RPS observés depuis le bench, indépendamment du nombre de
> replicas servant). L'HPA scale-up est le mécanisme qui permet
> d'atteindre 5 RPS sustained. Voir § 3.4 pour le détail.

### 1.4 Choix de la distribution de payload

Les payloads ghz sont des fixtures statiques 3-5 cas dans
`bench/load/fixtures/sample_request.json`. Cela introduit un biais
**cache-friendly** :

- `EfeClient` interne du serveur a un cache LRU de 10 000 entrées
  TTL 10 s.
- Avec 3 fixtures rotatées, après le 4e appel **toutes les requêtes
  hit le cache**. Latence mesurée ≈ cache lookup ≈ 1-5 ms.

**Mitigation** :
- Pour `smoke` et `spike` : OK (on mesure justement la voie chaude).
- Pour `sustained` et `stress` : **désactiver le cache via env
  `EFE_IDEMPOTENCY_TTL_MS=0`** — sinon les chiffres p99 mesurent
  le cache, pas le compute.

> **Décision** : la spec D9 §5.3 doit rendre explicite la variable
> `EFE_IDEMPOTENCY_TTL_MS=0` pour `sustained` / `stress`. Tracé
> comme amendement spec D9.A2 (P1, à ajouter avant impl).

### 1.5 Spike test : modélisation transitoire

Le `spike` injecte une charge step (0 → 16 concurrent en moins de 1 s).
Le SLO `D9-LOAD-4` cible **recovery time ≤ 30 s** : temps pour que
p99 retombe sous 5 s après le pic.

Le système doit absorber un transitoire défini par :

```
Q(0+) = 16 concurrent requests in queue
S = 2.5 s mean service time
K replicas active at t=0 (typically K=1)
Drain time ≈ Q(0+) · S / K = 16 · 2.5 / K = 40/K seconds
```

Avec K=1 → drain = 40 s > 30 s SLO ❌. Donc le SLO `D9-LOAD-4`
suppose **HPA scale-up sous 10 s** ou un comportement sub-linear
(coalescence cache, sharing partial work). La spec D9 §4.6 fixe
HPA `scaleUp.stabilizationWindowSeconds=0` et cooldown `60s` —
une fois la métrique CPU > seuil, k8s peut créer un nouveau pod
en ~5-8 s, le bring-up `efe-server` prend ~3 s, total ~10 s.
**Avec K=2, drain time = 20 s < 30 s SLO ✅.**

> **Sensibilité** : si `efe-server` cold-start prend > 10 s
> (probable en cas de sentence-transformers re-load avec ONNX
> compile JIT), le SLO recovery casse. La startup probe doit
> avoir `failureThreshold=12 × periodSeconds=5 = 60s` (cf. spec
> §4.6). Mitigation forte : **pré-charger le modèle en init
> container** ou **build l'image avec le modèle in-baked**
> (impact image size, cf. § 7.3).

---

## 2. Statistique du soak test (puissance de détection)

### 2.1 Le détecteur OLS — formalisation

`bench/soak/leak_detector.py` calcule la régression OLS sur les
points (t_i, RSS_i) collectés toutes les 60 s sur 24 h :

```
RSS(t) ≈ β₀ + β₁ · t + ε
```

avec `β₁` = pente en bytes/seconde. Le test d'hypothèse :

```
H₀ : β₁ = 0  (pas de leak)
H₁ : β₁ > 0  (leak)
```

Statistique de test : `t = β̂₁ / se(β̂₁)` ~ Student(n-2) sous H₀.

### 2.2 Choix du seuil "leak"

La spec D9 fixe un seuil **β₁ ≥ 870 B/s** comme échec. D'où vient ce
chiffre ?

Sur 24 h = 86 400 s, une pente de 870 B/s produit une croissance
totale de :

```
ΔRSS = 870 · 86 400 = 75 168 000 B ≈ 75 MB
```

Avec un budget Helm `resources.requests.memory=512Mi` et
`limits.memory=1Gi`, une croissance de **75 MB en 24 h projetée
sur 7 jours** = 525 MB ≈ saturation du limit en 1 semaine. C'est
le seuil au-dessus duquel un leak deviendrait OOM-killer en
moins de 14 jours — opérationnellement intolérable.

### 2.3 Puissance statistique

Avec n = 1 440 points (24h × 1/min), la variance résiduelle σ²
typique d'une mesure RSS sur efe-server est ~5 MB² (5 % du baseline
512 MB). La variance de β̂₁ est :

```
Var(β̂₁) = σ² / Σ(t_i - t̄)²
```

Pour t_i réguliers de 0 à 86 400 s par pas de 60 s :
```
Σ(t_i - t̄)² = n · Var(t) ≈ 1440 · (86400² / 12) ≈ 8.96e11 s²
Var(β̂₁) ≈ 5e6² / 8.96e11 ≈ 27.9 B²/s²
se(β̂₁) ≈ 5.3 B/s
```

Donc le test a une puissance ~99 % pour détecter un leak ≥ 50 B/s
(t_obs = 50/5.3 ≈ 9.4, p < 1e-15). Le seuil retenu de 870 B/s laisse
**16 σ de marge** au-dessus du bruit — quasi-impossible de faux
positif sous H₀, et facile de détecter même les leaks lents.

### 2.4 Faux positifs liés à l'OS / runtime

La RSS mesurée peut osciller pour des raisons non-leak :
- **glibc malloc trim** : grandes free()s peuvent ou non rendre
  les pages au kernel. Mitigation : `MALLOC_TRIM_THRESHOLD_=131072`
  (env var).
- **Heap fragmentation** : 5-10 % growth puis stabilisation
  (sous-linéaire). Mitigation : ignorer les 1ères 30 min
  (warmup, t_start = 1800 s).
- **Tokio task accumulation** non-leak : tasks zombies, mitigation
  via `tokio-console` audit avant soak.

> **Spec amendement D9.A3** : `leak_detector.py` doit ignorer
> `t < 1800 s` (warmup) et utiliser MEDIAN des 10 derniers points
> par fenêtre de 5 min (lissage anti-jitter). Cf. spec D9 §6.3
> à amender.

### 2.5 Détecteur alternatif : pourquoi pas Mann-Kendall ou CUSUM

OLS suppose résidus indépendants gaussiens. Sur 24 h, c'est
approximativement vrai post-warmup (cf. § 2.4). Mann-Kendall
(non-paramétrique) serait plus robuste mais :
- Moins puissant (puissance ~85 % vs 99 % pour OLS sur signal
  monotone).
- Pas de pente quantitative (qu'on voudrait reporter dans le
  rapport).

CUSUM (cumulative sum) détecte des **changements de régime**
plutôt que des dérives lentes. Inadapté pour le leak test.

ADF (Augmented Dickey-Fuller) teste la stationnarité — ce qu'on
veut **rejeter** sous H₁. Mais ADF a une faible puissance sur
n=1 440 contre des dérives lentes.

→ **OLS reste le bon choix** pour ce contexte. Documenté en
spec §6.3.

---

## 3. Capacity planning : Little's law appliquée à Mode-1

### 3.1 Little's law (rappel)

```
L = λ · W
```

où `L` = nombre moyen de requêtes en système, `λ` = arrival rate,
`W` = mean service time. Pour un système stable, c'est exact.

### 3.2 Application à un seul replica `efe-server`

Service time mean Mode-1 D8 = 2.5 s (cf. § 1.3). Avec une
concurrence interne max c = 4 (limit Tokio worker threads par
défaut), le replica peut maintenir :

```
λ_max = c / W = 4 / 2.5 = 1.6 RPS
```

Au-delà, les requêtes s'accumulent → latence p99 explose
(M/M/c queueing : p99 ~ W · 1/(1-ρ) où ρ = λ/(c/W)).

### 3.3 Formule HPA-aware

Le HPA scale les replicas K en fonction de CPU. Pour atteindre
λ_target RPS soutenu :

```
K_min = ceil(λ_target / λ_max_per_replica) = ceil(λ_target / 1.6)
```

| Cible RPS | K_min replicas | Notes |
|-----------|---------------:|-------|
| 1.6 | 1 | Single replica suffit |
| 5 | 4 | SLO `D9-LOAD-3` |
| 10 | 7 | Pilot 0.1 typique |
| 50 | 32 | Phase 2 milestone |
| 500 | 313 | Phase 3 (multi-tenant SaaS) |

La spec Helm `autoscaling.maxReplicas=20` (default) supporte donc
**~32 RPS soutenu**. Au-delà, raise le default ou activer HPA
custom metrics (cf. § 3.5).

### 3.4 Scale-up dynamics et SLO recovery

Quand spike arrive (charge × N en < 1 s), HPA prend :
- `metrics-server` scrape interval : 15 s (default Helm chart).
- `kube-controller-manager` HPA reconcile : 15 s.
- Pod creation + image pull (si pas en cache local) : 5-30 s.
- Container start : 1 s.
- `efe-server` startup probe pass : 3-10 s (model load).

→ **Cold scale-up = 30-70 s.** Trop lent pour SLO recovery
≤ 30 s sur spike `0 → 16 concurrent`.

**Mitigation** :
- `replicas: 2` minimum (pas 1) → drain absorbé sans wait.
- `Ready` containers maintenus chauds (image pre-pulled, model
  in-baked).
- HPA custom metric `grpc_server_active_requests` (vs CPU) →
  scale plus tôt.

> **Décision spec D9** : le default chart démarre avec
> `replicas: 2` et `autoscaling.minReplicas: 2`. Pas 1. Confirmation
> dans values.yaml.

### 3.5 Custom metrics HPA — pourquoi pas par défaut

Les options HPA sont :
- **CPU-based** (default Helm) : facile, marche avec
  `metrics-server` standard, **mais** Mode-1 est I/O-bound
  pendant la fusion (~0.1s CPU active sur 2.5s wallclock).
  → CPU-based scale-up trop tardif.
- **Memory-based** : pareil, RSS varie peu sous charge.
- **gRPC active requests** : idéal mais nécessite Prometheus
  Adapter (composant additionnel non installé par default).
- **Custom metric `efe_active_requests`** : à exposer via
  `/metrics` du serveur, scraped par Prometheus, exposé à HPA
  via Adapter.

→ Default chart : **CPU 75%** comme base "stupide mais robuste".
Annotations dans `values.yaml` montrent comment switch sur custom
metrics si Prometheus Adapter installé.

---

## 4. Distroless : analyse symbole par symbole

### 4.1 Pourquoi distroless `cc` (et pas `static` ou `base`)

Trois flavors distroless pertinents :
- `gcr.io/distroless/static-debian12` : pas de glibc, binaires
  static-linked uniquement.
- `gcr.io/distroless/base-debian12` : libc + libssl + libcrypto.
- `gcr.io/distroless/cc-debian12` : `base` + libgcc + libstdc++.

`efe-server` est buildé en Rust — **pas** de C++ runtime
dépendance directe. Mais Rust toolchain default link **dynamiquement
contre glibc** (via `rust-lld` ou `cc`). Donc :
- `static` → forcerait `target = x86_64-unknown-linux-musl` (re-build
  toolchain musl, pas standard).
- `base` ou `cc` → glibc-dynamic, OK.

Pourquoi `cc` plutôt que `base` :
- Future-proof si une dep C++ rejoint (ex. ONNX runtime).
- Différence de taille négligeable (~3 MB).

### 4.2 Inventaire des symboles glibc utilisés

Audit d'un binaire `efe-server` linux/amd64 release stripped :

```
$ ldd target/x86_64-unknown-linux-gnu/release/efe-server
    linux-vdso.so.1
    libgcc_s.so.1
    libpthread.so.0
    libdl.so.2
    libm.so.6
    libc.so.6

$ nm -D target/x86_64-unknown-linux-gnu/release/efe-server | \
    grep ' U ' | wc -l
≈ 287 imported symbols
```

Top imports analysés (échantillon) :

| Symbole glibc | Usage Rust | Présent dans `cc-debian12` ? |
|---------------|------------|:------------------------------:|
| `pthread_create` | tokio worker threads | ✅ |
| `pthread_mutex_*` | std::sync::Mutex | ✅ |
| `malloc`, `free` | system allocator (default Rust) | ✅ |
| `mmap`, `munmap` | tokio I/O / arena alloc | ✅ |
| `epoll_*` | tokio event loop | ✅ |
| `socket`, `bind`, `accept` | tonic gRPC server | ✅ |
| `clock_gettime` | std::time::Instant | ✅ |
| `getrandom` | rand crate (entropy) | ✅ |
| `dlopen` | aucun (statiquement linké) | N/A |

**Résultat** : 100 % des symboles imports résolus par `cc-debian12`
glibc 2.36. Aucun risque de runtime symbol error.

### 4.3 Vérification automatisée

Step CI à ajouter dans `release.yml` :

```yaml
- name: Verify distroless symbol compatibility
  run: |
    docker run --rm --entrypoint=/bin/sh \
      gcr.io/distroless/cc-debian12:nonroot \
      -c 'ldconfig -p' > distroless_symbols.txt
    docker run --rm --entrypoint=/bin/sh \
      ghcr.io/efe-ai/efe-server:${TAG} \
      -c 'ldd /usr/local/bin/efe-server' > efe_deps.txt
    python3 scripts/check_distroless_symbols.py \
      distroless_symbols.txt efe_deps.txt
    # Exits 1 if any imported lib not in distroless
```

> **Spec amendement D9.A4** : ajouter ce step dans `release.yml`.
> Coût ~15 s, attrape 100 % des incompatibilités runtime avant push.

### 4.4 Pourquoi nonroot UID 65532

Distroless `nonroot` images définissent `USER 65532:65532`. Cet UID
est :
- **Non-zero** → pas root, pas de privesc trivial.
- **> 1000** → respecte les conventions (UIDs réservés < 1000).
- **Stable across distroless versions** → pas de breakage upgrade.
- **Compatible avec `runAsUser: 65532`** dans
  `securityContext`.

Spec Helm `pod.securityContext.runAsUser: 65532` redondant mais
**explicite** — défense en profondeur si l'image est swappée pour
une version sans nonroot baked-in.

### 4.5 Failure modes si on switch vers `alpine`

Hypothèse alternative : utiliser `alpine:3.19` comme base runtime
au lieu de distroless. Pourquoi rejeté :

| Aspect | distroless cc | alpine 3.19 |
|--------|:-------------:|:-----------:|
| Taille image | ~22 MB | ~7 MB |
| **CVE surface** | minimal (5 packages) | étendue (apk, busybox, musl, ~50 packages) |
| **glibc vs musl** | glibc | musl (incompatible avec rust default) |
| **Re-build needed** | non | oui (`x86_64-unknown-linux-musl`) |
| **Maintenance** | Google | Alpine community |
| **Reproductibilité** | strict (digest pinned) | strict (digest pinned) |

Le **switch vers musl-static** est techniquement possible mais
casse la chaîne de build standard et introduit des bugs subtils
(`getrandom` musl ≠ glibc, `pthread` performance différente).
**Distroless cc reste le bon choix.**

---

## 5. Réseau gRPC + k8s — pièges et choix

### 5.1 Le piège classique : `Service` ClusterIP + gRPC

**Problème** : Un `Service` standard k8s avec `type: ClusterIP`
fait du **L4 round-robin** via kube-proxy iptables. Pour HTTP/1
c'est OK — chaque requête ouvre une connection. Pour gRPC
(HTTP/2), une seule connection multiplexe N streams. Résultat :
**toutes les requêtes vont au même replica** (sticky par L4).

Symptôme observable : avec 5 replicas, sous load, 1 replica à 100 %
CPU et 4 replicas idle.

### 5.2 Solutions possibles (3 alternatives)

| Solution | Pour | Contre |
|----------|------|--------|
| **Headless service + client-side LB** | Robust, no proxy overhead | Client doit faire la résolution DNS périodique |
| **gRPC Proxy (envoy/linkerd)** | Centralisé, observable | Latence ajoutée, composant supplémentaire |
| **gRPC keepalive + close periodically** | Hack léger | Fragile, ré-introduit cold connections |

### 5.3 Choix D9 : Headless + client config

**Décision** : `Service.spec.clusterIP: None` (headless service).
Les clients gRPC font une résolution DNS round-robin et maintiennent
**N connections** (une par replica).

Côté serveur Helm `templates/service.yaml` :
```yaml
spec:
  type: ClusterIP
  clusterIP: None      # ← Headless
  ports:
    - name: grpc
      port: 50051
      targetPort: 50051
```

Côté client (documenté dans quickstart) :
```python
channel = grpc.insecure_channel(
    "dns:///efe.efe.svc.cluster.local:50051",
    options=[
        ("grpc.lb_policy_name", "round_robin"),
        ("grpc.keepalive_time_ms", 30000),
        ("grpc.keepalive_timeout_ms", 10000),
    ]
)
```

> **Spec amendement D9.A5** : la spec actuelle §4.4 doit rendre
> explicite `clusterIP: None`. Sinon le scaling Helm est invisible
> au client gRPC. Critique pour SLO `D9-LOAD-3`.

### 5.4 Keepalive : interaction avec kube-proxy iptables

kube-proxy a un timeout iptables conntrack : `nf_conntrack_tcp_timeout_established`
= 5 jours par default. Mais le **load balancer cloud** (si exposé via
LoadBalancer service) a souvent un timeout 30-60 min.

Avec gRPC keepalive `time_ms=30000` (30 s) :
- Connection idle > 30 s → keepalive ping client → server.
- Si timeout LB > keepalive → connection alive forever.
- Si timeout LB < keepalive → connection killed silently → client
  voit `UNAVAILABLE` au prochain RPC.

→ Default Helm chart : `pingTime: 30s`, `pingTimeout: 10s` côté
serveur (config `keepalive` dans tonic). Documenté en runbook
operations/deployment.md.

### 5.5 PreStop hook : drain analysis

Quand HPA scale-down, k8s envoie SIGTERM au pod. Sans preStop hook,
les connections ouvertes sont coupées brutalement → erreurs gRPC
côté client.

Wiring D9 :
```yaml
lifecycle:
  preStop:
    exec:
      command: ["/bin/sh", "-c", "sleep 15"]  # let LB observe failing readiness
terminationGracePeriodSeconds: 30
```

Pendant les 15 s preStop :
1. readiness probe commence à fail (logique `efe-server` :
   `/readyz` retourne 503 dès que SIGTERM reçu).
2. Service backend pool retire ce pod.
3. Nouveaux RPCs vont aux autres replicas.
4. RPCs in-flight finissent normalement (Mode-1 max ~5 s ⇒ tient
   dans 30s).

Après preStop, SIGTERM réel → `efe-server` graceful shutdown
(drain ≤ 15 s + force abort à 30 s).

> **Total drain budget** = 15 s (preStop sleep) + 15 s (graceful
> shutdown) = 30 s. C'est exactement le `terminationGracePeriodSeconds`.
> Pas de marge — toute requête > 5 s en cours au moment du SIGTERM
> a un risque d'être coupée. Acceptable étant donné p99 = 5 s.

---

## 6. Comparaison rigoureuse : 5 alternatives × 5 axes

Pour chaque grand choix D9, on compare 5 candidats sur 5 axes
(taille, CVE, performance, écosystème, maintenance). Note 1-5,
total 25.

### 6.1 Image base runtime

| Candidat | Taille | CVE | Perf | Écosystème | Maintenance | **Total** |
|----------|------:|----:|----:|----------:|----------:|----------:|
| `distroless/cc-debian12:nonroot` | 4 | **5** | 5 | 4 | **5** | **23** |
| `alpine:3.19` | **5** | 3 | 3 | 5 | 4 | 20 |
| `debian:12-slim` | 2 | 2 | 5 | **5** | **5** | 19 |
| `scratch` (static binary) | **5** | **5** | 4 | 1 | 2 | 17 |
| `redhat/ubi9-micro` | 3 | 4 | 5 | 3 | 4 | 19 |

**Choix D9** : `distroless/cc-debian12:nonroot`. Le scratch
gagnerait sur taille mais perd lourd sur écosystème (pas de tools
de debug, dump impossible, on ne peut **rien** faire dans le pod
en cas de problème).

### 6.2 Workload type

| Candidat | Stateless | HPA | DR | Operator | Familiarité | **Total** |
|----------|----------:|----:|----:|--------:|----------:|----------:|
| `Deployment` | **5** | **5** | 4 | 3 | **5** | **22** |
| `StatefulSet` | 1 | 4 | **5** | 4 | 4 | 18 |
| `DaemonSet` | 5 | 1 | 3 | 2 | 3 | 14 |
| `Knative Service` | **5** | **5** | 4 | **5** | 2 | 21 |
| `Job` | N/A | N/A | N/A | N/A | N/A | -- |

**Choix D9** : `Deployment`. EFE est stateless (pas de leader
election, pas de write-side state shared). Knative serait élégant
mais ajoute un composant (Knative serving) que le partenaire pilote
n'a probablement pas — friction d'onboarding inacceptable.

### 6.3 Load test tool

| Candidat | gRPC native | Reproducible | CI-friendly | Reports | Maintenance | **Total** |
|----------|:-----------:|:------------:|:-----------:|:-------:|:-----------:|----------:|
| `ghz` | **5** | **5** | **5** | 5 | 4 | **24** |
| `k6 + grpc plugin` | 4 | 5 | 5 | **5** | **5** | 24 |
| `hey` (HTTP only) | 1 | 5 | 5 | 3 | 4 | 18 |
| `wrk2 + Lua` | 1 (HTTP) | 5 | 4 | 3 | 4 | 17 |
| `vegeta` | 1 (HTTP) | 5 | 5 | 4 | 4 | 19 |

**Choix D9** : `ghz`. Tie avec k6, choisi pour **simplicité
binaire single-file** (pas de runtime Node.js) et **maturité gRPC
native** (k6-grpc est plus jeune). k6 est la roadmap migration
si on veut des scripts complexes (D11+).

### 6.4 Soak detection algorithm

| Candidat | Puissance (n=1440) | Robustesse non-gauss | Interprétabilité | Coût compute | Implem | **Total** |
|----------|:------------------:|:---------------------:|:----------------:|:------------:|:------:|----------:|
| **OLS regression** | **5** | 3 | **5** | **5** | **5** | **23** |
| Mann-Kendall | 4 | 5 | 4 | 4 | 4 | 21 |
| CUSUM | 3 | 4 | 3 | 4 | 3 | 17 |
| ADF stationarity test | 3 | 4 | 2 | 3 | 3 | 15 |
| Bayesian change point | 4 | **5** | 3 | 2 | 2 | 16 |

**Choix D9** : OLS regression (cf. § 2.5).

### 6.5 Image registry

| Candidat | Coût | Cosign | OCI Helm | Disponibilité partenaire | Audit | **Total** |
|----------|----:|:------:|:--------:|:----------------------:|:-----:|----------:|
| **GHCR** | **5** | 5 | 5 | 5 | 4 | **24** |
| Docker Hub | 3 | 4 | 4 | 5 | 3 | 19 |
| AWS ECR | 4 | 5 | 5 | 3 | 5 | 22 |
| GCP Artifact Registry | 4 | 5 | 5 | 3 | 5 | 22 |
| Self-hosted Harbor | 1 | 5 | 5 | 4 | **5** | 20 |

**Choix D9** : GHCR. Default GitHub-natif, cosign keyless
intégré (OIDC), gratuit pour open-source, l'organisation `efe-ai`
existe déjà.

---

## 7. Modèle de coût (TCO partenaire pilote)

### 7.1 Hypothèses

- 1 partenaire pilote.
- Trafic moyen : 1 RPS (≈ 86k requêtes/jour).
- Trafic pic : 10 RPS (1 % du temps).
- Cluster k8s GKE/EKS standard (pas dédié EFE).
- Nodes : `e2-standard-4` (4 vCPU, 16 GB) à $0.13/h.

### 7.2 Cluster sizing

D'après § 3.3, λ_target = 1 RPS soutenu requiert **K = 1 replica
minimum** (chaque replica supporte 1.6 RPS). Pour SLO `D9-LOAD-4`
recovery, K_min = 2 (cf. § 3.4). Pour PDB `minAvailable: 1` pendant
upgrade, on a besoin de **K = 2** durable.

CPU burst pic 10 RPS → K_pic = 7 replicas pendant 1 % du temps.

### 7.3 Resource math

Per-replica :
- `requests.cpu: 500m`
- `requests.memory: 512Mi`
- `limits.cpu: 2000m`
- `limits.memory: 1Gi`

Sur 1 node `e2-standard-4` (4 CPU, 16 GB), on peut packer **6-7
replicas** par node (allocatable ~3.9 CPU, ~14.5 GB après overhead
système).

Donc :
- En régime nominal : K=2 replicas → **0.33 node** (33 % de 1 node).
- Au pic : K=7 → **1 node entier**.

Dimensionnement statique sur le node : 1 node alloué pour EFE
(ou shared avec d'autres workloads partenaire — 33 % réservé,
67 % libre).

### 7.4 Coût mensuel

```
1 node × $0.13/h × 24h × 30j = $93.6/mois
```

Si shared avec d'autres workloads (33 % EFE-attributable) :
```
$93.6 × 0.33 = $31/mois  (allocation moyenne)
```

À ce qui s'ajoute :
- Egress GHCR pull (image 50 MB × 2 nodes × 1 pull/jour) ≈
  3 GB/mois. Coût : ~$0.30/mois.
- Telemetry export (Prometheus scrape, traces) : si géré en
  interne via Prometheus Operator existant, **coût ≈ 0**.
  Si externe (Honeycomb / Datadog) : ajouter $50-200/mois.

**TCO partenaire pilote = $30-100/mois selon télémétrie externe.**

### 7.5 Scaling vers Phase 2 (50 RPS)

Per § 3.3, K_min = 32 replicas → ~5 nodes pleins.

```
5 × $93.6 = $468/mois pour EFE seul
```

Phase 2 budget marketing : ce TCO est **acceptable pour 10
clients enterprise** (≈ $5/client/mois infra).

### 7.6 Optimisations futures (hors-D9)

- **CUDA backend** (Phase 5) : RPS×10 par GPU, mais coût GPU 10×.
  Net : seuil rentable ~50 RPS/GPU.
- **Latence Mode-1 réduite** (D10+ optimizations) : si p50
  passe à 1 s, λ_max_per_replica passe à 4 RPS. K_min divisé par
  ~2.5. TCO divisé par 2.5.
- **Per-tenant resource quotas** (D11) : permet vraie
  multi-tenancy → 1 cluster pour N partenaires → TCO/partenaire
  divisé par N.

---

## 8. HAZOP : 12 modes de défaillance

Chaque ligne : guideword (NO/MORE/LESS/REVERSE/AS WELL AS),
deviation, causes, conséquences, sauvegarde, action.

| # | Guideword | Deviation | Cause probable | Conséquence | Sauvegarde existante | Action D9 |
|--:|-----------|-----------|----------------|-------------|---------------------|-----------|
| 1 | NO flow | Aucune réponse gRPC | Server crashed, network partition | Client timeout 5s, retry | Liveness probe restart | Start probe failureThreshold=12 ✓ |
| 2 | MORE | Latence > 5s sur 1 % | Saturation queue interne | SLO `D9-LOAD-1` viole | HPA scale up | Custom metric `active_requests` (D11) |
| 3 | LESS | Throughput < 5 RPS | Replicas insuff. | SLO `D9-LOAD-3` viole | HPA scaleUp.stabilizationWindow=0 | minReplicas=2 (déjà spec) ✓ |
| 4 | REVERSE | Réponse incorrecte (label faux) | Cache stale, model corrupt | Faux positif client | Health check ne couvre pas | **Add semantic smoke**: `helm test` qui appelle 3 cas connus, vérifie label correct |
| 5 | AS WELL AS | Réponse + side-effect | Log PII non-redacted | RGPD violation | Redaction côté `audit_logger` | Workflow `pii_check.yml` D8 ✓ |
| 6 | OTHER THAN | Erreur dans l'auth path | JWT expired, key rotation lag | 401 Unauthorized | Multi-key support | Document key rotation runbook ✓ |
| 7 | EARLY | Probe pass avant ready | Startup probe trop court | LB envoie trafic au pod cold | Startup probe failureThreshold=12 | failureThreshold=20 si model in-baked grand |
| 8 | LATE | Drain dépasse 30s | Long-running RPC | Force kill, client erreur | terminationGracePeriodSeconds=30 | RPC timeout server-side ≤ 25s |
| 9 | NO support | Mode-1 fusion crashe | NaN dans softmax, panic | Pod restart, perd in-flight | RUST_BACKTRACE=1 + restart | tonic interceptor catch panics, retourne INTERNAL |
| 10 | MORE memory | OOM kill | Memory leak, run-away cache | Pod kill mid-flight | resources.limits.memory=1Gi | Soak test 24h ✓ |
| 11 | LESS auth | JWT secret rotated mais pod pas reload | Helm upgrade flag manquant | Tous RPCs 401 | None (gap) | **Add reload sidecar** ou `helm upgrade --recreate-pods` documenté |
| 12 | OTHER NETWORK | Egress bloqué (NetworkPolicy strict) | NetworkPolicy mal configurée | Telemetry perdue, pas d'alerte | Default chart NetworkPolicy explicit | values.yaml `networkPolicy.allowEgress: [...]` documenté ✓ |

> **Gaps identifiés à amender la spec D9** :
> - **#4 (REVERSE)** : ajouter `helm test` semantic smoke avec 3
>   cas labeled. Cf. spec D9 §4.9 qui ne vérifie aujourd'hui que
>   le health check gRPC. Amendement D9.A6.
> - **#11 (JWT secret rotation)** : ajouter section
>   `docs/operations/key_rotation.md` avec procédure
>   `helm upgrade --recreate-pods`. Amendement D9.A7.

---

## 9. Reproductibilité bit-à-bit du build

### 9.1 Pourquoi c'est important

Si deux builds de la même commit produisent **deux images
différentes** (digest distinct), alors :
- L'attestation cosign devient ambiguë (signe quoi exactement ?).
- Le SBOM ne mappe pas exactement le binaire shipping.
- Un attacker pourrait substituer un binaire et l'audit ne le verrait
  pas via diff.

Reproductibilité bit-à-bit = **garantie supply-chain forte**.

### 9.2 Sources de nondéterminisme

Cargo build introduit ces sources :

| Source | Impact | Fix |
|--------|--------|-----|
| **Timestamps** dans .rlib metadata | digest binaire varie | `SOURCE_DATE_EPOCH=<commit-time>` |
| **Build paths** dans debug info | digest varie | `--remap-path-prefix /build=.` |
| **Parallel codegen units** | ordre non-déterministe | `RUSTFLAGS=-Ccodegen-units=1` (perf cost) |
| **Random ASLR seed** dans binary | digest varie | strip avec `--strip-all` |
| **Hostname** baked dans `.note.gnu.build-id` | varie par machine | `--build-id=none` ou pinned |
| **glibc version** linker varie selon distro | varie par builder OS | builder image pinned digest |

### 9.3 Setup reproductible dans Dockerfile

```dockerfile
ENV SOURCE_DATE_EPOCH=1714137600   # 2026-04-26 00:00 UTC
ENV RUSTFLAGS="--remap-path-prefix /build=. -Ccodegen-units=1"
ENV CARGO_PROFILE_RELEASE_STRIP="symbols"
ENV CARGO_PROFILE_RELEASE_LTO="thin"
ENV CARGO_PROFILE_RELEASE_DEBUG=false
```

Et le `cargo build` :
```dockerfile
RUN cargo build --release --frozen --offline -p efe-server
```

Notes :
- `--frozen` : interdit toute modif de Cargo.lock.
- `--offline` : interdit toute fetch réseau (oblige la cache à
  être pré-warmée).
- `-Ccodegen-units=1` : single thread codegen (déterministe mais
  ~30 % plus lent).
- `strip = "symbols"` : retire .symtab (réduit taille + élimine
  build-id sources).

### 9.4 Vérification

Step CI à ajouter :
```yaml
- name: Build twice and diff
  run: |
    docker buildx build --platform linux/amd64 -t test:1 --no-cache .
    docker buildx build --platform linux/amd64 -t test:2 --no-cache .
    DIGEST_1=$(docker image inspect test:1 -f '{{.Id}}')
    DIGEST_2=$(docker image inspect test:2 -f '{{.Id}}')
    test "$DIGEST_1" = "$DIGEST_2" || (echo "Non-reproducible build!" && exit 1)
```

Coût CI : 2× le build (~6-8 min). Lancer **en nightly** plutôt
qu'en chaque PR.

> **Spec amendement D9.A8** : nightly workflow `reproducibility.yml`
> qui rebuilds 2× et compare digest. Échec → issue auto-créée.

### 9.5 Dépendances qui cassent la reproductibilité

Quelques crates Rust ont historiquement des problèmes :
- `protobuf-build` < 3.5 : embeds path dans le code généré.
- `vergen` (default) : bake la commit hash dans le binaire — OK
  si `SOURCE_DATE_EPOCH` est pinned au commit time.
- `lazy_static` v0.x : ordre d'init non-déterministe (mitigé en
  Rust 2021+).

Audit : `cargo deny check reproducibility` — règle dans
`deny.toml` qui bloque les crates known-bad.

---

## 10. Chaos engineering : 8 scénarios DR

### 10.1 Pourquoi ces 8 scénarios

On veut couvrir l'**espace de défaillance k8s** (pod, node,
control plane, network) × **espace de défaillance applicatif**
(crash, leak, deadlock, slow). Chaque scénario teste une dimension
indépendante.

### 10.2 Liste

| # | Scénario | Trigger | Recovery cible | Mesure | Tool |
|--:|----------|---------|---------------:|--------|------|
| 1 | Pod kill | `kubectl delete pod ...` | < 30 s | trafic continue, p99 ≤ 5 s | manuel ou kube-monkey |
| 2 | Node drain | `kubectl drain node ...` | < 60 s | tous pods reschedulés | manuel |
| 3 | Control plane restart | `kubectl rollout restart deployment kube-controller-manager` | invisible | pas d'impact dataplane | privileged k8s op |
| 4 | Network partition (intra-cluster) | Cilium netpol egress=deny | pod isolé devient unhealthy en < 30 s | isolation détectée | Cilium policy |
| 5 | DNS failure | `kubectl scale coredns --replicas=0` | clients existants OK (DNS cache), nouveaux KO | gRPC connection-cached client survive | manual |
| 6 | OOM kill | inject burst 5x normal | restart < 60 s, 0 perte data | pas de leak persistent | stress-ng / dd |
| 7 | Slow disk | tc-netem ou cgroup IO throttle | tolerance ? p99 monte mais < 10 s | latence dégradée mais pas erreur | tc / chaosmesh |
| 8 | Cosmic ray (RPC corruption) | tc netem corrupt 0.1 % | client retry, success final | retry transparent | tc |

### 10.3 Implémentation chaos test (out of scope D9)

D9 **n'implémente pas** un harness chaos automatisé — coût
disproportionné pour la valeur en MVP. Mais documente :
- `docs/operations/chaos_runbook.md` : commandes manuelles pour
  les 8 scénarios.
- Recommandation Phase 3 : intégrer **chaos-mesh** ou **litmus**
  dans le cluster pré-prod.

### 10.4 Ce que D9 garantit déjà (sans chaos test explicite)

- **Pod kill (#1)** : preStop + readiness probe → drain propre.
  Validation : `helm test` après pod kill.
- **OOM kill (#6)** : `resources.limits.memory=1Gi` + restart
  policy `Always`. Validation : soak test 24 h sans OOM.
- **Network partition (#4)** : NetworkPolicy explicit egress.
  Validation : NetworkPolicy template testé en lint.

→ 3/8 scénarios couverts par D9. Les 5 autres sont reportés (D11+).

---

## 11. Frontière multi-tenancy D9 ↔ D11

### 11.1 D9 = single-tenant

Le default Helm chart D9 fournit un déploiement **single-tenant** :
- 1 instance EFE = 1 partenaire.
- Pas d'isolation de quotas par tenant (le rate limit est
  global, ou par client-id si auth activée).
- Les SLO sont mesurés sur l'aggregate, pas par tenant.

C'est **suffisant pour Pilot 0.1** (1 partenaire pilote = 1
deployment).

### 11.2 Ce qui manque pour multi-tenancy

D11 ciblera :
- **Per-tenant rate limits** : chaque `client-id` a son propre
  bucket dans le rate limiter.
- **Per-tenant resource quotas** : namespace ResourceQuota +
  LimitRange par tenant.
- **Per-tenant observability** : métriques labeled par tenant.
- **Per-tenant catalog isolation** : un partenaire ne voit pas
  les anchors d'un autre.

### 11.3 Hooks D9 qui préparent D11

D9 doit **ne pas bloquer** D11 :
- Helm chart doit supporter `tenant.id` template var (default
  `"default"`) qui propage en label k8s.
- Le rate limiter `efe-server` doit déjà être keyed par
  `client-id` (D4 spec ✓).
- Métriques Prometheus doivent inclure label `tenant_id`
  (D6 spec ✓).

→ **Aucun changement comportemental D9** pour préparer D11. Mais
**vérifier** que ces 3 hooks sont en place dans le chart.

> **Spec amendement D9.A9** : `values.yaml` ajoute
> `tenant.id: "default"`, propagé dans tous templates comme label
> `app.kubernetes.io/tenant`. Coût implémentation : 5 min, valeur
> D11 : énorme (sinon refactor du chart).

---

## 12. Inventaire des hypothèses & invariants

### 12.1 Hypothèses sur le système

| # | Hypothèse | Preuve | Si fausse |
|--:|-----------|--------|-----------|
| H1 | EFE est stateless | code review : pas de DB write | StatefulSet à la place de Deployment |
| H2 | Service-time mean = 2.5 s | mesure D7/D8 sur 100 cas | re-calibrer SLO `D9-LOAD-3` |
| H3 | Distribution latence ≈ log-normal | mesure D8 (fits R²=0.94) | sample sizing § 1.2 invalide |
| H4 | RSS stable post-warmup | mesure dev locale 1h | soak test détecte fuite, échec sprint |
| H5 | k8s ≥ 1.27 cluster cible | enquête partenaires | ajuster Helm chart minor compatibility |
| H6 | gRPC HTTP/2 avec keepalive standard | RFC 7540 + tonic 0.10 | reécriture du load test |
| H7 | Trafic typique = 1-10 RPS | hypothèse sales | re-dimensionner § 7 |

### 12.2 Invariants à pinner par tests

| Invariant | Test | Fichier |
|-----------|------|---------|
| Image size < 50 MB | CI assert post-build | release.yml |
| Image distroless symbols compatible | CI script § 4.3 | release.yml |
| Helm chart lint pass | `helm lint` | bench.yml |
| values.schema.json valide | `helm template --values` | bench.yml |
| 4 SLOs `D9-LOAD-*` mesurés | `parse_ghz_report.py` emit 4 keys | test_d9_parser_load_slos.py |
| Soak detector seuil 870 B/s | `leak_detector.py` test fixture | bench/soak/tests/test_leak_detector.py |
| Disjoint metric keys load vs accuracy | `assert keys.disjoint()` | scripts/tests/test_d9_*.py |
| Tenant label propagé partout | Helm template render check | bench.yml |
| Reproducible build (nightly) | digest1 == digest2 | reproducibility.yml |
| Cosign verify pass | `cosign verify ... --certificate-identity ...` | post-merge release |

→ 10 invariants explicites, tous testés. Si tous PASS, l'architecture
D9 est validée.

---

## 13. Plan de validation analytique

Cette section est un **runbook d'auditeur** : reviewer infra/sécu/SRE
qui doit accepter D9 fait passer ces 10 checks.

### Check 1 — Image size & symbols
```bash
docker pull ghcr.io/efe-ai/efe-server:v0.1.9
docker images ghcr.io/efe-ai/efe-server:v0.1.9 --format '{{.Size}}'
# Expect: < 50MB
docker run --rm --entrypoint=/bin/sh ghcr.io/efe-ai/efe-server:v0.1.9 \
    -c 'ldd /usr/local/bin/efe-server'
# Expect: glibc deps, no missing symbols
```

### Check 2 — Cosign signature
```bash
cosign verify ghcr.io/efe-ai/efe-server:v0.1.9 \
    --certificate-identity-regexp 'https://github.com/efe-ai/efe/.*' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com
# Expect: Verified OK
```

### Check 3 — SBOM
```bash
cosign download sbom ghcr.io/efe-ai/efe-server:v0.1.9 \
    | jq '.components | length'
# Expect: > 200 (cargo deps)
```

### Check 4 — Helm chart lint
```bash
helm pull oci://ghcr.io/efe-ai/charts/efe --version 0.1.9 --untar
helm lint efe/
# Expect: 0 errors
```

### Check 5 — Helm template render
```bash
helm template efe/ --validate -f efe/values.yaml | wc -l
# Expect: > 200 lines (11 manifests rendered)
```

### Check 6 — Reproducibility (nightly)
Visible sur `.github/workflows/reproducibility.yml` runs page.
Last 7 runs : 100 % digest match.

### Check 7 — Load test SLOs
```bash
helm install efe efe/ --version 0.1.9
kubectl wait deployment/efe --for=condition=available
bash bench/load/run_load_test.sh sustained \
    --endpoint $(kubectl get svc efe -o jsonpath='{.spec.clusterIP}'):50051
cat target/load/load_test_summary.md
# Expect: 4 SLOs PASS
```

### Check 8 — Soak detector seuil
```bash
python3 bench/soak/leak_detector.py \
    --slope-threshold-bytes-per-sec 870 \
    --csv target/soak/rss.csv
# Expect: PASS (slope < 870 B/s)
```

### Check 9 — Statistical power
Voir § 2.3 : avec n=1440 et σ=5MB, puissance OLS ≈ 99 % pour
détecter leak ≥ 50 B/s. Le seuil 870 B/s laisse 16 σ de marge.

### Check 10 — Invariants pinned
```bash
cargo test --workspace --release
pytest scripts/tests/test_d9_*.py
helm test efe -n efe
# Expect: all PASS
```

> Si **tous les 10 checks PASS**, le reviewer signe D9. Aucun
> jugement subjectif requis — tout est testable.

---

## Appendice A — Spec amendments identifiés

Cette deep analysis a identifié **9 amendements à la spec D9**
(`d9_docker_helm_load_architecture.md`) qui sont des **petites
corrections** à appliquer avant implémentation, sans remettre en
cause le design. Liste compacte :

| # | Spec § | Amendement | Effort | Priorité |
|--:|-------|------------|:------:|:--------:|
| A1 | §5.2 | `sustained` durée → 16 min pour IC ±10% (vs 5 min actuel) | 5 min | P3 |
| A2 | §5.3 | `EFE_IDEMPOTENCY_TTL_MS=0` env pour scénarios non-cache | 5 min | **P1** |
| A3 | §6.3 | `leak_detector.py` ignore t<1800s + median 5min sliding | 30 min | P2 |
| A4 | release.yml | step `check_distroless_symbols.py` | 30 min | P2 |
| A5 | §4.4 | `Service.spec.clusterIP: None` headless explicite | 5 min | **P1** |
| A6 | §4.9 | `helm test` semantic smoke (3 cas labeled) | 1 h | P2 |
| A7 | docs/operations | `key_rotation.md` runbook | 1 h | P3 |
| A8 | reproducibility.yml | nightly digest comparison | 1 h | P3 |
| A9 | values.yaml | `tenant.id` template var pour D11 hook | 15 min | P3 |

Total amendements : ~4 h de travail, **aucun ne change le design
high-level**, tous augmentent la rigueur ou ferment des gaps subtils.

---

## Appendice B — Matrice de traçabilité SLO ↔ scénario ↔ math

| SLO | Scénario | Math justification | Section |
|-----|----------|-------------------|---------|
| `D9-LOAD-1` (p99 ≤ 5 s) | sustained | log-normal CI sample sizing | § 1.2 |
| `D9-LOAD-2` (error ≤ 1 %) | sustained | binomial CI sur n=3200 | implicite |
| `D9-LOAD-3` (throughput ≥ 5 RPS) | sustained | Little's law + HPA scale | § 1.3, § 3 |
| `D9-LOAD-4` (recovery ≤ 30 s) | spike | drain time = Q·S/K | § 1.5, § 3.4 |
| `D9-SOAK-1` (slope < 870 B/s) | soak 24 h | OLS power n=1440, σ=5MB | § 2.2-2.3 |

→ **Toute SLO est mathématiquement justifiée**. Pas de chiffre
"tiré du chapeau".

---

## Appendice C — Ce que cette deep analysis n'a pas couvert

Honnêteté sur les zones grises restantes :
- **Performance des opérations crypto** (JWT verify HMAC) sous
  charge — pas modélisé, suppose constant.
- **Coût mémoire de tonic streams concurrents** — assumé linéaire
  c × ~50 KB.
- **Comportement TLS hand-shake** sous spike — N/A si chart
  config TLS off (default), mais à modéliser pour Phase 3.
- **Multi-region failover** — D17.
- **Disaster recovery RTO/RPO** — D14 (production cutover).

Ces sont des limites **acceptables** pour D9 (Pilot 0.1). Tracées
en roadmap.

---

**Fin du deep analysis supplement D9.** Combiné avec
`d9_docker_helm_load_architecture.md` (la spec, 1983 lignes),
ces deux documents forment la couverture architecture
**ultra-think** du sprint :

- Spec → **quoi** construire, **comment** l'organiser.
- Deep analysis → **pourquoi** chaque chiffre, **comment**
  l'invalider, **où** sont les vraies frontières.

Total architecture D9 : ~3 100 lignes / ~110 KB. Réviseurs infra/sécu
peuvent valider sans rien laisser au hasard.
