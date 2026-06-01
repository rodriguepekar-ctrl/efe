# Sprint D18 — Multi-réplica + bench D12 (architecture)

> **Status** : in-progress, v0.1.19 (post-D17 Backup + DR).
> **Versionnement BENCHMARKS.md** : `0.1.19` (sprint D18 = première
> livraison multi-réplica horizontalement scalable + bench formel
> D12 catalog operations).
> **Pré-requis** :
>   * D11 (per-tenant isolation) en place ;
>   * D12 + D12-bis (catalog admin + gRPC handler) actifs ;
>   * D14 (Postgres backend) opérationnel ;
>   * D14-bis (HMAC audit chain) actif ;
>   * D15 (OIDC) déployé ;
>   * D16 (Embedder bundle) actif ;
>   * D17 (Backup + DR) livré.
> **Pattern référence** : audits D8 (8 bugs, 2 critiques),
> D12-bis (7 bugs, 2 critiques), D14 (7 bugs, 1 critique),
> D14-bis (9 bugs, 3 critiques), D15 (17 bugs, 8 critiques),
> D16 (17 bugs, 5 critiques), D13-bis (13 bugs, 2 critiques),
> D17 (10 bugs, 5 critiques + 1 follow-up + 4 retro-anchors).

---

## 0. TL;DR

EFE tourne en **single-replica** depuis M12. Le Helm chart par
défaut ship `replicas: 1`, le crate efe-server fait l'hypothèse
implicite d'**un seul process** dans le cluster (idempotency cache
in-memory, rate-limit buckets in-memory, leader-only background
tasks). Cette posture limite :

* **Disponibilité** : un kill -9, un OOM, un node-drain pendant un
  rolling upgrade → 100 % du traffic 5xx pour ~5-30 s.
* **Throughput** : capped au CPU d'un seul pod (~1 vCPU peak
  utilisation typique pour 500 RPS Execute, ce qui est limitant
  pour les Tier 1 clients dépassant les 1000 RPS).
* **Tier 1 SLA** : "99.95 % monthly availability" est inatteignable
  avec un seul replica (SPOF cluster-wide).

D18 livre :

1. **Multi-réplica** par défaut : Helm `replicas: 3` (operator
   ajustable), avec :
   * `podAntiAffinity` pour spreading sur des nodes distincts ;
   * `PodDisruptionBudget` pour empêcher l'éviction simultanée
     pendant un cluster-autoscale ou node-drain ;
   * `HorizontalPodAutoscaler` (HPA) basé sur CPU + p95 latency ;
   * `topologySpreadConstraints` cross-AZ pour multi-AZ clusters.

2. **Migration des states per-replica vers shared store (PG)** :
   * `efe_idempotency` table → idempotency cache distribué ;
   * `efe_ratelimit_buckets` table → rate-limit cross-replica ;
   * Les caches secondaires (JWKS, TenantStore, Embedder bundles
     in-memory) restent per-replica avec eventual consistency
     bornée par TTL.

3. **Leader election K8s-native** pour background tasks :
   * D14-bis HMAC self-verify periodic ;
   * D17 backup CronJob deduplication (le CronJob spawn déjà 1
     pod, mais leader election protège contre concurrent runs
     pendant un upgrade) ;
   * Cone GC + idempotency expiry sweep.

4. **Bench harness formel** pour les opérations D12 :
   * Crate `efe-bench` shipping 6 workload patterns
     (read-heavy, write-heavy, balanced, burst, multi-tenant
     concurrent, soak).
   * SLOs `D18-bench-1` à `D18-bench-7` mesurés et alertés.
   * Comparaison avant/après multi-replica → speedup measurement.
   * Output : `docs/bench/d18_bench_results.md` régénéré à
     chaque release candidate via CI.

5. **Threat model étendu** : 12 menaces couvrant split-brain,
   leader fail-over, race conditions, partial failures, cascading
   failures, et la classe spécifique **"single-tenant N replica
   amplification"** où un client avec un mauvais retry-loop hammer
   les N replicas en parallèle.

6. **Cross-sprint integration audit** : revue des invariants
   D11/D12/D12-bis/D13/D13-bis/D14/D14-bis/D15/D16/D17 sous
   l'angle "qu'est-ce qui change quand N=3 au lieu de N=1 ?".

**Non-objectifs** : multi-region active-active (D19+ candidate),
sharded catalog (tenant routing par hash → un replica par shard),
auto-scaling vertical pod (out of scope ; HPA horizontal seulement),
read-replica Postgres (D14-bis l'avait positionné comme out-of-scope
+ en pratique le PG primary single-instance est le SPOF restant à
adresser en D19).

---

## 1. Pourquoi ce sprint maintenant

### 1.1 Pain point HA

EFE en `replicas: 1` :

```
[Client] ──gRPC──▶ [LB Service] ──▶ [efe-server-pod-0]
                                          │
                                          └─ unique runtime state
                                            (idem cache, rate buckets,
                                             bundle in-mem, audit signer)
```

Si `efe-server-pod-0` crash :

* **k8s detection time** : 10-30 s (defaut liveness probe period)
* **k8s respawn time** : 5-15 s (Init + bundle load + PG warmup)
* **gRPC client retry+backoff** : 1-5 s par retry, jusqu'à 30 s
* **Total perceived downtime** : 30-60 s

Pour un client Tier 1 avec un SLA 99.95 % monthly :
* Budget downtime mensuel : 21 min 56 s
* Un seul incident de 30 s consomme 2.3 % du budget mensuel
* **2-3 incidents par mois → SLA breach contractuelle**

### 1.2 Pain point throughput

Profile observé d'un pod single-replica saturated :

```
Component                CPU usage at 600 Execute RPS
─────────────────────────────────────────────────────
embedder.encode (D16)    ~45 % (ONNX session)
core.swarm.run            ~25 % (M3-M8 fusion)
proto.serialize           ~10 % (tonic + prost)
auth.oidc.validate (D15)   ~5 % (RS256 verify, JWKS hot)
metrics + spans           ~10 % (prom + tracing)
audit.emit (D14-bis)       ~5 % (HMAC sign)
                          ────
                          ~100 % saturated
```

Au-delà de 600 RPS Execute, p95 latency dégrade vite (queue depth
exhaustion). Les Tier 1 clients projettent 1500-3000 RPS au pic
pour Q4 2026 (e-commerce peak season). **Single-replica est
incompatible avec leur projection.**

Solution : multi-replica horizontal scaling avec load-balancing
au niveau gRPC. 3 replicas = 1800 RPS sustainable avec headroom
pour pic à 2400 RPS, et room to grow à 6+ replicas via HPA.

### 1.3 Pain point operational

Rolling upgrade actuel (`helm upgrade`) avec `replicas: 1` =
**downtime garanti** pendant la rotation du pod (typiquement 30-60 s).
Tier 1 clients ne tolèrent pas les fenêtres de maintenance
unplanned. Multi-replica + `maxUnavailable: 1` sur le Deployment
permet rolling upgrade **sans downtime**.

### 1.4 Pain point bench

D12 a livré le catalog admin il y a 4 sprints majeurs. **La perf
n'a jamais été mesurée formellement.** On sait que les opérations
fonctionnent (tests verts) mais on n'a pas de chiffre p95/p99/RPS
peak documenté pour répondre aux questions client :

> "Combien de tenants register/seconde votre plateforme tient ?"
> "Quel est votre p99 sur `resolve_catalog` ?"

Réponses pre-D18 : "On ne sait pas, ça marche dans nos tests
unitaires." Posture insoutenable en cycle de vente B2B Tier 1.

D18 produit des chiffres formels via `efe-bench` + un report
auto-régénéré à chaque release candidate.

---

## 2. Threat model

12 menaces nouvelles ou amplifiées par le passage à N>1 replica.

### 2.1 Liste exhaustive

| ID | Menace | Vecteur | Sévérité | Probabilité | Mitigation D18 |
|----|--------|---------|:--------:|:-----------:|----------------|
| **T-D18-1** | Split-brain leader election | 2 replicas se croient simultanément leader pour un background task → audit chain double-write, double GC | 🔴 CRIT | 🟠 MED | K8s Lease object (coordination.k8s.io/v1) avec lease-renew-deadline strict |
| **T-D18-2** | Idempotency cache miss cross-replica | Client retry sur replica B après que A a déjà traité ; sans shared cache, B re-traite → effets de bord doublés (audit row, rate-limit decrement) | 🔴 CRIT | 🟠 HIGH | `efe_idempotency` table PG avec UNIQUE(request_id) + UPSERT semantics |
| **T-D18-3** | Rate-limit bypass | Client routé sur des replicas différents par le LB → effective bucket = N × bucket-size → DoS sur un tenant | 🟠 HARD | 🟠 HIGH | `efe_ratelimit_buckets` table PG avec pessimistic lock per-tenant + leak-rate refresh |
| **T-D18-4** | Audit chain race condition | 2 replicas écrivent simultanément dans la même chain → seq race → HMAC chain divergent | 🔴 CRIT | 🟠 MED | D14-bis advisory lock per-chain déjà en place ; D18 doit confirmer cross-replica via integration test |
| **T-D18-5** | JWKS cache stampede | Token avec kid rotated arrive sur N replicas → N fetches concurrents au IdP → DoS du IdP | 🟠 HARD | 🟡 LOW | Per-replica cache acceptable (rate-limited refresh) ; documenter le worst case (N replicas × 1 fetch / kid-rotation event) |
| **T-D18-6** | Bundle memory amplification | D16 bundle 5 GiB chargé in-memory × N replicas = 5N GiB → OOM sur cluster tight | 🟠 HARD | 🟠 MED | `EmbedderRegistry` shared via tmpfs mount + mmap (option différée) ; pour D18 ship la solution naive (5 GiB × N) avec resource requests calibrés |
| **T-D18-7** | Single-tenant N-replica amplification | Client mal configuré (retry-loop infini sur erreur transitoire) hammer les N replicas en parallèle → audit log noise + DB load | 🟠 HARD | 🟠 HIGH | Rate-limit shared (T-D18-3 mitigation) catch ce cas |
| **T-D18-8** | Graceful shutdown asymmetry | Pod A reçoit SIGTERM, refuse new requests, drain in-flight ; pod B accepte. Mais le client a une connection persistante à A → 30 s de erreurs au lieu de re-route immédiate. | 🟠 HARD | 🟠 MED | Tonic `GOAWAY` frame envoyé au début du grace period + `terminationGracePeriodSeconds: 60` |
| **T-D18-9** | PG connection pool exhaustion | N replicas × `pool_max=20` = 60 connexions vers PG. PG `max_connections=100` par défaut. 6 replicas = 120 → exhaustion. | 🟠 HARD | 🟠 MED | Helm values param `pool_max` à reduce automatically par replica count, ou pgbouncer en sidecar |
| **T-D18-10** | Cone GC double-trigger | Background task `cone_gc_periodic` spawned dans 2 replicas → DELETE concurrent sur la même row → constraint violation OR row vanish race | 🔴 CRIT | 🟠 MED | Leader election via K8s Lease (T-D18-1 mitigation) |
| **T-D18-11** | Deadlock on shutdown ordering | Pod A drain attend `pg_pool.close()` ; pg_pool attend l'AuditEmitter en cours d'écriture ; AuditEmitter attend pg_pool. → deadlock 60 s timeout | 🟠 HARD | 🟡 LOW | Drain order documented : RPC handlers → AuditEmitter flush → pg_pool close. Timeout strict per stage (10/10/30 s) |
| **T-D18-12** | Bench leakage en prod | `efe-bench` invoked en prod par accident → traffic generation amplifié → cluster overload | 🟡 MED | 🟡 LOW | `efe-bench` exige `EFE_BENCH_TARGET_ENV != prod` ; refuse de tourner sinon |

### 2.2 Matrix sévérité × probabilité

```
                 Sévérité
            CRIT       HARD       MED        LOW
         ┌─────────┬─────────┬─────────┬─────────┐
HIGH    │         │ T-D18-2 │         │         │
        │         │ T-D18-3 │         │         │
        │         │ T-D18-7 │         │         │
         ├─────────┼─────────┼─────────┼─────────┤
MED     │ T-D18-1 │ T-D18-6 │         │         │
        │ T-D18-4 │ T-D18-8 │         │         │
        │T-D18-10 │ T-D18-9 │         │         │
         ├─────────┼─────────┼─────────┼─────────┤
LOW     │         │T-D18-11 │ T-D18-5 │         │
        │         │         │T-D18-12 │         │
         └─────────┴─────────┴─────────┴─────────┘
```

**Priorité fix** : T-D18-2 (CRIT × HIGH) et T-D18-3 (HARD × HIGH).
T-D18-1, T-D18-4, T-D18-10 (CRIT × MED) en deuxième rang.

---

## 3. Requirements

### 3.1 Fonctionnels (R-D18-*)

* **R-D18-1** — Le Helm chart doit supporter `replicas: N` avec
  N ∈ [1, 20]. `N=1` reste valide pour dev clusters.
* **R-D18-2** — `kubectl rollout status deployment/efe-server`
  doit montrer 0 unavailable pendant un rolling upgrade
  (zero-downtime).
* **R-D18-3** — Le LB Service doit distribuer le traffic uniformément
  (round-robin gRPC niveau connexion ; client-side LB recommandé
  via `headless service` + DNS SRV).
* **R-D18-4** — `efe_idempotency` table sert tous les replicas ;
  une requête avec `request_id=X` traitée par replica A retourne
  la même réponse depuis replica B sans re-exécuter le handler.
* **R-D18-5** — `efe_ratelimit_buckets` table sert tous les
  replicas ; un tenant configuré pour `capacity=100, refill=10/s`
  voit son bucket décrémenté de manière atomique cross-replica.
* **R-D18-6** — Les background tasks (HMAC self-verify periodic,
  cone GC, idempotency expiry sweep) tournent **exactement
  une fois** dans le cluster, pas N fois.
* **R-D18-7** — Le bench harness `efe-bench` doit être exécutable
  contre :
  * Un cluster local kind (CI workflow) ;
  * Un cluster staging (manuel) ;
  * **JAMAIS** prod (refus dur).
* **R-D18-8** — Le bench produit un report Markdown
  `docs/bench/d18_bench_results.md` avec p50/p95/p99 par
  opération, RPS sustainable, comparaison vs SLO targets.
* **R-D18-9** — La graceful shutdown doit drain les requêtes
  in-flight pendant ≤ `EFE_SHUTDOWN_DRAIN_MS` (default 10 s),
  puis flush l'audit emitter pendant ≤ 10 s, puis close le pg
  pool pendant ≤ 30 s. Total ≤ `terminationGracePeriodSeconds`
  K8s (60 s).
* **R-D18-10** — Liveness probe vérifie : process up + gRPC
  responding to `Health.Check`. Readiness probe vérifie en plus :
  PG pool established + bundle loaded + (si OIDC enabled) au
  moins un JWKS pré-fetché.

### 3.2 Non-fonctionnels (N-D18-*)

* **N-D18-1** — Multi-replica linear speedup ≥ 0.85 jusqu'à 8
  replicas (8 replicas → 6.8× single-replica throughput, pas
  8×, parce que le PG devient le contention).
* **N-D18-2** — Latency p99 dégrade ≤ 5 % en passant de 1 à 3
  replicas (le shared cache PG ajoute un round-trip de ~1 ms).
* **N-D18-3** — Bundle memory : `5 GiB × N` resource request
  documenté ; operator doit calibrer le node size en conséquence
  ou désactiver les bundles.
* **N-D18-4** — Bench duration : full bench suite ≤ 30 min
  wall-clock contre 1 + 3 replicas comparison.
* **N-D18-5** — `efe_idempotency` table cleanup : rows expirent
  après 24 h, pruned par background task.
* **N-D18-6** — `efe_ratelimit_buckets` table : 1 row par tenant ;
  pas d'historique conservé.

### 3.3 SLOs (D18-bench-*)

Mesurés par `efe-bench` à chaque CI release-candidate :

* **D18-bench-1** — `Execute` p99 latency ≤ 200 ms à 1000 RPS
  sustained.
* **D18-bench-2** — `register_catalog` p95 ≤ 50 ms à 100 RPS
  (write-heavy workload).
* **D18-bench-3** — `resolve_catalog` p99 ≤ 20 ms à 5000 RPS
  (read-heavy workload, idempotency cache hot).
* **D18-bench-4** — `list_catalogs` p95 ≤ 100 ms quelle que soit
  la cardinality (jusqu'à 100k catalogs).
* **D18-bench-5** — `register_class` + `register_sentence`
  combined throughput ≥ 200 ops/s sustained sur 5 min.
* **D18-bench-6** — Multi-replica linear speedup ≥ 0.85 entre
  1 et 3 replicas pour la suite Execute.
* **D18-bench-7** — Audit chain HMAC sign latency p99 ≤ 5 ms
  (validation que D14-bis ne devient pas le bottleneck cross-replica).

### 3.4 Compliance

* **C-D18-1** — Le `terminationGracePeriodSeconds` (60 s) doit
  permettre le flush de l'audit chain avant kill SIGKILL —
  sinon `audit_log` corrompue avec un row "in-flight" non-signé.
  RGPD Art. 32 (confidentialité + integrity du traitement).
* **C-D18-2** — Multi-replica est une exigence **contractuelle**
  pour Tier 1 SLAs ≥ 99.95 % uptime. SOC 2 Type II demande
  "redundant infrastructure" attestable.

---

## 4. Inventaire des state per-replica vs shared

Audit chirurgical du runtime efe-server pour identifier ce qui
DOIT migrer en shared store sous N>1.

| State | Type pre-D18 | Status sous N>1 | Action D18 |
|-------|--------------|-----------------|------------|
| Postgres connexions (D14) | per-replica pool | OK (déjà shared via PG) | Reduce `pool_max` automatically by replica count to avoid PG `max_connections` exhaustion |
| Audit log writes (D14-bis) | PG transactional | OK (advisory lock per-chain) | Confirm cross-replica integration test |
| HMAC keys (D14-bis) | per-replica copy chargé au boot | OK (immutable post-boot) | Pas d'action |
| OIDC JWKS cache (D15) | per-replica in-mem | Eventually consistent (TTL 1h) — acceptable | Document cache miss amplification (worst case 1 fetch / kid rotation × N replicas) |
| TenantStore cache (D15) | per-replica in-mem 30s TTL | Eventually consistent — acceptable | Pas d'action |
| Embedder bundle (D16) | per-replica in-mem 5 GiB | **MEMORY × N AMPLIFICATION** | Document resource requests ; future tmpfs mmap (D19+) |
| Embedder Registry singleflight (D16) | per-replica | OK (each replica dedups its own load) | Pas d'action |
| **Idempotency cache (D11)** | per-replica DashMap 5 min TTL | **CACHE MISS sur retry cross-replica** = double execution | **MIGRATE → `efe_idempotency` PG table** (T-D18-2 mitigation) |
| **Rate-limit buckets (D11)** | per-replica DashMap | **N-replica amplification** = bypass | **MIGRATE → `efe_ratelimit_buckets` PG table** (T-D18-3 mitigation) |
| Cone register-event channel (M9) | per-replica mpsc | OK (chaque replica a son audit sink + telemetry) | Pas d'action |
| Memory pool (M3) | per-request | Stateless | Pas d'action |
| Stream pool (M4) | per-request | Stateless | Pas d'action |
| Genesis engine (M5) | per-request | Stateless | Pas d'action |
| Telemetry recorder (M11) | per-replica Prometheus | Aggregated by Prometheus scrape | Pas d'action ; just confirm no per-pod cardinality drift |
| OTel TracerProvider (D13-bis) | per-replica OtelGuard | Each replica exports independently | Pas d'action |
| **Background task : HMAC self-verify (D14-bis)** | per-replica `tokio::spawn` loop | **MULTIPLE WRITERS** | **Leader election K8s Lease** (T-D18-1 mitigation) |
| **Background task : Cone GC (M9)** | non implémenté pre-D18 | (N/A) | À ajouter avec leader election (T-D18-10 mitigation) |
| **Background task : Idempotency expiry sweep (D18 nouveau)** | nouveau | Nouveau path | Leader election |

### 4.1 Synthèse

* **3 migrations vers PG-shared** : idempotency, rate-limit,
  background tasks coordination via K8s Lease.
* **1 amplification documentée non-mitigée** : embedder bundle
  memory (différé D19+).
* **1 amplification documentée acceptable** : JWKS cache stampede
  (worst case rare, rate-limited).

### 4.2 Diagramme global

```
┌─────────────────────────────────────────────────────────────────┐
│ K8s cluster (cluster-local LB)                                  │
│                                                                 │
│   ┌──────────┐   ┌──────────┐   ┌──────────┐                    │
│   │efe-server│   │efe-server│   │efe-server│  ← N=3 replicas    │
│   │  pod-0   │   │  pod-1   │   │  pod-2   │                    │
│   └────┬─────┘   └────┬─────┘   └────┬─────┘                    │
│        │              │              │                          │
│        │ per-replica :                                          │
│        │   * embedder bundle in-mem (5 GiB)                     │
│        │   * JWKS cache (small, eventually consistent)          │
│        │   * TenantStore cache (30s TTL)                        │
│        │   * OtelGuard + tracing-subscriber                     │
│        │                                                        │
│        ▼              ▼              ▼                          │
│   ┌──────────────────────────────────────────┐                  │
│   │ shared via Postgres :                    │                  │
│   │   * efe_idempotency (NEW D18)            │                  │
│   │   * efe_ratelimit_buckets (NEW D18)      │                  │
│   │   * audit_log (D14-bis HMAC chain)       │                  │
│   │   * catalogs / classes / sentences (D14) │                  │
│   │   * tenants (D15)                        │                  │
│   │   * efe_backup_history (D17)             │                  │
│   └──────────────────────────────────────────┘                  │
│                                                                 │
│   ┌──────────────────────────────────────────┐                  │
│   │ K8s coordination :                       │                  │
│   │   * Lease/efe-leader (NEW D18) → only    │                  │
│   │     one replica runs background tasks    │                  │
│   └──────────────────────────────────────────┘                  │
│                                                                 │
│   shared via Vault (D17) :                                      │
│     * HMAC keys, OIDC issuers, signing keys                     │
└─────────────────────────────────────────────────────────────────┘
```

---

## 5. State management : decision tree

Pour chaque shared state, choix entre 3 patterns :

* **A — PG transactional** : ACID, row-level locking, latency
  ~1-3 ms intra-cluster. Bon pour write-heavy avec invariants
  forts.
* **B — Redis (or Memcached)** : sub-ms latency, TTL natif.
  Bon pour cache haute fréquence read-mostly avec eventual
  consistency.
* **C — Per-replica avec sync barrier** : performances per-replica
  + reconciliation périodique vers PG. Bon pour rate-limit
  approximatif.

Pour D18, choix **A (PG)** pour idempotency + rate-limit, parce que :

* Pas de dep additionnelle (Redis n'est pas déployé) ;
* Latence acceptable (~1-3 ms intra-cluster sur le PG primary
  qui est cluster-local) ;
* Atomicité native via `FOR UPDATE` row locks ;
* Cohérence avec le reste du runtime qui utilise PG ;
* Simplicité ops (1 seul système à monitorer).

Trade-off accepté : ~1 ms additionnel par requête checkée pour
idempotency + rate-limit (typically < 5 % of total latency budget).

### 5.1 Pourquoi pas Redis

Architecture alternative envisagée : Redis Cluster en sidecar.

Pros :
* Sub-ms latency
* Out-of-the-box TTL + atomic INCR

Cons (qui l'emportent pour D18) :
* Dep additionnelle (sentinel, persistence, monitoring)
* SPOF additionnel (master Redis crash → all-replica idempotency
  + rate-limit perdent)
* Ops complexity : ScalableSet of sentinels, persistence sync,
  upgrade orchestration
* Cost : managed Redis (Elasticache, Memorystore) ~$50-200/mois

Décision : **PG-only D18**. Migration vers Redis envisageable en
D20+ si profile montre le PG comme bottleneck.

### 5.2 Pourquoi pas etcd

K8s utilise etcd. Tempting d'écrire idempotency + rate-limit
dans des CRDs ou des ConfigMaps. Refusé parce que :

* Latency etcd ≈ 5-10 ms (largement plus lent que PG)
* etcd pas conçu pour transactions row-level high-frequency
* Ops cluster-mgmt risk : remplir etcd avec des données de
  runtime corromp le cluster lui-même

---

## 6. Idempotency cache — design détaillé

### 6.1 Schema SQL (`migrations/0007_d18_idempotency.sql`)

```sql
CREATE TABLE efe_idempotency (
    request_id      UUID         PRIMARY KEY,
    tenant_id       TEXT         NOT NULL,
    method          TEXT         NOT NULL,         -- gRPC method name
    request_hash    BYTEA        NOT NULL,         -- SHA-256 of canonical request body
    response_blob   BYTEA        NULL,             -- nullable while in-flight
    response_status TEXT         NULL,             -- "ok" | "error" | "timeout"
    started_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ  NULL,
    expires_at      TIMESTAMPTZ  NOT NULL,
    -- Cross-field invariants :
    CONSTRAINT chk_completed_consistency CHECK (
        (completed_at IS NULL AND response_blob IS NULL AND response_status IS NULL)
        OR
        (completed_at IS NOT NULL AND response_status IS NOT NULL)
    ),
    CONSTRAINT chk_expires_after_started CHECK (expires_at > started_at)
);

CREATE INDEX idx_idem_expires ON efe_idempotency (expires_at);
CREATE INDEX idx_idem_tenant_started ON efe_idempotency (tenant_id, started_at DESC);
```

### 6.2 Workflow

```
Request arrives at replica A with `request_id = X` :

1. INSERT ... ON CONFLICT DO NOTHING RETURNING id
   → if INSERTED : we are the leader for this request_id ; proceed.
   → if NOT INSERTED (conflict) : we are a follower ; goto step 4.

2. Execute the actual handler.

3. UPDATE efe_idempotency
   SET response_blob = $1, response_status = 'ok',
       completed_at = NOW()
   WHERE request_id = $X;

4. (follower path) SELECT response_blob, response_status,
                          completed_at
   FROM efe_idempotency WHERE request_id = $X;
   → if completed_at IS NULL : in-flight, wait+poll ;
   → if response_status = 'ok' : return cached response_blob ;
   → if response_status = 'error' : return cached error.
```

### 6.3 In-flight wait strategy

Si replica B reçoit un retry pendant que replica A est en train
de traiter `request_id=X`, replica B doit attendre. Stratégie :

* **Polling** : SELECT every 50 ms, max 5 s wait, then fail with
  `IdempotencyTimeout`. Simple, suffit pour les requêtes typiques
  (Execute < 200 ms).
* Pas de `LISTEN/NOTIFY` (complexité + ne survit pas à un kill).

### 6.4 Background expiry

Tâche leader-only :

```rust
async fn idempotency_expiry_sweep(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let res = sqlx::query!(
        "DELETE FROM efe_idempotency
         WHERE expires_at < NOW() - INTERVAL '1 hour'"
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
```

Tourne toutes les 10 min via le leader. Cleanup buffered 1h
post-expiry pour tolérer les drifts d'horloges.

### 6.5 Cardinality

Estimation : 100 RPS × 24h × 3600 s = 8.64M rows/jour. Avec TTL
24h, steady-state ~10M rows. Index `idx_idem_expires` permet le
DELETE en O(log n). Acceptable.

### 6.6 Tests

* Unit : `idempotency_dedup_cross_replica` (2 PgPools simulant 2
  replicas, requête concurrente)
* Integration : `idempotency_in_flight_polling_race`
* Property : `idempotency_response_byte_exact` (no JSON re-encode)

---

## 7. Rate-limit buckets — design détaillé

### 7.1 Schema SQL (`migrations/0007_d18_idempotency.sql` continued)

```sql
CREATE TABLE efe_ratelimit_buckets (
    tenant_id       TEXT         PRIMARY KEY,
    capacity        INTEGER      NOT NULL,         -- bucket max
    refill_per_sec  REAL         NOT NULL,         -- leak rate
    available       REAL         NOT NULL,         -- current tokens
    last_refill_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    -- Invariant : available ∈ [0, capacity]
    CONSTRAINT chk_available_in_range CHECK (available >= 0 AND available <= capacity),
    CONSTRAINT chk_capacity_positive CHECK (capacity > 0),
    CONSTRAINT chk_refill_non_negative CHECK (refill_per_sec >= 0)
);

CREATE INDEX idx_ratelimit_last_refill ON efe_ratelimit_buckets (last_refill_at);
```

### 7.2 Algorithm : leaky bucket cross-replica

```rust
async fn rate_limit_check(
    pool: &PgPool,
    tenant_id: &str,
    cost: f64,
) -> Result<RateLimitDecision, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // 1. Lock the row for this tenant.
    let row = sqlx::query!(
        "SELECT capacity, refill_per_sec, available, last_refill_at
         FROM efe_ratelimit_buckets
         WHERE tenant_id = $1
         FOR UPDATE",
        tenant_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    let row = match row {
        Some(r) => r,
        None => {
            // Tenant not configured ; default policy.
            tx.commit().await?;
            return Ok(RateLimitDecision::Allowed);
        }
    };

    // 2. Refill based on elapsed time.
    let now = Utc::now();
    let elapsed_secs = (now - row.last_refill_at).num_seconds() as f64;
    let new_available = (row.available
        + (elapsed_secs * row.refill_per_sec))
        .min(row.capacity as f64);

    // 3. Check + decrement.
    if new_available < cost {
        // Persist refill but reject.
        sqlx::query!(
            "UPDATE efe_ratelimit_buckets
             SET available = $1, last_refill_at = $2
             WHERE tenant_id = $3",
            new_available, now, tenant_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(RateLimitDecision::Throttled {
            available: new_available,
            retry_after_ms: ((cost - new_available) / row.refill_per_sec * 1000.0)
                .ceil() as u64,
        });
    }

    // 4. Allow + decrement.
    let after = new_available - cost;
    sqlx::query!(
        "UPDATE efe_ratelimit_buckets
         SET available = $1, last_refill_at = $2
         WHERE tenant_id = $3",
        after, now, tenant_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(RateLimitDecision::Allowed)
}
```

### 7.3 Trade-offs

* **Pessimistic locking** (`FOR UPDATE`) garantit l'atomicité
  cross-replica mais sérialise les checks pour un même tenant.
  Avec 100 tenants × 1000 RPS chacun = 100k QPS lookup. PG peut
  tenir mais c'est tendu.
* **Alternative** : optimistic locking via `xmin` ou version
  number. Plus rapide, mais plus complexe (retry-on-conflict).
* **Décision D18** : ship pessimistic. Si le profile montre PG
  contention, switch optimistic en D18-bis.

### 7.4 Cardinality

1 row per tenant. ~1k tenants → 1k rows. Trivial.

### 7.5 Tests

* `ratelimit_cross_replica_no_burst` (100 RPS sustained vs 100 RPS
  effective, mesure le bucket cross-replica)
* `ratelimit_concurrent_decrement_atomic`
* `ratelimit_backfill_after_idle`

---

## 8. Leader election — K8s Lease

### 8.1 Why K8s Lease

Le projet déploie déjà sur K8s. Les CRDs `coordination.k8s.io/v1
Lease` sont built-in, pas de dep additionnelle. Pattern canonique
pour single-leader pour background tasks.

### 8.2 Architecture

```rust
use kube::{
    api::{Api, ListParams},
    Client,
};
use k8s_openapi::api::coordination::v1::Lease;

pub struct LeaderElection {
    client: Client,
    name: String,            // e.g. "efe-leader"
    namespace: String,       // typically "efe"
    holder_id: String,       // unique per replica (HOSTNAME = pod name)
    lease_duration: Duration, // typically 15 s
    renew_deadline: Duration, // typically 10 s
}

impl LeaderElection {
    /// Run the leader-election loop. The closure is invoked
    /// EACH TIME this replica becomes leader, and is given a
    /// shutdown signal that fires when leadership is lost.
    pub async fn run<F, Fut>(&self, on_lead: F) -> Result<(), kube::Error>
    where
        F: Fn(tokio::sync::watch::Receiver<bool>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        loop {
            // 1. Try acquire the lease.
            match self.try_acquire().await {
                Ok(true) => {
                    // 2. We are leader. Spawn the closure with a
                    //    shutdown channel.
                    let (tx, rx) = tokio::sync::watch::channel(true);
                    let task = tokio::spawn(on_lead(rx));

                    // 3. Renew the lease periodically.
                    while self.renew().await.is_ok() {
                        tokio::time::sleep(self.renew_deadline / 2).await;
                    }

                    // 4. Lost lease ; signal shutdown.
                    let _ = tx.send(false);
                    let _ = task.await;
                }
                Ok(false) | Err(_) => {
                    // Not leader ; back off + retry.
                    tokio::time::sleep(self.lease_duration).await;
                }
            }
        }
    }

    async fn try_acquire(&self) -> kube::Result<bool> { /* ... */ }
    async fn renew(&self) -> kube::Result<()> { /* ... */ }
}
```

### 8.3 RBAC requirements

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: efe-server-leader-election
rules:
  - apiGroups: ["coordination.k8s.io"]
    resources: ["leases"]
    verbs: ["get", "create", "update", "delete"]
    resourceNames: ["efe-leader"]   # principle of least privilege
```

### 8.4 Failure modes

* **K8s API down** : leader can't renew → loses leadership →
  no replica is leader → background tasks pause. Acceptable
  (K8s API is the platform's SPOF anyway).
* **Network partition** : split-brain risk. Mitigated by the
  Lease's `renewTime` strict deadline (10 s) ; an old leader
  whose API connection is partitioned will fail to renew within
  10 s, drop leadership, and stop background tasks before a
  new leader is elected. Window of double-leadership ≤ 5 s.

### 8.5 Background tasks running under leader

| Task | Source sprint | Pre-D18 location | D18 location |
|------|---------------|------------------|--------------|
| HMAC self-verify periodic | D14-bis | `runtime.rs::serve` | leader-only |
| Cone GC | (proposed) | non-impl | leader-only (NEW D18) |
| Idempotency expiry sweep | D18 NEW | (N/A) | leader-only |
| Rate-limit decay sweep | (optional) | (N/A) | leader-only (optional) |

---

## 9. Audit chain integrity sous N>1

### 9.1 Threat T-D18-4

D14-bis garantit l'intégrité de la HMAC chain via `pg_advisory_lock`
per chain (advisory lock acquire avant chaque INSERT). Le contract
est : un seul writer concurrent par chain.

Sous N replicas :

* Replica A et B reçoivent simultanément une requête générant un
  audit row pour `chain_id = "tenant:acme"`.
* A acquire le advisory lock → INSERT row (seq=42) → commit → release.
* B était bloqué sur le advisory lock → reprend → INSERT row (seq=43)
  → commit → release.

Conclu : **D14-bis advisory-lock pattern fonctionne déjà
cross-replica**. Pas de changement nécessaire au runtime.

### 9.2 Verification via integration test

Test à ajouter en D18 :

```rust
#[tokio::test]
async fn audit_chain_intact_under_concurrent_replica_writes() {
    // Spin 4 PgPools (simulating 4 replicas) against the same DB.
    let pools = vec![pool_clone(); 4];

    // Each replica writes 100 audit rows for the same chain.
    let handles: Vec<_> = pools
        .into_iter()
        .map(|p| tokio::spawn(async move {
            for i in 0..100 {
                emit_audit(&p, "tenant:acme", &format!("event-{i}")).await;
            }
        }))
        .collect();

    // Wait for all replicas.
    for h in handles {
        h.await.unwrap();
    }

    // Verify the chain is intact end-to-end.
    let result = verify_chain(&pools[0], "tenant:acme").await.unwrap();
    assert!(result.intact);
    assert_eq!(result.n_events, 400);
}
```

---

## 10. Bundle memory amplification (T-D18-6)

### 10.1 Le problème

D16 ship un embedder bundle (~5 GiB) chargé in-memory par chaque
pod. Avec N=3 replicas, c'est 15 GiB de RAM **dédupliquée par
le runtime** (pas par l'OS — chaque pod a son propre Vec<u8>).

### 10.2 Solutions envisagées

| Option | Pros | Cons | Décision |
|--------|------|------|----------|
| **A — Status quo** : 5 GiB × N | Simple, isolation totale | Cluster-side mémoire ×N | **D18 default** (avec resource calibration) |
| **B — `tmpfs` mount** + `mmap` | Memory-shared via Linux page cache, copie unique cluster-side | Complexe ops, requires PrivilegeEscalation pour mmap | Différé D19 |
| **C — Bundle service** : 1 sidecar par node, exposed via Unix socket | Isolation per-node, partage intra-node | Architecture nouveau composant | Out of scope |
| **D — Bundles plus petits** : pruner les modèles inutiles | Moins de mémoire global | Out of scope D18 | D19 candidate |

### 10.3 Resource calibration D18

Avec option A (status quo), D18 ship une recommandation Helm
values :

```yaml
resources:
  requests:
    memory: "8Gi"      # 5 GiB bundle + 3 GiB working set
    cpu: "1000m"
  limits:
    memory: "10Gi"     # +20% headroom
    cpu: "2000m"
```

Operator sur cluster tight (< 32 GiB nodes) doit :
* Réduire `replicas` à 2 ;
* OU désactiver les bundles (`backend: legacy`, D16) ;
* OU upgrade les nodes.

Documenté dans le Helm chart NOTES.txt + howto deployment doc.

---

## 11. Graceful shutdown sous N>1

### 11.1 Sequencing

K8s envoie SIGTERM au pod. Le `terminationGracePeriodSeconds`
(60 s default D18) borne le total wall-clock avant SIGKILL.

D18 séquence le shutdown en 3 phases :

```
T+0    SIGTERM reçu
       │
       ├─► tonic GOAWAY frame envoyé (clients re-route vers autres replicas)
       │
T+10s  Phase 1 : drain RPC handlers en cours
       │   * tonic shutdown (max 10s)
       │   * any RPC encore en cours peut continuer à compléter
       │
T+20s  Phase 2 : flush AuditEmitter (D14-bis HMAC chain pending writes)
       │   * `audit_emitter.drain_pending(timeout=10s)` (D14-bis pattern)
       │
T+50s  Phase 3 : close pg_pool + flush OTel BatchSpanProcessor
       │   * pg_pool.close() (max 30s)
       │   * OtelGuard::force_flush(10s) (D13-bis pattern)
       │
T+60s  SIGKILL (si pas déjà exit)
```

### 11.2 Critical invariant

**L'audit chain DOIT être flushed avant la fermeture du pg pool.**
Sinon les rows in-flight (HMAC computed mais INSERT pas encore
committed) sont perdus → chain "trou" → D14-bis verify fail.

Le sequencing ci-dessus garantit cette invariant.

### 11.3 Tonic GOAWAY

`tonic 0.12` envoie `GOAWAY` automatiquement sur shutdown
graceful. Les clients gRPC bien-implémentés re-route vers un
autre replica via le LB Service. Latency ≤ 100 ms typique.

---

## 12. Load balancing

### 12.1 Choix : headless service + client-side LB

K8s deux options pour un Service gRPC :

* **ClusterIP** : un seul Virtual IP, kube-proxy iptables route
  per-connection. **Problème** : gRPC over HTTP/2 multiplexe
  plusieurs requests sur une connection ; un client qui ouvre
  une connection à T0 reste pinned à un replica jusqu'à
  reconnect. → Load mal distribué sous fort volume.
* **Headless Service** (`clusterIP: None`) : le DNS résout vers
  les Pod IPs directement. Le client gRPC fait du client-side LB
  (round-robin sur les IPs). → Distribution équilibrée par-request.

D18 **headless service** par défaut, avec ClusterIP fallback
configurable :

```yaml
apiVersion: v1
kind: Service
metadata:
  name: {{ include "efe.fullname" . }}-grpc-headless
spec:
  clusterIP: None              # headless
  selector:
    app.kubernetes.io/name: efe-server
  ports:
    - port: 50051
      targetPort: grpc
      protocol: TCP
      name: grpc
```

Client gRPC discovery via DNS SRV records. Tonic supporta-t-il
out-of-the-box ? Oui via `tonic-resolver-dns` ou un balancer custom.
D18 ship une note dans la doc client expliquant comment configurer
un client gRPC pour utiliser le headless service.

### 12.2 HPA (HorizontalPodAutoscaler)

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: {{ include "efe.fullname" . }}-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: {{ include "efe.fullname" . }}
  minReplicas: {{ .Values.replicas.min }}    # default 3
  maxReplicas: {{ .Values.replicas.max }}    # default 10
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70             # target 70% CPU
    - type: Pods
      pods:
        metric:
          name: efe_grpc_request_duration_seconds
        target:
          type: AverageValue
          averageValue: 100m                 # 100ms p95 trigger scale-up
  behavior:
    scaleUp:
      stabilizationWindowSeconds: 30
      policies:
        - type: Percent
          value: 50
          periodSeconds: 60
    scaleDown:
      stabilizationWindowSeconds: 300        # slower scale-down
```

---

## 13. Health probes

### 13.1 Liveness probe

Vérifie que le process est responsive. Si fail → kill+restart.

```yaml
livenessProbe:
  exec:
    command:
      - /usr/local/bin/grpc_health_probe
      - -addr=:50051
      - -tls=false
  initialDelaySeconds: 30          # bundle load can take 20s
  periodSeconds: 10
  timeoutSeconds: 5
  failureThreshold: 3              # 3×10s = 30s before restart
```

### 13.2 Readiness probe

Vérifie en plus que le pod est prêt à recevoir du traffic. Si
fail → retiré du LB Service mais pas killed.

```yaml
readinessProbe:
  exec:
    command:
      - /usr/local/bin/grpc_health_probe
      - -addr=:50051
      - -service=efe.health.v1
  initialDelaySeconds: 30
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 2
```

Le service `efe.health.v1.Health.Check` (à implémenter D18) doit
return `SERVING` uniquement si :
* PG pool established (1 successful `SELECT 1`)
* Bundle loaded (si OIDC or D16 active)
* JWKS pré-fetché (si OIDC active)
* (Optional) audit chain integrity verified au boot

---

## 14. Helm chart D18 — changements

### 14.1 `deploy/helm/efe/values.yaml` — D18 block

```yaml
# =====================================================================
# Sprint D18 — Multi-réplica
# =====================================================================
replicas:
  min: 3
  max: 10

podAntiAffinity:
  enabled: true
  type: preferredDuringSchedulingIgnoredDuringExecution
  weight: 100

topologySpreadConstraints:
  enabled: true
  whenUnsatisfiable: ScheduleAnyway
  topologyKey: topology.kubernetes.io/zone

podDisruptionBudget:
  enabled: true
  minAvailable: 2

hpa:
  enabled: true
  cpuTarget: 70
  p95LatencyTargetMs: 100
  scaleUpStabilizationSeconds: 30
  scaleDownStabilizationSeconds: 300

# Headless service for client-side gRPC load-balancing.
service:
  headless: true                    # was implicitly false pre-D18
  port: 50051

# Graceful shutdown (D18-tuned).
terminationGracePeriodSeconds: 60

shutdown:
  drainMs: 10000                    # phase 1
  auditFlushMs: 10000               # phase 2
  pgCloseMs: 30000                  # phase 3

# Resource requests / limits calibrated for D16 bundles.
resources:
  requests:
    memory: "8Gi"
    cpu: "1000m"
  limits:
    memory: "10Gi"
    cpu: "2000m"

leaderElection:
  enabled: true
  leaseDurationSeconds: 15
  renewDeadlineSeconds: 10
  leaseName: "efe-leader"
```

### 14.2 Templates ajoutés

* `deploy/helm/efe/templates/d18-hpa.yaml`
* `deploy/helm/efe/templates/d18-pdb.yaml`
* `deploy/helm/efe/templates/d18-grpc-headless.yaml`
* `deploy/helm/efe/templates/d18-leader-election-rbac.yaml`

### 14.3 Templates modifiés

* `deployment.yaml` : ajout podAntiAffinity, topologySpreadConstraints,
  terminationGracePeriodSeconds, resources, livenessProbe + readinessProbe.
* `configmap.yaml` : nouveaux env vars
  `EFE_LEADER_LEASE_NAME`, `EFE_SHUTDOWN_DRAIN_MS`, etc.

---

## 15. Bench harness `efe-bench`

### 15.1 Scope

Le crate `efe-bench` (nouveau D18) est un binaire opérateur qui
génère du load synthétique contre un cluster EFE pour mesurer
les SLOs D12 + Execute throughput.

Pas un load tester généraliste — focus EFE-specific RPC mix +
data shape.

### 15.2 Architecture

```
crates/efe-bench/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── workload.rs      # 6 workload patterns
│   ├── gen.rs           # data generation (tenants, catalogs, ...)
│   ├── runner.rs        # parallel runner using tokio + tonic
│   ├── metrics.rs       # latency histogram + HDR + RPS counter
│   ├── report.rs        # markdown report generator
│   ├── client.rs        # tonic gRPC client wrapper
│   └── main.rs          # CLI : `efe-bench run --workload=<W>`
└── tests/
    └── e2e.rs
```

### 15.3 6 workload patterns

| Workload | Mix | Cardinality | Duration |
|----------|-----|-------------|----------|
| `read-heavy` | 95 % `resolve_catalog` + 5 % `Execute` | 1k catalogs, 100 tenants | 5 min |
| `write-heavy` | 100 % `register_class` + `register_sentence` | 100 catalogs, 50 tenants | 5 min |
| `balanced` | 50/50 read/write Execute | 500 catalogs, 200 tenants | 5 min |
| `burst` | Idle 30s, 1000 RPS spike 30s, repeat | 100 catalogs, 50 tenants | 10 min |
| `multi-tenant` | 1000 tenants concurrents, 1 RPC/tenant/s | 1k catalogs | 10 min |
| `soak` | balanced 10 RPS sustained | 500 catalogs, 200 tenants | 1 h |

### 15.4 Data generation

```rust
pub fn gen_dataset(spec: &DatasetSpec) -> Dataset {
    let mut tenants = Vec::with_capacity(spec.tenants);
    for i in 0..spec.tenants {
        let tenant_id = format!("bench-tenant-{i:06}");
        let catalogs = (0..spec.catalogs_per_tenant)
            .map(|j| Catalog {
                name: format!("cat-{j}"),
                version: "v1".into(),
                classes: gen_classes(spec.classes_per_catalog, spec.dim),
            })
            .collect();
        tenants.push(BenchTenant { id: tenant_id, catalogs });
    }
    Dataset { tenants }
}
```

Spec parametrable :
* `tenants` : 1 / 100 / 1k / 10k
* `catalogs_per_tenant` : 1 / 10 / 100
* `classes_per_catalog` : 5 / 50 / 500
* `sentences_per_class` : 1 / 10 / 100
* `dim` : 384 (matching D16 default)

### 15.5 Métriques exposées

```rust
pub struct BenchReport {
    pub workload: String,
    pub duration_s: u64,
    pub rps_attempted: u64,
    pub rps_achieved: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub latency_max_ms: f64,
    pub error_breakdown: HashMap<String, u64>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    /// Comparison vs the D18-bench-N SLO targets.
    pub slo_pass: HashMap<String, bool>,
}
```

### 15.6 Refus de tourner contre prod

```rust
pub fn validate_target_env(env: &str) -> Result<(), BenchError> {
    if env.eq_ignore_ascii_case("prod") || env.eq_ignore_ascii_case("production") {
        return Err(BenchError::RefusedProdTarget);
    }
    Ok(())
}
```

CLI `efe-bench run` exige `--target-env <env>` non-prod et
refuse `EFE_BENCH_TARGET_ENV=prod`. T-D18-12 mitigation.

---

## 16. Bench métriques + SLOs

### 16.1 SLOs détaillés (D18-bench-1 à D18-bench-7)

Cf. § 3.3.

### 16.2 Report Markdown

Auto-régénéré à chaque CI run :

```markdown
# Sprint D18 — Bench Results

> Generated 2026-04-28T15:30:00Z by efe-bench v0.1.19.
> Cluster : kind-efe-bench-staging.
> Replicas : 3.

## Workload : `read-heavy` (5 min)

| Metric | Value | SLO Target | Pass |
|--------|------:|-----------:|:----:|
| RPS achieved | 4 832 | ≥ 5 000 | ❌ |
| p50 latency  | 8 ms  | n/a        | ✓ |
| p95 latency  | 18 ms | n/a        | ✓ |
| p99 latency  | 22 ms | ≤ 20 ms    | ❌ |
| Errors       | 0.01% | ≤ 0.1%     | ✓ |

[...]
```

### 16.3 SLO regression CI gate

Si une release-candidate échoue sur un SLO, le CI workflow fail
le build :

```yaml
- name: Bench D18
  run: |
    ./bin/efe-bench run --workload=read-heavy --target-env=staging --json > bench.json
    jq -e '.slo_pass | all' bench.json
```

---

## 17. Métriques Prometheus D18 nouveaux

| # | Métrique | Type | Labels | Cardinalité |
|---|----------|------|--------|-------------|
| 1 | `efe_replica_id` | Gauge | `pod_name` | N replicas (3-10) |
| 2 | `efe_idempotency_hits_total` | Counter | `result` ∈ `{cached, miss, in_flight}` | 3 |
| 3 | `efe_idempotency_size` | Gauge | (no label) | 1 |
| 4 | `efe_ratelimit_decisions_total` | Counter | `decision` ∈ `{allowed, throttled}` | 2 |
| 5 | `efe_ratelimit_bucket_available` | Gauge | `tenant_id` | up to 1k |
| 6 | `efe_leader_is_leader` | Gauge | `pod_name` | 1 per pod |
| 7 | `efe_leader_acquire_attempts_total` | Counter | `result` ∈ `{success, denied, error}` | 3 |
| 8 | `efe_shutdown_phase_seconds` | Histogram | `phase` ∈ `{drain, audit_flush, pg_close}` | 3 |
| 9 | `efe_bench_runs_total` | Counter | `workload`, `result` | 12 |

**Total D18 ≈ 1k+ séries (dominé par per-tenant rate-limit gauge)**.

### 16.1 Cardinality risk : per-tenant rate-limit gauge

`efe_ratelimit_bucket_available{tenant_id=...}` : 1 série par
tenant. Sur 1k tenants → 1k séries. **Acceptable** mais à
surveiller : sur 100k tenants ça devient cardinalty bomb.

Mitigation : exposer en aggregate seulement (top 10, ou histogram
bucket-distribution) si on dépasse 10k tenants. Différé.

---

## 18. Alertes (A-D18-*)

| ID | Nom | Expression | Sévérité |
|----|-----|------------|:--------:|
| A-D18-1 | `D18ReplicasUnderTarget` | `efe_replica_id` count < `Values.replicas.min` for 5 min | 🔴 CRIT |
| A-D18-2 | `D18LeaderMissing` | sum(efe_leader_is_leader) == 0 for 1 min | 🔴 CRIT |
| A-D18-3 | `D18LeaderSplit` | sum(efe_leader_is_leader) > 1 for 30s | 🔴 CRIT |
| A-D18-4 | `D18IdempotencyHitMiss` | rate(efe_idempotency_hits_total{result="miss"}) > 100/s | 🟠 HARD |
| A-D18-5 | `D18RateLimitThrottleSpike` | rate(efe_ratelimit_decisions_total{decision="throttled"}) > 1000/s | 🟠 HARD |
| A-D18-6 | `D18ShutdownTimeout` | `efe_shutdown_phase_seconds_bucket{le="60"}` increment | 🟠 HARD |
| A-D18-7 | `D18BenchSloRegression` | `efe_bench_runs_total{result="slo_failed"}` > 0 in 1h | 🔴 CRIT |

---

## 19. Test plan

### 19.1 Unit tests (~50 tests visés)

* `idempotency::tests` — INSERT/SELECT/expire workflow, race conditions
* `ratelimit::tests` — leak rate, capacity bound, atomic decrement
* `leader::tests` — acquire, renew, fail-over (mocked K8s API)
* `shutdown::tests` — sequencing, timeout per phase
* `bench::workload::tests` — workload spec parsing
* `bench::gen::tests` — data generation determinism

### 19.2 Integration tests (~10 tests)

* `audit_chain_intact_under_concurrent_replica_writes`
* `idempotency_dedup_cross_replica`
* `ratelimit_atomic_cross_replica`
* `leader_failover_after_kill`
* `bench_e2e_against_kind_cluster`

### 19.3 Chaos tests (CI)

GitHub Actions workflow `dr-chaos-test.yml` :

```yaml
- name: Spin 3-replica kind cluster
  run: ./scripts/setup-multi-replica-kind.sh

- name: Start traffic
  run: efe-bench run --workload=balanced --target-env=staging --duration=60s &

- name: Kill -9 a random replica every 10s
  run: |
    for i in 1 2 3; do
      sleep 10
      victim=$(kubectl get pods -n efe -l app=efe-server \
                 -o jsonpath='{.items[*].metadata.name}' \
                 | tr ' ' '\n' | shuf -n 1)
      kubectl delete pod -n efe $victim --grace-period=0 --force
    done

- name: Verify : success rate ≥ 99 %
  run: |
    rate=$(jq '.success_count / (.success_count + .error_count)' bench.json)
    [[ $(echo "$rate >= 0.99" | bc) -eq 1 ]] || exit 1
```

### 19.4 Bench smoke (CI)

Sur chaque release-candidate, run le bench `read-heavy` + `write-heavy`
contre un cluster staging. Fail le build si SLO regress.

### 19.5 Python pin tests (~30 tests)

Cf. § 22.

---

## 20. Cardinality envelope

| Source | Rows / Series | Notes |
|--------|--------------:|-------|
| `efe_idempotency` table | ~10M rows steady | TTL 24h, swept hourly |
| `efe_ratelimit_buckets` table | ~1k rows | 1 per tenant |
| `efe_leader_is_leader` Gauge | N replicas | trivial |
| `efe_ratelimit_bucket_available` Gauge | up to 1k | acceptable, monitored for > 10k |
| K8s Lease object | 1 | trivial |

Total D18 cardinality additions : **~10M PG rows + ~1k Prometheus
series**. Negligible vs the existing project budget.

---

## 21. Migration plan

### 21.1 Phase 0 — Pre-deploy (J-7)

* Apply migration `0007_d18_idempotency.sql` to all clusters.
* Verify : `SELECT * FROM efe_idempotency LIMIT 1` works (table
  exists, no permission error).

### 21.2 Phase 1 — Ship D18 code with `replicas: 1` (J-0)

* `helm upgrade` ships the new D18 binaries (idempotency PG-backed,
  rate-limit PG-backed, leader election RBAC, etc.) but with
  `replicas: 1` to verify no regression in a known-good config.
* Run smoke test : `efe-bench run --workload=balanced --replicas=1 --target-env=staging`.

### 21.3 Phase 2 — Activate multi-replica in staging (J+3)

* `helm upgrade --set replicas.min=3` in staging.
* Verify :
  * Leader election : exactly 1 leader.
  * Idempotency cross-replica : retry on different replica returns
    cached.
  * Rate-limit cross-replica : tenant bucket shared.
* Run full bench suite ; compare to single-replica baseline.

### 21.4 Phase 3 — Production rollout (J+7)

* `helm upgrade --set replicas.min=3` in prod, with rolling
  upgrade strategy.
* Monitor for 24 h.

### 21.5 Phase 4 — HPA enablement (J+14)

* `helm upgrade --set hpa.enabled=true` in prod.
* Verify HPA correctly scales up/down based on CPU + p95 latency.

### 21.6 Rollback plan

If any phase fails :
* `helm rollback efe <revision-N-1>` reverts to single-replica.
* Idempotency + rate-limit table writes from the failed deploy
  are tolerated (they're TTL-bound and backwards-compat).

---

## 22. Critères d'acceptation D18

D18 est livré quand **tous** les critères ci-dessous sont
satisfaits sur `main` :

- [ ] `cargo test -p efe-server --lib` : ≥ 470 verts (411 D17 + ~60 D18 unit).
- [ ] `cargo test -p efe-server --tests` : ≥ 565 verts.
- [ ] `cargo test -p efe-bench --lib` : ≥ 30 verts.
- [ ] `pytest scripts/tests/test_d18_multi_replica.py` : ≥ 30 verts.
- [ ] Helm chart renders cleanly (`helm template -f values-prod.yaml`).
- [ ] Migration `0007_d18_idempotency.sql` applied via sqlx-cli.
- [ ] Multi-replica chaos test passes (≥ 99 % success during random
      kill -9 every 10 s).
- [ ] Bench `read-heavy` SLO `D18-bench-3` (resolve p99 ≤ 20 ms)
      passes at 5000 RPS.
- [ ] Bench `write-heavy` SLO `D18-bench-2` passes.
- [ ] Bench `balanced` linear speedup ≥ 0.85 between 1 and 3
      replicas.
- [ ] No regression in D11 / D12 / D14 / D14-bis / D15 / D16 / D17
      test suites.
- [ ] CHANGELOG.md entry.
- [ ] Audit interne livré : `docs/audits/d18_multi_replica_audit_report.md`.
- [ ] DR drill (D17) re-run successfully against multi-replica
      cluster.

---

## 23. Anti-corruption — versioning

### 23.1 Idempotency table schema version

Ajouter `schema_version INTEGER NOT NULL DEFAULT 1` à
`efe_idempotency`. Le runtime refuse les rows de version inconnue.

### 23.2 K8s Lease lease-duration coupling

Changer `lease_duration` requiert un upgrade coordonné : tous les
replicas doivent avoir le même `lease_duration` configuré, sinon
flapping. Documenter dans le runbook D18.

### 23.3 Bundle compatibility

Les bundles D16 ne changent pas en D18. Operators bundle-side
(D16) restent valides.

---

## 24. Non-objectifs explicites

1. **Multi-region active-active** — out of scope, D19+ candidate.
   Cf. D17 § 17 pour les fondations.
2. **Sharded catalog** : tenant-routing par hash → un replica par
   shard. Out of scope. Le shared PG est suffisant pour 100k tenants.
3. **Read-replica Postgres** : la primary PG reste single-instance
   en D18. Failover automatique via cloudnative-pg out of scope.
4. **Bundle sharing intra-node** (tmpfs mmap) : out of scope D18.
   Deferred to D19+.
5. **Auto-scaling vertical** (Pod resource adjustment) : HPA
   horizontal seulement.
6. **Service mesh** (Istio, Linkerd) : out of scope ; le LB K8s
   built-in suffit.
7. **Custom load balancing algorithm** : round-robin DNS + tonic
   client-side LB suffisent.
8. **Cross-replica session affinity** : out of scope ; chaque
   request peut router vers n'importe quel replica.

---

## 25. Inventaire ASCII complet

```
SPRINT D18 — Multi-réplica + bench D12
═════════════════════════════════════════════════════════════════

NEW FILES
  crates/efe-server/src/                       ~600 lignes Rust
    idempotency.rs                                ~250
    ratelimit_shared.rs                           ~200
    leader_election.rs                            ~200
    shutdown_sequence.rs                          ~150
    health_grpc.rs                                ~120

  crates/efe-bench/                            ~1 800 lignes Rust
    Cargo.toml                                     ~50
    src/lib.rs                                     ~80
    src/workload.rs                               ~280
    src/gen.rs                                    ~280
    src/runner.rs                                 ~320
    src/metrics.rs                                ~250
    src/report.rs                                 ~300
    src/client.rs                                 ~180
    src/main.rs                                   ~250
    tests/e2e.rs                                  ~150

  deploy/helm/efe/templates/
    d18-hpa.yaml                                   ~80
    d18-pdb.yaml                                   ~30
    d18-grpc-headless.yaml                         ~40
    d18-leader-election-rbac.yaml                  ~60
    d18-prometheus-rules.yaml                      ~120

  migrations/0007_d18_idempotency.sql              ~80

  docs/bench/d18_bench_results.md           (auto-generated by CI)
  docs/runbook/D18-1.md → D18-7.md           ~480 (6 runbooks)

  scripts/tests/test_d18_multi_replica.py     ~430 Python pin
  scripts/setup-multi-replica-kind.sh         ~120 bash

CHANGES TO EXISTING
  Cargo.toml workspace                          +1 ligne (efe-bench)
  deploy/helm/efe/values.yaml                 ~120 lignes (D18 block)
  deploy/helm/efe/templates/configmap.yaml    +14 lignes (D18 envvars)
  deploy/helm/efe/templates/deployment.yaml   +50 lignes (probes, affinity)
  CHANGELOG.md                                +60 lignes

TOTAL D18 net ajout : ~5 000 lignes.
```

---

## 26. Conclusion + roadmap

D18 transforme EFE d'un service **single-replica de démonstration**
en un service **multi-replica HA** avec :

1. **Cohérence cross-replica** via PG-backed idempotency + rate-limit
2. **Background tasks safe** via K8s Lease leader election
3. **Graceful shutdown sequenced** garantissant l'audit chain
   integrity
4. **HPA + PDB + topology spread** pour HA cluster-wide
5. **Bench formel** D12 + Execute avec SLOs alertés

Le sprint suivant (D19) construira dessus pour livrer :

* **Multi-region active-active** : 2+ régions K8s avec PG
  cross-region replication + DNS-based regional fail-over.
* **Bundle memory deduplication** : tmpfs/mmap pour partager 1
  copie des bundles intra-node.
* **Sharded catalog** : tenant-routing par hash si 100k+ tenants.
* **Read-replica Postgres** : offload reads vers replicas pour
  réduire la contention sur le primary.

---

*Document architecture livré 2026-04-28. Auteur : architecture
interne (Claude). Pour questions ou amendements, contact
maintainer EFE.*
