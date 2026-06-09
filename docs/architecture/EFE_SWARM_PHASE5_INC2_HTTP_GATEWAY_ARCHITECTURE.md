# EFE — Phase 5 Inc.2 : API HTTP — `POST /v1/swarm/run` · `GET /v1/swarm/verify/:id` · `/metrics`

> **Statut** : Architecture (gelée pour build) · **Date** : 2026-06-09 · **Phase** : 5 (Productisation) · **Inc.** : 2 (gateway HTTP)
> **Sur** : l'**axum 0.7 existant** (`efe-server` pattern) · **Crate cible (neuf, léger)** : `crates/efe-swarm-gateway`
> **Pré-requis** : Phase 5 Inc.1 (`efe-swarm-sdk` : `Swarm::run`, `SwarmRun`, provenance auto + `verify_swarm`).

---

## 0. Thèse — exposer le SDK *et le moat* sur HTTP

Inc.1 a livré `swarm.run(task, topology, budget)` en process. Inc.2 l'expose sur **HTTP**, et —
surtout — expose le **moat** comme une **route** :

> **`GET /v1/swarm/verify/:id`** : un **régulateur** re-prouve cryptographiquement un run
> (chaîne + contenu + DAG + `provenance_root`) **sans faire confiance à l'opérateur**.

C'est l'argument produit décisif : *« voici l'endpoint qui prouve ce que l'essaim a fait — pas
notre parole, une preuve que vous re-vérifiez vous-même, en ligne ou hors-ligne ».*

Trois routes :
- `POST /v1/swarm/run` — exécute un essaim (le SDK), persiste ses **records**, renvoie le `SwarmRun`.
- `GET /v1/swarm/verify/:id` — re-prouve un run stocké (le **vérificateur indépendant** en HTTP).
- `GET /metrics` — exposition Prometheus (provenance + substrat).

---

## 1. Le réel — ce qu'on réutilise (sans réinventer)

### 1.1 L'axum d'`efe-server` (pattern à reproduire)
```rust
// efe-server/src/v1/agent/mod.rs — pattern de router par module.
pub fn router(state: AgentApiState) -> Router {
    Router::new()
        .route("/runs", post(create).get(list))
        .route("/runs/:id", get(get_one))
        .route("/runs/:id/steps/stream", get(stream_steps)) // SSE (async_stream)
        .with_state(state)
}
// main.rs : axum::serve(listener, router).with_graceful_shutdown(…)
```
- **State** : `#[derive(Clone)] struct …State { service: Arc<dyn Facade>, pg: PgPool }`.
- **Auth/tenant** : extracteur `AuthCtx` (`FromRequestParts`) — valide un **JWT OIDC ES256** (clés via
  `EFE_OIDC_PUBLIC_KEY_PEM`/`EFE_OIDC_ISSUER_URL`, **server-side**) → `{subject, tenant, roles}` +
  `require_role/require_tenant`. **Aucune clé manipulée par nous.**
- **Handler** : `async fn h(State(s): State<…>, auth: AuthCtx, Json(body): Json<…>) -> Result<Json<…>, ApiError>`.
- **SSE** : `async_stream::stream! { … }` + `Sse::new(s).keep_alive(KeepAlive::default())`.
- **Deps** : `axum 0.7` (json, tokio, http1, matched-path, query), `async-stream 0.3`, `tower 0.5`.

### 1.2 Les métriques (`efe-telemetry`, sidecar Prometheus)
`efe-server` n'expose pas `/metrics` dans le REST gateway : un **sidecar** (`efe-telemetry`,
`prometheus::TextEncoder`, registre global, :9090) le fait. Nos métriques swarm
(`ProvenanceMetrics`, `SubstrateMetrics`) produisent **déjà** du texte Prometheus
(`render_prometheus()`, **sans dépendance**) — on les expose directement.

### 1.3 Le SDK (Inc.1) — les écarts à combler pour Inc.2
- **DTO** : `TopologySpec`/`Budget`/`ClientMode` ne dérivent **pas `Deserialize`** → on définit des
  **DTO de fil** côté passerelle (contrat HTTP versionné, SDK non pollué).
- **Records** : `Swarm::run` scelle puis **jette** les records (ne renvoie qu'un résumé) → il faut
  **exposer la `SealedProvenance`** pour persister les records (la preuve de `verify/:id`). Les
  `AgentCallRecord` **sont** serde (Phase 4) → stockables/exportables tels quels.

---

## 2. Décision d'architecture — un crate **léger** `efe-swarm-gateway`

> **Ne pas coupler le SDK léger dans `efe-server` (lourd : redis, nats, sqlx, OIDC…).**

`efe-swarm-gateway` expose un **`axum::Router`** (les 3 routes + état), qui peut être :
- **monté** dans l'app `efe-server` existante : `app.nest("/v1/swarm", efe_swarm_gateway::router(state))` ;
- **ou servi en standalone** (`axum::serve`) pour un déploiement souverain minimal (self-host).

Deps : `efe-swarm-sdk`, `efe-swarm-provenance` (verify + records + métriques), `axum 0.7`,
`async-stream`, `tower`, `serde`, `tokio`. **Réutilise `AuthCtx`** d'`efe-server` (trait partagé ou
copie du pattern) pour l'auth/tenant. C'est « sur l'axum existant » (même framework, mergeable),
**sans** alourdir le SDK ni `efe-server`.

---

## 3. Les trois endpoints

### 3.1 `POST /v1/swarm/run` — exécuter
**Requête (DTO de fil)** :
```jsonc
{
  "task": { "text": "The court ruled the clause unenforceable.", "expected": "legal" },
  "topology": {                          // enum tagué (serde "kind")
    "kind": "consensus",
    "generator": "Classify the topic in one word from {technology,finance,health,legal}.",
    "judges": ["Judge how well the label fits.", "Assess relevance."],
    "k": 3, "pattern": "borda", "escalate": false
  },                                     // ou {"kind":"auto"} | "generate_verify" | "pipeline" | "hierarchical"
  "budget": { "max_cents": 100, "on_exceed": "stop" },
  "mode":   "keyless"                    // ou {"self_host":{"endpoint":"http://127.0.0.1:11434","model":"gemma3:12b"}}
}
```
**Handler** :
```rust
async fn run(State(s): State<SwarmGatewayState>, auth: AuthCtx, Json(req): Json<RunRequest>)
    -> Result<Json<RunResponse>, GatewayError>
{
    auth.require_any_role(&["operator", "admin"])?;
    let (spec, budget, mode) = req.into_sdk()?;             // DTO → types SDK
    let swarm = s.swarm_for(&auth.tenant, mode)?;           // Swarm par tenant (clé HKDF server-side)
    let outcome = swarm.run_traced(req.task, spec, budget).await?;  // ← SDK : run + SealedProvenance
    let run_id = Uuid::new_v4();
    s.store.put(run_id, &auth.tenant, &outcome).await?;     // persiste les RECORDS (preuve)
    Ok(Json(RunResponse { run_id, run: outcome.run }))      // SwarmRun (déjà Serialize)
}
```
- **Tenant scoping** : le `Swarm` est construit avec la clé HKDF **du tenant** (server-side).
- **Persistance** : `store.put` enregistre `{run_id, tenant, swarm_id, records, root, output, topology, created_at}`.
- **`ClientMode::Network`** : ici, la passerelle **EST** le serveur — elle peut détenir la clé du
  routeur (server-side, env/Vault) et exécuter Network (ce que l'in-process refusait). *(Câblage du
  routeur réel = sous-tranche ; le SDK renvoie `NetworkRequiresGateway` que la passerelle remplace.)*

**Réponse** : `{ "run_id": "…", "run": { output, topology, accounting, budget, provenance{root,verified,…}, quality } }`.

### 3.2 `GET /v1/swarm/verify/:id` — **re-prouver (le moat)**
```rust
async fn verify(State(s): State<SwarmGatewayState>, auth: AuthCtx, Path(id): Path<Uuid>)
    -> Result<Json<VerifyResponse>, GatewayError>
{
    auth.require_any_role(&["viewer","operator","admin","auditor"])?;
    let stored = s.store.get(id, &auth.tenant).await?.ok_or(GatewayError::NotFound)?; // tenant-scopé
    let key = s.keys.derive(&auth.tenant)?;               // HKDF par tenant (server-side)
    let report = verify_swarm(&stored.records, &key);     // ← le vérificateur INDÉPENDANT
    Ok(Json(VerifyResponse {
        run_id: id, verified: report.ok(), root: report.root_hex(),
        chain_intact: report.chain_intact, content_intact: report.content_intact,
        dag_consistent: report.dag_consistent, traced_pct: report.traced_pct,
        records: report.records, first_break: report.first_break,
    }))
}
```
**Propriété clé (la force du moat)** : `verify_swarm` **recalcule** chaîne + contenu + DAG depuis les
records + la clé. **Même si le store a été altéré** après coup, la chaîne casse → `verified=false`,
`first_break` localise la faute. L'endpoint ne demande pas de *croire* le store ; il *re-prouve*.

**Export hors-ligne (indépendance totale)** : `GET /v1/swarm/runs/:id/records` renvoie les
`AgentCallRecord` (JSON) ; un tiers les vérifie **hors-ligne** avec sa copie de la clé tenant
(`verify_swarm`). C'est l'indépendance maximale : *aucune confiance, même envers notre serveur*.

### 3.3 `GET /metrics` — Prometheus
```rust
async fn metrics(State(s): State<SwarmGatewayState>) -> impl IntoResponse {
    let body = format!("{}{}", s.prov_metrics.render_prometheus(), s.substrate_metrics.render_prometheus());
    ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}
```
Expose `efe_swarm_prov_{records_total,traced_ratio,overhead_ns,bytes,chain_breaks_total,…}` +
`swarm_{blackboard_writes_total,events_published_total,…}` — agrégés **sur tous les runs** (le
`Swarm` alimente un `Arc<ProvenanceMetrics>` partagé par `observe_sealed`/`observe_verify`).
*(Option : bridger vers le registre `prometheus` global d'`efe-telemetry` pour le sidecar unifié.)*

---

## 4. Le `RunStore` — la preuve durable

```rust
pub struct StoredRun {
    pub tenant: String, pub swarm_id: SwarmId,
    pub records: Vec<AgentCallRecord>,   // ← l'ÉVIDENCE (serde, Phase 4)
    pub root: Option<String>, pub output: Value, pub topology: String,
    pub created_at: SystemTime,
}
#[async_trait] pub trait RunStore: Send + Sync {
    async fn put(&self, id: Uuid, tenant: &str, outcome: &RunOutcome) -> Result<(), GatewayError>;
    async fn get(&self, id: Uuid, tenant: &str) -> Result<Option<StoredRun>, GatewayError>; // tenant-scopé
}
```
- **Inc.2** : `InMemoryRunStore` (DashMap, borné/TTL) — suffisant pour la démo + le standalone souverain.
- **Prod** : `PgRunStore` (réutilise le `PgPool` d'`efe-server` : table `swarm_runs` { id, tenant,
  records JSONB, root, created_at }). Les records sont **immutables** (write-once) — toute mutation
  est détectée par `verify_swarm` de toute façon.

---

## 5. Changements SDK requis (additifs, minimes)

1. **`Swarm::run_traced(task, spec, budget) -> Result<RunOutcome, SwarmError>`** où
   `RunOutcome { run: SwarmRun, sealed: SealedProvenance }` — expose les **records scellés** pour la
   persistance. `run` reste le wrapper (jette `sealed`, comportement Inc.1 inchangé).
2. **Métriques partagées** : `SwarmBuilder::with_metrics(Arc<ProvenanceMetrics>)` → `run` appelle
   `metrics.observe_sealed(&sealed)` (alimente `/metrics`).
3. **DTO** : **pas** de serde ajouté au SDK ; les DTO de fil vivent dans `efe-swarm-gateway` et mappent
   vers `TopologySpec`/`Budget`/`ClientMode` (contrat HTTP versionné, SDK pur).

---

## 6. Sécurité & clés (règle absolue, rappelée)
- **Auth** : `AuthCtx` (OIDC ES256, clés `EFE_OIDC_*` **server-side**). RBAC + tenant via
  `require_role`/`require_tenant`. **Je ne saisis ni ne lis aucune clé applicative.**
- **Provenance** : clé **HKDF par tenant** (`HmacKeySource::EnvHkdf`, master server-side). La
  passerelle **dérive** la clé pour sceller/vérifier ; elle ne l'**expose jamais** (le `root` oui, la
  clé non).
- **Isolation** : tout est **tenant-scopé** (run, verify, records, store).
- **Network** : la passerelle (serveur) peut détenir les clés du routeur (Vault) — c'est le bon
  endroit, pas l'in-process.

---

## 7. Asynchronisme — runs courts vs longs
- **Keyless / Mode A** : run ~ms → `POST /v1/swarm/run` **synchrone** (renvoie le `SwarmRun`).
- **Self-host / Mode C (LLM, secondes)** : deux options, exposées :
  - **Sync borné** (timeout) — simple, défaut Inc.2.
  - **Job** : `POST` renvoie `{run_id, status:"running"}` immédiatement ; `GET /v1/swarm/runs/:id`
    poll le statut/résultat ; **SSE** `GET /v1/swarm/runs/:id/events` streame la progression
    (réutilise le pattern `async_stream`/`Sse` — détaillé en **Inc.3 dashboard**).

---

## 8. Schémas (résumé)
| route | méthode | corps / params | réponse |
|---|---|---|---|
| `/v1/swarm/run` | POST | `RunRequest{task,topology,budget,mode}` | `RunResponse{run_id, run:SwarmRun}` |
| `/v1/swarm/runs/:id` | GET | `id` | `StoredRunView{run_id, output, topology, root, created_at}` |
| `/v1/swarm/runs/:id/records` | GET | `id` | `[AgentCallRecord]` (export hors-ligne) |
| `/v1/swarm/verify/:id` | GET | `id` | `VerifyResponse{verified, root, chain/content/dag, traced_pct, first_break}` |
| `/v1/swarm/runs/:id/events` | GET (SSE) | `id` | flux d'événements (Inc.3) |
| `/metrics` | GET | — | texte Prometheus |

Erreurs → `GatewayError` → status HTTP + JSON : `Unauthenticated(401)`, `Forbidden(403)`,
`NotFound(404)`, `BadRequest(400, DTO invalide)`, `Budget(402, BudgetTooLow/Exceeded)`,
`Internal(500)`.

---

## 9. Métriques & SLO
| métrique | cible |
|---|---|
| overhead passerelle (hors SDK/LLM) | **p50 < 5 ms** |
| latence `verify/:id` (re-preuve) | **< 50 ms** pour ~100 records |
| **100 % des runs persistés + re-vérifiables** | assertion (chaque `run` → store) |
| `verify` détecte l'altération | test de falsification (muter 1 record stocké → `verified=false`) |

---

## 10. Phasage de l'Inc.2
- **2.a — SDK** : `run_traced` (→ `RunOutcome` avec `SealedProvenance`) + `with_metrics`. Tests.
- **2.b — gateway core** : crate `efe-swarm-gateway` ; DTO de fil (`RunRequest` + mapping) ;
  `POST /v1/swarm/run` (sync, keyless+self-host) ; `InMemoryRunStore`. Tests (axum `oneshot`).
- **2.c — verify + export** : `GET /v1/swarm/verify/:id` + `GET …/records` ; test de falsification.
- **2.d — /metrics + auth** : `/metrics` (prov+substrat) ; `AuthCtx` (OIDC, tenant scoping) ;
  `PgRunStore` (prod). Exemple `curl` e2e + démo self-host.

---

## 11. Esquisse d'API (gelée pour 2.a-2.d)
```rust
// efe-swarm-gateway
pub fn router(state: SwarmGatewayState) -> axum::Router;   // mergeable: app.nest("/v1/swarm", router(state))
pub struct SwarmGatewayState {
    pub swarms: SwarmFactory,                 // Swarm par tenant (clé HKDF) + mode
    pub store: Arc<dyn RunStore>,
    pub keys: HmacKeySource,                  // EnvHkdf (server-side)
    pub prov_metrics: Arc<ProvenanceMetrics>,
    pub substrate_metrics: Arc<SubstrateMetrics>,
}
// DTO de fil (Deserialize) → mapping vers les types SDK
pub struct RunRequest { task: Value, topology: TopologyDto, budget: BudgetDto, mode: ModeDto }
pub struct RunResponse { run_id: Uuid, run: SwarmRun }
pub struct VerifyResponse { run_id: Uuid, verified: bool, root: Option<String>, chain_intact: bool,
                            content_intact: bool, dag_consistent: bool, traced_pct: f64,
                            records: usize, first_break: Option<Uuid> }
```

---

## 12. Arborescence cible
```
crates/efe-swarm-gateway/
  src/{lib, state, dto, routes/{run,verify,records,metrics}, store, error, auth}.rs
  examples/{serve_standalone.rs, curl_e2e.sh}
crates/efe-swarm-sdk/src/sdk.rs          # + run_traced + with_metrics (additif)
docs/docs/adr/ADR-SWARM-009-verify-endpoint-is-the-product.md
```

---

## 13. Risques & parades
| risque | parade |
|---|---|
| Coupler le SDK léger dans `efe-server` lourd | crate `efe-swarm-gateway` séparé, **mergeable** via `.nest` |
| `verify/:id` ne re-prouve pas vraiment (fait confiance au store) | `verify_swarm` **recalcule** tout ; falsification testée ; **export** pour vérif hors-ligne |
| Polluer le SDK avec du serde HTTP | **DTO de fil** côté gateway (contrat versionné) |
| Runs LLM longs bloquent la connexion | modèle **job** + statut + SSE (Inc.3) |
| Fuite de clé via l'API | la clé tenant (HKDF) reste server-side ; on n'expose **que** le `root` |
| Persistance non durable | `PgRunStore` (write-once) ; et de toute façon `verify_swarm` détecte l'altération |

---

## 14. Définition de « fait » (Inc.2)
1. `POST /v1/swarm/run` : exécute le SDK (keyless + self-host), **persiste les records**, renvoie `SwarmRun`.
2. `GET /v1/swarm/verify/:id` : re-prouve (chaîne+contenu+DAG+root) — **le moat en HTTP** — + `…/records` (export).
3. `GET /metrics` : Prometheus (provenance + substrat).
4. Auth `AuthCtx` (OIDC) + **tenant scoping** sur toutes les routes ; **aucune clé** manipulée par nous.
5. Mergeable dans l'axum d'`efe-server` (`.nest("/v1/swarm", …)`) **et** servable standalone.
6. Tests (axum `oneshot`) verts, **0 warning** ; test de **falsification** sur `verify`. ADR-SWARM-009.

> **Le livrable** : *un dev fait `POST /v1/swarm/run` ; son régulateur fait `GET /v1/swarm/verify/:id`
> — et obtient une preuve qu'il re-vérifie lui-même, en ligne ou hors-ligne, sans nous croire ni
> croire le provider. Le moat, exposé en une URL.*
