# Couche 2 — Roadmap d'implémentation

> Compagnon de [`COUCHE2-coordination-frontiere-multi-llm-servie-reprovable.md`](./COUCHE2-coordination-frontiere-multi-llm-servie-reprovable.md) · 2026-07-10
> Objectif : rendre la coordination frontière (Claude/Gemini/GPT) **servie, multi-tenant et re-vérifiable**.
> Base : commit `93178e5`. Rien de ce qui suit ne modifie le chemin essaim ni ne s'active sans le flag `EFE_LLM_FRONTIER_ENABLED`.

---

## Vue d'ensemble

5 jalons séquentiels. Le chemin critique est **M2** (synthèse de graphe) — c'est là que le reçu devient *honnête*. Les 5 correctifs bloquants issus de la critique adverse sont répartis en gates : aucun jalon n'est « fait » tant que son correctif n'a pas de test qui le prouve.

```
M0 ──► M1 ──► M2 ──► M3 ──► M4
flag   capture GRAPHE handlers  gate
&deps  observée HONNÊTE servis   prod
 S       M      L★      M        S
                │
          chemin critique
```

| Jalon | Nom | Effort | Correctifs fermés | Sortie clé (binaire) |
|---|---|---|---|---|
| **M0** | Fondations | S (1-2 j) | — | flag off = build byte-identique ; deps alignées |
| **M1** | Capture observée | M (2-4 j) | #2 endpoint, #3 échecs, #4 R4 | 1 record R4 par appel, endpoint **observé**, échecs inclus |
| **M2** | Reçu honnête ★ | L (5-8 j) | #1 topologie déclarée | **un appel manquant ⟹ `verified=false`** |
| **M3** | Surface servie | M (3-5 j) | #5 aud `efe-llm` | e2e POST→verify vert sur les 5 missions |
| **M4** | Gate production | S (1-2 j) | — (checklist go/no-go) | fail-fast master HMAC ; Pg durable ; guard re-synchronisé |

**Total ~3-4 semaines-ingénieur** (dev focalisé). Chemin critique = M2. Buffer d'intégration à prévoir sur M0 (alignement `tonic`/`reqwest`/`prost`).

---

## M0 — Fondations *(S, 1-2 j)*

**But** : câbler les dépendances et le flag, sans exposer aucune route.

**Tâches**
- Ajouter `efe-coordinator`, `efe-multi-llm`, `efe-nano-sdk` aux deps de `efe-swarm-gateway` ; **aligner les versions** `tonic`/`reqwest`/`prost` avec le workspace (risque de conflit réel).
- Introduire le flag `EFE_LLM_FRONTIER_ENABLED` (défaut **false**), sur le patron de `EFE_META_SWARM_ENABLED` (plumbing-only).
- Champ `coordinator: Option<Arc<Coordinator>>` sur `SwarmGatewayState` + builder `with_coordinator()` (calque `with_router_endpoint`, `state.rs:386`).

**Definition of Done**
- [ ] `cargo build` OK avec les nouvelles deps ; pas de conflit de versions.
- [ ] Flag **off** ⟹ aucune route montée, aucun listener, comportement **byte-identique** à aujourd'hui (test de non-montage).
- [ ] `cargo test -p efe-swarm-gateway` (isolé) inchangé.

**Parallélisable** : le squelette DTO de M3 (`Llm*Request`/`LlmMissionResponse`) peut être écrit dès maintenant ; la décision auth **aud `efe-llm`** (M3/#5) peut être tranchée ici.

---

## M1 — Capture observée *(M, 2-4 j)* — ferme #2, #3, #4

**But** : chaque appel LLM produit un `LlmProvenanceRecord` fidèle, avec l'endpoint **observé** (pas inféré).

**Tâches**
- `ProvenanceProvider` : décorateur `Arc<dyn LlmProvider>` scellant chaque `call` en `LlmProvenanceRecord` (blake3 prompt/output, `cost_microcents`, `model_identity`).
- **Correctif #2** — accesseur additif `fn endpoint(&self) -> Option<EndpointIdentity>` sur le trait `LlmProvider` du coordinator ; le décorateur exécute `is_local_endpoint`/`host_of` sur l'URL réelle.
- **Correctif #4** — invariant : endpoint **non-local ⟹ `R4RemoteChannel{keyed:true}`** posé systématiquement (jamais le défaut `LegacyUnspecified`).
- **Correctif #3** — émettre le record **aussi sur le chemin d'erreur** (avant de propager l'`Err`) ; basculer `call_many_ok` → `call_many`.
- `MissionScope` (task-local, miroir de `efe-swarm-sdk/src/n17.rs:53`) + `MissionProvenanceSink` (un `provenance_id` frais par appel).

**Definition of Done**
- [ ] Un appel ⟹ un record avec `R4RemoteChannel{keyed:true}` + hachés corrects.
- [ ] **Test #4** : un record dont l'endpoint est non-local avec stamp `< R4` est rejeté.
- [ ] **Test #3** : un appel en échec produit un record (pas de drop silencieux).
- [ ] **Test model-agnostic** : appel vers un SLM local (`EFE_LOCAL_SLM_URL`) ⟹ barreau **R2** observé (pas R4).
- [ ] `efe-multi-llm` **inchangé** ; changement `efe-coordinator` limité à l'accesseur `endpoint()`.

---

## M2 — Reçu honnête *(L, 5-8 j)* ★ chemin critique — ferme #1

**But** : la faille centrale. Le reçu doit prouver **la complétude de l'egress**, pas seulement l'intégrité de ce qu'il contient.

> Rappel du danger : `verify_run_offline` renvoie `fully_proven=true` sur un ensemble **vide/partiel** (chaîne N17 à 0 record « trivialement intègre »). Un `scoped()` oublié ou des appels droppés ⟹ reçu vide qui se re-prouve ⟹ trou de souveraineté silencieux.

**Tâches**
- **Correctif #1** — `MissionGraphBuilder` construit le graphe **depuis la topologie DÉCLARÉE** de chaque mission :
  - `vote` = N feuilles + 1 nœud réducteur (Borda/Condorcet, sans egress) ;
  - `relay` = chaîne linéaire plan→code→synthèse ;
  - `debate` = (pro, contra) par round + jury parent ;
  - `specialist` = decompose→N sous-tâches→aggregate (I11 ≤0.60, borné 10) ;
  - `cascade` = drafter→verifier→(validator si utilisé).
  - **Chaque nœud déclaré exige un `llm_provenance_id` résolu** → appel manquant ⟹ `Unlinked` ⟹ `all_mode_c_consistent=false` ⟹ `verified=false`.
- Scellement : `SwarmChain::append_all_canonical` → `provenance_root` ; `seal_n17(&llm_records, key)` → `n17_root`.
- `StoredRun::from_mission` + `RunStore::put_stored` — **pas trivial** : étendre le trait `RunStore` (couplé à `&RunOutcome`) + les 3 impls (`InMemory`, `PgRunStore`, `store_pg_memory`).

**Definition of Done**
- [ ] **Test d'omission (le test-gardien)** : dropper un appel ⟹ `verify` renvoie `verified=false` avec `Unlinked` localisé. *(Ce test échoue si le graphe est construit depuis le sink — il valide le fix.)*
- [ ] **Golden crypto** : `provenance_root`/`n17_root` recomputés cross-crate identiques ; roundtrip `seal → verify_run_offline`.
- [ ] **Tamper** : muter un record ⟹ `verified=false`, `first_break` localisé (N17 **et** essaim).
- [ ] Les 5 topologies produisent la bonne arité/arêtes (test par mission).

---

## M3 — Surface servie *(M, 3-5 j)* — ferme #5

**But** : exposer les 5 missions en HTTP, **mode sync uniquement**.

**Tâches**
- `routes/llm.rs` : 5 handlers calqués sur `routes/run.rs:33` (`require_any_role` → `admit()` → gate egress → mission → `store.put_stored`) + alias `GET /v1/llm/verify/:id`.
- **Correctif #5** — auth : **`aud = efe-llm`** (ou extracteur dédié), pas `efe-agent-runtime`.
- **Gate egress** : `req.llms ⊆ allowed_llms(tenant)` (`PolicyRegistry`) ⟹ **403 `EgressDenied`** avant tout fan-out.
- DTO + `LlmApiError` (mapping HTTP : 400 `DebateTooFewLlms`, 403, 429, 502/503) ; extension `N17VerifyView` (`held_strength`).

**Definition of Done**
- [ ] **e2e** : `POST /v1/llm/{les 5}` → `GET verify` ⟹ `verified=true`, roots présents, `held_strength.any_remote=true`, `weakest=R4`.
- [ ] **Souveraineté** : `req.llms` hors `allowed_llms` ⟹ 403, aucun record ; grep `endpoint`/`provider_metadata` ⟹ le credential n'apparaît **nulle part**.
- [ ] **Isolation tenant** : run A, verify B ⟹ 404 ; clé de B ne re-prouve pas A.
- [ ] **Admission** : saturation ⟹ 429+Retry-After.
- [ ] Erreurs missions : `debate` <3 LLM ou rounds>5 ⟹ 400 ; `specialist` borné 10 ; I11 ≤0.60 respecté.

**Hors MVP** : le mode job `?stream=true` (le `DashboardBus` est alimenté par le SDK essaim, pas par les missions — chantier non démontré). Le sync suffit.

---

## M4 — Gate production *(S, 1-2 j)*

**But** : porte go/no-go. Aucun de ces points n'est optionnel.

**Checklist go/no-go**
- [ ] `main.rs` lit le flag + clés (`ANTHROPIC_API_KEY`/`OPENAI_API_KEY`/`GOOGLE_AI_KEY`).
- [ ] **Fail-fast au boot** si `EFE_LLM_FRONTIER_ENABLED` sans `EFE_NANO_AUDIT_HMAC_MASTER` (sinon `test_key()` publique ⟹ **provenance forgeable**).
- [ ] `PgRunStore` (feature `postgres`) en prod ⟹ reçus **durables** (l'`InMemory` les perd au restart, avec re-scellement sous une autre clé).
- [ ] **Guard re-synchronisé** : `provenance-capture/guard.rs` et `efe-swarm-sdk/backend/mod.rs:266` durcis **ensemble** (drift SSRF déjà survenu).
- [ ] **Doc du modèle de confiance** publiée : reçu R4 = *tamper-evident + re-vérifiable par le détenteur de la clé*, **PAS trustless-tiers**. Ne pas sur-vendre.
- [ ] **Non-régression essaim** : `/v1/swarm/*` inchangé, exécuté **isolé** (flakiness workspace connue).

---

## Ce qui est explicitement hors périmètre (différé)

| Élément | Pourquoi différé | Débloque quoi |
|---|---|---|
| SSE mission-level (`?stream=true`) | `DashboardBus` non alimenté par les missions | UX temps-réel |
| Chemin `R3RemoteCosigned` (ed25519) pour le frontière | Aucun provider frontière ne cosigne | reçu **trustless-tiers** sur R4 |
| Unification / activation `efe-llm-router` | Non re-vérifiable offline + non adressable par modèle | gouvernance cost/rate centralisée |
| Retrait du placeholder `audit_chain_hash` de `efe-multi-llm` | Nettoyage, non bloquant | dette technique |

---

## Comment on sait que la Couche 2 est « finie »

1. Les 5 missions sont appelables via `/v1/llm/*` derrière le flag, multi-tenant, avec admission et gate d'egress.
2. Chaque mission produit un reçu (`provenance_root` + `n17_root`) re-vérifiable par `GET verify`, **et un appel omis fait échouer la vérif** (le test-gardien de M2 passe).
3. L'egress externe est enregistré (R4 observé) et gaté ; aucun credential ne fuit dans le bundle ; la clé n'y entre jamais.
4. La formulation publique du reçu est honnête (détenteur-de-clé, pas trustless-tiers) ; `verified` ne prétend jamais à la correction.
5. En prod : master HMAC obligatoire, store Postgres durable, guard re-synchronisé, chemin essaim non régressé.

---

## Risques & parades

| Risque | Parade |
|---|---|
| Conflit de versions deps (tonic/reqwest/prost) tirées dans la gateway | Aligner en M0 ; buffer calendaire |
| Task-local perdu si un provider spawn hors scope | Cadrer le scope autour des `run_*` ; le décorateur capte dans sa frame (robuste au fan-out `FuturesUnordered` même-tâche) |
| Mismatch `id()` providers (« claude » vs `claude_claude-sonnet-4-6`) | Verrouiller les alias au câblage + test |
| Master HMAC absent en prod ⟹ provenance forgeable | Fail-fast au boot (M4) |
| `put_stored`/`from_mission` sous-estimé | Chiffré en M2 (trait + 3 impls), pas « trivial » |
| Sur-vente du reçu à Kovant | Doc modèle de confiance en M4, formulation figée |
