# Couche 2 — M2 : le reçu honnête (architecture, préparation complète)

> Document d'architecture · 2026-07-11 · branche `couche2-m0-frontier` (M0+M1 livrés/poussés)
> Issu d'une reconnaissance chirurgicale (4 agents) + **2 passes adverses** qui ont **corrigé une faille de conception** de la proposition initiale. Prolonge [`COUCHE2-coordination-frontiere-multi-llm-servie-reprovable.md`](./COUCHE2-coordination-frontiere-multi-llm-servie-reprovable.md) et sa [roadmap](./COUCHE2-roadmap.md).

---

## 0. La correction majeure (à lire en premier)

La roadmap M2 promettait : *« un appel déclaré mais absent ⟹ `verified=false` ; un vérifieur offline re-dérive N attendu »*. **La moitié de cette promesse est fausse** — et la critique adverse l'a démontré, preuves à l'appui.

`verify_run_offline` calcule `fully_proven = swarm.ok() && n17_chain.intact && bridge.all_mode_c_consistent()` ([n17_chain.rs:341](crates/efe-swarm-provenance/src/n17_chain.rs)). Il vérifie seulement que les nœuds Mode-C **présents** dans le graphe **résolvent** vers un record N17. **Il ne re-dérive AUCUNE arité, ne compte rien, n'ouvre pas les payloads, et `routes/verify.rs` (gelé) ne reçoit jamais les paramètres de requête.** Committer `N`/`K`/`rounds` dans un nœud rend la déclaration **tamper-evident** (on ne peut plus la muter sans casser la chaîne), **pas enforced** : aucun vérifieur tiers ne recompte les enfants attendus.

**Conclusion — le modèle de confiance de M2 (à tenir sans sur-vente) :**

> La **complétude de l'egress** est une garantie **côté-écriture** (un builder de confiance + la clé tenant), **tamper-evident après scellement** — **PAS** une re-dérivation *trustless* par un tiers.

C'est **exactement** le modèle R4 déjà acté en Couche 2 (*tamper-evident / détenteur-de-clé, pas trustless-tiers*). Ce qui reste **vrai et précieux** : le builder **matérialise** un nœud `Missing` pour tout appel déclaré-mais-absent, et le reçu stocké **re-prouve `verified=false`** — un auditeur détenteur de la clé **voit** l'incomplétude. Ce qui est **faux** : « n'importe quel tiers re-dérive N ». Ne jamais l'écrire ni le pitcher.

---

## 1. Ce que `verify_run_offline` fait — et ne fait PAS

**Fait** (le mécanisme que M2 exploite) :
- Un `AgentCallRecord` **Mode-C** dont le `llm_provenance_id` ne résout vers **rien** ⟹ `classify_link` renvoie `LinkStatus::Missing` ([bridge.rs:148](crates/efe-swarm-provenance/src/bridge.rs), Missing aux l159-160) ⟹ `verify_bridge` compte `broken += 1` (l228) ⟹ `all_mode_c_consistent() = (unlinked==0 && broken==0) = false` (l209) ⟹ `fully_proven = false`.
- **Prouvé** par le test-oracle `offline_verify_catches_a_dangling_link_after_n17_loss` ([n17_chain.rs:1027](crates/efe-swarm-provenance/src/n17_chain.rs)) : un nœud Mode-C lié à `0xDEAD`, `verify_run_offline(&recs, &[], &[], None, key)` ⟹ `swarm.ok()=true`, `fully_proven=false`, `bridge.broken=1`.

**Ne fait PAS :**
- Re-dériver l'arité attendue (ne lit ni `kind`, ni `K`, ni `rounds`, ni `stored.topology`).
- Compter `#liens Mode-C Consistent` vs `#records N17` — chaque nœud résout **indépendamment**, sans consommation ([bridge.rs:209](crates/efe-swarm-provenance/src/bridge.rs)).
- Comparer l'identité provider ou le prompt du record apparié (`classify_link` ne teste que `tenant_id` + `swarm_request_id`).

---

## 2. Les deux invariants du builder (le cœur de M2)

Puisque la détection vit **au build**, `MissionGraphBuilder` doit garantir **deux** invariants — le second est le correctif qui manquait :

### Invariant #1 — anti reçu-vide / sous-déclaré
Le graphe contient **exactement `arité_attendue`** nœuds Mode-C. L'arité vient de la **requête** (scellée) ou d'un **recompute pur**, **jamais** de l'output producteur. **Refuser de sceller** un graphe à 0 nœud Mode-C pour une mission d'arité ≥ 1.

### Invariant #2 — appariement INJECTIF (le correctif décisif)
Chaque `LlmProvenanceRecord` drainé est consommé **au plus une fois**. Un nœud déclaré **sans record libre** ⟹ lien vers un `ProvenanceId` **absent** ⟹ `Missing`. **Assert au build :** `#nœuds Mode-C == arité_attendue` **ET** `#records réellement liés (distincts) == #records drainés`.

> **Pourquoi #2 est indispensable** : sans lui, un reçu **partiel** (K nœuds déclarés, 1 seul record réel) passe `fully_proven=true` si le builder pointe plusieurs nœuds vers le **même** `provenance_id` présent — `verify_bridge` résout chaque nœud indépendamment (aucune injectivité, [bridge.rs:148-172](crates/efe-swarm-provenance/src/bridge.rs)). C'est le trou de souveraineté **qui survit** à la matérialisation naïve des nœuds. Aggravé par l'ambiguïté provider (debate même `pro_llm` sur `rounds`, specialist anti-monoculture, cascade `drafter==verifier`).

---

## 3. Dériver l'arité — la source de vérité par mission (JAMAIS l'output)

L'output d'une mission est **producteur-contrôlé** (forgeable) : `per_llm_scores`, `arguments_per_round.len()`, `assignments.len()` ne sont **pas** des sources d'arité fiables. Sources autorisées : **paramètres de requête** ou **recompute de fonctions pures**.

| Mission | Arité | Source **autorisée** | Structure d'appel (réelle) |
|---|---|---|---|
| **RELAY** | **3** fixe | `{planner,implementer,synthesizer}_llm` | 3 `call_one` séquentiels ([relay.rs:45/92/140](crates/efe-multi-llm/src/workflows/relay.rs)) ; `?` sur erreur ⟹ un relay partiel **n'émet aucun reçu** |
| **VOTE** | **K** fixe | `params.llms.len()` ([vote.rs:57](crates/efe-multi-llm/src/workflows/vote.rs)) | 1 fan-out `call_many_ok(&req.llms)` ; M1 scelle **aussi les échecs** ⟹ K records même si des LLM tombent ; ordre = **complétion** (FuturesUnordered), pas `req.llms` |
| **DEBATE** | **2·rounds + 1** | `params.rounds` (1..=5, [debate.rs:23](crates/efe-multi-llm/src/workflows/debate.rs)) | par round `tokio::join!(pro, contra)` (debate.rs:101) + 1 jury (debate.rs:208) ; **`pro_llm`/`contra_llm` répétés** ⟹ ambiguïté provider |
| **SPECIALIST** | **N** dynamique | `Decomposer::default().decompose(&user_text).len()` **RE-EXÉCUTÉ** (pur, ≤ 10) — [specialist.rs:32](crates/efe-multi-llm/src/workflows/specialist.rs), [decomposer.rs:77-84](crates/efe-multi-llm/src/decompose/decomposer.rs). **PAS** `assignments.len()` (producteur) | `decompose` (pur) → N `call_one` parallèles (`join_all`) → `aggregate` ; anti-monoculture route plusieurs sous-tâches vers un même LLM ⟹ ambiguïté provider |
| **CASCADE** | **2 ou 3** | drafter+verifier **toujours** ; validator = **indétectable** (voir §9) | drafter (cascade.rs:49) + verifier (cascade.rs:79) toujours ; validator **SSI** `verifier_decision=='REGEN_ALL'` (cascade.rs:110) ; décision = `jaccard(draft,verifier) ≥ threshold` |

**Règle d'or du builder** : dériver l'arité de la **requête** ou d'un **recompute pur** ; **interdire** toute lecture d'arité depuis l'output.

---

## 4. Le graphe : nœuds Mode-C (egress) + nœuds Mode-A (déclaration/réduction)

- **1 nœud Mode-C par appel LLM déclaré** : `AgentCallDraft::new(SwarmId::from_uuid(mission_id), scope.tenant_id, agent_id, agent_role, AgentKind::ModeC, input_hash, output_hash)` ([record.rs:227](crates/efe-swarm-provenance/src/record.rs)) `.with_llm_provenance(record.provenance_id)` (record.rs:298). Un appel **omis** ⟹ `.with_llm_provenance(ProvenanceId::new())` (id frais, **absent** du puits) ⟹ `Missing`.
  - **Cohérence pont obligatoire** : `swarm_id = SwarmId::from_uuid(mission_id)` et `tenant_id = scope.tenant_id` — **identiques** à ce que M1 pose sur les `LlmProvenanceRecord` (`swarm_request_id = mission_id`, `tenant_id = scope.tenant_id`, mission.rs). Sinon `classify_link` renvoie `SwarmMismatch`/`TenantMismatch` sur un run **honnête** ([bridge.rs:162-166](crates/efe-swarm-provenance/src/bridge.rs)).
- **Nœuds Mode-A** (déterministes, **pas** de lien LLM) :
  - **Nœud racine « requête/dispatch »** : `output_hash` committe `(kind, K, rounds, hash(user_text), decompose_mode, liste ORDONNÉE des providers/assignations)`. Rend la déclaration **tamper-evident** et alimente le checker out-of-band (§7).
  - **Nœuds réducteurs** : vote-tally (Plurality/Borda/Condorcet, vote.rs:116/137/159), specialist-`aggregate`, relay-chaînage — réduction **pure** ⟹ Mode-A committant entrées→sortie. ⚠️ **Le jury de debate est un `call_one` LLM** ⟹ nœud **Mode-C**, pas Mode-A.
- **Arêtes** (`caused_by(parent_record_id)`, [record.rs:294](crates/efe-swarm-provenance/src/record.rs)) = dataflow **réel** : vote = K feuilles → tally ; relay = plan→code→synthèse ; debate = round r rebutte r-1, jury dépend du dernier pro+contra ; specialist = dispatch → N sous-tâches → aggregate ; cascade = drafter → verifier → (validator).

---

## 5. L'appariement injectif — clé de jointure

Le pont ne compare que `tenant_id`+`swarm_request_id` ; il **ne re-dérive pas** le contenu du prompt (les prompts aval embarquent des sorties LLM intermédiaires, et le puits ne garde que `output_hash` **one-way**, [llm_provenance.rs:833-836](crates/efe-nano-sdk/src/types/llm_provenance.rs)). Donc l'appariement se fait par **présence + consommation unique**, pas par re-dérivation cryptographique :

1. Le builder itère sur les positions **déclarées** (dans l'ordre de la topologie : les K providers de vote, les 3 phases de relay, les 2·rounds+1 de debate, les N sous-tâches de specialist).
2. Pour chaque position, il **consomme** un record libre du puits qui correspond (par `model_identity.provider` quand distinct ; sinon le prochain record libre du bon provider). Record consommé ⟹ marqué, **jamais réutilisé** (Invariant #2).
3. Position sans record libre ⟹ nœud lié à un `ProvenanceId` **absent** ⟹ `Missing`.
4. Assert final : tous les records drainés ont été consommés **exactement** une fois ; `#nœuds == arité`.

> Ne **jamais** apparier par **index positionnel** : l'ordre du puits est la **complétion** (FuturesUnordered), pas l'ordre déclaré.

---

## 6. Scellement & persistance

```
SwarmChain::from_key_source(tenant, SwarmId::from_uuid(mission_id), &state.keys)   // chain.rs:46
    .append_all_canonical(drafts) -> Vec<AgentCallRecord>                          // chain.rs:111 (ordre ADR-007)
    .root_hex()  =>  provenance_root                                               // chain.rs:139
seal_n17(&llm_records, &state.derive_key(tenant)) -> SealedN17                      // n17_chain.rs:90
    .leaves_hex() / .root_hex()  =>  n17_leaves / n17_root
```
- `StoredRun::from_mission(tenant, records, provenance_root, llm_records, n17_leaves, n17_root, output, topology, request)` — **struct-literal direct** (pas de `RunOutcome`/`SwarmRun`, absents côté mission ; le patron est [store.rs:46 `from_outcome`](crates/efe-swarm-gateway/src/store.rs) + le helper de test `test_support::synthetic_mode_c_run`).
- **Nouveau champ `StoredRun.request`** : `{kind, K, rounds, user_text_hash, decompose_mode}` — sans lui, **aucune** re-dérivation d'arité n'est possible sur **aucun** chemin (verify.rs ne reçoit pas la requête). Il alimente le checker out-of-band (§7).
- `RunStore::put_stored(&self, id: Uuid, tenant: &str, run: StoredRun)` — **ajouté au trait** ([store.rs:109](crates/efe-swarm-gateway/src/store.rs)) **+ 2 impls seulement** : `InMemoryRunStore` et `PgRunStore` (feature `postgres`). **PAS** `store_pg_memory` — c'est un `PgMemoryStore : MemoryStore` (orthogonal, la roadmap se trompait en disant « 3 impls »).
- **Clé/tenant** : sceller avec `state.derive_key(caller.tenant)` — la **même** que `GET verify` recalcule ([verify.rs:28](crates/efe-swarm-gateway/src/routes/verify.rs)) ; `scope.tenant_id` (posé par M3) doit égaler `state.tenant_id(caller.tenant)` ; `stored.tenant` = la `String` qui re-dérive ce `TenantId` (aligner sur `synthetic_mode_c_run` = `TenantId::system().to_string()`).

---

## 7. Re-preuve

- **`GET /v1/swarm/verify/:id` (ou `/v1/llm/verify` en M3) — INCHANGÉ** → `verify_run_offline(records, llm_records, leaves, root, key)` → `fully_proven`. Un appel déclaré **omis** ⟹ `Missing` ⟹ `false`. Un **tamper** ⟹ `RecordContentMutated` ⟹ `false`, `first_break` localisé (N17 **et** essaim).
- **Checker OUT-OF-BAND (optionnel — le SEUL chemin d'audit tierce de l'arité)** : une **nouvelle** route (jamais `verify.rs`) qui relit `stored.request` (scellé dans le nœud racine), **re-calcule** l'arité (K/rounds/`Decomposer::decompose` pur) et **compte** les nœuds Mode-C ⟹ signale une sous-déclaration. C'est ce qui transforme la garantie *write-side* en *auditable par un tiers* — à faire en M3+ si le pitch l'exige.

---

## 8. Golden-crypto = per-run (pas de reproductibilité)

La feuille N17 sérialise le **record entier**, dont `recorded_at: SystemTime` et `trace_id: Uuid` v4 aléatoire ⟹ `n17_root` est une **empreinte par-run**, pas un vecteur golden universel ([n17_chain.rs:52-56](crates/efe-swarm-provenance/src/n17_chain.rs), doc l30-33 « tamper-evidence, pas une racine universelle »). Donc :
- `from_mission` **scelle ET persiste le MÊME `Vec`** (une seule frappe de `recorded_at`/`trace_id`) ; `n17_leaves = sealed.leaves_hex()`, `n17_root = sealed.root_hex()` (jamais des bytes bruts).
- Tests M2 = **sceller-puis-vérifier le MÊME objet** (round-trip), **jamais** d'assert sur un hex root fixe.

---

## 9. Tableau d'honnêteté par mission (ce que M2 garantit VRAIMENT)

| Mission | Omission détectable **par le détenteur de clé** (au build) | Auditable par un tiers | Réserve |
|---|---|---|---|
| **RELAY** | ✅ **oui** — arité fixe 3, providers distincts ; relay partiel ⟹ pas de reçu | via checker out-of-band | **cas de référence** (borne haute) |
| **VOTE** | ✅ oui (Invariant #2) — K = `params.llms.len()` | via checker out-of-band | ordre = complétion ; apparier par provider+consommation |
| **DEBATE** | ✅ oui (Invariant #2) — 2·rounds+1 (param scellé) | via checker out-of-band | `pro/contra` répétés ⟹ injectivité **critique** |
| **SPECIALIST** | ✅ oui (Invariant #2) — N = `decompose` **pur** re-exécuté | via checker out-of-band | ⚠️ dépend de `Decomposer` **pur** ⟹ test de régression + commit du mode |
| **CASCADE** | ◑ **oui** via Tier A+B (§9bis) — drafter+verifier : oui | via digest word-set (Tier B) | résidu étroit : REGEN_ALL vs REGEN_RANGES sans token REGEN_ALL |

### 9bis. CASCADE — la solution (ce n'est PAS un dead-end)

Le « dead-end » ne tenait qu'à **une hypothèse trop forte** : que la décision dépende du **texte** (one-way). En réalité elle dépend d'un **ensemble de mots** — et un ensemble de mots est **committable sans plaintext**.

**La règle réelle** ([cascade.rs:97-110](crates/efe-multi-llm/src/workflows/cascade.rs)) : `sim = jaccard_words(draft, verifier)` — un **jaccard d'ensembles de mots** (`split_whitespace()`, sensible à la casse, [cascade.rs:168](crates/efe-multi-llm/src/workflows/cascade.rs)) ; `validator appelé ⟺ (sim < accept_threshold) ET (verifier.to_uppercase().contains("REGEN_ALL"))`.

**Tier A — topologie à 3 slots FIXES + nœud décision scellé (M2, coût nul, aucune contrainte violée).** Cascade déclare **toujours 3 slots**. Le slot 3 est **soit** le nœud validator Mode-C (si appelé), **soit** un nœud Mode-A « skip-proof » committant `(output_hash(draft), output_hash(verifier), verifier_decision, accept_threshold)` + la règle `slot3=validator ⟺ decision==REGEN_ALL`. Effets : (1) l'arité redevient **fixe (3 slots)** ⟹ un slot 3 **totalement absent** est détecté par l'invariant d'arité, comme les missions fixes ; (2) la présence du validator est **structurellement liée** à la décision scellée et **tamper-evident** ; (3) un checker out-of-band vérifie la **cohérence structurelle** (type du slot 3 ⟺ décision scellée) **sans plaintext**. Résidu : la *valeur* de `decision` reste asserée par le builder (confiance write-side — comme tout M2). Keyer l'attente du validator sur `verifier_decision=='REGEN_ALL'`, **pas** sur `validator_used` (un validator appelé-mais-échoué a `validator_used=false` alors qu'un record **existe**, cascade.rs:140).

**Tier B — digest d'ensemble de mots (rend la décision RE-DÉRIVABLE ; générique, modeste).** Le décorateur M1 committe, pour **chaque** appel, le digest `{ blake3(token) : token ∈ output.split_whitespace() }` (trié) dans `provider_metadata["wordset_blake3"]` ([llm_provenance.rs:853](crates/efe-nano-sdk/src/types/llm_provenance.rs), `HashMap<String, serde_json::Value>`, **scellé dans la feuille N17**). Un auditeur re-calcule alors le **jaccard EXACT** `|A∩B|/|A∪B|` sur les hash-sets (= `jaccard_words` exact : même tokenisation, blake3 injectif sur mots distincts) et vérifie `sim ≥ threshold ⟺ ACCEPT`. **Cela attrape complètement le mensonge dominant** : « décision=ACCEPT (donc pas de validator) alors que draft et verifier divergent (sim < threshold) ». **Générique** (le décorateur ne connaît pas cascade — il committe un digest pour *tout* appel), **aucune** modif d'`efe-multi-llm` ni de `verify.rs`. Coût : +32 o/mot distinct dans le record ; fuite d'appartenance de mots (dictionnaire sur mots courants) — acceptable pour une donnée **scellée sous la clé tenant** ; une variante **MinHash** réduit la fuite au prix d'une approximation près du seuil.

**Résidu honnête (étroit et caractérisé).** Reste le sous-branchement `REGEN_ALL` vs `REGEN_RANGES` (tous deux `sim < threshold`, tous deux sans/avec validator) : il dépend d'un test de **sous-chaîne** `to_uppercase().contains("REGEN_ALL")`. Le digest d'ensemble de mots teste l'**appartenance du token** « REGEN_ALL » (cas réaliste : le verifier émet REGEN_ALL comme token de contrôle) mais pas une sous-chaîne enfouie ni une casse arbitraire. Fermeture complète = committer en plus un petit **digest de q-grammes** du texte majuscule (test de sous-chaîne exact) — sinon on retombe, pour ce seul sous-cas, sur la décision scellée du Tier A (tamper-evident, write-side). Dans **tous** les cas, l'auditeur **sait** (jaccard re-dérivé) que `sim < threshold` ⟹ « une régénération a eu lieu, un validator a **pu** être dû » : le résidu est borné à « REGEN_ALL vs REGEN_RANGES quand le verifier n'émet pas REGEN_ALL comme token », **pas** à « le validator a disparu sans trace ».

**Bilan.** Tier A (M2) sort cascade du statut « pire que les autres » (arité fixe, tamper-evident, cohérence structurelle auditable). Tier B (M2 ou M3) rend la décision **re-dérivable** et attrape le mensonge dominant ⟹ cascade atteint la **parité** avec vote/debate/specialist. Ce n'est pas un dead-end — c'est un résidu étroit, documenté.

---

## 10. Plan d'implémentation

- **`gateway/src/mission.rs`** (étend le M1) : `MissionKind`, `MissionRequestSummary {kind, k, rounds, user_text_hash, decompose_mode}`, `MissionGraphBuilder::build(kind, request, output, drained_records) -> Result<Vec<AgentCallRecord>, BuildError>` appliquant **les 2 invariants** (arité depuis requête/recompute pur ; appariement injectif ; nœud racine Mode-A ; réducteurs Mode-A ; arêtes `caused_by`). Le scellement (`SwarmChain` + `seal_n17`).
- **`gateway/src/store.rs`** : `StoredRun` + champ `request` ; `StoredRun::from_mission(...)` ; `RunStore::put_stored` au **trait** + impl `InMemoryRunStore`.
- **`gateway/src/store_pg.rs`** : `put_stored` (feature `postgres`, colonne `request` JSONB).
- **M3** branchera les 5 handlers (`scoped(scope, run_*) → drain → build → seal → put_stored`) ; **M2 fournit build+seal+store, testés en isolation** (sans routes).

### Tests M2 (la DoD, révisée par l'analyse adverse)
1. **Test-gardien d'omission** (par mission fixe) : dropper 1 appel déclaré ⟹ `build` émet un nœud `Missing` ⟹ `verify_run_offline` ⟹ `fully_proven=false`, `bridge.broken≥1`.
2. **Test d'injectivité** (le correctif #2) : K=3 nœuds déclarés, **1 seul** record réel ⟹ **2 nœuds Missing** (pas 3 Consistent via réutilisation d'id) ⟹ `false`.
3. **Anti reçu-vide** : mission d'arité ≥1 sans record ⟹ `build` **refuse** (ou émet arité nœuds Missing) ⟹ jamais `fully_proven=true` sur un bundle vide.
4. **Tamper** : muter un record stocké ⟹ `verified=false`, `first_break` localisé.
5. **Round-trip par mission** (5) : `build → seal → from_mission → verify_run_offline` ⟹ `fully_proven=true` sur un run complet (seal+verify **le même objet**, pas d'hex fixe — golden per-run).
6. **Specialist régression** : un test qui **échoue** si `Decomposer` cesse d'être pur (egress) — l'enforcement d'arité en dépend.
7. **Cascade — test NÉGATIF documentant la limite** : `verifier_decision=='ACCEPT'` sans validator ⟹ `fully_proven=true` (attendu), **avec un commentaire** actant l'indétectabilité de l'omission du validator.

---

## 11. Ce qu'on NE promet PAS (encadré d'honnêteté — mis à jour par l'analyse adverse)

- **Pas de re-dérivation d'arité trustless par un tiers.** `verify.rs` gelé ne re-dérive rien. La complétude est **write-side, tamper-evident, détenteur-de-clé** (le modèle R4 de Couche 2). Ne jamais pitcher « un tiers re-dérive N ».
- **CASCADE** : **pas** un dead-end (§9bis) — Tier A (3 slots fixes + décision scellée, tamper-evident) + Tier B (digest d'ensemble de mots ⟹ jaccard EXACT re-dérivable ⟹ le mensonge « ACCEPT pour sauter le validator » est attrapé). Résidu étroit : REGEN_ALL vs REGEN_RANGES quand le verifier n'émet pas « REGEN_ALL » comme token.
- **SPECIALIST** : l'enforcement d'arité dépend de `Decomposer` **pur** (régression testée) ; si un mode LLM-backed fait un jour un vrai egress, l'enforcement **s'évapore** — à committer explicitement.
- **Golden per-run** : le reçu prouve la tamper-evidence d'**un** run (`uuid`/`recorded_at`), **pas** « même mission → même racine ».
- **Intégrité ≠ correction** ; **jamais « small beats big »**.

---

## 12. Ce que l'analyse adverse a changé (traçabilité)

| Proposition initiale (roadmap M2) | Réalité (preuve) | Correctif intégré |
|---|---|---|
| « un vérifieur offline re-dérive N attendu » | `verify_run_offline` ne re-dérive aucune arité ([n17_chain.rs:341](crates/efe-swarm-provenance/src/n17_chain.rs)) | §0 reframe write-side + §7 checker out-of-band |
| « matérialiser le nœud manquant suffit » | appariement **non-injectif** : K nœuds → 1 record réutilisé = tous Consistent ([bridge.rs:148](crates/efe-swarm-provenance/src/bridge.rs)) | §2 **Invariant #2** (consommation unique) |
| arité depuis l'output (`assignments.len`, `per_llm_scores`) | output **producteur-contrôlé**, forgeable | §3 arité depuis requête **ou** recompute pur |
| cascade = `2 ou 3` détectable | décision = jaccard d'**ensembles de mots** (pas les textes) ⟹ re-dérivable via digest scellé | §9bis **solution Tier A+B** (pas un dead-end) |
| « 3 impls RunStore » | `store_pg_memory` = `PgMemoryStore : MemoryStore` (orthogonal) | §6 **2 impls** (InMemory, Pg) |

*Préparation M2 : le reçu honnête n'est pas « trustless », il est tamper-evident write-side — et c'est déjà un moat réel, à condition de le dire juste.*
