# Couche 2 — M3 : la surface servie (architecture, préparation complète)

> Document d'architecture · 2026-07-11 · branche `couche2-m0-frontier` (M0+M1+M2 livrés/poussés)
> Issu d'une reconnaissance chirurgicale (4 agents) + **2 passes adverses** (sécurité ; honnêteté de la topologie servie). Prolonge [`COUCHE2-M2-recu-honnete-ARCHITECTURE.md`](./COUCHE2-M2-recu-honnete-ARCHITECTURE.md) et la [roadmap](./COUCHE2-roadmap.md).

---

## 0. Verdict & principe

**Le design M3 est sain** (les deux critiques concluent « trous-corrigeables », pas de faille de conception). M3 = créer `routes/llm.rs` avec 5 handlers `POST /v1/llm/{vote,relay,debate,specialist,cascade}`, chacun calqué sur `routes/run.rs` : chaque mission tourne **sous un scope M1**, produit un reçu M2 re-vérifiable, et est servie **en mode sync uniquement**. `efe-multi-llm` et `routes/verify.rs` restent **inchangés**.

Mais l'analyse adverse a imposé **4 corrections non-négociables** avant toute implémentation :
1. **Gate d'egress FAIL-CLOSED + ⊆ registre** (la souveraineté par défaut était fictive).
2. **Rédaction du canal d'erreur** (la clé Gemini fuit via le `Display` de l'erreur).
3. **Auth `aud=efe-llm` dédié** (l'extracteur actuel ne porte aucun `aud` par route).
4. **Un seul `mission_id`** threadé (double-mint → `SwarmMismatch` sur tout run honnête).

---

## 1. Le squelette de handler (le gabarit `run.rs`)

Les 5 `run_*` ont une **signature uniforme** ([workflows/*.rs](crates/efe-multi-llm/src/workflows)) :
`pub async fn run_X(coord: &Coordinator, req: XRequest) -> MultiLlmResult<XResponse>` — aucun bus/budget externe (instanciés en interne). Le handler passe **seulement** `&Coordinator` + la requête.

```rust
// crates/efe-swarm-gateway/src/routes/llm.rs  (#[cfg(feature = "frontier-llm")])
pub async fn vote(
    State(s): State<SwarmGatewayState>,
    caller: LlmCaller,                       // §4 — aud = efe-llm
    GatewayJson(req): GatewayJson<LlmVoteRequest>,   // dernier arg (consomme le corps ; 400 si malformé)
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    let coord = s.coordinator.as_ref()
        .ok_or_else(|| GatewayError::NotImplemented("frontier-llm non câblé".into()))?;   // 501, jamais unwrap
    // §2 — GATE FAIL-CLOSED + ⊆ registre (AVANT tout appel) :
    egress_gate(coord, &s.tenant_id(&caller.tenant)?.0, &req.llms)?;                       // 403 sinon
    // Admission RAII (429 INLINE, pas GatewayError::TooManyRequests) :
    let _permit = match s.admit(&caller.tenant) { Ok(p) => p, Err(reject) => return Ok(too_many_response(&s, reject)) };
    // §4/#4 — UN SEUL mission_id, threadé au scope ET au scellement :
    let mission_id = Uuid::new_v4();
    let sink = MissionProvenanceSink::new();
    let scope = MissionScope { tenant_id: s.tenant_id(&caller.tenant)?, mission_id, sink: sink.clone() };
    // La mission tourne SOUS scope → les providers décorés (M1) capturent dans le puits :
    let resp = scoped(scope, run_vote_3llm(coord, req.clone().into())).await.map_err(map_mission_err)?;   // §3
    let topo = topology_vote(&req.llms);                                                   // arité DEPUIS la requête
    let output = serde_json::to_value(&resp).unwrap_or(Value::Null);
    let request = mission_request_summary("llm/vote", &req.llms, None, None);              // §8 — pour le checker
    let run_id = seal_and_store_mission(&s, &caller.tenant, mission_id, &topo, &sink, output.clone(), request).await?;
    Ok((StatusCode::OK, Json(mission_response(&s, run_id, &caller.tenant, output).await?)).into_response())
}
```

- **`routes()` reste sans `.with_state`** ; monter par un `routes_llm() -> Router<SwarmGatewayState>` (routes/llm.rs) + `.nest("/v1/llm", routes::routes_llm())` dans `router()` ([lib.rs:107](crates/efe-swarm-gateway/src/lib.rs)) et `swarm_router()`. **`routes_llm()` + le `.nest` doivent être sous le MÊME `#[cfg(feature = "frontier-llm")]`** que le champ `coordinator` (sinon référence à un champ inexistant quand la feature est off ⟹ échec de compilation).
- **Pas de `Query<RunParams>`** (le `?stream` job — DashboardBus + `tokio::spawn` — est **hors MVP** ; sync-only : le `_permit` est simplement tenu jusqu'au retour).

---

## 2. Le gate d'egress — FAIL-CLOSED + ⊆ registre (la correction de tête)

C'est **à la fois** le rempart de souveraineté **et** la garantie d'honnêteté d'arité. Deux étages, tous deux **fail-closed** :

```rust
fn egress_gate(coord: &Coordinator, tenant_uuid: &Uuid, llms: &[String]) -> Result<(), GatewayError> {
    // Étage 1 — SOUVERAINETÉ (fail-closed) : le frontière est le chemin NON-souverain.
    let pol = coord.policies.get(tenant_uuid);          // TenantId coordinator = Uuid NU (pas le newtype)
    let allowed = match pol.and_then(|p| p.allowed_llms.as_ref()) {
        Some(list) => list,                              // Some([...]) = allowlist explicite
        None => return Err(GatewayError::Forbidden),     // policy absente OU allowed_llms=None ⟹ DENY
    };
    for id in llms {
        if !allowed.iter().any(|a| a == id) {
            return Err(GatewayError::Forbidden);         // 403 EgressDenied : hors allowlist
        }
    }
    // Étage 2 — INTÉGRITÉ (⊆ registre) : un id autorisé mais NON enregistré court-circuite
    // call_one AVANT le décorateur M1 (bridge.rs:47) → aucun record → nœud Missing → faux verified=false.
    for id in llms {
        if coord.bridge.registry().get(id).is_none() {
            return Err(GatewayError::BadRequest(format!("provider non enregistré : {id}")));
        }
    }
    Ok(())
}
```

**Pourquoi FAIL-CLOSED** ([multi_tenant.rs:129](crates/efe-coordinator/src/multi_tenant.rs)) : par défaut le `Coordinator` frontière est bâti **sans policies** ([frontier.rs](crates/efe-swarm-gateway/src/frontier.rs), pas de `.policies()`) ⟹ `PolicyRegistry` vide ⟹ `get(tenant)=None` ⟹ aucun filtre ; et `allowed_llms=None ⟹ permits_llm=true`. Un tenant sans policy ferait de l'egress **non gouverné** — prompt + contexte **en clair** vers Anthropic/OpenAI/Google. Sur un chemin explicitement non-souverain, `None ⟹ DENY` est la seule position honnête. **Ne pas utiliser `filter_llms`** (il réduit silencieusement au lieu de refuser).

**Pourquoi ⊆ registre** ([bridge.rs:47](crates/efe-coordinator/src/llm/bridge.rs)) : `call_one` fait `registry.get(id).ok_or_else(...)?` **avant** `provider.call()`. Un id autorisé mais absent du registre ne déclenche **jamais** le décorateur M1 ⟹ aucun record scellé ⟹ l'arité déclarée dépasse le puits ⟹ nœud `Missing` ⟹ `verified=false` sur un run qui a pourtant retourné `Ok`. Le check ⊆ registre garantit **`K_déclaré == K_dispatché == K_records`** sur le chemin `Ok`.

> **Le vrai gain d'honnêteté de M1 (correctif seal-on-error) neutralise déjà l'échec-runtime d'un provider ENREGISTRÉ** : un `call()` qui échoue produit quand même un record `Errored` ([mission.rs](crates/efe-swarm-gateway/src/mission.rs)) ⟹ nœud apparié ⟹ `verified=true` + `held_strength` faible (intégrité ≠ correction). Le **seul** trou d'under-capture est donc le **registry-miss**, fermé ici.

**Câblage requis** : `frontier::coordinator_from_env` doit charger un `PolicyRegistry` par tenant (env/config) et le passer à `Coordinator::builder().policies(reg)`, sinon le gate refuse tout (fail-closed sans allowlist). C'est un item M3.

---

## 3. Rédaction du canal d'erreur (fuite de credential — bloquant)

**Gemini place la clé dans l'URL** (`?key={api_key}`) puis, sur erreur transport, `reason = format!("transport: {e}")` où `e: reqwest::Error` **embarque l'URL complète avec `?key=<clé>`** dans son `Display`. Cette `reason` remonte : `CoordError::LlmProvider "...{reason}"` → `MultiLlmError::Coord "coordinator: {0}"` ([error.rs:28/14](crates/efe-coordinator/src/error.rs)). Donc `format!("{}", err)` **contient la clé en clair**.

- **Le reçu scellé est PROPRE** : le record M1 pose `error_kind` **constant** (`"provider_error_or_empty"`), jamais la `reason`, et `endpoint()` rend l'hôte **nu** (sans `?key=`). La fuite est **uniquement** dans la réponse HTTP live + les logs `tracing`.
- **`map_mission_err` NE DOIT JAMAIS sérialiser le `Display`/`reason` de l'erreur** dans le corps HTTP :

```rust
fn map_mission_err(e: MultiLlmError) -> GatewayError {
    // détail server-side uniquement (tracing::warn!), JAMAIS dans le corps.
    tracing::warn!(target: "couche2.frontier", error = %e, "mission LLM échouée");
    match &e {
        MultiLlmError::DebateTooFewLlms { .. } => GatewayError::BadRequest("debate: ≥3 LLM distincts requis".into()),
        MultiLlmError::TooManySubTasks { .. }  => GatewayError::BadRequest("specialist: trop de sous-tâches".into()),
        MultiLlmError::MonocultureViolation { .. } => GatewayError::BadRequest("specialist: diversité de routage insuffisante".into()),
        MultiLlmError::BudgetExceeded { .. }   => GatewayError::Budget,                 // 402
        MultiLlmError::CrossTenantAccess { .. } => GatewayError::Forbidden,             // 403
        _ if e.is_retryable()                  => GatewayError::Unavailable,            // 503 (générique)
        _                                      => GatewayError::Internal("provider error".into()), // message GÉNÉRIQUE
    }
}
```

Réutiliser `MultiLlmError::is_retryable()` / `CoordError::is_retryable()` ([error.rs:118](crates/efe-multi-llm/src/error.rs)) pour choisir 502/503 **sans jamais** exposer la `reason`. (Défense en profondeur — hors M3, à noter comme dette : `gemini.rs` devrait passer la clé en header `x-goog-api-key` au lieu de la query-string.)

---

## 4. Auth — un `aud = efe-llm` dédié

**Réfutation** : le `aud=efe-agent-runtime` codé en dur vit dans **efe-server** ([v1/agent/auth.rs:95](crates/efe-server/src/v1/agent/auth.rs)), **pas** dans la gateway. Le problème réel : l'extracteur `Caller` de la gateway ([auth.rs:24](crates/efe-swarm-gateway/src/auth.rs)) ne porte **aucun `aud`**, et l'`OidcVerifier` est **unique et global** (vérifie un `aud` seulement si `EFE_OIDC_AUDIENCE` est défini, [oidc.rs:56/68](crates/efe-swarm-gateway/src/oidc.rs)). Réutiliser `Caller` pour `/v1/llm/*` = **conflation de portée** : un jeton essaim souverain bon marché déverrouillerait l'egress tiers coûteux non-souverain. (Et en dev, `require_any_role` passe dès que `roles` est vide, [auth.rs:34](crates/efe-swarm-gateway/src/auth.rs).)

**Correctif — nouvel extracteur `LlmCaller`** (la voie propre, calquée sur efe-server) :
- Un `LlmOidcVerifier` = clone de `OidcVerifier::from_pem` avec `Validation.set_audience(&["efe-llm"])` **NON conditionnel**, porté en `state.oidc_llm`.
- Un extracteur `LlmCaller` (FromRequestParts) **utilisé UNIQUEMENT par `routes_llm`** ; **401** si `aud != efe-llm`.
- `require_any_role(&["operator","admin"])` pour les 5 handlers (compute) ; `&["viewer","operator","admin","auditor"]` pour le checker `/v1/llm/audit/:id` (lecture).
- **Décision dev à assumer** : pour un chemin non-souverain, **ne pas** autoriser le fallback keyless (roles vides) — exiger un jeton `efe-llm` explicite.

---

## 5. Isolation & le threading du `mission_id`

**Bloquant** : M1 pose `swarm_request_id = scope.mission_id` sur chaque record ; M2 pose `swarm_id = SwarmId::from_uuid(mission_id)` sur chaque nœud Mode-C. Si le handler **minte deux `Uuid` distincts** (un pour `scoped()`, un pour `seal_and_store_mission`), le pont N17 classe **`SwarmMismatch` sur TOUT run honnête**. ⟹ **un seul `let mission_id = Uuid::new_v4()`** par requête, passé **à la fois** à `MissionScope.mission_id` **et** à `seal_and_store_mission`. (`run_id` est distinct, minté **dans** `seal_and_store_mission`, [mission_graph.rs:321](crates/efe-swarm-gateway/src/mission_graph.rs) — c'est la clé du store, pas l'identité de la chaîne.) **Test à ajouter** : `bridge_consistent=true` sur un vote servi.

**Isolation inter-requêtes — SOLIDE mais FRAGILE** : chaque requête a son `mission_id`/`run_id` (`Uuid::new_v4`), son `MissionProvenanceSink` neuf, son `MissionScope`. `scoped()` = `MISSION.scope(scope, fut)` (task-local) ⟹ deux requêtes concurrentes = deux task-locals disjoints, **zéro fuite**. Propriété **cruciale et fragile** : la capture tient **uniquement** parce qu'**aucun `run_*` n'utilise `tokio::spawn`** (specialist=`join_all` inline, vote=`FuturesUnordered` inline, debate=`tokio::join!` inline, relay/cascade séquentiels). Un futur refactor introduisant `tokio::spawn` **perdrait silencieusement** `MISSION` ⟹ sous-capture. **Invariant à figer** (test de non-régression).

---

## 6. Honnêteté de la topologie — tableau par mission

Arité **depuis la requête** ou **recompute pur**, jamais l'output producteur (seule exception : la décision cascade — résidu Tier A documenté).

| Mission | Arité (source) | Spécificité handler |
|---|---|---|
| **VOTE** | K = `req.llms.len()` (requête) | gate ⊆ registre = seul rempart d'under-capture ; ne jamais dériver de `per_llm_scores` |
| **RELAY** | 3 fixe | **fail-closed par construction** : toute phase ratée ⟹ `RelayPhaseFailed` ⟹ `Err` ⟹ le handler ne scelle **jamais** (aucun Ok-avec-undercount) |
| **DEBATE** | 2·`rounds`+1 (requête) | **pré-valider `rounds∈1..=5` au handler → 400** (sinon `run_debate` rend `Config`→500) ; passer `req.rounds`, **jamais** `response.arguments_per_round.len()` |
| **SPECIALIST** | N = `specialist_declared_arity(user_text)` — **le MÊME `Decomposer::default().decompose` PUR** que `run_specialist` ([specialist.rs:52](crates/efe-multi-llm/src/workflows/specialist.rs)), déterministe octet-pour-octet | après gate : si **< 2 LLM permis** ET N ≥ 3 → **400/403** (éviter `MonocultureViolation`→500) |
| **CASCADE** | 2 + (`response.verifier_decision=="REGEN_ALL"` ? 1 : 0) | **résidu Tier A** : la décision est un pur produit des outputs, committée (tamper-evident) mais **non re-dérivable** du reçu (textes one-way ; Tier B différé). Sur-production (validator appelé + décision mentie ACCEPT) ⟹ **attrapée par l'Invariant #2** (`UndeclaredCall`) |

---

## 7. `held_strength` — dans un DTO NEUF, PAS `N17VerifyView`

**Réfutation de la roadmap** : `N17VerifyView` est construit **à deux endroits** — `verify.rs:52` **ET** `replay.rs:125`. Y ajouter un champ **casse la compilation** des deux littéraux (Rust n'omet pas un champ ; `#[serde(default)]` ne compile pas le manque). Étendre `N17VerifyView` violerait le non-négociable « `verify.rs` inchangé ».

⟹ **`held_strength` va dans `LlmMissionResponse`** (DTO neuf propre à `/v1/llm`), pas dans `N17VerifyView`. `HeldStrength` ([n17_chain.rs:203](crates/efe-swarm-provenance/src/n17_chain.rs)) = `{ weakest_rank: Option<u8>, weakest_label: Option<String>, any_remote: bool }`, déjà calculé par `verify_run_offline` (`OfflineVerdict.held_strength`, jeté par `verify.rs`). Le handler le récupère via un `verify_run_offline` sur le run scellé (helper `mission_response`) :

```rust
async fn mission_response(s: &SwarmGatewayState, run_id: Uuid, tenant: &str, output: Value) -> Result<LlmMissionResponse, GatewayError> {
    let stored = s.store.get(run_id, tenant).await?.ok_or(GatewayError::Internal("run perdu".into()))?;
    let key = s.derive_key(tenant)?;
    let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
    Ok(LlmMissionResponse {
        run_id, output,
        provenance_root: stored.root, n17_root: stored.n17_root,
        held: HeldStrengthView { weakest_rank: v.held_strength.weakest_rank, weakest_label: v.held_strength.weakest_label, any_remote: v.held_strength.any_remote },
    })
}
```

**Re-preuve** : `GET /v1/swarm/verify/:run_id` (INCHANGÉ) re-prouve **déjà** un run mission (store partagé) — pas d'alias `/v1/llm/verify` nécessaire (au plus cosmétique, mais l'`aud` diffère ⟹ on documente que la re-preuve d'une mission passe par `/v1/swarm/verify`).

---

## 8. `StoredRun.request` + le checker out-of-band (le seul audit tierce de l'arité)

**`StoredRun.request: Value`** (non-Option, `DEFAULT '{}'`, miroir exact de `output`) :
- `store.rs` : champ + `from_mission`/`from_outcome` gagnent un param `request: Value` (`from_outcome` passe `json!({})`).
- `store_pg.rs` : DDL `CREATE TABLE` + **`ALTER TABLE swarm_runs ADD COLUMN IF NOT EXISTS request JSONB NOT NULL DEFAULT '{}'::jsonb`** (idempotent, bases existantes) + les **3** sites INSERT (`put`, `put_stored`, `insert_stored`) `.bind(Json(&stored.request))` + `SELECT` + `try_get::<Json<Value>>("request")`.
- `seal_and_store_mission` gagne un param `request: Value` (construit par le handler : `{kind, k, rounds, user_text_hash, decompose_mode, verifier_decision}`).

**Checker `GET /v1/llm/audit/:id`** (route NEUVE ≠ `verify.rs`) — l'axe **inverse** de l'Invariant #2 (sous-déclaration), re-dérivé **hors bande** :
1. auth `aud=efe-llm` + `require_any_role([viewer,…])` ; `stored = s.store.get(id, tenant)?` (tenant-scopé).
2. **Re-dériver l'arité déclarée depuis `stored.request`** (jamais l'output) : `K`/`rounds` (vote/relay/debate) ; specialist ⟹ **ré-exécuter** `specialist_declared_arity(request.user_text)` ; cascade ⟹ `2 + (request.verifier_decision=="REGEN_ALL" ? 1 : 0)`.
3. **Compter les nœuds Mode-C** scellés : `stored.records.iter().filter(|r| matches!(r.agent_kind, AgentKind::ModeC)).count()` (patron [sdk.rs:491](crates/efe-swarm-sdk/src/sdk.rs)).
4. **Signaler** : `declared_arity > mode_c_count` ⟹ **sous-déclaration**. DTO `LlmAuditView { kind, declared_arity, mode_c_records, under_declared }`.

C'est ce qui transforme la garantie *write-side* en **auditable par un tiers** (détenteur de clé) — le complément de l'Invariant #2 (sur-déclaration, rejetée au build).

---

## 9. DTOs, mapping d'erreurs, montage

- **DTOs** (`dto.rs`) : `Llm{Vote,Relay,Debate,Specialist,Cascade}Request` (miroir des `XRequest` d'efe-multi-llm + `From`/`into()` vers eux), `LlmMissionResponse { run_id, output, provenance_root, n17_root, held }`, `HeldStrengthView`, `LlmAuditView`. `#[serde(deny_unknown_fields)]` comme `RunRequest`.
- **Erreurs** : réutiliser **`GatewayError`** (403=Forbidden, 400=BadRequest, 402=Budget, 501=NotImplemented, 503=Unavailable existent) via `map_mission_err` (§3). Le **429 d'admission est INLINE** (`too_many_response`, calqué [run.rs:48-68](crates/efe-swarm-gateway/src/routes/run.rs)), **pas** `GatewayError::TooManyRequests`.
- **Montage** : `pub mod llm` dans `routes/mod.rs` ; `routes_llm()` ; `.nest("/v1/llm", …)` dans `router()`+`swarm_router()` — **le tout sous `#[cfg(feature = "frontier-llm")]`**.

---

## 10. Plan d'implémentation & DoD

**Fichiers** : `routes/llm.rs` (neuf : 5 handlers + audit + `egress_gate`/`map_mission_err`/`too_many_response`/`mission_response` + `routes_llm()`), `routes/mod.rs` (mod llm), `lib.rs` (nest + exports), `dto.rs` (Llm*Request/Response/View), `auth.rs`+`oidc.rs` (LlmCaller/LlmOidcVerifier + `state.oidc_llm`), `store.rs`+`store_pg.rs`+`mission_graph.rs` (champ `request`), `frontier.rs` (charger un `PolicyRegistry`).

**DoD (révisée par l'analyse adverse)** :
1. **e2e par mission (5)** : `POST /v1/llm/{x}` (providers autorisés+enregistrés) → 200 ; `GET /v1/swarm/verify/:run_id` ⟹ `verified=true`, `any_remote=true`, `weakest=R4`, `bridge_consistent=true` (thread mission_id).
2. **Souveraineté fail-closed** : tenant **sans policy** ⟹ **403** (aucun record émis) ; llm hors allowlist ⟹ 403 ; llm non enregistré ⟹ 400.
3. **Fuite credential** : un provider en erreur ⟹ corps HTTP **générique** (jamais la clé/URL/reason) ; le `StoredRun` reste propre.
4. **Auth** : jeton sans `aud=efe-llm` ⟹ 401 ; isolation tenant (run A / verify B ⟹ 404) ; admission ⟹ 429+Retry-After.
5. **Erreurs missions** : debate `rounds>5` ⟹ 400 ; specialist < 2 LLM permis + N≥3 ⟹ 400/403 ; MonocultureViolation ⟹ 4xx (pas 500).
6. **Checker** : sous-déclaration détectée (`declared_arity > mode_c_records`) via `/v1/llm/audit/:id`.
7. **Non-régression task-local** : test figeant « aucun `tokio::spawn` dans les `run_*` » (capture complète).
8. Défaut **byte-identique** (tout `#[cfg(feature="frontier-llm")]`).

---

## 11. Ce qu'on NE promet PAS

- **Pas de streaming** (`?stream`) — MVP sync-only.
- **CASCADE** : l'arité 2-vs-3 repose sur `verifier_decision` **committée mais non re-dérivable** (résidu Tier A ; Tier B — digest d'ensemble de mots — différé).
- **Re-preuve trustless-tiers** : inchangé (modèle R4 write-side / détenteur-de-clé, cf. M2 §0). `held_strength` est **rapporté** (`any_remote`, `weakest=R4`), jamais replié dans `verified`.
- **Intégrité ≠ correction** ; **jamais « small beats big »**.

---

## 12. Corrections vs la roadmap M3 (traçabilité)

| Proposition initiale | Réalité (preuve) | Correctif |
|---|---|---|
| gate `⊆ allowed_llms`, défaut par tenant | fail-**open** (registre vide, `None⟹true`, [multi_tenant.rs:129](crates/efe-coordinator/src/multi_tenant.rs)) | §2 **fail-closed + ⊆ registre** |
| — (non prévu) | clé Gemini dans l'URL ⟹ `Display` de l'erreur ([gemini.rs](crates/efe-coordinator/src/llm/providers/gemini.rs)) | §3 **rédaction du canal d'erreur** |
| « `aud=efe-agent-runtime` dans la gateway » | c'est dans **efe-server** ; la gateway n'a aucun `aud` par route | §4 **LlmCaller/LlmOidcVerifier `efe-llm`** |
| étendre `N17VerifyView` (held_strength) | casse `verify.rs`+`replay.rs` (littéraux partagés) | §7 **held_strength dans `LlmMissionResponse`** |
| — (discipline) | double-mint `mission_id` ⟹ `SwarmMismatch` | §5 **un seul `mission_id` threadé** |

*Préparation M3 : la surface servie est mécanique — mais la souveraineté du chemin frontière EXIGE fail-closed + rédaction d'erreur + aud dédié. Sans ces trois, servir les missions serait servir une fuite.*

---

## 13. Statut d'implémentation (M3 livré — 2026-07-11, post-audit de couverture)

Après une **passe d'audit de couverture** (6 lentilles : câblage-policy, auth-aud, injection-state, DTOs, tests-DoD, critique-complétude), voici l'état réel item par item.

**Fait & vérifié** (build par défaut byte-identique ; `--features frontier-llm` ; `--features frontier-llm,oidc` ; `--features frontier-llm,postgres`) :

| Item | Où | Preuve |
|---|---|---|
| 5 handlers + `/v1/llm/audit/:id` + `routes_llm()` | `routes/llm.rs` | monté sous `#[cfg]` dans `router()` + `llm_router()` |
| §2 gate **fail-closed + ⊆ registre** | `egress_gate` | 3 tests unit + 403/400 HTTP e2e |
| §2 **chargement du `PolicyRegistry`** (`EFE_LLM_TENANT_ALLOWLIST`) | `frontier.rs` `policy_registry_from_env`/`_from_str` | test parseur (pur, race-free) |
| §3 **rédaction du canal d'erreur** | `map_mission_err` | `map_mission_err_redacts_provider_secret` (2 secrets, 0 fuite) |
| §4 **`LlmCaller` + `OidcVerifier::llm_from_env` (aud=efe-llm) + `state.oidc_llm`** | `auth.rs`/`oidc.rs`/`state.rs`/`main.rs` | RBAC strict (unit) + `vote_401_when_wrong_audience_then_200_when_efe_llm` (oidc) |
| §5 **un seul `mission_id`** threadé | 5 handlers | `vote_e2e…` ⟹ `bridge_consistent=true` |
| §7 **`held_strength` dans `LlmMissionResponse`** | `routes/llm.rs` | `held.any_remote=true` (R4) sur un vote servi |
| §8 **`StoredRun.request` + checker** | `store.rs`/`store_pg.rs`/`mission_graph.rs`/`audit` | positif + négatif + tenant-scope 404 |
| **DoD #1..#8** | `routes/llm.rs` + `lib.rs` (#8) + `efe-multi-llm/tests/no_spawn_guard.rs` (#7) | e2e servi, 403/400, redaction, 429+aud, erreurs-missions, checker, garde no-`spawn`, byte-identique |

**Dette assumée (tracée, non-bloquante)** :
- **§9 DTOs miroir `Llm*Request` + `#[serde(deny_unknown_fields)]`** — **DÉFÉRÉ**. Ce n'est aucun des 8 items DoD ; l'audit le classe « acceptable MVP, valeur-sécurité nulle » (isolation/egress/auth sont indépendants des champs de requête). Une implémentation propre exigerait soit un couplage gateway → `efe_nano_app_coordinator::types::router::TaskClass` (non réexporté par `efe-multi-llm`/`efe-coordinator`), soit de toucher `efe-multi-llm` (non-négociable). Les 5 requêtes d'`efe-multi-llm` sont réutilisées directement comme DTOs HTTP. **Risque résiduel borné** : un champ optionnel mal orthographié (ex. `max_budget` au lieu de `max_budget_cents`) retombe au défaut (500 c = 5 $) au lieu d'un 400 — jamais une faille, jamais un dépassement non-borné. À reprendre si/quand un contrat HTTP stable devient requis (le miroir vivra dans `dto.rs` avec `From`).
- **§3 (défense en profondeur, hors M3)** : `gemini.rs` devrait passer la clé en header `x-goog-api-key` plutôt qu'en query-string (le reçu scellé + le corps HTTP sont déjà propres via `map_mission_err`, mais les logs bruts restent une surface).
- **Reste** : **M4** (le prod gate) au plan roadmap.
