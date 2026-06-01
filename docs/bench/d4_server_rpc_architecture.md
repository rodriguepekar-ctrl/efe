# Sprint D4 — Architecture : `crates/efe-server/benches/server_rpc.rs`

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position dans la séquence** : suit D1 (backend prod), D2 (infra
> mémoire), D3 (compute pipeline). D4 mesure la **couche RPC gRPC**
> qui sert d'enveloppe externe : tonic codec + interceptors (auth,
> rate-limit, idempotency) + bridge proto↔domain + transport
> HTTP/2. C'est ce qu'un *vrai* client gRPC paie en plus de la
> chaîne backend.
> **Prérequis** : avoir lu
> [`d1_real_strategies_architecture.md`](d1_real_strategies_architecture.md)
> ; les conventions communes (criterion 0.5 + async_tokio, fixtures
> v2, baseline freezing, SLO IDs, CI coefficient ×1.5) ne sont pas
> redéveloppées ici.

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [Gap analysis : ce qui n'est PAS mesuré aujourd'hui](#2-gap-analysis--ce-qui-nest-pas-mesuré-aujourdhui)
3. [Surface API à bencher (cartographie chirurgicale)](#3-surface-api-à-bencher-cartographie-chirurgicale)
4. [Le request path canonique de `SwarmServiceImpl::execute` (annoté)](#4-le-request-path-canonique-de-swarmserviceimplexecute-annoté)
5. [Architecture cible : structure du fichier](#5-architecture-cible--structure-du-fichier)
6. [Les 4 groupes criterion détaillés](#6-les-4-groupes-criterion-détaillés)
7. [12 décisions de conception clés](#7-12-décisions-de-conception-clés)
8. [10 risques identifiés et mitigations](#8-10-risques-identifiés-et-mitigations)
9. [Plan jour-par-jour (3-4 j wallclock)](#9-plan-jour-par-jour-3-4-j-wallclock)
10. [Calibration & nouveaux SLO `D4-*`](#10-calibration--nouveaux-slo-d4)
11. [Anti-régression & realism check](#11-anti-régression--realism-check)
12. [Intégration CI + parser + baseline v0.1.4](#12-intégration-ci--parser--baseline-v014)
13. [Critères d'acceptation (DoD)](#13-critères-dacceptation-dod)
14. [Out of scope & roadmap follow-ups](#14-out-of-scope--roadmap-follow-ups)

---

## 1. Contexte & mission

### 1.1 Pourquoi remonter au-dessus de D1

D1 mesure `Mode1Backend.execute(req: Request)` — la chaîne complète
de stratégies prod **mais sans le wire**. Quand un client gRPC
envoie une requête, ce que le serveur paie *en plus* de D1 est :

| Couche | Coût attendu sur small | Couvert par |
|--------|----------------------:|:-----------:|
| Transport HTTP/2 (tonic) | ~30-100 µs | ❌ rien |
| Frame parsing + codec proto | ~10-50 µs | ❌ rien |
| `MetricsLayer` (Tower) | ~5-20 µs | ❌ rien |
| `AuthGuard.check` (JWT validate) | ~20-200 µs (selon enabled) | ❌ rien |
| `check_rate_limit` (token bucket) | ~5-20 µs | ❌ rien |
| `IdempotencyCache.get + insert` | ~10-50 µs | ❌ rien |
| `bridge::request::from_proto` | ~10-100 µs (selon taille spec) | ❌ rien |
| `bridge::response::to_proto` | ~5-50 µs | ❌ rien |
| `emit_request_completion` (Prometheus) | ~5-15 µs | ❌ rien |
| **Σ overhead RPC nominal** | **~100-600 µs** | — |
| `backend.execute()` | ~5 ms (D1) | ✅ D1 |

**Sans D4, ces 100-600 µs sont invisibles**. Une régression
d'auth qui ferait passer la JWT validation de 20 µs à 5 ms ne
serait détectée par aucun bench actuel — D1 ne touche pas l'auth,
les tests d'intégration vérifient la *correction* pas la *vitesse*.

### 1.2 Mission D4 (énoncé chirurgical)

> Produire un bench criterion **`crates/efe-server/benches/server_rpc.rs`**
> qui mesure, **à travers une connexion gRPC réelle (loopback TCP)**,
> les coûts unitaires et concurrents de :
>
> 1. La chaîne RPC complète client → serveur → client avec **toutes
>    les couches actives** (auth disabled mais path traversé,
>    rate-limit + idempotency activés, bridge actif, métriques
>    actives), sur les 3 v2 fixtures.
> 2. Une variante **pile minimale** (interceptors désactivés) qui
>    sert de baseline pour calculer le **coût marginal des
>    interceptors** (Δ = full − minimal).
> 3. Le **throughput concurrent** : N=8 clients qui hammer la
>    même socket-loopback simultanément.
> 4. Le **codec proto pur** : `prost::Message::encode + decode`
>    round-trip d'un `ExecuteRequest`, sans transport ni serveur.
>
> Le bench produit **6 SLO `D4-*`** gardés par CI, et est protégé
> par **3 tests anti-régression** dans
> `tests/server_rpc_realism_check.rs`.

### 1.3 Pourquoi loopback TCP et pas in-process

Tonic supporte deux modèles de transport pour benchmark :

| Modèle | Pour | Contre |
|--------|------|--------|
| **Loopback TCP** (ephemeral port) | Fidèle à prod (HTTP/2 framing, syscalls) ; pattern déjà testé dans `tests/integration_grpc.rs` | +50-100 µs OS overhead par RTT |
| **In-process** (`tokio::io::duplex`) | Élimine OS networking ; mesure pure de tonic+stack | **Différent du chemin réel** : un client externe n'aura jamais cette perf |

**Décision** : **loopback TCP**. Trois raisons :

1. Réutilise le pattern de `tests/integration_grpc.rs::TestServer::start`
   qui est déjà dans la codebase, validé, et idiomatique.
2. Les ~50-100 µs OS overhead **font partie** du coût qu'un client
   gRPC paie ; les exclure produirait des chiffres optimistes et
   misleading pour la consommation prod.
3. Les SLO doivent refléter les engagements externes : un client
   gRPC qui voit p95 ≤ 8 ms est exactement ce que D4 mesure.

**Trade-off accepté** : variabilité du loopback (~10 %) plus haute
qu'in-process (~3 %) ; criterion absorbe via `noise_threshold(0.10)`
au niveau groupe.

### 1.4 Localisation décidée : `efe-server/benches/`

Comme D1 (`real_strategies_e2e.rs`, `real_memory.rs`). Justifié :

* La cible (`SwarmServiceImpl`, `tonic::transport::Server`,
  `AuthGuard`, `BucketRegistry`, `IdempotencyCache`) vit toute dans
  `efe_server`.
* Aucune dépendance vers une crate externe nouvelle.
* Réutilise les helpers D1 existants
  (`build_real_backend`, `load_realistic_v2_spec`,
  `default_mode1_config`) qui sont dans `efe-server/benches/common/mod.rs`.

---

## 2. Gap analysis : ce qui n'est PAS mesuré aujourd'hui

### 2.1 Inventaire post-D3

| Suite | Fichier | Crate | Ce qu'elle couvre |
|-------|---------|-------|-------------------|
| `swarm_e2e` (Sprint C) | core | controller mocks |
| `memory_e2e` (Sprint C) | core | heap mocks |
| `compute_sprint{2,3,4}` | core | ops compute en isolation |
| `real_strategies_e2e` (D1) | server | `Mode1Backend.execute` (sans gRPC) |
| `real_memory` (D1) | server | heap prod (sans gRPC) |
| `memory_real_arena` (D2) | core | pool/arena |
| `compute_pipeline` (D3) | core | chaîne 6 ops compute |
| **`server_rpc`** *(D4)* | server | **gRPC stack au-dessus du backend** |

### 2.2 Tests d'intégration gRPC ≠ bench gRPC

`crates/efe-server/tests/integration_grpc.rs` exerce le wire
end-to-end mais avec un **stub backend** (cf. `StubBackend` dans
le fichier) qui retourne en ~10 µs. Conclusion : ces tests valident
la **correction du wire**, pas le **coût latentiel**. Si un futur
release ajoute un middleware naïf qui mémorise toutes les requêtes
en mémoire, les tests passent mais le serveur fuit en prod — D4 le
catche.

### 2.3 La pile interceptor n'est mesurée nulle part

`AuthGuard.check`, `check_rate_limit`, `IdempotencyCache.{get,insert}`
ont des **tests unitaires** (correction sémantique) mais aucune
**garantie de latence**. Si Sprint M14 remplace
`jsonwebtoken::decode` par une lib qui passe par OpenSSL et tape
des allocations, l'auth grimpe à 3 ms et personne ne s'en aperçoit
avant le canary prod.

### 2.4 Bridge proto↔domain : 8 fichiers, ~600 LOC, 0 bench

`crates/efe-server/src/bridge/{request,response,spec,budget,decode,verdict,error,cone}.rs`
représente la traduction proto ↔ types core. Pour les v2 fixtures
*medium*, c'est un `SwarmSpec` avec `dim=256, n_agents=32, 8 anchors`
— le bridge alloue/copie ~70 KiB de tenseurs à chaque requête.
Aucun bench actuel ne le quantifie.

### 2.5 Conclusion : 4e moitié de la couverture

Les D1+D2+D3 attribuent ~95 % du temps d'une requête backend. D4
ferme les 5-10 % restants (RPC layer) **et** les ~10 % de surcoût
RPC qu'un client externe paie en plus.

À l'issue de D4, on aura **100 % attribution** :

```
Client gRPC RPC latency = D4 RPC overhead
                        + D1 backend chain
                        ≈ D2 (mémoire) + D3 (compute) + orchestration
```

---

## 3. Surface API à bencher (cartographie chirurgicale)

### 3.1 `Server` (`efe_server::Server`)

```rust
pub struct Server { /* opaque */ }
impl Server {
    pub fn builder() -> ServerBuilder;
    pub fn config(&self) -> &ServerConfig;
    pub async fn serve(self, addr: SocketAddr, shutdown: Shutdown) -> anyhow::Result<()>;
}

pub struct ServerBuilder { /* … */ }
impl ServerBuilder {
    pub fn config(self, cfg: ServerConfig) -> Self;
    pub fn backend(self, backend: DynExecuteBackend) -> Self;
    pub fn registry(self, registry: Arc<dyn AlignmentRegistry>) -> Self;
    pub fn token_store(self, ts: Arc<TokenStore>) -> Self;
    pub fn build(self) -> anyhow::Result<Server>;
}
```

### 3.2 `ServerConfig` — toggles d'interceptor

```rust
pub struct ServerConfig {
    pub listen_addr: SocketAddr,
    pub auth: AuthConfig,             // .enabled: bool
    pub ratelimit: RateLimitConfig,    // .enabled: bool, .quotas: …
    pub idempotency: IdempotencyConfig,// .enabled: bool, .ttl: Duration
    pub max_concurrent_streams: u32,
    pub request_timeout: Duration,
    pub shutdown: ShutdownConfig,
}
```

Les 3 toggles (`auth.enabled`, `ratelimit.enabled`,
`idempotency.enabled`) sont les **leviers** que D4 utilise pour
isoler les couches.

### 3.3 `SwarmServiceImpl` — la classe coeur du handler

```rust
pub struct SwarmServiceImpl {
    backend: DynExecuteBackend,
    auth: Arc<AuthGuard>,
    idempotency: Option<Arc<IdempotencyCache>>,
    rate_limiter: Option<Arc<BucketRegistry>>,
}
impl SwarmService for SwarmServiceImpl {
    async fn execute(&self, req: TRequest<ExecuteRequest>) -> Result<TResponse<ExecuteResponse>, Status>;
    async fn execute_streaming(&self, ...) -> Result<...>;
    async fn describe_spec(&self, ...) -> Result<...>;
}
```

D4 mesure le `execute` synchrone (pas `execute_streaming` —
streaming est out-of-scope, follow-up D5).

### 3.4 `tonic` clients générés

```rust
// proto-generated:
pub mod swarm_service_client {
    pub struct SwarmServiceClient<T> { /* … */ }
    impl<T> SwarmServiceClient<T> {
        pub fn new(channel: T) -> Self;
        pub async fn execute(&mut self, request: impl IntoRequest<ExecuteRequest>)
            -> Result<TResponse<ExecuteResponse>, Status>;
    }
}
```

Construit via `SwarmServiceClient::new(channel)` où
`channel = Channel::from_shared(format!("http://{addr}")).connect().await`.

### 3.5 Pattern serveur de test (depuis `tests/integration_grpc.rs`)

```rust
async fn start_server(backend: Arc<dyn ExecuteBackend>, cfg: ServerConfig) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);                      // bind only to reserve, server rebinds
    let mut cfg = cfg;
    cfg.listen_addr = addr;
    let server = Server::builder().config(cfg).backend(backend)
        .registry(...).build().unwrap();
    let shutdown = Shutdown::new();
    let handle = tokio::spawn(server.serve(addr, shutdown.clone()));
    sleep(100ms).await;                  // bind warmup
    TestServer { addr, shutdown, handle }
}
```

D4 réutilise ce pattern verbatim. Un **seul** serveur est levé par
groupe (ou par variante), pas par iter.

### 3.6 Synthèse : les 8 latences fondamentales que D4 doit exposer

| Mesure | Composant interne | Ordre de grandeur attendu (small) |
|--------|-------------------|----------------------------------:|
| **RPC full** | tout | **~5-10 ms** (D1 + RPC overhead) |
| **RPC minimal** | bridge + backend, no interceptors | **~5-7 ms** (D1 + tonic codec only) |
| **Δ interceptors** | full − minimal | **~200-500 µs** |
| **codec encode** | `prost::Message::encode` | **~5-30 µs** |
| **codec decode** | `prost::Message::decode` | **~5-30 µs** |
| **bridge req** | `bridge::request::from_proto` | **~10-100 µs** |
| **bridge resp** | `bridge::response::to_proto` | **~5-50 µs** |
| **concurrent 8c** | 8 clients hammering | **~30-60 ms / batch** |

---

## 4. Le request path canonique de `SwarmServiceImpl::execute` (annoté)

Lecture fidèle de `crates/efe-server/src/services/swarm.rs:79-146` :

```text
execute(req: TRequest<ExecuteRequest>) -> Result<TResponse<…>, Status>:

    let started = Instant::now();
                                              ↑ Tower MetricsLayer's outer
                                                instrumentation already
                                                started before we got here.

    ┌── L1 — AuthGuard.check ─────────────────────────────┐
    let auth_res = self.auth.check(&mut req, Some(SCOPE));
    let tenant   = Self::tenant_of(&req);
    if let Err(e) = auth_res { return Err(e); }
    └─────────────────────────────────────────────────────┘
       Disabled mode: ~50 ns (anonymous claim install)
       Enabled mode : ~30-200 µs (JWT decode + sig verify)

    ┌── L2 — Rate limit check ────────────────────────────┐
    if let Err(e) = check_rate_limit(self.rate_limiter, &tenant) {…}
    └─────────────────────────────────────────────────────┘
       Token bucket atomic compare-and-swap : ~5-20 µs
       Skipped when `rate_limiter is None`.

    let trace_context = Self::trace_of(&req);
    let mut inner    = req.into_inner();
    let request_id_pre = inner.request_id.clone();

    ┌── L3 — Idempotency cache lookup ────────────────────┐
    if let Some(cache) = &self.idempotency {
        if !request_id_pre.is_empty() {
            if let Some(cached) = cache.get(&tenant, &request_id_pre) {
                return Ok(cached);                 ← short-circuit hit
            }
        }
    }
    └─────────────────────────────────────────────────────┘
       Hit  : ~5-15 µs (DashMap.get + Clone of cached resp)
       Miss : ~5-15 µs (DashMap.get returning None)

    if inner.request_id.is_empty() {
        inner.request_id = RequestId::new().as_str().to_string();
    }

    ┌── L4 — Bridge proto → domain ───────────────────────┐
    let domain_req = bridge::request::from_proto(inner, tenant.clone(), trace_context);
    └─────────────────────────────────────────────────────┘
       small (dim=128, 8 agents)  : ~10-30 µs
       medium (dim=256, 32 agents): ~30-60 µs
       large  (dim=256, 128 agents): ~80-200 µs

    ┌── L5 — Backend execute (D1 territory) ──────────────┐
    let domain_resp = self.backend.execute(domain_req).await?;
    └─────────────────────────────────────────────────────┘
       small  : ~5 ms ; medium : ~50 ms ; large : ~250 ms

    ┌── L6 — Bridge domain → proto ───────────────────────┐
    let proto_resp = bridge::response::to_proto(domain_resp);
    └─────────────────────────────────────────────────────┘
       Generally fast : ~5-30 µs (Vec<f32> → repeated f32)

    ┌── L7 — Idempotency cache insert ────────────────────┐
    if let Some(cache) = &self.idempotency {
        cache.insert(&tenant, &request_id, proto_resp.clone());
    }
    └─────────────────────────────────────────────────────┘
       ~10-30 µs (Clone of full proto resp + DashMap.insert)

    ┌── L8 — Metrics emit ────────────────────────────────┐
    emit_request_completion(EXECUTE_PATH, &tenant, &outcome, started);
    └─────────────────────────────────────────────────────┘
       ~3-8 µs (3 atomic counter increments + histogram observe)

    return Ok(TResponse::new(proto_resp));
```

**Plus** la couche tonic externe (transport HTTP/2, frame parsing,
codec) qui n'est pas dans le code applicatif mais qui contribue
~30-100 µs (loopback) selon la taille proto.

---

## 5. Architecture cible : structure du fichier

### 5.1 Localisation et déclaration Cargo

**Fichier** : `crates/efe-server/benches/server_rpc.rs`
**LOC cible** : ~700 lignes (commentaires denses inclus).
**Cargo.toml** :

```toml
[[bench]]
name = "server_rpc"
harness = false
```

Pas de feature à ajouter — `tonic`, `tokio`, `criterion` sont déjà
dans `[dev-dependencies]` depuis D1.

### 5.2 Structure du fichier (squelette)

```rust
//! D4 — bench gRPC RPC stack (loopback TCP).

#![allow(missing_docs)]
#![allow(clippy::cast_precision_loss)]

mod common; // partagé avec real_strategies_e2e.rs (D1)

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use tokio::net::TcpListener;
use tonic::transport::Channel;

use efe_core::alignment::{AlignmentRegistry, InMemoryRegistry};
use efe_server::proto::v1::{
    self as v1, swarm_service_client::SwarmServiceClient, ExecuteRequest, ExecuteResponse,
};
use efe_server::services::{DynExecuteBackend, ExecuteBackend};
use efe_server::{Server, ServerConfig, Shutdown};

use common::{build_real_backend, load_realistic_v2_spec, make_runtime};

// =====================================================================
// Bench-tuning constants
// =====================================================================
const BIND_WARMUP: Duration = Duration::from_millis(150);
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(3);
const N_CONCURRENT_CLIENTS: usize = 8;

// =====================================================================
// Server harness (mirror of integration_grpc::TestServer)
// =====================================================================
struct BenchServer {
    addr: SocketAddr,
    shutdown: Shutdown,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}
impl BenchServer {
    async fn start(backend: DynExecuteBackend, cfg: ServerConfig) -> Self;
    async fn channel(&self) -> Channel;
    async fn stop(self);
}

fn make_full_stack_config(addr: SocketAddr) -> ServerConfig;
fn make_minimal_stack_config(addr: SocketAddr) -> ServerConfig;

// =====================================================================
// Spec → Proto fixture conversion
// =====================================================================
fn proto_request_for_v2_fixture(name: &str) -> ExecuteRequest;

// =====================================================================
// 4 criterion groups
// =====================================================================
fn bench_server_rpc_full_stack(c: &mut Criterion);
fn bench_server_rpc_minimal_stack(c: &mut Criterion);
fn bench_server_rpc_concurrent(c: &mut Criterion);
fn bench_server_rpc_codec_only(c: &mut Criterion);

criterion_group!(
    name = server_rpc_benches;
    config = Criterion::default()
        .significance_level(0.05)
        .noise_threshold(0.10);   // loopback TCP variance
    targets =
        bench_server_rpc_full_stack,
        bench_server_rpc_minimal_stack,
        bench_server_rpc_concurrent,
        bench_server_rpc_codec_only,
);
criterion_main!(server_rpc_benches);
```

### 5.3 Helpers détaillés

#### `BenchServer::start`

```rust
async fn start(backend: DynExecuteBackend, cfg: ServerConfig) -> Self {
    // 1. Reserve an ephemeral port via std (sync TcpListener),
    //    then drop the listener so the server can rebind.
    //    Same pattern as integration_grpc::TestServer.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("ephemeral bind");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let mut cfg = cfg;
    cfg.listen_addr = addr;
    // Disable shutdown grace to keep teardown snappy.
    cfg.shutdown.grace_period = Duration::from_millis(50);
    cfg.shutdown.force_abort_after = Duration::from_secs(2);

    let registry: Arc<dyn AlignmentRegistry> =
        Arc::new(InMemoryRegistry::with_default_config().expect("registry"));

    let server = Server::builder()
        .config(cfg)
        .backend(backend)
        .registry(registry)
        .build()
        .expect("server build");

    let shutdown = Shutdown::new();
    let shutdown_for_task = shutdown.clone();
    let handle = tokio::spawn(async move { server.serve(addr, shutdown_for_task).await });

    // Bind warmup. 150 ms is conservative ; 100 ms (the test pattern)
    // is occasionally racy on a busy CI runner.
    tokio::time::sleep(BIND_WARMUP).await;

    BenchServer { addr, shutdown, handle }
}
```

#### `BenchServer::channel`

```rust
async fn channel(&self) -> Channel {
    Channel::from_shared(format!("http://{}", self.addr))
        .expect("uri parse")
        .connect()
        .await
        .expect("channel connect")
}
```

#### `make_full_stack_config` / `make_minimal_stack_config`

```rust
fn make_full_stack_config(_addr: SocketAddr) -> ServerConfig {
    let mut cfg = ServerConfig::default();
    // We DO NOT enable auth (enabling it requires a valid JWT in
    // every bench iter — adds JWT signing cost to setup which we
    // don't want to bench here). The bench measures the *gating
    // path* of `AuthGuard.check` in disabled mode, which still
    // installs the anonymous claim and parses the trace header.
    // Enabling auth is a follow-up D4.5 (`server_rpc_auth_on`).
    cfg.auth.enabled = false;
    cfg.ratelimit.enabled = true;
    cfg.idempotency.enabled = true;
    cfg
}

fn make_minimal_stack_config(_addr: SocketAddr) -> ServerConfig {
    let mut cfg = ServerConfig::default();
    cfg.auth.enabled = false;
    cfg.ratelimit.enabled = false;
    cfg.idempotency.enabled = false;
    cfg
}
```

#### `proto_request_for_v2_fixture`

```rust
fn proto_request_for_v2_fixture(name: &str) -> ExecuteRequest {
    let spec = load_realistic_v2_spec(name);  // SwarmSpec from D1 helper
    let proto_spec = v1::SwarmSpec {
        swarm_id: String::new(),
        problem_embedding: spec.problem_embedding.clone(),
        dim_latent: spec.dim_latent.get(),
        population: spec
            .population
            .as_pairs()
            .iter()
            .map(|(code, count)| v1::PopulationEntry {
                template_code: code.as_str().to_string(),
                count: *count,
            })
            .collect(),
        max_rounds: spec.max_rounds,
        convergence: None,  // server fills defaults
        cone_source: Some(v1::swarm_spec::ConeSource::AlignmentCone(
            v1::AlignmentCone {
                anchors: spec.alignment_cone.anchors.iter()
                    .map(|a| v1::Anchor { values: a.clone() }).collect(),
                max_projection_distance: spec.alignment_cone.max_projection_distance,
            },
        )),
        budget: None,
    };
    ExecuteRequest {
        request_id: String::new(),  // server-side generation
        spec: Some(proto_spec),
        decode_mode: None,
        raw_prompt: String::new(),
        deadline: None,
    }
}
```

---

## 6. Les 4 groupes criterion détaillés

### 6.1 `server_rpc_full_stack` — la métrique-roi

**Intention** : ce qu'un client gRPC paie sur la pile prod
complète. Toutes les couches actives (auth path traversé en
disabled mode, rate-limit on, idempotency on, bridge, backend,
metrics).

**Setup** :
```rust
let rt = make_runtime();
let _guard = rt.enter();
let backend = build_real_backend(&load_realistic_v2_spec("small"));  // shared
let cfg = make_full_stack_config(SocketAddr::from(([127,0,0,1], 0)));
let server = rt.block_on(BenchServer::start(backend.clone(), cfg));
let channel = rt.block_on(server.channel());
let client = SwarmServiceClient::new(channel);
let req_template = proto_request_for_v2_fixture("small");
```

**Itération** :
```rust
b.to_async(&rt).iter(|| {
    let mut client = client.clone();
    let req = req_template.clone();
    async move {
        let resp = client
            .execute(tonic::Request::new(req))
            .await
            .expect("rpc execute");
        black_box(resp);
    }
});
```

**Sample size adapté par fixture** :

| Fixture | Coût attendu | Samples |
|---------|-------------:|--------:|
| small | ~6-8 ms | 30 |
| medium | ~50-60 ms | 20 |
| large | ~250-300 ms | 10 |

**Throughput** : `Throughput::Elements(1)` (1 RPC/iter).

**SLO** : `D4-1` / `D4-2` / `D4-3`.

**Coût décomposé attendu (small, M3 Pro)** :

| Couche | Coût (µs) | % |
|--------|----------:|--:|
| TCP loopback HTTP/2 | 50 | 1 % |
| tonic codec encode (client) | 10 | 0.2 % |
| tonic codec decode (server) | 15 | 0.3 % |
| AuthGuard.check (disabled) | 5 | 0.1 % |
| check_rate_limit | 10 | 0.2 % |
| IdempotencyCache.get (miss) | 10 | 0.2 % |
| bridge::request::from_proto | 20 | 0.4 % |
| **backend.execute (D1)** | **5 000** | **89 %** |
| bridge::response::to_proto | 15 | 0.3 % |
| IdempotencyCache.insert | 25 | 0.4 % |
| emit_request_completion | 5 | 0.1 % |
| tonic codec encode (server) | 10 | 0.2 % |
| tonic codec decode (client) | 5 | 0.1 % |
| **TOTAL** | **~5 600 µs** | 100 % |

**Confirmé** : ~89 % du temps est dans le backend ; le RPC stack
ajoute ~600 µs. SLO `D4-1` ≤ 8 ms laisse une marge ×1.4 sur ce
budget.

### 6.2 `server_rpc_minimal_stack` — baseline pour calcul d'overhead

**Intention** : exécuter la même requête sur une pile **sans**
rate-limit ni idempotency, pour calculer le **coût marginal des
interceptors** par soustraction :

```
Δ_interceptors = full_stack − minimal_stack
              = ratelimit + idempotency
```

`auth.enabled = false` est commun aux deux variantes (on ne bench
pas l'auth ON ici — D4.5).

**Setup** : identique à `full_stack` mais avec `make_minimal_stack_config`.

**Itération** : identique.

**Sample size** : identique.

**Lecture attendue** :

| Fixture | full | minimal | Δ interceptors |
|---------|-----:|--------:|---------------:|
| small | ~5.6 ms | ~5.55 ms | ~50 µs |
| medium | ~52 ms | ~52 ms | ~50 µs |
| large | ~285 ms | ~285 ms | ~50 µs |

L'overhead interceptor est **constant** (~50 µs, indépendant de
la fixture) parce que rate-limit et idempotency operent sur des
clés (tenant, request_id), pas sur la taille de la spec.

**Pas de SLO direct** — c'est un *bench de référence* dont la
delta vs `full_stack` est lue dans BENCHMARKS.md mais pas gatée
en CI.

### 6.3 `server_rpc_concurrent` — throughput sous charge

**Intention** : 8 clients qui hammer la même socket. Mesure le
**throughput** du serveur en steady-state, exposé en ops/s via
`Throughput::Elements(N)`.

**Setup** :
```rust
let rt = make_runtime();
let backend = build_real_backend(&load_realistic_v2_spec("small"));
let cfg = make_full_stack_config(([127,0,0,1], 0).into());
let server = rt.block_on(BenchServer::start(backend, cfg));
// Build N=8 *distinct* clients — they share the same Channel
// (multiplexed over HTTP/2) but each owns its own client-side
// state (request id sequencer, etc.).
let channel = rt.block_on(server.channel());
let clients: Vec<_> = (0..8).map(|_| SwarmServiceClient::new(channel.clone())).collect();
let req_template = proto_request_for_v2_fixture("small");
```

**Itération** :
```rust
group.throughput(Throughput::Elements(8));
b.to_async(&rt).iter(|| {
    let req_template = req_template.clone();
    let clients = clients.clone();
    async move {
        let mut tasks = Vec::with_capacity(8);
        for mut client in clients {
            let req = req_template.clone();
            tasks.push(tokio::spawn(async move {
                client.execute(tonic::Request::new(req))
                    .await
                    .expect("concurrent rpc")
            }));
        }
        for t in tasks {
            black_box(t.await.expect("join"));
        }
    }
});
```

**Sample size** : 20 (chaque iter envoie 8 RPCs ⇒ ~50 ms/iter sur small).

**SLO** : `D4-T1` (≤ 60 ms / batch ⇒ ≥ 133 req/s sur small).

**Note sur le multiplexing HTTP/2** : tonic réutilise les *streams*
HTTP/2 sur la même connexion TCP. Ce bench mesure donc le coût
d'**ouvrir 8 streams simultanés**, pas 8 connexions TCP. C'est le
mode d'usage réel des clients qui pool leurs connexions.

### 6.4 `server_rpc_codec_only` — proto encode/decode pur

**Intention** : isoler le coût du codec proto sans transport. Sert
de **borne inférieure** : aucune iter du bench full_stack ne peut
être plus rapide que cette mesure × 2 (encode côté client + decode
côté serveur) + transport.

**Setup** :
```rust
let req = proto_request_for_v2_fixture("medium");
let mut buf = Vec::with_capacity(64 * 1024);
prost::Message::encode(&req, &mut buf).expect("encode warmup");
```

**Itération (encode + decode round-trip)** :
```rust
group.bench_function("medium_roundtrip", |b| {
    b.iter(|| {
        let mut buf = Vec::with_capacity(64 * 1024);
        prost::Message::encode(black_box(&req), &mut buf).expect("encode");
        let decoded: ExecuteRequest =
            prost::Message::decode(buf.as_slice()).expect("decode");
        black_box(decoded);
    });
});
```

**Sample size** : 100 (codec ns/µs scale, faible variance).

**Cibles** : ~5-30 µs encode + ~5-30 µs decode = ~10-60 µs total
pour medium. Pas de SLO direct, métrique d'observation.

**Lecture** : si demain le proto évolue (ajout d'un champ pesant),
ce bench le détecte avant que le full_stack le diluera dans les
~50 ms de backend.

---

## 7. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Rationale |
|---|----------|---------------------|-----------|
| D1 | Loopback TCP (vs in-process duplex) | in-process | matche prod ; pattern déjà testé dans `tests/integration_grpc.rs` ; OS overhead fait partie du coût client |
| D2 | `auth.enabled = false` partout dans D4 | enabled avec JWT | activer JWT ajoute la *cost de signature* dans le setup et brouille la mesure. D4.5 est la suite avec auth ON |
| D3 | `ratelimit.enabled = true` dans full_stack, false dans minimal | always ON | minimal sert de baseline pour calculer Δ_interceptors |
| D4 | `idempotency.enabled = true` avec `request_id = ""` | TTL=0 | request_id empty force le serveur à en générer un — pas de short-circuit cache hit, on mesure le miss path qui est la majorité du trafic |
| D5 | 1 serveur par groupe (vs 1 par iter) | reset per iter | lever un serveur tonic = ~150 ms (bind warmup) ; reset par iter ferait exploser le wall time |
| D6 | 4 groupes (vs 8 ou plus) | breakdown per-interceptor | 4 = full / minimal / concurrent / codec couvre 95 % de la valeur ; per-interceptor = D4.5 |
| D7 | `BIND_WARMUP = 150 ms` (vs 100 ms du test) | 100 ms | sur runners CI chargés, 100 ms est parfois racy ; 50 ms supplémentaires ne changent pas le wall time bench |
| D8 | Pas de cfg `_blas`/`_naive` | mirroir D1/D3 | la pile RPC n'utilise pas de BLAS ; le seul matmul est dans le backend (D1 traite la question) |
| D9 | `N_CONCURRENT_CLIENTS = 8` | 16 ou 4 | 8 matche les `max_arenas` du pool (D2) ; au-delà, on benche les retries `BudgetExhausted` |
| D10 | Réutilise `common/mod.rs` de D1 | duplication | les helpers `load_realistic_v2_spec`, `build_real_backend`, `make_runtime` sont déjà là ; D4 est dans la même crate |
| D11 | `noise_threshold(0.10)` (vs 0.05 default) | tighter | loopback TCP a une variance naturelle de ~10 % ; serrer à 5 % produirait des "régressions" dues au bruit OS |
| D12 | Pas de `request_id` côté client | UUID per iter | un id vide force le serveur à en générer ; mesure le path miss côté idempotency. Un id constant produirait des hits = bench différent |

---

## 8. 10 risques identifiés et mitigations

### R1 — Port collision sur runner CI

**Impact** : `127.0.0.1:0` réserve un port éphémère. Sur runner
CI très chargé, deux benches lançant en parallèle pourraient
théoriquement collisioner si les ports sont reaffectés trop vite.

**Mitigation** :
* `bench.yml` lance les benches **séquentiellement** dans
  `bench_runner.sh` (déjà le cas).
* Le pattern `drop(listener)` puis `Server::serve(addr)` introduit
  une fenêtre TOCTOU mais rare en pratique (~< 0.1 % observed sur
  Apple M3 / Ubuntu runners).
* Si un fail intermittent émerge, ajout d'un retry `1×` dans
  `BenchServer::start`.

### R2 — `tokio::sleep(BIND_WARMUP)` est gros sur CI lent

**Impact** : 150 ms × 4 groupes × 3 fixtures ≈ 1.8 s de waste.

**Mitigation** : c'est le coût du loopback TCP ; documenter dans
le commentaire en tête.

### R3 — Variance OS du loopback

**Impact** : page faults, scheduler jitter, GC tokio task pool.
Variance ~10 % typique.

**Mitigation** :
* `noise_threshold(0.10)` au niveau Criterion default.
* Sample size adapté (30 sur small) — 30 samples de 6 ms = 180 ms,
  CI tight.
* Documenté en BENCHMARKS.md.

### R4 — Drift entre bench et test integration

**Impact** : le pattern `BenchServer` clone celui de
`tests/integration_grpc.rs::TestServer`. Si l'un change, l'autre
peut diverger silencieusement.

**Mitigation** : le test `tests/server_rpc_realism_check.rs`
(cf. § 11) compare les deux behaviors (timeouts, response shape).
Drift = test rouge.

### R5 — Backend partagé entre iter

**Impact** : un seul `Mode1Backend` est construit en setup. Le
pool d'arènes est commun. Les iters partagent le pool — possible
contention si concurrent.

**Mitigation** :
* Pool sized pour 8 max_arenas (cf. D1 default), suffit pour
  `N_CONCURRENT_CLIENTS = 8`.
* Le bench séquentiel n'expose pas la contention (1 RPC at a
  time) ; le concurrent group la mesure intentionnellement.

### R6 — Idempotency cache ne bloat pas entre iter

**Impact** : `IdempotencyCache::insert` après chaque iter ⇒ 30
entries après 30 samples small. Plus de pression mémoire ⇒ Drift
des numéros au fil du sample.

**Mitigation** :
* `IdempotencyConfig.max_entries` est typiquement 10 000 — bench
  entries ≪ cap.
* Chaque iter a un `request_id` distinct (server-generated UUID),
  donc 30 entries fresh, pas de hits.
* Si Drift apparait, baisser `IdempotencyConfig.ttl` à 100 ms pour
  evict eagerly.

### R7 — `prost::Message::encode` Vec growth

**Impact** : le codec_only group fait `Vec::with_capacity(64KB)`
chaque iter. Sur small, ~600 bytes utilisés, mais l'allocation
de 64 KB domine.

**Mitigation** : `Vec::with_capacity` est un memset 64KB sur
unitialisé memory ⇒ ~5 µs négligeable devant les ~30 µs encode.
Si jamais le ratio devient bad, remplacer par un `BytesMut` réutilisé.

### R8 — Tonic version drift

**Impact** : tonic 0.12 est pinned dans Cargo.toml. Une bump à
0.13 pourrait changer le profile latency.

**Mitigation** : workspace pin sur `tonic.workspace = true` ⇒
upgrade unifié, regression test catch.

### R9 — TLS overhead absent

**Impact** : prod **devra** passer en TLS. Le bench mesure HTTP/2
sans TLS, ce qui sous-estime le coût RPC réel.

**Mitigation** : documenté en BENCHMARKS.md. **D4.6 follow-up**
ajoutera un variant `_tls` avec un cert self-signed.

### R10 — Auth disabled = mesure incomplète

**Impact** : avec `auth.enabled = false`, `AuthGuard.check`
retourne en ~50 ns (anonymous claim install). En prod, JWT
validation peut être 20-200 µs (jsonwebtoken decode + signature
check). D4 ne le mesure pas.

**Mitigation** : **D4.5 follow-up** — `bench_server_rpc_auth_on`
group. Pour D4 baseline, on documente le delta théorique et
recommande de le valider en D4.5.

---

## 9. Plan jour-par-jour (3-4 j wallclock)

### D4.1 (½ j) — Squelette + helpers

* Créer `compute_pipeline.rs` skeleton, imports, constantes.
* Implémenter `BenchServer::start/channel/stop`, `make_*_config`,
  `proto_request_for_v2_fixture`.
* Ajout `[[bench]] name = "server_rpc"` dans `efe-server/Cargo.toml`.
* `cargo build -p efe-server --benches` propre.

**DoD** :
- [ ] Squelette compile.
- [ ] `BenchServer::start` produit un serveur qui réponds en local.

### D4.2 (1 j) — Group 1 : `server_rpc_full_stack`

* 3 fixtures avec config full.
* 1 serveur par fixture.
* Vérifier que les RPC retournent `OK` (no Error) sur les 3.
* `cargo bench --bench server_rpc -- --quick "full_stack"` propre.

**DoD** :
- [ ] 3 sub-benches produisent des mesures stables.
- [ ] Aucune iter ne timeout.

### D4.3 (½ j) — Group 2 : `server_rpc_minimal_stack`

* 3 fixtures avec config minimal.
* Validation : Δ ≥ 0 entre full et minimal sur les 3.

**DoD** :
- [ ] Δ_interceptors ≈ 30-100 µs sur les 3 fixtures.

### D4.4 (½ j) — Group 3 : `server_rpc_concurrent`

* 8 clients concurrents, small fixture.
* Validation : pas de `BudgetExhausted` ; throughput stable.

**DoD** :
- [ ] Concurrent batch < 60 ms sur M3 Pro.

### D4.5 (½ j) — Group 4 : `server_rpc_codec_only`

* `prost::encode + decode` round-trip.
* Validation : codec time ≪ full_stack time.

**DoD** :
- [ ] Codec medium < 100 µs.

### D4.6 (½ j) — Tests anti-régression + tooling

* `crates/efe-server/tests/server_rpc_realism_check.rs` (3 tests).
* `parse_criterion_report.py` : `WANTED_PREFIXES` + 4 SLO `D4-*`.
* `bench_runner.sh` : ajout du run.
* `BENCHMARKS.md` § 6 « Server RPC (D4) ».
* `slo.md` § 7 « SLO bench server RPC ».
* `bench_baselines/v0.1.4/baseline.json` placeholder.
* `.github/workflows/bench.yml` upload artefact étendu.

**DoD** :
- [ ] 3 tests passent.
- [ ] `cargo clippy -p efe-server --benches -- -D warnings` clean.

---

## 10. Calibration & nouveaux SLO `D4-*`

Référence host : Apple M3 Pro, 11 cores, 36 GB, rustc 1.85.0,
loopback TCP, no-TLS, no-auth.

| ID | Nom | Bench key | Cible p95 (M3 Pro) |
|----|-----|-----------|-------------------:|
| `SLO-D4-1` | RPC full small | `server_rpc_full_stack::small` | ≤ **8 ms** |
| `SLO-D4-2` | RPC full medium | `server_rpc_full_stack::medium` | ≤ **60 ms** |
| `SLO-D4-3` | RPC full large | `server_rpc_full_stack::large` | ≤ **300 ms** |
| `SLO-D4-T1` | RPC concurrent 8c small | `server_rpc_concurrent::c8` | ≤ **60 ms** |

**Note** : `SLO-D4-1` (8 ms) = `SLO-RD1-1` (5 ms, D1) + 3 ms de
budget RPC overhead (loopback TCP + tonic + interceptors). Marge
généreuse ×1.4 absorbe la variance loopback.

**Pas de SLO sur `minimal_stack`, `codec_only`** — ce sont des
metrics de référence, pas des engagements.

**Coefficient CI** : ×1.5 (matche D1/D2/D3).

---

## 11. Anti-régression & realism check

**Fichier** : `crates/efe-server/tests/server_rpc_realism_check.rs`
(nouveau)

### Test 1 — `rpc_full_stack_returns_ok_on_all_fixtures`

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rpc_full_stack_returns_ok_on_all_fixtures() {
    for fixture in ["small", "medium", "large"] {
        let backend = build_real_backend(&load_realistic_v2_spec(fixture));
        let cfg = make_full_stack_config(([127,0,0,1], 0).into());
        let server = BenchServer::start(backend, cfg).await;
        let channel = server.channel().await;
        let mut client = SwarmServiceClient::new(channel);
        let req = proto_request_for_v2_fixture(fixture);
        let resp = tokio::time::timeout(
            Duration::from_secs(60),
            client.execute(tonic::Request::new(req))
        ).await.expect("timeout").expect("rpc execute");
        let r = resp.into_inner();
        assert!(!r.swarm_id.is_empty(), "[{}] swarm_id empty", fixture);
        assert!(r.confidence >= 0.0 && r.confidence <= 1.0,
            "[{}] confidence out of [0,1]: {}", fixture, r.confidence);
        assert!(r.rounds_executed >= 1, "[{}] rounds = 0", fixture);
        server.stop().await;
    }
}
```

Détecte : régression dans bridge proto↔domain, dans le serveur
qui retournerait des `Status::error` au lieu de `Ok`, dans les
v2 fixtures qui produiraient une spec mal formée pour le proto.

### Test 2 — `rpc_idempotency_short_circuits_on_repeat`

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rpc_idempotency_short_circuits_on_repeat() {
    let backend = build_real_backend(&load_realistic_v2_spec("small"));
    let cfg = make_full_stack_config(([127,0,0,1], 0).into());
    let server = BenchServer::start(backend, cfg).await;
    let channel = server.channel().await;
    let mut client = SwarmServiceClient::new(channel);
    let mut req = proto_request_for_v2_fixture("small");
    req.request_id = "deterministic-test-id".to_string();

    // First call: should hit the backend.
    let t1 = std::time::Instant::now();
    let _r1 = client.execute(tonic::Request::new(req.clone())).await
        .expect("first call");
    let dt1 = t1.elapsed();

    // Second call: should hit the idempotency cache (≤ 1 ms).
    let t2 = std::time::Instant::now();
    let _r2 = client.execute(tonic::Request::new(req.clone())).await
        .expect("cached call");
    let dt2 = t2.elapsed();

    assert!(dt2 < dt1 / 2,
        "idempotency cache did not short-circuit: dt1={:?}, dt2={:?}",
        dt1, dt2);
    server.stop().await;
}
```

Détecte : régression dans `IdempotencyCache.{get,insert}` ; le
cache devrait servir le 2nd call en < 1 ms (vs ~5 ms backend hit).

### Test 3 — `rpc_concurrent_clients_do_not_serialize`

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rpc_concurrent_clients_do_not_serialize() {
    let backend = build_real_backend(&load_realistic_v2_spec("small"));
    let cfg = make_full_stack_config(([127,0,0,1], 0).into());
    let server = BenchServer::start(backend, cfg).await;
    let channel = server.channel().await;
    let req = proto_request_for_v2_fixture("small");

    let t = std::time::Instant::now();
    let mut tasks = Vec::with_capacity(8);
    for _ in 0..8 {
        let mut client = SwarmServiceClient::new(channel.clone());
        let req = req.clone();
        tasks.push(tokio::spawn(async move {
            client.execute(tonic::Request::new(req)).await
        }));
    }
    for t in tasks { t.await.expect("join").expect("rpc"); }
    let dt = t.elapsed();

    // Sequential single-call ~5-10 ms ; serialized × 8 = 40-80 ms.
    // Concurrent should be ~2× the single-call cost (some
    // contention but mostly overlapped).
    assert!(dt < Duration::from_millis(40),
        "concurrent took {:?}, expected < 40 ms (serializing?)", dt);
    server.stop().await;
}
```

Détecte : régression où le serveur serialiserait les RPCs (un
mutex global qui éviterait le parallélisme). Doit observer
parallel execution.

---

## 12. Intégration CI + parser + baseline v0.1.4

### 12.1 `parse_criterion_report.py`

```python
WANTED_PREFIXES = (
    # ... existing ...
    "server_rpc_full_stack",
    "server_rpc_minimal_stack",
    "server_rpc_concurrent",
    "server_rpc_codec_only",
)

# 4 nouveaux SLO D4-* dans SLO_DEFINITIONS (cf. § 10).
```

### 12.2 `bench_runner.sh`

Dans la branche `[ "$ONLY" != "mock" ]` (le bench est dans
`efe-server`), ajouter :

```bash
echo "==> Running server_rpc bench (Sprint D4, gRPC stack)"
$CARGO bench -p efe-server --bench server_rpc $SERVER_FEATURES_FLAG -- \
    --output-format bencher $CRITERION_FLAGS \
    > "target/bench-server-rpc.txt" 2>&1 \
    || echo "WARN: server_rpc returned non-zero (continuing)"
```

### 12.3 `.github/workflows/bench.yml`

Aucun ajout au path filter (`crates/efe-server/benches/**`
couvre déjà). Étendre l'upload artefact :

```yaml
path: |
  ...
  target/bench-server-rpc.txt
```

### 12.4 `bench_baselines/v0.1.4/baseline.json`

Placeholder identique au pattern v0.1.{1,2,3} : `results: {}`,
4 SLO `skipped: true`, `_note` explicatif.

### 12.5 `BENCHMARKS.md` — nouvelle § 6

Insérer entre § 5 (compute pipeline D3) et § 7 (Reproductibilité) :

```markdown
## 6. Server RPC stack (Sprint D4, v0.1.4)

Mesures du wire gRPC réel sur loopback TCP. Lecture clé : le RPC
full stack ajoute ~600 µs au-dessus du backend (D1) — ~10 % du
temps total sur small, ~1 % sur large.

### 6.1 RPC full vs minimal

| Fixture | full (M3) | minimal | Δ interceptors | SLO |
|---------|----------:|--------:|---------------:|----:|
| small | ~6 ms | ~5.6 ms | ~50 µs | D4-1 ≤ 8 ms ×1.3 |
| medium | ~52 ms | ~52 ms | ~50 µs | D4-2 ≤ 60 ms ×1.15 |
| large | ~285 ms | ~285 ms | ~50 µs | D4-3 ≤ 300 ms ×1.05 |

### 6.2 RPC concurrent (small, c=8)

| Mesure | M3 Pro | SLO |
|--------|-------:|----:|
| 8c batch | ~50 ms | D4-T1 ≤ 60 ms |
| Throughput extrapolé | ~160 req/s | — |

### 6.3 Codec proto round-trip (medium)

| Op | µs |
|----|---:|
| `prost::encode` | ~25 |
| `prost::decode` | ~20 |
| **round-trip** | **~45 µs** |
```

### 12.6 `docs/slo.md` — nouvelle § 7

Mirror exact des SLO `D4-*` listés en § 10 ; ajout après § 6 (D3).

---

## 13. Critères d'acceptation (DoD)

### 13.1 Code & build

- [ ] `crates/efe-server/benches/server_rpc.rs` créé, ~700 LOC.
- [ ] `crates/efe-server/Cargo.toml` ajout `[[bench]] name = "server_rpc"`.
- [ ] `cargo build -p efe-server --benches --release` clean.
- [ ] `cargo clippy -p efe-server --benches -- -D warnings` clean.

### 13.2 Tests

- [ ] `crates/efe-server/tests/server_rpc_realism_check.rs` créé,
      3 tests.
- [ ] `cargo test -p efe-server --test server_rpc_realism_check` →
      3/3 pass.
- [ ] `cargo test --workspace` (suite complète, 1 431+ tests) →
      tous pass.

### 13.3 Bench actually runs

- [ ] `cargo bench -p efe-server --bench server_rpc -- --quick`
      complète sans erreur en < 5 min.
- [ ] 4 groupes criterion produisent des mesures sur les 3 fixtures.
- [ ] Aucun timeout, aucun `BudgetExhausted`.

### 13.4 Tooling

- [ ] `scripts/bench_runner.sh` lance le nouveau bench.
- [ ] `scripts/parse_criterion_report.py` reconnaît les groupes D4.
- [ ] `bench_baselines/v0.1.4/baseline.json` placeholder créé.
- [ ] `.github/workflows/bench.yml` upload artefact étendu.

### 13.5 Documentation

- [ ] `BENCHMARKS.md` § 6 ajoutée.
- [ ] `docs/slo.md` § 7 ajoutée.
- [ ] `docs/bench/d4_server_rpc_architecture.md` (ce document)
      référencé en intro de `BENCHMARKS.md`.

### 13.6 Validation finale

- [ ] `server_rpc_full_stack::small` < 8 ms sur M3 Pro (cible
      primaire).
- [ ] Δ_interceptors entre full et minimal ≥ 0 et < 200 µs sur
      les 3 fixtures.
- [ ] Concurrent 8c < 60 ms sur small.

---

## 14. Out of scope & roadmap follow-ups

### 14.1 Hors-scope D4

* **Auth ON** (JWT validation) — D4.5 follow-up.
* **TLS** — D4.6.
* **Streaming RPC** (`execute_streaming`) — D5.
* **`describe_spec`** RPC — couvert par tests, peu de surface.
* **Admin service** — couvert par tests, hors chemin chaud.
* **gRPC error path** (Status codes) — couvert par tests
  d'intégration.
* **Idempotency cache hit benchmark** — réalisé par le test
  realism #2 ; pas de bench dédié (cas de niche en prod).

### 14.2 Roadmap après D4

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D4.5** | `bench_server_rpc_auth_on` | Mesure de la JWT validation chaude (~20-200 µs) |
| **D4.6** | `bench_server_rpc_tls` | Coût TLS sur connexion (~200-1000 µs handshake amorti) |
| **D5** | `bench_server_rpc_streaming` | `execute_streaming` per-chunk latency |
| **D6** | `bench_admin_rpc` | `AdminService` registry CRUD |

### 14.3 Métrique de succès post-D4

À 30 jours après merge D4 :

> *« Toute régression > 50 % sur la pile RPC d'efe-server produit
>  un fail SLO bench-niveau dans les 24 h après le merge, avec une
>  attribution exacte à `tonic`, `bridge`, ou les interceptors. La
>  somme D1 + (D4 - D1) explique 100 % de la latence RPC observée
>  côté client. »*

---

## Annexe A — Schéma de séquence d'une requête RPC complète

```
Client                Tonic Channel       Server (Tower)        SwarmService          Backend
  │                       │                    │                     │                    │
  │  client.execute(req)  │                    │                     │                    │
  ├──────────────────────►│                    │                     │                    │
  │                       │ HTTP/2 framing     │                     │                    │
  │                       ├ proto encode ─────►│ MetricsLayer.start  │                    │
  │                       │                    ├ proto decode ──────►│                    │
  │                       │                    │                     │ AuthGuard.check    │
  │                       │                    │                     │ (~50ns disabled)   │
  │                       │                    │                     │ check_rate_limit   │
  │                       │                    │                     │ (~10µs)            │
  │                       │                    │                     │ idemp.get → miss   │
  │                       │                    │                     │ (~10µs)            │
  │                       │                    │                     │ bridge::from_proto │
  │                       │                    │                     │ (~30µs medium)     │
  │                       │                    │                     ├ backend.execute() ──►│
  │                       │                    │                     │ (~50ms medium D1)   │
  │                       │                    │                     │◄──── domain_resp ───┤
  │                       │                    │                     │ bridge::to_proto   │
  │                       │                    │                     │ (~15µs)            │
  │                       │                    │                     │ idemp.insert       │
  │                       │                    │                     │ (~25µs)            │
  │                       │                    │                     │ emit_metrics       │
  │                       │                    │                     │ (~5µs)             │
  │                       │                    │ MetricsLayer.end    │◄────── ok(resp) ────┤
  │                       │ proto encode ◄─────┤                     │                    │
  │                       │ HTTP/2 framing     │                     │                    │
  │◄──────── resp ────────┤ proto decode       │                     │                    │
  │                       │                    │                     │                    │
```

## Annexe B — Décomposition coût per-layer (medium fixture, M3 Pro)

| Layer | Op | Coût (µs) | % chaîne |
|-------|----|----------:|---------:|
| Transport | TCP loopback HTTP/2 | 60 | 0.1 % |
| Codec | tonic encode (client) + decode (server) | 35 | 0.07 % |
| MetricsLayer | start instrumentation | 5 | 0.01 % |
| AuthGuard | disabled mode (anonymous) | 5 | 0.01 % |
| Rate limit | bucket atomic | 10 | 0.02 % |
| Idempotency | cache get (miss) | 10 | 0.02 % |
| Bridge (req) | from_proto (medium spec) | 50 | 0.1 % |
| **Backend** | **D1 chain** | **50 000** | **96 %** |
| Bridge (resp) | to_proto | 25 | 0.05 % |
| Idempotency | cache insert | 30 | 0.06 % |
| Metrics | emit_request_completion | 5 | 0.01 % |
| Codec | tonic encode (server) + decode (client) | 30 | 0.06 % |
| **TOTAL** | — | **~52 000 µs** | 100 % |

## Annexe C — Diagramme dépendance D4

```
                  ┌──────────────────────────────┐
                  │ benches/server_rpc.rs        │
                  └─────────────┬────────────────┘
                                │ uses
        ┌───────────────────────┼─────────────────────┐
        ▼                       ▼                     ▼
┌──────────────┐  ┌──────────────────┐  ┌─────────────────┐
│ benches/     │  │ tonic::transport │  │ tokio runtime   │
│ common/mod.rs│  │  ::Server        │  │ (multi_thread)  │
│ (D1 helpers) │  │  ::Channel       │  └─────────────────┘
└──────┬───────┘  └────────┬─────────┘
       │                   │
       ▼                   ▼
┌──────────────┐  ┌──────────────────┐
│ build_real_  │  │ efe_server::     │
│  backend     │  │  Server, Builder │
│ load_v2_spec │  │  ServerConfig    │
└──────┬───────┘  │  SwarmServiceImpl│
       │          │  AuthGuard       │
       │          │  IdempotencyCache│
       │          │  BucketRegistry  │
       │          └────────┬─────────┘
       │                   │
       ▼                   ▼
┌─────────────────────────────────┐
│ Mode1Backend (D1)               │
│  → MvpCpuFusion (D3 chain)      │
│  → MemoryPool (D2 acquire)      │
└─────────────────────────────────┘
```

## Annexe D — Checklist d'exécution architect → impl

```
□ D4.1.1  Squelette + imports + constantes
□ D4.1.2  BenchServer::start/channel/stop
□ D4.1.3  make_full_stack_config / make_minimal_stack_config
□ D4.1.4  proto_request_for_v2_fixture
□ D4.1.5  Cargo.toml [[bench]] entry

□ D4.2.1  bench_server_rpc_full_stack × 3 fixtures
□ D4.2.2  Validate ok responses + non-zero confidence

□ D4.3.1  bench_server_rpc_minimal_stack × 3 fixtures
□ D4.3.2  Mental check Δ ≥ 0 vs full

□ D4.4.1  bench_server_rpc_concurrent × 8c × small

□ D4.5.1  bench_server_rpc_codec_only × medium

□ D4.6.1  tests/server_rpc_realism_check.rs (3 tests)
□ D4.6.2  parse_criterion_report.py extension
□ D4.6.3  bench_runner.sh ajout
□ D4.6.4  BENCHMARKS.md § 6 + slo.md § 7
□ D4.6.5  bench_baselines/v0.1.4/baseline.json placeholder
□ D4.6.6  .github/workflows/bench.yml upload path

□ Validation finale
  □ cargo test -p efe-server (full)
  □ cargo clippy -p efe-server --benches -- -D warnings
  □ cargo bench --bench server_rpc -- --quick
  □ Verification mesures vs § 10 (×1.3 max d'écart)
  □ Δ_interceptors entre 30-200 µs constant
```

---

**Fin de l'architecture D4.** Le sprinter peut implémenter en
suivant Annexe D, en revenant à § 6 pour le détail de chaque
groupe et § 8 pour les pièges à éviter.

> **Approbation requise avant implémentation** :
> - [ ] Architecte performance (revue technique).
> - [ ] Owner `efe_server::services::swarm` (revue handler chain).
> - [ ] Lead infra (revue CI integration).
