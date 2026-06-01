# P1 Surface — Architecture chirurgicale de la **Latence opposable**

> **Statut** : architecture détaillée, prête à implémenter (jalons
> sur Φ-A → Φ-D du roadmap `PRODUCT_COMPLETION_ARCHITECTURE.md`).
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : ce document **n'invente pas** un nouveau sprint —
> il **cartographie la surface complète** (code + mesure + SLO +
> contrat + défense + test) qui rend la promesse `P1 — Latence
> opposable < 200 ms p99` *défendable* face à un client, un auditeur
> ou un régulateur. Il consolide ce qui est éparpillé entre D5
> (bench foundation), D8 (hybrid latency note), D9 (load test),
> D13 (dashboards), D18 (multi-réplica bench), §3 + §8 + §10 de
> `PRODUCT_COMPLETION_ARCHITECTURE.md`, et nourrit les jalons
> v0.5.0 (CUDA), v0.6.0 (multi-GPU), v1.0.0 GA.
> **Pré-requis** : avoir lu
> [`PRODUCT_COMPLETION_ARCHITECTURE.md`](PRODUCT_COMPLETION_ARCHITECTURE.md)
> §1 (mission), §3 (contrat C-1 → C-20), §8 (Phase Φ-D CUDA),
> §10 (SLO matrix). Connaître `docs/bench/d9_docker_helm_load_architecture.md`
> (load harness) et `docs/audits/d9_load_soak_report.md`.

---

## Table des matières

1. [Mission & contrat opposable P1](#1-mission--contrat-opposable-p1)
2. [Décomposition canonique du budget 200 ms](#2-décomposition-canonique-du-budget-200-ms)
3. [Surface 1 — Code chaud (hot-path 12 étages)](#3-surface-1--code-chaud-hot-path-12-étages)
4. [Surface 2 — Mesure & instrumentation](#4-surface-2--mesure--instrumentation)
5. [Surface 3 — SLO & gating (CI + prod)](#5-surface-3--slo--gating-ci--prod)
6. [Surface 4 — Tests (8 niveaux d'assurance)](#6-surface-4--tests-8-niveaux-dassurance)
7. [Surface 5 — Défensive (12 garde-fous runtime)](#7-surface-5--défensive-12-garde-fous-runtime)
8. [Surface 6 — Profilage & outillage perf](#8-surface-6--profilage--outillage-perf)
9. [Surface 7 — Contractuelle & opposabilité juridique](#9-surface-7--contractuelle--opposabilité-juridique)
10. [Surface 8 — Économique (TCO du SLO)](#10-surface-8--économique-tco-du-slo)
11. [Matrice complète des artefacts P1](#11-matrice-complète-des-artefacts-p1)
12. [Plan d'implémentation (mapping sprints D19 → D32)](#12-plan-dimplémentation-mapping-sprints-d19--d32)
13. [Risques identifiés & mitigations (16 risques)](#13-risques-identifiés--mitigations-16-risques)
14. [Décisions de conception clés (15)](#14-décisions-de-conception-clés-15)
15. [Anti-régression & realism check P1](#15-anti-régression--realism-check-p1)
16. [Critères d'acceptation (DoD P1-Surface)](#16-critères-dacceptation-dod-p1-surface)
17. [Hand-off & dépendances downstream](#17-hand-off--dépendances-downstream)
18. [Annexes (A → F)](#18-annexes-a--f)

---

## 1. Mission & contrat opposable P1

### 1.1 Rappel de la promesse

> **P1 — Latence opposable** : un classify aller-retour
> *p99 < 200 ms* sur **Mode-1 + bundle MiniLM + GPU NVIDIA T4 +
> 3 réplicas + charge 200 RPS**.
> *(source : `PRODUCT_COMPLETION_ARCHITECTURE.md` §1.1, ligne 120 ;
> SLO C-1 §3.1 ligne 334)*

### 1.2 Le mot "opposable"

Au sens commercial et juridique, **opposable** signifie :

1. **Mesurable** par un tiers, hors-environnement EFE, avec un
   protocole documenté (`docs/bench/d9_docker_helm_load_architecture.md`).
2. **Reproductible** : 2 mesures consécutives doivent agréer à
   ±5 % (cf. SLO-PC-REPRO ci-dessous § 5.2.4).
3. **Attestée** par une chaîne d'audit signée (`HMAC-SHA-256`
   sur le tuple `(mesure, contexte, version, host_fingerprint,
   timestamp)`) qui survit au rollover (cf. D14-bis chaîne).
4. **Borne supérieure stricte** : tout dépassement déclenche un
   `SLO_BURN_ALERT` PagerDuty P1 (cf. §5.3.4) ET un délai contractuel
   de 4 h pour publier une RCA dans le run-book client.

Une promesse non-opposable est une promesse marketing. **Une
promesse opposable engage juridiquement** — typiquement via une
clause SLA avec pénalité financière en cas de violation soutenue
(cf. §9.3, modèle de clause SLA).

### 1.3 Pourquoi 200 ms (et pas 100 ms, ou 500 ms)

| Valeur | Justification | Décision |
|-------:|---------------|:--------:|
| 100 ms | Plafond psychologique « instantané » mais infaisable Mode-1 avec embedder ONNX CUDA T4 (l'inférence seule consomme 30 ms p99 sur MiniLM-L12-v2, cf. §2.4) | ❌ Rejeté |
| **200 ms** | **Budget tenable** = embedder 30 + genesis 5 + fusion 100 + consensus/dissolution 2 + alignment 5 + gRPC 50 = **192 ms**, marge 8 ms. Couvre les pics de tail au-delà du p99 sans sur-provisioning | ✅ **Choisi** |
| 300 ms | Plus de marge mais perd l'argument commercial « 5-10× plus rapide qu'une chaîne LLM » (chaînes LLM modernes = 1-2 s) | ❌ Compromis trop mou |
| 500 ms | Atteignable même CPU (cf. SLO C-2 = 800 ms CPU), mais ne justifie plus un GPU dédié → casse P2 (coût opposable) | ❌ Casse P2 |

La valeur 200 ms est le **seul point qui satisfait simultanément** :
- Argumentaire commercial « 5-10× LLM chain » ✓
- Faisabilité physique GPU T4 + bundle MiniLM ✓
- Marge de défense (8 ms = 4 % du budget) ✓
- Coût hardware justifié vs CPU-only ✓

### 1.4 Périmètre exact de la mesure

Le `t=0` du roundtrip est le **`recv` sur le socket TCP du client**
(côté gRPC ingress), et `t=fin` est le **`send` du dernier byte de
`ExecuteResponse`** sur le même socket.

Sont **inclus** dans le budget :
- TLS handshake amortizé (keep-alive ; coût ≈ 0 sur connexion
  réutilisée),
- Sérialisation / désérialisation Protobuf,
- gRPC framing,
- Inférence embedder (incl. tokenization),
- Genesis + Fusion + Consensus + Dissolution,
- Alignment + projection sortie,
- Audit chain append (HMAC + persistance memoire-DB),
- Sérialisation `ExecuteResponse`,
- Forwarding gRPC ingress.

Sont **exclus** du budget (par convention contractuelle, à
documenter dans le SLA) :
- DNS lookup côté client,
- Latence réseau Internet hors-DC client → DC EFE,
- mTLS handshake **initial** (connexion froide), facturé séparément
  via SLO-PC-mTLS-INIT (≤ 60 ms p99, cf. §5.2.5).

### 1.5 Conditions hardware contractuelles

Le SLO C-1 n'est défendu que **dans les conditions suivantes** :

| Paramètre | Valeur contractuelle | Tolérance |
|-----------|---------------------|-----------|
| GPU | NVIDIA T4 (16 GiB VRAM) | T4 / L4 / A10 / A100 acceptables |
| CPU host | 4 vCPU Intel Cascade Lake / AMD Milan | ≥ 4 vCPU x86_64 récents |
| RAM host | 16 GiB | ≥ 16 GiB |
| Réplicas | 3 (HA, leader+follower) | ≥ 2 |
| Charge | 200 RPS (sustained 10 min) | 0 → 220 RPS (110 % du nominal) |
| Bundle embedder | MiniLM-L12-v2 (D16) | bundle officiel signé cosign |
| Mode | Mode-1 uniquement (zero-shot) | n/a |
| Network | DC-local p99 < 1 ms RTT host↔host | DC-local |
| Kernel Linux | ≥ 5.15 (epoll, io_uring optionnel) | ≥ 5.10 |
| Tokio runtime | 4 workers, work-stealing | n/a |

**Toute déviation matérielle** ré-ouvre la mesure et invalide
l'opposabilité. C'est le sens du `host_fingerprint` dans la chaîne
d'audit (§4.5).

---

## 2. Décomposition canonique du budget 200 ms

### 2.1 Tableau de référence (12 étages)

> Le tableau ci-dessous est la **source de vérité unique** pour la
> répartition du budget. Toute optimisation P1 doit citer la ligne
> du tableau qu'elle touche.

| # | Étage | p99 cible | p50 attendu | Localisation code | Métrique Prometheus |
|--:|-------|----------:|-----------:|-------------------|---------------------|
| 1 | TCP `recv` + TLS framing (keep-alive) | 1.5 ms | 0.3 ms | `tonic::transport::Server` | `grpc_recv_duration_seconds` |
| 2 | Protobuf `decode(ExecuteRequest)` | 0.5 ms | 0.1 ms | `efe_server::services::swarm` ligne ~80 | `grpc_decode_duration_seconds` |
| 3 | gRPC interceptors (auth, rate-limit, tenant) | 3 ms | 0.8 ms | `efe_server::interceptors::*` | `grpc_interceptor_duration_seconds` |
| 4 | Tokenization (Hugging Face tokenizer) | 4 ms | 1.5 ms | `efe_server::embedder::tokenize` | `embed_tokenize_duration_seconds` |
| 5 | ONNX CUDA inference (forward pass) | 26 ms | 18 ms | `efe_server::embedder::onnx_cuda` | `embed_inference_duration_seconds` |
| 6 | Genesis (spawn N agents + prior) | 5 ms | 2 ms | `efe_core::genesis::engine` | `genesis_duration_seconds` |
| 7 | Fusion (10 rounds × ~10 ms) | 100 ms | 65 ms | `efe_core::fusion::mvp_cpu` ou GPU | `fusion_round_duration_seconds` (par round) |
| 8 | Consensus (multi-criteria detector) | 1 ms | 0.4 ms | `efe_core::consensus::multi_criteria` | `consensus_duration_seconds` |
| 9 | Dissolution + Alignment + projection | 6 ms | 3 ms | `efe_core::dissolution::engine` + `efe_core::alignment::*` | `dissolution_alignment_duration_seconds` |
| 10 | Audit chain HMAC append | 4 ms | 1.2 ms | `efe_server::v1::audit_chain` (D14-bis) | `audit_chain_append_duration_seconds` |
| 11 | Protobuf `encode(ExecuteResponse)` | 1 ms | 0.3 ms | idem ligne 2 | `grpc_encode_duration_seconds` |
| 12 | TCP `send` + framing | 1 ms | 0.2 ms | idem ligne 1 | `grpc_send_duration_seconds` |
| — | **TOTAL budget p99** | **153 ms** | ~ 93 ms | — | — |
| — | **Marge anti-tail (47 ms)** | 47 ms | n/a | n/a (réserve système : GC, scheduler, NUMA tail) | n/a |
| — | **SLO C-1 plafond** | **200 ms** | n/a | n/a | `efe_swarm_duration_seconds` (bucket p99) |

### 2.2 Pourquoi 47 ms de marge

Trois sources connues de tail-latency hors-application :

1. **Tokio scheduler tail** : sur 4 workers + 200 RPS, le steal-queue
   latency p99 mesuré historiquement ≈ 8-12 ms (cf. mesures D18).
2. **CUDA context switch + kernel queue** : avec un GPU partagé
   entre 3 réplicas (cas SLO-C-1 nominal), le batch coalescing
   peut ajouter 10-20 ms en pic.
3. **Allocator fragmentation tail** : `jemalloc` p99 mmap latency
   sur arena RT > 1 GiB ≈ 5-15 ms (cf. D2 arena bench).

Total budget tail système ≈ 23-47 ms. On garde **47 ms** (worst
case raisonnable, P99 sur des hosts qui n'ont pas l'arena déjà
chaude).

### 2.3 Variantes de budget par mode

| Mode | Budget total | Détail différentiel |
|------|-------------:|---------------------|
| **Mode-1 GPU (P1)** | **200 ms** | Référence ci-dessus, §2.1 |
| Mode-1 CPU | 800 ms | Étage 5 (embedder) = 80 ms, étage 7 (fusion) = 580 ms |
| Mode-2 GPU | 1500 ms | +1 LLM call routing (≈ 1100 ms p99 vs LLM provider), reste = 400 ms |
| Mode-3 GPU | 600 ms | k=3 turns en parallèle, plafond = 3× Mode-1 = 600 ms (SLO C-4) |
| Mode-1 GPU dégradé (1 réplica) | 350 ms | +150 ms d'attente queue côté serveur sous charge |

### 2.4 Profil "Inference is the long pole" (sanity check)

À l'échelle du Mode-1 GPU :

```
Embedder ONNX CUDA   ████████████████████████  26 ms (17 %)
Fusion 10 rounds     ████████████████████████████████████████████████  100 ms (65 %)
Audit chain          ████  4 ms (3 %)
Tokenization         ████  4 ms (3 %)
gRPC + interceptors  █████  4.5 ms (3 %)
Alignment + diss     ██████  6 ms (4 %)
Genesis              █████  5 ms (3 %)
Consensus            █  1 ms (0.7 %)
Encode + send        ██  2 ms (1 %)
TCP recv + decode    ██  2 ms (1 %)
                     ────────────────────────────────────
TOTAL                                                   153 ms
```

→ **82 % du budget est consommé par 2 modules : Fusion (65 %) +
Embedder (17 %)**. Toute optimisation au-delà du seuil 80/20 doit
prioritairement cibler ces deux modules. Les autres étages sont
"déjà payés".

---

## 3. Surface 1 — Code chaud (hot-path 12 étages)

### 3.1 Vue arborescente des modules sur le hot-path

```
crates/efe-server/src/
├── services/swarm.rs              ← étages 1, 2, 11, 12 (gRPC)
├── interceptors/                  ← étage 3 (auth, rate-limit, tenant)
│   ├── auth_mtls.rs
│   ├── rate_limit_leaky_bucket.rs (D18)
│   └── tenant_resolution.rs
├── embedder/
│   ├── tokenize.rs                ← étage 4
│   ├── onnx_cuda.rs               ← étage 5 (CRITIQUE : 17 % du budget)
│   └── batch_coalesce.rs          ← étage 5 (CUDA batching ; v0.5.0+)
├── mode1.rs                       ← orchestrateur étages 6-9
├── v1/audit_chain.rs              ← étage 10 (D14-bis)
└── runtime.rs                     ← config Tokio (workers, blocking pool)

crates/efe-core/src/
├── genesis/engine.rs              ← étage 6
├── fusion/
│   ├── mvp_cpu.rs                 ← étage 7 (CPU MVP, fallback)
│   └── (gpu_cuda.rs à venir Φ-D)  ← étage 7 (CRITIQUE : 65 % du budget)
├── consensus/multi_criteria.rs    ← étage 8
├── dissolution/engine.rs          ← étage 9
└── alignment/                     ← étage 9

crates/efe-kernels/                ← CUDA kernels (Φ-D D28-D32)
└── src/                            (actuellement 29 L stub ; cible 5000+ L)
```

### 3.2 Allocation mémoire sur le hot-path

| Étage | Allocations attendues | Stratégie |
|------:|----------------------|-----------|
| 1 | 1 buffer `[u8; 4096]` pré-alloué par connexion | Connection pool tonic |
| 2 | 1 `Bytes` zero-copy via prost | Pas d'alloc fresh |
| 3 | 0 (lecture de claims pré-décodées) | TLS session cache |
| 4 | 1 `Vec<i64>` tokens (taille variable, max 512) | Pool `Vec<i64>` recyclable |
| 5 | 1 `ndarray<f32, [B, T, D]>` activations + sortie `Vec<f32; D>` | ONNX runtime arena alloué au démarrage |
| 6 | N × `Agent` (N = swarm size, typique 128-256) | **Arena `efe_core::memory::arena`** (D2) |
| 7 | 0 fresh par round (réutilisation arena) | Arena reset entre rounds |
| 8 | 1 `ConsensusEvent` | Stack ou pool |
| 9 | 1 `Verdict` final | Stack |
| 10 | 1 `AuditEntry` + HMAC state | Pool HMAC contexts |
| 11 | 1 `Bytes` encode prost | Pas d'alloc fresh |
| 12 | 0 | Connection pool |

> **Règle d'or P1** : *aucune* allocation `Vec::new()` ou
> `Box::new()` ne doit apparaître sur le hot-path en dehors des
> étages 4 / 5 / 6. Toute exception est gated par `#[allow(p1_alloc)]`
> + commentaire justifiant. CI lint via `cargo-deny` (cf. §6.3).

### 3.3 Étage 5 — Embedder ONNX CUDA en détail

**Stratégie p99 ≤ 26 ms** pour MiniLM-L12-v2 sur T4 :

1. **Pre-warm au démarrage** : 100 inférences dummy avec longueurs
   variées {16, 64, 128, 256, 512} pour pré-allouer les arenes
   CUDA et trigger les kernel compilations JIT. Sans pre-warm, la
   1ère inférence cold = 250 ms (inacceptable).
2. **Sequence length bucketing** : padder à la prochaine puissance
   de 2 ∈ {32, 64, 128, 256, 512}. Au-delà de 512 → erreur 400
   (le SLA stipule max 512 tokens).
3. **Batch coalescing serveur-side** : agréger jusqu'à 32 requêtes
   arrivant dans une fenêtre de 4 ms, lancer un batch unique GPU.
   Réduit le coût per-request quand RPS > 100. Cf. §3.4.
4. **CUDA streams 2× round-robin** : 2 streams CUDA permettent au
   GPU de pipeliner le forward du batch N et la copie D→H du batch
   N-1. Gain ≈ 12 % p99.
5. **fp16 ONNX precision** : MiniLM tolère fp16 (perte BERTScore
   < 0.001 sur eval set). Gain ≈ 35 % vs fp32.
6. **Disable ONNX session pool dynamic resizing** : fixer
   `intra_op_num_threads = 1` (le batching gère la parallélisation),
   `inter_op_num_threads = 1`. Évite jitter du scheduler ORT.

### 3.4 Batch coalescing — algorithme

```rust
// crates/efe-server/src/embedder/batch_coalesce.rs (à livrer Φ-C)
pub struct BatchCoalescer {
    queue:           tokio::sync::Mutex<VecDeque<PendingInfer>>,
    max_batch_size:  usize,           // 32
    max_wait_micros: u64,             // 4000 (= 4 ms)
    inference_chan:  mpsc::Sender<Batch>,
}

impl BatchCoalescer {
    pub async fn infer(&self, tokens: Vec<i64>) -> Result<Vec<f32>> {
        let (tx, rx) = oneshot::channel();
        self.queue.lock().await.push_back(PendingInfer { tokens, tx });
        // Le scheduler de fond appelle `run_batch` quand
        //  - queue.len() ≥ max_batch_size, OR
        //  - oldest_age ≥ max_wait_micros.
        rx.await?
    }

    async fn run_batch(&self) {
        let mut g = self.queue.lock().await;
        if g.is_empty() { return; }
        let batch: Vec<PendingInfer> = g.drain(..g.len().min(self.max_batch_size)).collect();
        drop(g);
        let outputs = self.session.run(&batch).await?;
        for (PendingInfer { tx, .. }, out) in batch.into_iter().zip(outputs) {
            let _ = tx.send(Ok(out));
        }
    }
}
```

Invariants critiques :
- `max_wait_micros` est **inférieur au budget restant** à ce moment
  (étages 1-3 = 5 ms, donc à l'entrée de l'étage 4 on a 195 ms de
  budget restant ; 4 ms de wait = 2 % du restant, acceptable).
- En basse charge (< 5 RPS), le wait peut dominer ; on bypass le
  coalescer si `queue.is_empty()` à l'arrivée. **Test pin** :
  `bench_p1_solo_request_does_not_wait_for_batch` (§6.4).
- L'option `--no-batch-coalesce` est exposée dans `efe-server`
  pour les hôtes single-tenant ou les bench profile P50-only.

### 3.5 Étage 7 — Fusion 10 rounds × ~10 ms

**Stratégie CPU MVP (v0.1.x → v0.4.x)** : 8-wide chunked
accumulation (M32 distance kernels) + tableau de scores
pré-alloué + aucun allocation par round.

**Stratégie GPU (v0.5.0+)** : 1 kernel CUDA "fused softmax + entropy"
(profile nsight ciblé) :
- Chaque round = 1 invocation kernel + 1 copie D→H du centroïde.
- Round trip 0.7 ms (kernel) + 0.3 ms (copie) ≈ 1 ms / round.
- 10 rounds = 10 ms (vs 100 ms CPU MVP).

Le gain attendu CPU → GPU sur l'étage 7 est ×10, ce qui libère
~90 ms du budget — c'est le levier qui rend possible Mode-2 (1500 ms,
dont 1100 ms LLM call) sans casser P1 sur Mode-1.

### 3.6 Hot-path policy : *ce qui est interdit*

| Catégorie | Interdit | Exception |
|-----------|---------|-----------|
| Sync I/O | `std::fs::*`, `std::net::*` blocking | Bootstrap uniquement |
| Lock contention | `parking_lot::Mutex` tenu > 1 ms | Logger structuré (≤ 100 µs) |
| Async lock across await | `parking_lot` guard franchissant `.await` | INTERDIT (lint `clippy::await_holding_lock`) |
| Logging niveau `INFO`+ | dans le code Rust hot-path | `DEBUG` seulement, sampling 1/1000 ; cf. §4.3 |
| Allocations | `Vec::new`, `Box::new`, `String::from` | Pools + arena (cf. §3.2) |
| Channel unbounded | `mpsc::unbounded_channel` | INTERDIT (DoS surface) |
| Blocking compute | > 1 ms sur worker Tokio | `spawn_blocking` obligatoire |
| `unwrap()` | sur path utilisateur | Tests + bootstrap uniquement |
| Print / eprintln | partout | INTERDIT — pas même en debug |
| External HTTP call | sur Mode-1 hot-path | INTERDIT (rompt l'isolation P1) |

> Toute violation est captée par `cargo clippy -- -D
> clippy::await_holding_lock -D clippy::unwrap_used` (cf. §5.4)
> et le **test `tests/p1_hot_path_lint.rs`** qui parse l'AST des
> modules listés en §3.1 et rejette les patterns interdits.

---

## 4. Surface 2 — Mesure & instrumentation

### 4.1 Trois piliers (logs / metrics / traces)

Le P1-Surface adopte la doctrine OTel "three pillars" héritée de
D13 :

| Pilier | Backend recommandé | Cardinalité | Rétention |
|--------|--------------------|:-----------:|:---------:|
| **Metrics** | Prometheus (scrape /metrics) | basse (≤ 100 labels distincts) | 90 j |
| **Traces** | OTel collector → Tempo / Jaeger | haute (1 par requête) | 7 j (sampling 100 %) puis 30 j (sampling 1 %) |
| **Logs structurés** | Loki / Cloudwatch | médium | 30 j (info) / 365 j (audit) |

### 4.2 Métriques Prometheus minimales (12 instruments P1)

```
# Métriques P1 (P1-METRIC-1 → P1-METRIC-12)

efe_p1_grpc_recv_duration_seconds       histogram, le-buckets {0.5,1,1.5,2,3,5,10}ms
efe_p1_grpc_decode_duration_seconds     histogram, idem
efe_p1_interceptor_duration_seconds     histogram, label = interceptor_name
efe_p1_embed_tokenize_duration_seconds  histogram, le-buckets {1,2,4,8}ms
efe_p1_embed_inference_duration_seconds histogram, le-buckets {10,20,30,40,60,100}ms, label = batch_size
efe_p1_genesis_duration_seconds         histogram, idem
efe_p1_fusion_round_duration_seconds    histogram, label = round_idx (0..9)
efe_p1_consensus_duration_seconds       histogram
efe_p1_dissolution_alignment_duration_seconds histogram
efe_p1_audit_chain_append_duration_seconds    histogram
efe_p1_grpc_encode_duration_seconds     histogram
efe_p1_grpc_send_duration_seconds       histogram

# Métriques agrégées (utilisée par SLO C-1)
efe_swarm_duration_seconds              histogram (déjà existante D13)
                                        # = somme étages 1..12
```

Le label `mode = mode1` est OBLIGATOIRE sur chaque histogram pour
isoler P1 vs Mode-2 / Mode-3 (qui ont leurs propres SLO).

### 4.3 Sampling des logs sur le hot-path

| Niveau | Fréquence hot-path | Contenu |
|:------:|-------------------|---------|
| ERROR | 100 % | Exception, panic catch, RPC failure |
| WARN | 100 % | Budget overrun, tail outlier, queue depth > 80 % |
| INFO | **0 %** sur hot-path (mais 100 % sur startup, shutdown, config change) | n/a |
| DEBUG | 1/1000 sampling | trace ID + étage + duration |

> **Pourquoi 1/1000** : à 200 RPS soutenu × 3 réplicas = 600 RPS
> agrégé ; 1/1000 = ~0.6 log debug/s par étage. Acceptable côté
> Loki, suffisant pour diagnostic post-mortem ciblé.

### 4.4 Traces OTel — span layout

```
Span tree (1 trace = 1 ExecuteRequest):

efe.mode1.execute               span racine, root_kind = SERVER
├── efe.grpc.recv               1 ms
├── efe.grpc.decode             0.5 ms
├── efe.interceptors            3 ms
│   ├── auth                    1.5 ms
│   ├── rate_limit              0.8 ms
│   └── tenant                  0.7 ms
├── efe.embed                   30 ms
│   ├── tokenize                4 ms
│   ├── batch_wait              2 ms (= temps en queue de coalescer)
│   └── onnx_cuda_inference     24 ms (incl. batch_size = N)
├── efe.swarm                   115 ms
│   ├── genesis                 5 ms
│   ├── fusion.rounds[0..9]     100 ms (1 span / round, attrs = round_idx, agents_count, centroid_norm)
│   ├── consensus               1 ms
│   └── dissolution_alignment   6 ms
├── efe.audit_chain.append      4 ms
└── efe.grpc.send_encode        2 ms
```

Attributs OTel obligatoires sur le span racine :
- `tenant_id` (string), `catalog_version` (string semver),
- `mode = "mode1"`, `swarm_id` (uuid v7),
- `agents_count` (int), `rounds_executed` (int),
- `verdict_label` (string), `confidence` (float, 0..1),
- `budget_remaining_ms` (float, **calculé au moment de l'envoi**)
  → permet de filtrer en post-mortem les requêtes qui ont
  consommé ≥ 80 % du budget (alerte précurseur SLO burn).

### 4.5 Fingerprint d'hôte (chaîne d'audit perf)

Chaque mesure Prometheus archivée dans la chaîne D14-bis est
accompagnée du `host_fingerprint` calculé une fois au démarrage
du serveur :

```rust
pub struct HostFingerprint {
    pub cpu_model:     String,    // "Intel(R) Xeon(R) Gold 6248 @ 2.50 GHz"
    pub cpu_cores:     u32,
    pub ram_gib:       u32,
    pub gpu_model:     Option<String>,   // "NVIDIA T4"
    pub gpu_vram_gib:  Option<u32>,
    pub kernel:        String,    // "Linux 5.15.0-86-generic"
    pub rustc:         String,    // "rustc 1.88.0"
    pub blas:          Option<String>,   // "openblas-0.3.28" or None
    pub cuda:          Option<String>,   // "CUDA 12.4"
    pub container:     String,    // distroless image SHA-256
    pub fingerprint_sha256: [u8; 32],   // hash de tous les champs ci-dessus
}
```

Une mesure prétendant SLO-C-1 PASS avec un `fingerprint_sha256`
différent de la baseline contractuelle est **inopposable** et
déclenche `SLO_REFUSED_BY_FINGERPRINT_MISMATCH` dans le rapport
SLO. Cf. §9.4.

---

## 5. Surface 3 — SLO & gating (CI + prod)

### 5.1 Inventaire des SLO P1

| ID | Nom | Comparator | Seuil | Source | État cible v1.0 |
|----|-----|:----------:|------:|--------|:--------------:|
| **SLO-C-1** | Mode-1 GPU p99 latency | lte | 200 ms | `efe_swarm_duration_seconds_bucket{mode="mode1"}` | **PASS** (contractuel) |
| SLO-C-2 | Mode-1 CPU p99 latency | lte | 800 ms | idem, GPU absent | PASS |
| **SLO-PC-REPRO** | Reproductibilité 2 mesures consécutives | lte | 5 % drift | bench harness D18 | PASS |
| **SLO-PC-mTLS-INIT** | mTLS handshake initial p99 | lte | 60 ms | `tonic_tls_handshake_duration_seconds` | PASS |
| SLO-P1-BUDGET-1..12 | Budget par étage (12 SLO) | lte | cibles §2.1 | les 12 histograms §4.2 | PASS (gate budget) |
| SLO-P1-BURN | Burn rate SLO C-1 sur 1 h | lte | 14.4× (= 1 h burn dans 30 j) | error budget calc Multi-window | PASS |
| SLO-P1-TAIL-1 | p99.9 latency (= 1/1000 worst) | lte | 400 ms | idem | PASS |

Total : **17 SLO** P1-related (1 racine C-1 + 12 budget + 4 défense).

### 5.2 Définition opérationnelle de chaque SLO

#### 5.2.1 SLO-C-1 (racine)

```yaml
slo_id: SLO-C-1
metric: efe_swarm_duration_seconds_bucket
mode: mode1
comparator: lte
threshold_ns: 200_000_000
display_as: ms
scale: 1e9
window: 30d
error_budget: 0.001   # 99.9 % SLO compliance ; = 43 min/mois de burn admis
source_bench: bench_load_p1_sustained.json   # produit par ghz harness D9
host_fingerprint_required: true
contract_clause: "SLA §3.1"
```

#### 5.2.2 SLO-P1-BUDGET-* (les 12)

Chaque SLO budget cible une borne par étage. Format :

```yaml
- id: SLO-P1-BUDGET-7   # exemple : fusion 10 rounds
  metric: sum(efe_p1_fusion_round_duration_seconds_bucket) by (le)  # over 10 rounds
  comparator: lte
  threshold_ns: 100_000_000   # 100 ms
  window: 1h
  source_bench: bench_load_p1_sustained.json
```

#### 5.2.3 SLO-P1-BURN (burn rate)

Implémente le pattern **multi-window multi-burn-rate** (cf. SRE
Workbook ch. 5) :

```
fast burn window  = 1 h,  threshold = 14.4× error budget
slow burn window  = 6 h,  threshold = 6×   error budget
ticket window     = 24 h, threshold = 1×   error budget
```

→ alerte PagerDuty P1 si **fast ET slow** sont rouges, P3 ticket
si seulement `ticket` est rouge. Permet de différencier panne aiguë
vs dérive lente.

#### 5.2.4 SLO-PC-REPRO

Procédure :
1. Lancer `efe-bench run --workload p1 --duration 10m --rps 200`.
2. Attendre 60 s (cold-down).
3. Re-lancer la même commande.
4. Calculer `|p99_run2 - p99_run1| / p99_run1`.
5. **PASS** si ≤ 0.05 (5 %).

Vise à attraper les régressions non-déterministes (allocator
thrashing, GPU contention résiduelle, etc.).

#### 5.2.5 SLO-PC-mTLS-INIT

Budget séparé pour le handshake mTLS initial (TLS 1.3 + cert
vérif chain) — typiquement 40-55 ms sur T4, exclu du budget P1
hot-path mais gated indépendamment pour qu'on attrape les
régressions cert-mgr.

### 5.3 Gating CI

#### 5.3.1 Pipeline CI bench (extension `bench.yml`)

```yaml
bench-p1-surface:
  runs-on: [self-hosted, gpu-t4]   # runner GPU obligatoire
  needs: [build-release]
  steps:
    - uses: actions/checkout@v4
    - name: Pull bundle MiniLM
      run: ./scripts/pull_bundle.sh miniLM-L12-v2 ${{ secrets.BUNDLE_SAS_URL }}
    - name: Start efe-server (3 replicas, docker-compose)
      run: docker compose -f deploy/compose-p1.yml up -d
    - name: Wait healthy
      run: timeout 60 bash -c 'until curl -fs http://localhost:50051/healthz; do sleep 1; done'
    - name: Run P1 sustained workload
      run: |
        ./bin/efe-bench run \
          --workload p1 \
          --duration 10m \
          --rps 200 \
          --output target/bench_p1_sustained.json
    - name: Compute SLO compliance
      run: |
        python eval/scripts/check_slo.py \
          --baseline bench_baselines/v1.0.0/baseline.json \
          --measure target/bench_p1_sustained.json
    - name: Gate on SLO-C-1
      run: |
        jq '.slo_compliance["SLO-C-1"].passed' target/bench_p1_sustained.json | grep -q true
    - uses: actions/upload-artifact@v4
      with:
        path: target/bench_p1_sustained.json
```

Le job échoue (rouge) si :
- `SLO-C-1.passed = false`,
- OU `host_fingerprint` du runner CI != fingerprint baseline,
- OU `SLO-PC-REPRO` détecte > 5 % drift.

#### 5.3.2 Skip rules

- `host_fingerprint` mismatch → SKIP propre avec `note: "Runner
  hardware diverge — mesure inopposable, voir RFC P1-Surface §1.5"`.
  Ne **bloque pas** le merge (le test est skipped, pas failed),
  mais déclenche une alerte slack `#sre`.
- Aucun runner GPU disponible → SKIP avec alerte `#capacity`.

#### 5.3.3 Cadence

- `pull_request` : SKIP par défaut (coût T4 ~$0.50 / run, on ne
  veut pas le payer sur chaque PR).
- `push main` : RUN à chaque merge.
- `schedule: nightly 02:00 UTC` : RUN sur baseline + master.
- `release tag v*` : RUN obligatoire, bloque la release si fail.

#### 5.3.4 Alerting prod

Règle Prometheus :

```yaml
- alert: P1_BURN_FAST
  expr: |
    (
      sum(rate(efe_swarm_duration_seconds_bucket{mode="mode1",le="0.2"}[1h]))
      /
      sum(rate(efe_swarm_duration_seconds_count{mode="mode1"}[1h]))
    ) < 0.99
  for: 5m
  labels: { severity: page, sla_clause: "C-1" }
  annotations:
    summary: "SLO C-1 burn rate 14.4× (1h window)"
    runbook: "https://docs.efe.ai/runbook/p1-burn-fast.html"
```

PagerDuty integration cible PD-EFE-CORE-P1.

### 5.4 Lint CI (compile-time gates)

```yaml
lint-p1-hotpath:
  steps:
    - run: |
        cargo clippy --workspace --all-targets -- \
          -D clippy::await_holding_lock \
          -D clippy::unwrap_used \
          -D clippy::expect_used \
          -D clippy::missing_panics_doc
    - run: cargo test --test p1_hot_path_lint --release
```

Le test `tests/p1_hot_path_lint.rs` (nouveau, livré sprint D19) :
- Parse via `syn` les modules listés §3.1.
- Reject toute occurrence de `Vec::new()`, `Box::new()`, `String::from()`
  hors-bootstrap.
- Reject tout `info!()`, `warn!()` sans `#[allow(p1_hot_path_log)]`
  + commentaire de justification.
- Reject tout `tokio::time::sleep` sur le hot-path.

---

## 6. Surface 4 — Tests (8 niveaux d'assurance)

### 6.1 Pyramide des tests P1

```
        ┌──────────────────────────┐
       8│  Chaos engineering tests │  1× / quarter
        ├──────────────────────────┤
       7│  Soak 24h GPU            │  1× / release
        ├──────────────────────────┤
       6│  Load tests (ghz)        │  1× / nightly
        ├──────────────────────────┤
       5│  Bench Criterion         │  1× / commit (push main)
        ├──────────────────────────┤
       4│  Integration end-to-end  │  1× / PR
        ├──────────────────────────┤
       3│  Fuzz tests              │  1× / PR (1 min budget)
        ├──────────────────────────┤
       2│  Unit tests (lint+logic) │  1× / save (LSP)
        ├──────────────────────────┤
       1│  AST lint p1_hot_path    │  1× / save (LSP via cargo check)
        └──────────────────────────┘
```

### 6.2 Tests Criterion (niveau 5) — par étage

Un binaire Criterion par étage P1 :

```
crates/efe-server/benches/
├── p1_stage_01_grpc_recv.rs        target ≤ 1.5 ms
├── p1_stage_02_pb_decode.rs        target ≤ 0.5 ms
├── p1_stage_03_interceptors.rs     target ≤ 3 ms
├── p1_stage_04_tokenize.rs         target ≤ 4 ms
├── p1_stage_05_onnx_cuda.rs        target ≤ 26 ms (GPU runner only)
├── p1_stage_06_genesis.rs          target ≤ 5 ms
├── p1_stage_07_fusion.rs           target ≤ 100 ms
├── p1_stage_08_consensus.rs        target ≤ 1 ms
├── p1_stage_09_dissolution.rs      target ≤ 6 ms
├── p1_stage_10_audit_chain.rs      target ≤ 4 ms
├── p1_stage_11_pb_encode.rs        target ≤ 1 ms
└── p1_stage_12_grpc_send.rs        target ≤ 1 ms

crates/efe-server/benches/
└── p1_e2e_in_proc.rs               target ≤ 153 ms (somme attendue),
                                    in-process (pas de socket réel)
```

Chaque bench Criterion produit un sample size ≥ 100 et un
`measurement_time` ≥ 5 s pour stabilité statistique. Le rapport
JSON est uploadé en CI artifact et comparé à `bench_baselines/.../bench_p1_*.json`.

### 6.3 Test AST lint (`p1_hot_path_lint`)

```rust
// tests/p1_hot_path_lint.rs
use syn::{parse_file, Item, ExprCall, visit::Visit};

const HOT_PATH_MODULES: &[&str] = &[
    "crates/efe-server/src/services/swarm.rs",
    "crates/efe-server/src/embedder/onnx_cuda.rs",
    "crates/efe-core/src/fusion/mvp_cpu.rs",
    "crates/efe-core/src/genesis/engine.rs",
    "crates/efe-core/src/consensus/multi_criteria.rs",
    "crates/efe-core/src/dissolution/engine.rs",
    "crates/efe-core/src/alignment/mod.rs",
];

const BANNED_CALLS: &[&str] = &[
    "Vec::new", "Vec::with_capacity",   // capacity OK via arena
    "Box::new",
    "String::from", "String::new",
    "tokio::time::sleep",
    "std::thread::sleep",
];

#[test]
fn p1_hot_path_has_no_banned_calls() {
    for path in HOT_PATH_MODULES {
        let src = std::fs::read_to_string(path).expect(path);
        let file = parse_file(&src).expect("parse fail");
        let mut v = BannedCallVisitor::new(path);
        v.visit_file(&file);
        assert!(v.violations.is_empty(),
                "P1 hot-path violations in {path}: {:#?}", v.violations);
    }
}
```

### 6.4 Tests d'invariance (niveau 4)

- `bench_p1_solo_request_does_not_wait_for_batch` :
  vérifie qu'une requête isolée (queue vide) ne paye pas
  `max_wait_micros` du coalescer.
- `bench_p1_warm_path_zero_alloc_in_steady_state` : compte les
  allocations via `dhat` profiler ; doit ≤ 3 / requête (cf. §3.2
  étages 4, 5, 6 — autres = 0).
- `bench_p1_cancel_propagates_within_5ms` : un client qui ferme
  son stream doit voir le swarm s'arrêter dans ≤ 5 ms (sinon
  on continue à brûler du budget GPU pour rien).
- `bench_p1_handshake_excluded_from_budget` : mesure que `t=0` est
  bien après le handshake.

### 6.5 Load tests (ghz, niveau 6)

```yaml
# bench/load/p1_sustained.yaml
proto: ./proto/swarm.proto
call: efe.swarm.SwarmService.Execute
import-paths: [./proto]
n: 240000              # 200 RPS × 20 min × 60 s ≈ 240k requests
c: 64                  # 64 concurrent goroutines
rps: 200
duration: 20m
data-file: ./bench/load/p1_payloads.json   # 100 payloads variés
host: localhost:50051
insecure: false
cert: ./certs/client.pem
key:  ./certs/client-key.pem
cacert: ./certs/ca.pem
output: target/bench_p1_sustained.json
format: json
```

### 6.6 Soak test 24 h (niveau 7)

Cf. `d9_load_soak_report.md` + nouvelle extension P1 :
- Charge : 100 RPS constant 24 h.
- Critère PASS :
  - p99 latency reste ≤ 200 ms tout du long,
  - aucune fuite mémoire détectée par OLS (slope < 1 MiB/h),
  - aucun memory pressure event (GC pause, OOM kill),
  - audit chain integrity check 100 % (1 verify all-the-chain
    chaque 1 h, accumulé).

### 6.7 Chaos engineering (niveau 8)

| Scénario | Méthode | Critère PASS |
|----------|---------|--------------|
| GPU eviction (replica perd son GPU temporairement) | `nvidia-smi drain` | failover ≤ 30 s, p99 SLO reste vert |
| Network partition entre 2 réplicas | iptables drop | leader re-élection ≤ 10 s, P1 dégradé < 50 % du trafic |
| Disk full (audit chain write fail) | `dd if=/dev/zero of=/var/lib/efe/junk` | back-pressure activée, P1 retourne 503 (pas un latency violation) |
| ONNX session crash (signal SIGBUS forcé) | gdb attach + signal | restart < 5 s, P1 dégradé 5 s puis vert |
| Catalogue corruption (hash mismatch) | flip 1 bit sur disque | détection au boot, replica isolé, P1 routé vers les 2 autres |
| Tokio worker starvation (1 worker bloqué CPU spin) | infinite-loop futur injecté | scheduler steal, p99 dégradé < 20 % |
| Tail-latency injection (sleep 500 ms 1 % du temps) | mock embedder | SLO-P1-TAIL-1 (p99.9 ≤ 400 ms) déclenche alerte |
| Memory leak (Box leak 1 MiB / req) | injection compilée | OOM kill, restart, P1 vert post-restart |

---

## 7. Surface 5 — Défensive (12 garde-fous runtime)

### 7.1 Inventaire

| # | Nom | Localisation | Effet |
|--:|-----|--------------|-------|
| 1 | **Per-stage deadline propagation** | tokio context | Cancel auto si budget restant < estimation étage |
| 2 | Per-request **timeout 250 ms** | tonic middleware | Force-cancel toute requête > 250 ms (= 200 ms + 25 % marge) |
| 3 | **Request queue size cap** | services/swarm.rs | Reject (RESOURCE_EXHAUSTED) si queue > 100 |
| 4 | **Backpressure leaky bucket** | interceptors/rate_limit | Bucket per-tenant, refill 250 RPS, capacity 50 |
| 5 | **Circuit breaker embedder** | embedder/onnx_cuda.rs | Si 5 ERREURS consécutives → OPEN 30 s |
| 6 | **GPU OOM guard** | embedder/onnx_cuda.rs | Si VRAM > 90 % → reject batch, fail-fast |
| 7 | **Arena pre-warm at boot** | runtime.rs | 100 dummy inferences avant accept |
| 8 | **Worker thread pinning (NUMA-aware)** | runtime.rs | `numactl --cpunodebind=0` |
| 9 | **CUDA stream affinity** | embedder/onnx_cuda.rs | 1 stream / worker Tokio (cap 4) |
| 10 | **Slow-request tracer** | middleware | Si p99 outlier détecté → flame graph snapshot pushed |
| 11 | **Auto-shed under SLO burn** | rate_limit_leaky_bucket.rs | Si SLO_BURN_FAST > 50 % budget mensuel → reject 10 % du trafic |
| 12 | **Read-only mode on audit chain fail** | v1/audit_chain.rs | Si write fail → return 503, audit safe (pas de verdict non-audité) |

### 7.2 Détail 1 — Deadline propagation

```rust
// services/swarm.rs (extrait)
async fn execute(&self, req: Request<ExecuteRequest>) -> Result<Response<...>, Status> {
    // Le client gRPC envoie un `grpc-timeout` header. Si absent,
    // on injecte la deadline P1 par défaut.
    let deadline = req.metadata().get("grpc-timeout")
        .and_then(|v| parse_grpc_timeout(v))
        .unwrap_or(Duration::from_millis(250));
    let deadline_instant = Instant::now() + deadline;

    // Chaque étage reçoit `BudgetTracker` qui sait combien il
    // reste, et est responsable de checker.
    let tracker = BudgetTracker::new(deadline_instant);

    // Étage 4 : tokenization (budget alloué = 4 ms ; au-delà → cancel)
    let tokens = tokio::time::timeout(
        tracker.allocate_for_stage(Stage::Tokenize, Duration::from_millis(4))?,
        self.embedder.tokenize(&req.text),
    ).await
     .map_err(|_| Status::deadline_exceeded("tokenize budget exceeded"))?;

    // ... etc. pour chaque étage
}
```

`BudgetTracker` est testé par `tests/p1_budget_tracker.rs` qui
vérifie :
- chaque étage reçoit sa quote-part,
- une étape qui dépasse fait échouer la requête, mais **n'affecte
  pas** les requêtes en parallèle,
- la métrique `efe_p1_budget_overrun_total{stage="..."}` est
  incrémentée correctement.

### 7.3 Détail 11 — Auto-shed under SLO burn

Le système calcule en continu `error_budget_remaining_ratio` (cf.
SRE Workbook). Quand il chute sous 50 %, on commence à shed le
trafic préventivement :

```python
# Pseudo-code de la logique de shed
budget = 1.0 - (current_violations / monthly_allowance)
if budget < 0.50:
    shed_rate = (0.50 - budget) * 2   # ramp linéaire 0 → 100 %
    if random.random() < shed_rate:
        return Err(Status::resource_exhausted("preserving SLO budget"))
```

Compromise : on **dégrade l'availability** (return 503 / RESOURCE_EXHAUSTED)
pour **protéger la latence** (les requêtes restantes restent dans
le budget). C'est défendable contractuellement parce que :
- Le SLA EFE distingue clairement P1 (latence) de C-12 (availability ≥ 99.9 %).
- Une requête 503 est techniquement une "perte" mais ne viole **pas**
  la promesse "200 ms p99 sur les requêtes servies".

---

## 8. Surface 6 — Profilage & outillage perf

### 8.1 Stack outillage

| Outil | Usage | Cadence |
|-------|-------|---------|
| `cargo bench` (Criterion) | micro-bench par étage | dev + CI |
| `cargo flamegraph` | flame graph en dev | manuel |
| `tokio-console` | traces async, blocking, busy waiters | manuel |
| `nvprof` / `nsight-compute` | kernels CUDA | manuel + nightly |
| `perf` (Linux) | low-level CPU profiling | manuel |
| `dhat` | allocations | dev + CI test alloc-count |
| `pprof` (Go-style, via `pprof-rs`) | sample profiler en prod | toujours actif, 1 % sampling |
| `eBPF` (bpftrace) | tail-latency, scheduler, page faults | on-demand |
| `Pyroscope` | continuous profiling cluster-wide | always-on prod |
| `efe-bench` (D5, D9, D18) | bench harness home-built | CI + manuel |
| `efe-server tune` (Φ-D) | auto-tune GPU env-vars | post-deploy |

### 8.2 Tableau de bord Grafana « P1 SLO »

Dashboard `efe-p1-slo.json` (à livrer sprint D19) :
- Panel 1 : `efe_swarm_duration_seconds` p50 / p95 / p99 / p99.9
  ligne (rolling 5 min) avec seuil 200 ms tracé en rouge.
- Panel 2 : burn-down du budget mensuel (% restant).
- Panel 3 : 12 sub-panels — un par étage — qui montrent
  `efe_p1_<stage>_duration_seconds` p99 vs target.
- Panel 4 : RPS effectif (`efe_swarm_duration_seconds_count`).
- Panel 5 : Top 10 traces les plus lentes des dernières 15 min
  (lien direct vers Tempo / Jaeger).
- Panel 6 : Heatmap latency × time (granularité 1 min, 200 ms
  Y-axis cap).

### 8.3 Procédure post-mortem latency regression

1. **Détection** : alerte `P1_BURN_FAST` (cf. §5.3.4).
2. **Containment** (≤ 15 min) : si récent deploy, rollback ;
   sinon shed traffic (gardefou #11).
3. **Investigation** (≤ 1 h) : flame graph snapshot le plus récent
   + traces OTel des slow-outliers + diff git depuis dernier
   GREEN.
4. **Mitigation** (≤ 4 h) : hotfix ou rollback définitif.
5. **RCA publique** (≤ 24 h) : posted in runbook client +
   public status page.
6. **Post-mortem interne** (≤ 5 j) : blameless review, action
   items, ajout test régression.

Chaque RCA enrichit le corpus de chaos engineering (§6.7).

---

## 9. Surface 7 — Contractuelle & opposabilité juridique

### 9.1 Modèle de clause SLA P1 (extrait pour client commercial)

> ## §3.1 — Engagement de service de latence (« P1 »)
>
> Le Fournisseur garantit que **99 % des requêtes Mode-1**
> (paramètre `mode=1` au sens de la documentation technique) émises
> par le Client à destination de l'environnement de production de
> l'Espace Latent Éphémère, dans les conditions matérielles
> définies en Annexe A (« Conditions de référence »), seront
> servies en **moins de 200 millisecondes (200 ms)** mesurées du
> côté client par horloge système monotone (CLOCK_MONOTONIC).
>
> La mesure de référence est calculée sur des **fenêtres mensuelles
> calendaires**. Le Fournisseur produit chaque mois un Rapport SLA
> contenant : (i) le p99 mesuré, (ii) le burn-rate au 1 h / 6 h / 24 h,
> (iii) la liste des incidents documentés ayant impacté la mesure,
> (iv) le `host_fingerprint_sha256` de l'environnement de mesure.
>
> ### Pénalités
> En cas de violation soutenue (p99 > 200 ms sur ≥ 7 jours dans le
> mois), le Fournisseur consent au Client un crédit de service
> égal à 10 % de la facture mensuelle. Au-delà de 14 jours, le
> crédit est porté à 25 %. La violation est constatée par un
> tiers technique sur demande du Client, frais à la charge du
> Fournisseur si la violation est confirmée.

### 9.2 Annexe A type (conditions matérielles)

Reproduit exactement §1.5 du présent document, avec mention :
*« Toute modification matérielle suspend l'opposabilité jusqu'à
nouvelle baseline signée. »*

### 9.3 Procédure de constatation par tiers

Le tiers technique (ex. cabinet d'audit indépendant) :
1. Reçoit un accès read-only à `/metrics` Prometheus + Loki / Tempo
   read access.
2. Reçoit le `bench_baselines/v1.0.0/baseline.json` signé cosign.
3. Récupère les rapports SLA mensuels signés HMAC-SHA-256 par le
   Fournisseur.
4. Recalcule p99 mensuel à partir des buckets Prometheus exportés.
5. Vérifie le HMAC de chaque rapport vs la chaîne d'audit D14-bis.
6. Si tout matche → atteste la conformité ou la violation.

### 9.4 Outil `efe-attest`

Nouveau binaire à livrer sprint D40 (Φ-G) :

```bash
$ efe-attest verify-sla \
    --baseline bench_baselines/v1.0.0/baseline.json \
    --reports reports/2026-04.json reports/2026-05.json ... \
    --audit-chain s3://efe-audit/2026-q2.chain \
    --fingerprint-baseline 0a3b...c91f
```

Sortie :
```
[PASS] 2026-04 : p99 = 187 ms, burn = 0.34, fingerprint MATCH
[PASS] 2026-05 : p99 = 192 ms, burn = 0.48, fingerprint MATCH
[FAIL] 2026-06 : p99 = 213 ms, burn = 1.14, fingerprint MATCH
       └─ Violation soutenue : 8/30 jours rouges
       └─ Crédit dû : 10 % de la facture mensuelle (clause §3.1)
```

Le tool fait partie du SDK signé cosign — un client peut le runner
sur ses propres rapports pour vérifier l'attestation du Fournisseur.

---

## 10. Surface 8 — Économique (TCO du SLO)

### 10.1 Coût défensif vs coût offensif

| Coût | Description | Montant annuel ordre de grandeur |
|------|-------------|----------------------------------|
| **Offensif** | GPU T4 dédié 3× réplicas (avg $0.35/h × 3 × 8760 h) | ≈ 9 200 $ |
| Offensif | Continuous profiling Pyroscope SaaS | ≈ 3 600 $ |
| Offensif | Self-hosted GPU runner CI (~$0.50 × 365 nightly + ~50 release) | ≈ 210 $ |
| Offensif | Tier 1 on-call (2 SRE × 25 % de leur capacité) | ≈ 80 000 $ |
| Sub-total offensif | | **≈ 93 000 $** |
| **Défensif** | Pénalités SLA si une violation 10 % × revenue annuel | dépend du book ; cas type 100 k$ ARR → 10 k$ |
| Défensif | RCA + post-mortem (10 h SRE × 4-5 RCAs / an) | ≈ 5 000 $ |
| Défensif | Audit tiers (1× / an) | ≈ 8 000 $ |
| Sub-total défensif (cas type 100 k$ ARR) | | **≈ 23 000 $** |
| **Coût total annuel** | | **≈ 116 000 $** |

### 10.2 Break-even

Le revenue annuel doit être ≥ ~120 k$ pour que la promesse P1
opposable soit économiquement défendable. **En dessous**, on shipperait
P1 sans clause SLA (= promesse non-opposable), ou avec un seuil
plus relâché (300-400 ms).

### 10.3 Marge SLO vs marge produit

| Stratégie | p99 cible | Marge anti-tail | Coût infra (relative) | Risque burn |
|-----------|----------:|----------------:|:--:|:--:|
| Aggressive | 150 ms | 5 ms | 2× | élevé |
| **Choisi (P1)** | **200 ms** | **47 ms** | **1×** | **bas** |
| Conservative | 300 ms | 100 ms | 0.7× | très bas |

L'analyse de Pareto incluse en Annexe B montre que **200 ms maximise
la valeur commerciale par dollar d'infra** dans la fenêtre d'usage
typique (10-1000 RPS, 95 % requêtes textes < 256 tokens).

---

## 11. Matrice complète des artefacts P1

> Cette matrice est la liste exhaustive de tout ce qui doit
> exister pour que la promesse P1 soit opposable. Elle est
> auditable item-par-item.

| # | Artefact | État cible v1.0 | Sprint livreur |
|--:|----------|:--:|:--:|
| 1 | `docs/EFE_P1_SURFACE_ARCHITECTURE.md` (ce doc) | ✅ | D19 |
| 2 | `crates/efe-server/benches/p1_stage_01..12*.rs` | ✅ | D19-D22 |
| 3 | `crates/efe-server/benches/p1_e2e_in_proc.rs` | ✅ | D21 |
| 4 | `tests/p1_hot_path_lint.rs` | ✅ | D19 |
| 5 | `tests/p1_budget_tracker.rs` | ✅ | D19 |
| 6 | `tests/p1_invariance_*.rs` (4 tests) | ✅ | D20 |
| 7 | `bench/load/p1_sustained.yaml` (ghz config) | ✅ | D20 |
| 8 | `bench/load/p1_payloads.json` (100 payloads) | ✅ | D20 |
| 9 | `bench_baselines/v0.2.0..v1.0.0/baseline.json` (SLO-P1-*) | ✅ | D20-D42 |
| 10 | `crates/efe-server/src/embedder/batch_coalesce.rs` | ✅ | D26 (Φ-C) |
| 11 | `crates/efe-kernels/src/fused_softmax_entropy.cu` | ✅ | D28-D30 (Φ-D) |
| 12 | `crates/efe-server/src/runtime.rs` (NUMA + Tokio config) | ✅ | D19 |
| 13 | `crates/efe-server/src/runtime.rs` (CUDA stream pool) | ✅ | D28 |
| 14 | `deploy/compose-p1.yml` (3-replica T4 setup) | ✅ | D20 |
| 15 | `deploy/grafana/efe-p1-slo.json` | ✅ | D19 |
| 16 | `deploy/prometheus/p1_alerts.yml` | ✅ | D19 |
| 17 | `.github/workflows/bench-p1-surface.yml` | ✅ | D19 |
| 18 | `clients/python/efe_tool/attest.py` (CLI `efe-attest`) | ✅ | D40 (Φ-G) |
| 19 | `docs/runbook/p1_burn_fast.md` | ✅ | D19 |
| 20 | `docs/runbook/p1_post_mortem_template.md` | ✅ | D19 |
| 21 | `docs/sla/sla_template.md` (clause §3.1) | ✅ | D40 |
| 22 | `docs/audits/p1_attestation_2026Q3.md` (1× / Q) | ✅ | continu post-GA |
| 23 | `eval/scripts/check_slo.py` (extension : 17 SLO P1) | ✅ | D19 |
| 24 | `efe-bench` workload `p1` (sustained, smoke, stress, spike) | ✅ | D19 |
| 25 | `crates/efe-server/src/runtime.rs` (pre-warm 100 inferences) | ✅ | D19 |
| 26 | `crates/efe-telemetry/src/p1_metrics.rs` (12 instruments §4.2) | ✅ | D19 |
| 27 | OTel span attrs `budget_remaining_ms` | ✅ | D19 |
| 28 | `crates/efe-server/src/middleware/budget_tracker.rs` | ✅ | D19 |
| 29 | `tests/p1_chaos_*.rs` (8 scénarios §6.7) | ✅ | D31 |
| 30 | `efe-server tune` subcommand | ✅ | D32 |

→ **30 artefacts**. C'est la définition exhaustive de "P1 Surface".

---

## 12. Plan d'implémentation (mapping sprints D19 → D32)

### 12.1 Vue d'ensemble

```
D19  ───── Sprint P1-Foundation ─────  Φ-A
           - Doc P1 surface (ce doc)
           - 12 SLO P1-BUDGET-*
           - 12 Prom metrics
           - Lint hot-path (AST test)
           - Budget tracker middleware
           - Runbooks burn-fast / RCA
           - Grafana dashboard
           - Prometheus alerts

D20  ───── Sprint P1-CI-GPU ─────  Φ-A
           - Self-hosted GPU runner
           - bench-p1-surface workflow
           - ghz P1 sustained config
           - 100 payloads dataset
           - First baseline v0.2.0

D21  ───── Sprint P1-Bench-Stages ─────  Φ-B
           - 12 Criterion benches par étage
           - p1_e2e_in_proc bench
           - Documentation des cibles par étage
           - Pin baselines v0.2.x

D22-D26 ─── Φ-C : Bundle réel + PyO3 ───
           - Bundle MiniLM réel
           - Batch coalescer
           - Soak 24 h CPU baseline
           - SLO C-2 (CPU 800 ms) attesté
           
D28-D32 ─── Φ-D : CUDA kernels + GPU ───
           - Fused softmax + entropy kernel
           - CUDA stream pool runtime
           - Multi-GPU sharding
           - Tune subcommand
           - SLO C-1 (GPU 200 ms) attesté
           - Chaos engineering 8 scénarios
           
D40  ───── Sprint P1-Attestation ────  Φ-G
           - efe-attest binary
           - Modèle clause SLA
           - Premier audit tiers
           - Procédure RCA standardisée

D42  ───── GA v1.0.0 ─────
           - 17 SLO P1 PASS attestés
           - Audit chain integrity 100 %
           - 30 artefacts P1 livrés
```

### 12.2 Détail D19 — Sprint P1-Foundation (9 j wallclock)

| Jour | Tâche | Output |
|----:|-------|--------|
| J1 | Écrire `docs/EFE_P1_SURFACE_ARCHITECTURE.md` (ce doc) | doc commit |
| J2 | Écrire `crates/efe-telemetry/src/p1_metrics.rs` (12 histograms) | crate compile + 12 tests |
| J3 | Écrire `crates/efe-server/src/middleware/budget_tracker.rs` | middleware + 6 tests |
| J4 | Écrire `tests/p1_hot_path_lint.rs` (AST visitor) | test runs, ferme PR sur violation |
| J5 | Écrire les alerts Prometheus + dashboard Grafana | YAML / JSON |
| J6 | Wire les histograms dans les 7 étages les plus critiques (1, 5, 7, 10, 11) | refactor minimal |
| J7 | Écrire runbooks `p1_burn_fast.md` + RCA template | docs |
| J8 | Audit interne sprint (8-12 bugs typiques cf. patterns D8/D18) | audit doc |
| J9 | Tag `v0.2.0-rc1`, freeze baseline initiale (CPU only encore) | tag + signed baseline |

### 12.3 Détail D20 — Sprint P1-CI-GPU (10 j wallclock)

J1 : provisioning self-hosted runner GPU T4 (AWS g4dn.xlarge ou
GCP n1-standard-4 + T4).
J2 : `bench-p1-surface.yml` workflow.
J3 : ghz config + 100 payloads (random sampling de catalogs réels
anonymisés).
J4-J5 : itérer jusqu'à GREEN sur runner.
J6 : `efe-bench run --workload p1` end-to-end.
J7 : intégration `check_slo.py` étendu pour les 17 SLO P1.
J8 : test régression : faire fail un bench artificiellement
(injecter 50 ms) pour valider que CI bloque correctement.
J9 : doc CI dans `docs/operations/p1_ci_gpu.md`.
J10 : sprint audit + freeze baseline `v0.2.0`.

### 12.4 Mapping sprints → artefacts

Cf. §11 ci-dessus, colonne "Sprint livreur".

---

## 13. Risques identifiés & mitigations (16 risques)

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | CUDA kernels custom (Φ-D) ne battent pas l'ONNX CUDA EP générique | HIGH | MED | Bench CPU MVP vs ONNX CUDA EP vs custom kernel, gate `cargo bench` ; revenir à ONNX si custom < 1.5× |
| 2 | Bundle MiniLM réel = inférence > 30 ms | HIGH | LOW | Pre-warm + batch coalescer ; alternative quantized fp16 baseline |
| 3 | Multi-GPU sharding : NCCL overhead > saving | MED | MED | Test seuil empirique N=512 ; sinon désactivé |
| 4 | Tail latency p99.9 dérive (jitter scheduler) | MED | HIGH | `numactl` pinning + isolcpus boot param + SLO-P1-TAIL-1 alerte précoce |
| 5 | Audit chain HMAC append > 4 ms | LOW | LOW | Pool HMAC contexts + batch fsync (D14-bis) |
| 6 | Self-hosted GPU runner CI down | MED | MED | Backup runner T4 + skip rules CI |
| 7 | `host_fingerprint` mismatch après upgrade Kernel | LOW | HIGH | Procedure documented + baseline re-sign rituel mensuel |
| 8 | Pénalités SLA payées si soutenu violation 7+ j | HIGH (financière) | LOW (avec marge 47 ms) | Auto-shed under burn (gardefou #11) protège la mesure |
| 9 | Coût T4 dédié 3 réplicas dépasse $9k/an attendu | LOW | LOW | Spot instances si environnement non-prod |
| 10 | `cargo bench` Criterion flaky sur GPU runner | MED | MED | `measurement_time ≥ 5 s` + `sample_size ≥ 100` + outlier rejection |
| 11 | OTel trace cardinality explose (1 span / round × 200 RPS) | LOW | MED | Tail sampling 1 % + adaptive sampling sur slow-traces |
| 12 | Pyroscope SaaS lock-in | LOW | LOW | Alternative pprof-rs self-hosted documentée |
| 13 | NUMA-aware pinning casse sur cloud VMs (numa nodes virtuels) | MED | HIGH | Fallback : si `numactl --hardware` retourne 1 node → désactive pinning |
| 14 | Test AST lint trop strict casse les PR refactor légitimes | LOW | MED | `#[allow(p1_alloc)]` escape hatch documenté |
| 15 | `efe-attest` tool nécessite SDK release mais le SDK est lui-même bloqué par P1 | LOW | LOW | Ship `efe-attest` en standalone binary indépendant du SDK Python |
| 16 | Chaos engineering tests cassent l'environnement nightly | MED | LOW | Isolation via namespace k8s dédié + cleanup auto |

---

## 14. Décisions de conception clés (15)

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | Budget 200 ms (vs 100 ms ou 500 ms) | 100 ms infaisable, 500 ms casse argument commercial | §1.3 |
| 2 | 12 étages explicites (vs 1 SLO racine) | 1 SLO global | Permet d'attribuer la violation à un étage précis ; gating sur 12 SLO budget |
| 3 | Self-hosted GPU runner CI (vs GitHub-hosted) | Pas de GPU sur GH-hosted en 2026 | Coût $210/an raisonnable |
| 4 | `host_fingerprint` obligatoire | Confiance dans l'output sans contexte | Opposabilité juridique stricte |
| 5 | Auto-shed under burn (vs SLO violation acceptée) | Garder la haute availability | Préserve P1 ; SLA distingue P1 vs C-12 |
| 6 | Custom CUDA kernel (vs ONNX CUDA EP partout) | ONNX CUDA EP générique | Levier 80/20 sur l'étage 7 (65 % du budget) |
| 7 | Pre-warm 100 dummy inferences au boot | Lazy initialization | Premier query post-deploy ne paye pas le cost JIT compile |
| 8 | Batch coalescer côté serveur (vs côté SDK) | Délègue au client | Cohérence et fairness inter-tenants |
| 9 | Test AST lint hot-path (vs convention) | Convention + code review | Hard-enforce, impossible à bypasser silencieusement |
| 10 | Marge anti-tail 47 ms (vs 20 ms) | Plus serré | Données empiriques D18 montrent tail jitter jusqu'à 40 ms |
| 11 | SLO P1-BURN multi-window (1h / 6h / 24h) | Single window | Distingue panne aiguë vs dérive lente |
| 12 | `efe-attest` binaire standalone (vs intégré SDK) | Couplage SDK | Permet vérification tiers sans SDK installé |
| 13 | Continuous profiling Pyroscope always-on (vs on-demand) | On-demand seulement | $3.6k/an raisonnable, détection précoce dérive |
| 14 | Bench Criterion par étage + e2e (vs e2e seul) | Bench end-to-end seulement | Attribution fine des régressions |
| 15 | OTel attr `budget_remaining_ms` | Spans seuls sans attr | Permet filtres post-mortem ciblés |

---

## 15. Anti-régression & realism check P1

### 15.1 Anti-régression

Tous les SLO P1-* sont gatés dans CI ; aucune régression admise
en main. Le baseline `bench_baselines/v0.2.0..v1.0.0/baseline.json`
contient l'historique frozen ; chaque version doit montrer un
delta ≥ 0 (i.e. amélioration ou stabilité) vs la version
précédente.

### 15.2 Realism check

- **Mesure** : doit être faite sur du hardware réel, pas sur un
  émulateur GPU. Le `host_fingerprint` est rejeté si
  `gpu_model = None` pour SLO-C-1.
- **Catalogue** : doit être un catalogue de production-like (au
  moins 3 catalogs synthétiques D8 + 1 catalog réel D8.2 quand
  disponible). Refuse le test "1-catalog-trivial".
- **Charge** : doit être ≥ 100 RPS pendant ≥ 5 min pour qu'un p99
  soit statistiquement défendable (≥ 30 000 échantillons).
- **Audit chain** : doit être active (pas en `--no-audit` debug).

### 15.3 Sanity checks au démarrage

`efe-server` au démarrage :
1. Vérifie que GPU est détecté si `EFE_MODE1_GPU=true`.
2. Vérifie que bundle est présent et hash signé matche.
3. Pre-warm 100 inferences (étage 5).
4. Émet un log `P1_BOOT_READINESS` avec p50 / p99 du pre-warm.
5. Si p99 du pre-warm > 50 ms → log warning et refuse de
   passer `ready=true` au healthz (le service ne sera pas routé
   par le load balancer tant que pre-warm n'est pas conforme).

---

## 16. Critères d'acceptation (DoD P1-Surface)

### 16.1 Code

- [ ] 30 artefacts P1 livrés (§11).
- [ ] 12 histograms Prometheus émis sur chaque ExecuteRequest.
- [ ] `BudgetTracker` propagé sur les 12 étages.
- [ ] 12 garde-fous runtime actifs (§7.1).
- [ ] Test AST lint p1_hot_path GREEN.
- [ ] Test budget tracker GREEN.
- [ ] 4 tests invariance GREEN.

### 16.2 CI

- [ ] Job `bench-p1-surface` runs nightly sur runner GPU.
- [ ] 17 SLO P1 gatés dans `check_slo.py`.
- [ ] Failure d'un SLO bloque le merge main (push) et la release tag.
- [ ] Artifact `bench_p1_sustained.json` uploadé chaque run.

### 16.3 Mesure live

- [ ] p99 ≤ 200 ms (SLO C-1) mesuré sur baseline `v1.0.0`.
- [ ] p99.9 ≤ 400 ms (SLO-P1-TAIL-1).
- [ ] Reproductibilité ≤ 5 % drift (SLO-PC-REPRO).
- [ ] Soak 24 h GREEN (cf. §6.6).
- [ ] 8 chaos scenarios GREEN.

### 16.4 Documentation

- [ ] `docs/EFE_P1_SURFACE_ARCHITECTURE.md` (ce doc).
- [ ] `docs/runbook/p1_burn_fast.md`.
- [ ] `docs/runbook/p1_post_mortem_template.md`.
- [ ] `docs/sla/sla_template.md` clause §3.1.
- [ ] `docs/audits/p1_attestation_<year>Q<n>.md` chaque trimestre.
- [ ] CHANGELOG v1.0.0 mentionne explicitement P1 PASS.

### 16.5 Contractuel

- [ ] Modèle de clause SLA validé juridiquement (cabinet externe).
- [ ] Annexe A « conditions matérielles » signée.
- [ ] `efe-attest` binaire publié + signé cosign.
- [ ] Procédure tiers vérificateur documentée.

### 16.6 Économique

- [ ] Coût annuel infra GPU ≤ 12 k$ par tenant moyen.
- [ ] Break-even ARR ≥ 120 k$ documenté.
- [ ] Marge SLO 47 ms validée empiriquement (cf. §10.3).

### 16.7 Audit interne (1× / sprint)

- [ ] 8+ bugs trouvés, 8+ résolus dans le sprint livreur.
- [ ] CI verte post-fix.
- [ ] No clippy warning sur efe-server, efe-core, efe-kernels.

---

## 17. Hand-off & dépendances downstream

### 17.1 Dépendances entrantes (ce que P1 attend des autres sprints)

| Sprint | Livré | Ce que P1 réutilise |
|--------|-------|--------------------|
| D4 | gRPC server stack | étages 1, 2, 11, 12 |
| D5 | Bench harness D5 + framework SLO | check_slo.py infra |
| D6 | Consensus | étage 8 |
| D9 | Load test ghz + soak D9 | base du load test P1 |
| D13 | Dashboards Grafana | template du P1 SLO dashboard |
| D13-bis | OTel collector | trace pipeline |
| D14-bis | Audit chain HMAC | étage 10 |
| D16 | Embedder bundle | étage 5 (bundle MiniLM) |
| D17 | Backup / DR | sécurité audit chain |
| D18 | Multi-réplica + bench D12 | base 3-replica setup |

### 17.2 Dépendances sortantes (ce que P1 débloque)

| Sprint downstream | Ce qu'il prend de P1 |
|-------------------|----------------------|
| Φ-C (D22-D26) | Bundle réel mesuré contre P1 SLO |
| Φ-D (D28-D32) | CUDA kernels mesurés contre P1 SLO (gate Φ-D.1) |
| Φ-E (D33-D36) | Mode-2 prend ce qui reste du budget après LLM call |
| Φ-F (D37-D39) | Mode-3 calibré comme 3× Mode-1 |
| Φ-G (D40-D42) | `efe-attest` et compliance, clause SLA |
| GA v1.0.0 | SLO C-1 PASS attesté = critère de release |

### 17.3 Risque : ordre des sprints

Si Φ-D (CUDA) glisse, alors :
- Le SLO C-1 (GPU 200 ms) ne peut pas être PASS.
- Le SLO C-2 (CPU 800 ms) reste valable mais ne tient pas la
  promesse P1 (qui est GPU-spécifique).
- Décision contractuelle : **différer P1 opposable** à la version
  qui ship CUDA. Annoncer en CHANGELOG v0.5.0+ → la clause SLA est
  activée à cette version. Les v0.2..v0.4.x sont marquées
  « technical preview, P1 not contractual ».

---

## 18. Annexes (A → F)

### Annexe A — Liste exhaustive des SLO P1

```yaml
# bench_baselines/v1.0.0/p1_slos.yaml
slos:
  - id: SLO-C-1
    description: Mode-1 GPU p99 latency
    metric: efe_swarm_duration_seconds_bucket{mode="mode1"}
    quantile: 0.99
    comparator: lte
    threshold_ns: 200_000_000
    
  - id: SLO-C-2
    description: Mode-1 CPU p99 latency
    metric: efe_swarm_duration_seconds_bucket{mode="mode1"}
    quantile: 0.99
    comparator: lte
    threshold_ns: 800_000_000

  - id: SLO-PC-REPRO
    description: Reproductibilité 2 mesures consécutives
    metric: derived from 2 runs
    comparator: lte
    threshold_ratio: 0.05

  - id: SLO-PC-mTLS-INIT
    description: mTLS handshake p99
    metric: tonic_tls_handshake_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 60_000_000

  - id: SLO-P1-BUDGET-1
    description: TCP recv + TLS framing p99
    metric: efe_p1_grpc_recv_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 1_500_000

  - id: SLO-P1-BUDGET-2
    description: Protobuf decode p99
    metric: efe_p1_grpc_decode_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 500_000

  - id: SLO-P1-BUDGET-3
    description: gRPC interceptors p99
    metric: efe_p1_interceptor_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 3_000_000

  - id: SLO-P1-BUDGET-4
    description: Tokenization p99
    metric: efe_p1_embed_tokenize_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 4_000_000

  - id: SLO-P1-BUDGET-5
    description: ONNX CUDA inference p99
    metric: efe_p1_embed_inference_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 26_000_000

  - id: SLO-P1-BUDGET-6
    description: Genesis p99
    metric: efe_p1_genesis_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 5_000_000

  - id: SLO-P1-BUDGET-7
    description: Fusion 10 rounds p99
    metric: efe_p1_fusion_round_duration_seconds_bucket  # sum over 10
    quantile: 0.99
    comparator: lte
    threshold_ns: 100_000_000

  - id: SLO-P1-BUDGET-8
    description: Consensus p99
    metric: efe_p1_consensus_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 1_000_000

  - id: SLO-P1-BUDGET-9
    description: Dissolution + Alignment p99
    metric: efe_p1_dissolution_alignment_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 6_000_000

  - id: SLO-P1-BUDGET-10
    description: Audit chain append p99
    metric: efe_p1_audit_chain_append_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 4_000_000

  - id: SLO-P1-BUDGET-11
    description: Protobuf encode p99
    metric: efe_p1_grpc_encode_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 1_000_000

  - id: SLO-P1-BUDGET-12
    description: TCP send p99
    metric: efe_p1_grpc_send_duration_seconds_bucket
    quantile: 0.99
    comparator: lte
    threshold_ns: 1_000_000

  - id: SLO-P1-BURN
    description: Multi-window burn rate
    metric: derived (1h vs 6h vs 24h windows)
    comparator: lte
    thresholds:
      fast_burn_1h:   14.4
      slow_burn_6h:    6.0
      ticket_24h:      1.0

  - id: SLO-P1-TAIL-1
    description: p99.9 latency
    metric: efe_swarm_duration_seconds_bucket{mode="mode1"}
    quantile: 0.999
    comparator: lte
    threshold_ns: 400_000_000
```

### Annexe B — Pareto analyse marge SLO vs coût infra

```
Coût relatif (T4 par tenant moyen, normalisé à 1.0 = 200 ms target)

   2.0│●  150ms
      │
   1.5│
      │       ●  170ms
      │
   1.0│             ●  200ms (choisi)
      │
   0.7│                       ●  300ms
      │
   0.5│                                ●  500ms
      └──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬───
         150 170 200 220 250 300 350 400 500 ms (target p99)

* 150 ms : nécessite multi-GPU systématique → 2× T4 = 2× coût
* 170 ms : marge anti-tail trop courte (~20 ms) → burn élevé → SLA
            pénalités probables → 1.5× coût effectif
* 200 ms : pareto-optimum (1.0×)
* 300 ms : SLO trop relâché vs marché LLM → perte de leverage
            commercial → revenue 30 % plus bas → mauvais TCO
* 500 ms : aucune justification GPU → CPU only → 0.5× infra
            mais 0.0× argument vs LLM chain → casse promesse
```

### Annexe C — Glossaire P1

| Terme | Définition |
|-------|------------|
| **P1** | Promesse n°1 du contrat EFE : Latence opposable |
| **Opposable** | Mesurable, reproductible, attestable, défendable juridiquement |
| **Hot-path** | Code exécuté à chaque ExecuteRequest, sur ≥ 1 % des requêtes |
| **Budget** | Allocation temporelle p99 par étage (cf. §2.1) |
| **Burn rate** | Vitesse à laquelle l'error budget mensuel est consommé |
| **Marge anti-tail** | Différence entre p99 cible et somme des p99 par étage |
| **Pre-warm** | Inférences dummy au boot pour amortir JIT + arena |
| **Batch coalescer** | Agrégateur côté serveur de requêtes embedder concurrentes |
| **Auto-shed** | Reject préventif sous burn élevé pour préserver le p99 |
| **Host fingerprint** | Hash SHA-256 du tuple (CPU, GPU, RAM, kernel, rustc, …) |
| **Attest** | Procédure formelle de vérification SLO par tiers signée |
| **Realism check** | Refuser un test qui ne reflète pas la production |
| **Slack worker tail** | Jitter Tokio scheduler hors-application |
| **Multi-window burn** | Pattern d'alerting 1h / 6h / 24h pour distinguer panne vs dérive |

### Annexe D — Calcul du burn rate

```
error_budget_monthly = 1 - 0.999 = 0.001 = 0.1 %
total_requests_monthly ≈ 200 RPS × 86400 × 30 = 518 M
allowed_violations_monthly = 518 M × 0.001 = 518 000 requêtes

burn_rate_1h_threshold = 14.4 ×  (= 100 % du budget dans 1 h × 14.4)
  → violations_in_1h ≥ 518 000 × (1 / 24 / 30) × 14.4 = 10 360 / h
```

L'alerte `P1_BURN_FAST` se déclenche dès que le débit horaire de
violations dépasse 10 360 (∼ 2.9 par seconde de violations).

### Annexe E — Mapping P1 → contrats C-1..C-20

| Contrat | Description | Couvert par |
|---------|-------------|-------------|
| C-1 | Mode-1 GPU p99 ≤ 200 ms | **SLO-C-1** (P1 racine) |
| C-2 | Mode-1 CPU p99 ≤ 800 ms | SLO-C-2 |
| C-10 | RPO ≤ 5 min | Hors P1 (D17 backup) |
| C-11 | RTO ≤ 30 min | Hors P1 (D17 DR drill) |
| C-12 | Availability ≥ 99.9 % | Hors P1 (mais auto-shed §7.1 interagit) |
| C-13 | mTLS | SLO-PC-mTLS-INIT |
| C-16 | Observabilité | Surface 2 (§4) |

→ P1 couvre **directement** C-1, C-2, C-13, C-16.
→ P1 **interagit** avec C-12 (trade-off auto-shed).

### Annexe F — Commandes de vérification rapide

```bash
# Vérifier que les 12 histograms sont émis
$ curl -s http://localhost:50051/metrics | grep '^efe_p1_'

# Lancer le bench P1 sustained
$ ./bin/efe-bench run \
    --workload p1 \
    --duration 10m \
    --rps 200 \
    --output target/bench_p1_sustained.json

# Vérifier la conformité SLO
$ python eval/scripts/check_slo.py \
    --baseline bench_baselines/v1.0.0/baseline.json \
    --measure target/bench_p1_sustained.json

# Vérifier que le hot-path est lint-clean
$ cargo test --test p1_hot_path_lint --release

# Vérifier l'attestation SLA d'un mois donné
$ efe-attest verify-sla \
    --baseline bench_baselines/v1.0.0/baseline.json \
    --reports reports/2026-05.json \
    --audit-chain s3://efe-audit/2026-q2.chain \
    --fingerprint-baseline $(cat bench_baselines/v1.0.0/fingerprint.sha256)

# Voir le burn rate live
$ curl -G --data-urlencode 'query=increase(efe_swarm_duration_seconds_count{mode="mode1"}[1h]) - increase(efe_swarm_duration_seconds_bucket{mode="mode1",le="0.2"}[1h])' http://prometheus:9090/api/v1/query

# Snapshot d'un flame graph en prod
$ kubectl exec -n efe efe-server-0 -- pprof-snapshot 30s > target/flame.svg
```

---

— **fin du document** —
