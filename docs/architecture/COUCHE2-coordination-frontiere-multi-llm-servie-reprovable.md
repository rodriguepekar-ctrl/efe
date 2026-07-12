# Couche 2 — Coordination des modèles frontière (Claude / Gemini / GPT), servie et re-vérifiable

> Document d'architecture · 2026-07-10 · état du code au commit `93178e5`
> Prolonge `ADR-SWARM-016` (forensique N17 durable & re-prouvable hors-ligne) et `ADR-D93-003` (stratégie de montage REST).
> Issu d'un panel de 3 architectures indépendantes passées au crible adverse (scores 78 / 66 / 51).

---

## 1. Le problème en une phrase

La coordination des modèles frontière **existe et est testée** (`efe-coordinator` : backends HTTP réels Anthropic/OpenAI/Gemini + `MultiLlmBridge` fan-out ; `efe-multi-llm` : 5 missions `vote` / `relay` / `debate` / `specialist` / `cascade`, ~928 tests cumulés), mais c'est une **bibliothèque que rien ne sert** : `efe-server` ne dépend ni de `efe-coordinator` ni de `efe-multi-llm`, aucun handler HTTP de production n'appelle les 5 missions. Le but de ce document : la rendre **servie, multi-tenant, et re-vérifiable** — chaque mission produit un reçu qu'on re-prouve hors-ligne, comme un run d'essaim.

Les 4 non-négociables du projet cadrent tout le design :

1. **Reçu re-vérifiable** : une mission produit une racine de provenance rejouable hors-ligne, zéro provider, la clé tenant HKDF n'étant **jamais** écrite dans le bundle. Crypto **identique** à `efe-swarm-provenance`/`efe-nano` — aucun mécanisme ad hoc.
2. **Souveraineté honnête** : les appels frontière **envoient des données à l'extérieur** (Anthropic/OpenAI/Google). Le reçu doit **enregistrer** cet egress externe (quel prompt haché, vers quel modèle), le **gater** par politique tenant, et ne jamais contourner l'egress guard. Le frontière est le chemin **haute-capacité NON-souverain**, à opposer proprement au méta-essaim (où seul le latent sort).
3. **Intégrité ≠ correction** : le reçu prouve *ce qui s'est passé* (quels modèles, quels hachés, quel vote), pas que la réponse est juste.
4. **Isolation tenant** de bout en bout ; **model-agnostic** (un tenant peut router vers un SLM local ou vers le frontière) ; ne jamais écrire « small beats big ».

---

## 2. Décision d'architecture

**Étendre `efe-swarm-gateway`** (thèse A du panel, score **78/100**) : greffer les 5 missions comme des « runs » supplémentaires **dans la gateway existante**, sur le même `SwarmGatewayState`, la même admission `admit()`, le même `RunStore` et surtout le **même `verify_run_offline`** déjà exposé par `GET /v1/swarm/verify/:id`.

Écartées :

- **Thèse B — service borné dans `efe-server`** (score 66) : fidèle au précédent `plumbing-only` du méta-essaim, mais co-localise 6 nouvelles deps (clients HTTP frontière) dans le serveur produit et duplique la machinerie de reçu déjà présente dans la gateway. Retenu de B : le patron de flag `EFE_*_ENABLED` default-off.
- **Thèse C — unifier sur `efe-llm-router` puis servir** (score 51) : **rejetée** pour des raisons de fond (§7).

### Le rejet du chemin router (unanime des 3 critiques)

`efe-llm-router` **ne peut pas** porter la coordination servie ni le reçu :

- **Reçu non re-vérifiable hors-ligne** : son audit HMAC est **adossé à Postgres** (`audit.rs`, `PgPool`, table `llm_call_audit`, chaîne SHA-256 sur des lignes DB). Crypto différente de l'essaim (Blake3 + `chain_hmac`) et **DB-dépendante** → viole le non-négociable #1.
- **Non adressable par modèle** : `select_providers` (`router.rs:267`) choisit le provider depuis `allowed_providers` du tenant + coût + **failover** ; `RouterRequest` n'a **aucun champ « utilise exactement ce provider »**. Déléguer chaque `call_one` au router les fait **tous converger vers un seul provider** → le fan-out multi-modèle, le vote, et l'anti-monoculture **I11** (≤60 %/LLM devient 100 %) sont **détruits**.
- **`temperature = 0.0` codée en dur** dans `router.complete` → tue la diversité voulue par `debate`/`vote`.
- **Impédance de protocole** : `router.complete` rend un prompt depuis un template de **classification** (`{catalog, sentence, candidates}`), pas un chat libre ; la clé de cache ignore la mission → **collision inter-missions** (une réponse de `vote` peut revenir pour un round de `debate`).

**Conséquence** : on **garde les backends réels du coordinator** comme chemin d'exécution frontière, et on **gèle `efe-llm-router` comme non-servi** (nettoyage de suivi : retirer les deps `efe-llm-router` inutilisées dans `efe-active-learning`/`efe-curation`).

---

## 3. Principe directeur : d'une mission à un reçu

```
Une mission = N appels LLM
      │
      ├─►  N × LlmProvenanceRecord ──► seal_n17 ─────────►  n17_root      (chaîne forensique)
      │        (efe-nano-sdk)         (HMAC-SHA256)
      │
      └─►  graphe AgentCallRecord Mode-C ──► SwarmChain ──►  provenance_root  (graphe causal)
               (efe-swarm-provenance)        (HMAC)
                          │
                          └─ chaque nœud porte llm_provenance_id ─► pont Mode-A↔Mode-C
                                                                     (MapResolver + verify_bridge)

Re-preuve :  verify_run_offline(records, llm_records, n17_leaves, n17_root, key)
             → recalcule chaîne + contenu + DAG + N17 + pont, SANS aucun provider.
```

On réutilise **directement**, sans rien réinventer : `LlmProvenanceRecord`/`ComputeAttestation` (`efe-nano-sdk`), `seal_n17`/`verify_run_offline`/`AgentCallRecord`/`SwarmChain` (`efe-swarm-provenance`, `n17_chain.rs:90/311`), `chain_hmac`/`HmacKeySource` (`efe-audit-core`), `StoredRun`/`RunStore`/`verify` (`efe-swarm-gateway`). La clé tenant est dérivée HKDF (`HmacKeySource::derive`, `info = b"efe-nano/audit/v1"`), crypto **byte-identique** au reste, golden-verrouillée.

---

## 4. Placement & surface d'API

Tout vit dans `crates/efe-swarm-gateway` :

| Élément | Fichier | Nature |
|---|---|---|
| 5 handlers + alias verify | `src/routes/llm.rs` **(nouveau)** | monté par `routes::routes()` ou un `routes_llm()` jumeau via `.nest("/v1/llm", …)` |
| Capture + graphe + scellement | `src/mission.rs` **(nouveau)** | `MissionScope` (task-local), `ProvenanceProvider` (décorateur), `MissionProvenanceSink`, `MissionGraphBuilder` |
| Coordinator frontière | `src/state.rs` | champ `coordinator: Option<Arc<Coordinator>>` près de `router_endpoint` (`state.rs:164`) + builder `with_coordinator()` (calque `with_router_endpoint`, `state.rs:386`) |
| DTO | `src/dto.rs` | `Llm{Vote,Relay,Debate,Specialist,Cascade}Request`, `LlmMissionResponse`, `LlmApiError` ; extension `N17VerifyView` (`held_strength`) |
| Persistance | `src/store.rs` | `RunStore::put_stored()` + `StoredRun::from_mission()` |
| Câblage prod | `src/main.rs` | env → `with_coordinator` sous flag |

**Endpoints** (calque exact de `routes/run.rs:33` : `require_any_role([operator,admin])` → `admit()` → gate egress → mission → `store.put`) :

| Route | Méthode | Réponses |
|---|---|---|
| `/v1/llm/vote` | POST | 200 `LlmMissionResponse` · 202 `JobAccepted` (?stream) · 429+Retry-After · **403 EgressDenied** · 501 (frontière non câblée) |
| `/v1/llm/relay` | POST | 200 (chaîne plan→code→synthèse) · 202 · 429 · 403 |
| `/v1/llm/debate` | POST | 200 (verdict) · **400 DebateTooFewLlms** (<3 LLM distincts ou rounds∉1..=5) · 429 · 403 |
| `/v1/llm/specialist` | POST | 200 (assignments, I11 ≤0.60, borné 10 sous-tâches) · 202 · 429 · 403 |
| `/v1/llm/cascade` | POST | 200 (décision + `validator_used`) · 202 · 429 · 403 |
| `/v1/llm/verify/:id` | GET | 200 `VerifyResponse` (200 **même si** `verified=false`) · 404 |

`GET /v1/llm/verify/:id` est un **alias** de `/v1/swarm/verify/:id` (store partagé) — `verify_run_offline` couvre déjà le cas « N17 seul ». Le reçu se re-prouve donc sans code neuf côté verify ; on n'étend que le DTO pour **surfacer** `held_strength`.

---

## 5. Le reçu re-vérifiable — et les 3 correctifs de correction obligatoires

Le noyau (`verify_run_offline`) re-vérifie **réellement** hors-ligne, zéro provider, clé jamais dans le bundle, cross-tenant cassé — c'est prouvé dans le code. **Mais les 3 critiques adverses ont convergé sur la même faille**, et deux autres. Elles sont **bloquantes** : à corriger *avant* implémentation.

### Correctif #1 — Construire le graphe depuis la topologie DÉCLARÉE (la faille centrale)

`verify_run_offline` renvoie `fully_proven = true` sur un ensemble **vide ou partiel** : `verify_swarm(&[])` est trivialement vrai, une chaîne N17 à 0 record est « trivialement intègre », et `verify_bridge` ne vérifie que le sens Mode-C→N17 (chaque nœud présent résout) — **rien ne lie l'ensemble des records à l'ensemble réel des appels d'egress**. Donc un `scoped()` oublié, un provider non décoré, ou tous les appels échoués-et-droppés produisent un **reçu vide qui se re-prouve `fully_proven=true`** tout en étant incomplet. Puisque « enregistrer l'egress » est le non-négociable #2, **un egress omis = trou de souveraineté silencieux qui passe la vérif**.

**Fix** : `MissionGraphBuilder` construit le graphe depuis la **topologie déclarée** de chaque mission (arité attendue : `vote` = N feuilles + 1 réducteur ; `relay` = chaîne de 3 ; `debate` = (pro, contra) par round + jury ; `specialist` = decompose→N→aggregate ; `cascade` = drafter→verifier→(validator si utilisé)). **Chaque nœud déclaré EXIGE un `llm_provenance_id` résolu** → un appel manquant devient `Unlinked` → `all_mode_c_consistent = false` → `verified = false`. L'omission devient **détectable**. En complément : asserter `records.len() == appels_attendus` au drain, pas seulement en test.

### Correctif #2 — Observer l'endpoint, ne pas l'inférer

Le trait `LlmProvider` du coordinator n'expose que `id()`/`call()`, **pas l'URL**. Poser `R4RemoteChannel{endpoint}` au niveau décorateur/registry = endpoint **inféré** d'une table alias→host, jamais observé, et **l'egress guard `is_local_endpoint` ne peut pas classer** (pas d'URL). Pour les 3 providers frontière (URLs constantes `anthropic.rs:29`/`openai.rs:45`/`gemini.rs:28`) l'inférence == vérité — mais pour le cas **model-agnostic** exigé (`local_slm` avec `EFE_LOCAL_SLM_URL` surchargeable), le barreau R2-local vs R4-distant devient **indécidable au bon endroit**.

**Fix** : étendre le trait `LlmProvider` du coordinator d'un accesseur **additif** `fn endpoint(&self) -> Option<EndpointIdentity>` (défaut `None`). Le décorateur `ProvenanceProvider` **observe** alors l'endpoint réel, exécute `is_local_endpoint`/`host_of` dessus (anti-SSRF : coupe fragment/query/userinfo), et **impose l'invariant** : endpoint **non-local ⟹ `R4RemoteChannel{keyed:true}`**. C'est un changement minime et additif du coordinator, justifié : la souveraineté doit être **observée**, pas posée à l'aveugle.

### Correctif #3 — Capturer les échecs, `call_many` (pas `call_many_ok`)

Le record doit être émis **sur le chemin d'erreur aussi** (avant de propager l'`Err`) : un appel parti-mais-échoué a fait sortir des octets, il **doit** figurer au reçu. Le décorateur émet le record puis propage l'erreur ; on bascule `call_many_ok` → `call_many` pour que les providers morts remontent à la mission **et** au reçu (fermeture du piège « échecs invisibles »).

### Correctif #4 — Enforcer l'invariant R4 (anti-mensonge par défaut)

`ComputeAttestation::LegacyUnspecified` est `#[default]`, **rang 0**, `is_remote() = false`. Un seul chemin qui oublie de poser `R4RemoteChannel` fait **mentir** `held_strength.any_remote` — et `verify_run_offline` **ne fait aucun cross-check** (il fait confiance au stamp). Fix : le décorateur pose **toujours** R4 pour un endpoint non-local (rendu possible par #2) **+ un test qui échoue** si un record dont l'endpoint est non-local porte un stamp < R4.

### Correctif #5 — Auth : ne pas conflater les portées

`AuthCtx` épingle `aud = efe-agent-runtime`. Monter `/v1/llm/*` sous le même extracteur ferait accepter des jetons de portée « agent-runtime » pour piloter l'egress frontière. Fix : **nouvel `aud = efe-llm`** ou extracteur dédié, décidé avant la Phase 3.

---

## 6. Souveraineté — et l'honnêteté sur le modèle de confiance

**Egress enregistré** : chaque appel est estampillé `R4RemoteChannel{keyed:true, endpoint}` — le reçu scelle **quel prompt haché** (blake3) part vers **quel `model_identity`/endpoint**. **Egress gaté** : avant le fan-out, le handler vérifie `req.llms ⊆ allowed_llms(tenant)` (`PolicyRegistry`) et refuse en **403 `EgressDenied`** — enforcement que les missions elles-mêmes **n'appliquent pas** (`efe-multi-llm` n'a **aucune** référence à `allowed_llms` : le gate à l'entrée est réellement load-bearing). **Guard non contourné** : `is_local_endpoint`/`host_of` réutilisés pour **classer** (jamais `assert_local` sur le chemin frontière, qui refuserait à raison un endpoint non-local) ; les **deux copies** (`provenance-capture/guard.rs` + `efe-swarm-sdk/backend/mod.rs`) sont durcies **ensemble** (drift SSRF déjà survenu). **Clé jamais dans le bundle** : `StoredRun` ne porte que records + feuilles + racines (hex) ; `derive_key` re-dérive server-side à la vérif ; `keyed:true` **signale** qu'un credential a transité, le credential lui-même n'est **jamais** sérialisé. **Isolation tenant** : `derive_key(tenant)` + `store.get(id, tenant)` (404 cross-tenant).

### ⚠️ Le modèle de confiance — à énoncer sans sur-vente

Sur le chemin frontière (R4), `chain_hmac` + `HmacKeySource` sont **symétriques**, `keyed:true`, **sans cosignature**. Donc le reçu est :

> **inviolable après-coup par un non-détenteur de la clé, et re-vérifiable par le détenteur de la clé** (le tenant, ou un auditeur à qui le tenant confie la clé).

Il **n'est PAS** re-vérifiable par un tiers *sans confiance* : un détenteur de la clé aurait pu forger tout le bundle. Seul le chemin **`R3RemoteCosigned`** (ed25519 / `NodeRegistry`) est trustless — et les providers frontière **n'y arrivent jamais** (`weakest = R4`). **Ne pas vendre « n'importe quel tiers re-prouve hors-ligne » pour le frontière.** C'est la formulation exacte à tenir face à un interlocuteur technique. (Un chemin R3 cosigné pour le frontière est une extension future possible, hors périmètre.)

**Opposition honnête au méta-essaim** : le reçu affiche `held_strength.weakest = R4 / any_remote = true` — la mission frontière **est** le chemin haute-capacité **non-souverain** (prompt + latent sortent en clair vers Anthropic/OpenAI/Google), à opposer proprement au méta-essaim (où seul le latent sort). **Intégrité ≠ correction** : `fully_proven` prouve le graphe, jamais la justesse ; `held_strength` est rapporté **à côté**, jamais replié dans `verified`.

---

## 7. Stratégie providers

**Garder** les backends HTTP réels du coordinator (Anthropic `claude-sonnet-4-6` / OpenAI `gpt-4o` / Gemini `gemini-2.5-flash`, ~728 tests) comme chemin d'exécution des missions. **Ne pas** router les missions par `efe-llm-router` (§2). On assume **deux chemins d'egress** (essaim → backend SDK/Network ; mission → backends coordinator) mais on **unifie la souveraineté** : les deux passent par la **même** classification `is_local_endpoint`, le **même** barreau `R4RemoteChannel` et le **même** `seal_n17` sous clé tenant. Le seam `router_endpoint` (501) reste réservé au chemin essaim, non touché.

---

## 8. Flux bout-en-bout (`POST /v1/llm/vote`)

1. Extracteur `Caller` → `require_any_role([operator, admin])` (**aud `efe-llm`**, cf. correctif #5).
2. `s.admit(&caller.tenant)` → permit composite global+tenant, sinon **429 + Retry-After**. Permit tenu en sync, **déplacé** dans le `tokio::spawn` en mode job (anti-fuite de compteur).
3. `GatewayJson<LlmVoteRequest>` (400 sur corps malformé) → **normalisation** des ids `req.llms` vers les alias enregistrés (« claude »/« gpt »/« gemini »), résolvant le mismatch `id()`.
4. **Gate egress** : `req.llms ⊆ allowed_llms(tenant)` ; classer chaque endpoint via `is_local_endpoint` ; refus → **403 EgressDenied**.
5. Résoudre budget/deadline ; préserver `price_per_call` en microcents.
6. **Entrer** le scope task-local `MissionScope { tenant_id, mission_id, sink }`.
7. `run_vote_3llm(&coord, req).await` : les `ProvenanceProvider` décorés poussent **un record R4 par appel** (blake3 prompt/output, coût, **échecs compris**) dans le sink — **missions inchangées**.
8. **Sortir** le scope → drainer → `MissionGraphBuilder` construit le graphe **depuis la topologie déclarée** (correctif #1), pose `llm_provenance_id` sur chaque nœud, `SwarmChain::append_all_canonical` → `provenance_root` ; `seal_n17(&llm_records, key)` → `n17_root`.
9. `StoredRun::from_mission(tenant, records, provenance_root, llm_records, n17, output, topology="llm/vote")`.
10. `redact_output` selon la politique PII du tenant (G9).
11. `store.put_stored(run_id, tenant, stored)` — un échec de persistance **se signale**, jamais avalé (le moat n'est pas négociable).
12. **200** `LlmMissionResponse { run_id, output, winner_llm, provenance_root, n17_root, held_strength, cost_microcents }`.
13. **Re-preuve** (détenteur de la clé, hors-ligne) : `GET /v1/llm/verify/:id` → `derive_key` + `verify_run_offline` recalcule chaîne+contenu+DAG+N17+pont, zéro provider → `VerifyResponse { verified, provenance_root, n17_root, held_strength=R4/any_remote }`. **200 même si `verified=false`** (la falsification est un fait, pas une erreur transport).

Le mode job (`?stream=true`) est **hors MVP** (§10) : le `DashboardBus` est alimenté par le chemin essaim SDK, pas par les 5 missions — le SSE mission-level est un chantier non démontré.

---

## 9. Crates touchés & types nouveaux

**Touchés** : `efe-swarm-gateway` (cœur : `routes/llm.rs`, `mission.rs`, champ state, DTO, `put_stored`, `main.rs`, deps) ; `efe-coordinator` (**changement additif minime** : accesseur `endpoint()` sur le trait `LlmProvider`, correctif #2 — sinon zéro) ; `efe-multi-llm` (**zéro** : les 5 `run_*` appelés tels quels, `audit_chain_hash` placeholder ignoré) ; `provenance-capture` + `efe-swarm-sdk` (re-synchroniser les 2 copies du guard).

**Nouveaux types** : `ProvenanceProvider` (décorateur `Arc<dyn LlmProvider>` qui scelle chaque `call` en `LlmProvenanceRecord`, observe l'endpoint, capture les échecs) · `MissionScope` (task-local, miroir de `efe-swarm-sdk/src/n17.rs:53`) · `MissionProvenanceSink` (puits, un `provenance_id` frais par appel) · `MissionGraphBuilder` (**topologie déclarée** → `Vec<AgentCallRecord>` Mode-C) · `Llm{…}Request`/`LlmMissionResponse`/`LlmApiError` (DTO miroir de `RunRequest`/`RunResponse`) · `StoredRun::from_mission` + `RunStore::put_stored` (**pas trivial** : trait `RunStore::put` couplé à `&RunOutcome` → étendre le trait + `InMemory` + `PgRunStore` + `store_pg_memory`, à chiffrer en Phase 2).

---

## 10. Rollout (feature-flaggé, default-off)

- **Phase 0 — flag & deps** : `EFE_LLM_FRONTIER_ENABLED` (défaut **false** : off = aucune route montée, byte-identique). Ajouter `efe-coordinator`/`efe-multi-llm`/`efe-nano-sdk` ; **aligner `tonic`/`reqwest`/`prost`** avec le workspace. *(S, 1-2 j)*
- **Phase 1 — capture** : `ProvenanceProvider` + `endpoint()` (correctif #2) + `MissionScope` + sink, testés avec les mocks `#[cfg(test)]`. Aucune route. *(M, 2-4 j)*
- **Phase 2 — synthèse de graphe (chemin critique)** : `MissionGraphBuilder` **depuis la topologie déclarée** (correctif #1) pour les 5 topologies + `StoredRun::from_mission` + `put_stored` (trait + 3 impls) + goldens de re-preuve. *(L, 5-8 j)*
- **Phase 3 — handlers** : monter `/v1/llm/*` sous flag, **aud `efe-llm`** (correctif #5), gate egress, mapping `LlmApiError`, extension `N17VerifyView` (`held_strength`). Mode **sync uniquement**. *(M, 3-5 j)*
- **Phase 4 — câblage prod** : `main.rs` lit le flag + clés (`ANTHROPIC_API_KEY`/`OPENAI_API_KEY`/`GOOGLE_AI_KEY`) + **`EFE_NANO_AUDIT_HMAC_MASTER` (OBLIGATOIRE** — sinon `test_key()` publique = provenance forgeable, **fail-fast au boot**) ; `PgRunStore` (feature `postgres`) pour des reçus durables. *(S, 1-2 j)*

**Back-compat** : `/v1/swarm/*` et le chemin essaim **inchangés** ; store et verify partagés → `/v1/swarm/verify/:id` re-prouve déjà une mission. Rollback = flag off. DB : réutiliser la table `swarm_runs`, aucune table neuve. **Total ~3-4 semaines-ingénieur, chemin critique = Phase 2.**

---

## 11. Plan de test

- **Capture** : `ProvenanceProvider` produit un record par appel avec `R4RemoteChannel{keyed:true}` + hachés blake3 corrects ; un appel **en échec** produit un record (pas de drop silencieux).
- **Graphe** : pour chaque mission, `MissionGraphBuilder` produit la bonne topologie (arités, arêtes, `llm_provenance_id` posé) ; `provenance_root` stable à ensemble fixé.
- **Omission (correctif #1)** : dropper un appel → `verify` doit renvoyer `verified=false` (`Unlinked`). *Ce test échoue avec un graphe construit depuis le sink — il valide le fix « topologie déclarée ».*
- **Invariant R4 (correctif #4)** : un record dont l'endpoint est non-local avec stamp < R4 est **rejeté/flaggé**.
- **Model-agnostic** : mission routée vers un **SLM local** (`EFE_LOCAL_SLM_URL`) → barreau **R2** observé (pas R4).
- **Re-preuve e2e** : `POST /v1/llm/vote` → `GET verify` → `verified=true`, roots présents, `held_strength.any_remote=true`, `weakest=R4`.
- **Tamper** : muter un record dans le store → `verify` 200 `verified=false`, `first_break` localisé (N17 **et** essaim).
- **Souveraineté** : `req.llms` hors `allowed_llms` → 403, aucun record émis ; grep sur `endpoint`/`provider_metadata` : **le credential n'apparaît nulle part**.
- **Isolation tenant** : run tenant A, verify tenant B → 404 ; la clé de B ne re-prouve pas le bundle de A.
- **Admission** : saturer → 429+Retry-After.
- **Missions** : `debate` <3 LLM distincts ou rounds>5 → 400 ; `specialist` borné 10 ; **I11 ≤0.60** respecté.
- **Doc négatif** : consigner qu'un tiers **sans** la clé tenant **ne peut pas** vérifier le chemin R4 (modèle de confiance, §6).
- **Golden crypto** : `provenance_root`/`n17_root` recomputés cross-crate identiques.
- **Non-régression essaim** : `/v1/swarm/*` inchangé — **exécuter isolé** (flakiness workspace connue).

---

## 12. Ce qu'on NE promet PAS (encadré d'honnêteté)

- **Pas de re-preuve trustless par un tiers** sur le chemin frontière R4 (HMAC symétrique, sans cosignature) : c'est **tamper-evident + re-vérifiable par le détenteur de la clé**, pas trustless. Le trustless exige R3 cosigné (absent des providers frontière).
- **Pas de reproductibilité par re-exécution** : le reçu prouve la tamper-evidence d'un run donné (`uuid`/`recorded_at` par run), **pas** « même mission → même racine ».
- **Intégrité ≠ correction** : `verified=true` ne dit **rien** de la justesse de la réponse.
- **Jamais « small beats big »** : le reçu vaut identiquement pour un SLM local (R1/R2) ou le frontière (R4) — c'est du model-agnosticism, pas une hiérarchie de qualité.

---

*Synthèse d'un panel de 3 architectures (A : étendre la gateway, 78 ; B : service `efe-server`, 66 ; C : unifier le router, 51) et de 3 critiques adverses convergentes. Décision : thèse A + 5 correctifs bloquants. Le chantier réel est la Phase 2 (synthèse de graphe depuis la topologie déclarée) — c'est là que le reçu devient honnête.*
