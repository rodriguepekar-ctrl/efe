# EFE — Phase 4 : Provenance & confiance **par-agent** — le *moat*

> **Statut** : Architecture (gelée pour build) · **Date** : 2026-06-09 · **Durée** : ≈3-4 semaines · **Maturité** : 80 % → 95 %
> **Thème** : un enregistrement **signé/hashé par appel d'agent** dans l'essaim → piste d'audit complète, **rejouable**, **vérifiable par un tiers indépendant**.
> **Crate cible (neuf)** : `crates/efe-swarm-provenance`
> **ADR associés** : ADR-SWARM-006 (per-agent provenance = moat) · ADR-SWARM-007 (rejeu déterministe sur la *version*, pas l'horloge) · réutilise N17 (`efe-llm-provenance`) + audit HMAC (`efe-nano::audit`).

---

## 0. Thèse — pourquoi *maintenant*, et pourquoi c'est *le* moat

L'essaim EFE génère du trafic en **N agents minuscules en parallèle** (la vision produit). Chaque agent « dit quelque chose à partir de quelque chose ». La question que **aucun provider ne peut résoudre pour vous** :

> *« Quel agent a dit quoi, quand, à partir de quoi — et puis-je le **re-prouver** sans faire confiance à celui qui l'a produit ? »*

Un provider vous donne **sa parole** (sa facture, ses logs, sa version de modèle *déclarée*). La provenance par-agent donne une **preuve cryptographique reconstructible par un vérificateur indépendant**, sans coopération du provider. C'est :

- **Ce qui ne s'érode pas** quand le prix des tokens tombe : la valeur n'est pas le calcul, c'est **la traçabilité prouvable**.
- **Non-réplicable par un provider mono-modèle** : il ne voit qu'**un** agent ; nous voyons **l'essaim entier** et le **DAG causal** entre agents.
- **Le verrou de conformité/entreprise** : audit réglementaire, attribution de responsabilité, reproductibilité scientifique, litiges.

**Le moat se résume en une phrase** : *EFE est un vérificateur indépendant de ce que l'essaim a fait.*

---

## 1. Le réel — l'inventaire exact (ce sur quoi on **construit**, sans réinventer)

> Discipline EFE : on n'écrit pas une primitive qui existe déjà ailleurs dans le monorepo. Voici ce qui est **déjà construit et prouvé**, avec les types exacts.

### 1.1 `efe-llm-provenance` (N17) — provenance **par appel LLM** (BLAKE3)
- `PreCallProvenance` / `PostCallProvenance` → `LlmProvenanceRecord` (défini dans `efe_nano_sdk`).
- **4 hashs canoniques BLAKE3-256** par appel : `prompt_inputs_hash`, `system_prompt_hash`, `user_prompt_hash`, `final_prompt_hash` ; plus `output_hash: Option<Blake3Hash>`.
- **Write-once** (immutabilité I10) : `InMemoryLlmProvenanceStore::insert()` rejette un `trace_id` dupliqué (atomique).
- **Liens de lignage** : `replay_of: Option<ProvenanceId>`, `cache_hit: Option<CacheProvenance>`.
- `hash_bytes()` → `blake3::hash()` déterministe (test `hash_bytes_is_deterministic`).
- **Granularité** : un **appel LLM** (Mode C). **Limite** : les agents **Mode A** (Rust pur) n'émettent **rien** ; pas de **chaînage** entre enregistrements ; pas de **signature**.

### 1.2 `efe-nano::audit` — la **chaîne HMAC** tamper-evident (déjà complète)
C'est le **joyau** à réutiliser. Types exacts :
```rust
// efe-nano/src/audit/chain_store.rs
pub struct ChainEntry {
    pub trace: NanoAgentTrace,
    pub chain_parent_id: Option<Uuid>,   // chaînage au précédent
    pub payload_hash: [u8; 32],          // SHA-256 du corps canonique (tri-clés)
    pub hmac: [u8; 32],                  // HMAC-SHA256(parent ‖ payload_hash, clé)
    pub inserted_at: DateTime<Utc>,
}
// efe-nano/src/audit/payload.rs
pub fn compute_payload_hash(/* trace */) -> [u8;32];          // JSON canonique, clés triées
pub fn compute_hmac(parent: Option<Uuid>, payload_hash: &[u8;32], key: &[u8]) -> [u8;32];
// efe-nano/src/audit/hmac_chain.rs
pub struct HmacChainAuditWriter { /* tips + keys par tenant */ }
// efe-nano/src/audit/verify.rs
pub async fn verify_chain(store, key_source, tenant, since) -> Result<ChainIntegrityReport, _>;
```
- Genèse : `chain_parent` absent ⇒ littéral `b"genesis"`.
- **Toute mutation d'une entrée casse tous les HMAC en aval** (tamper-evidence).
- **Clés par tenant** via HKDF :
```rust
// efe-nano/src/audit/hkdf.rs
pub enum HmacKeySource {
    EnvHkdf,                       // lit EFE_NANO_AUDIT_HMAC_MASTER (≥32o)
    StaticMaster(Vec<u8>),         // tests UNIQUEMENT
    VaultTransit { path: String }, // « Phase 4 placeholder » (NotImplemented) ← NOUS
}
// salt = UUID tenant (16o), info = b"efe-nano/audit/v1", SHA-256, sortie 32o
pub fn derive(&self, tenant: TenantId) -> Result<Vec<u8>, AuditChainError>;
```
> **Conséquence majeure** : la gestion de clés **existe déjà**, server-side, et `VaultTransit` est **explicitement réservé à Phase 4**. **Je ne manipule jamais de clé** : le master vient de l'environnement serveur ; le harness utilise `StaticMaster` (tests).

### 1.3 `efe-nano::replay` — rejeu déterministe **par swarm** (déjà là)
```rust
pub struct ReplayEngine { chain_store, key_source, dispatcher, catalog }
impl ReplayEngine {
    pub async fn envelope_for_swarm(&self, tenant, request_id) -> Result<SwarmEnvelopePayload, _>;
}
```
Pipeline : `traces_for_swarm` → `verify_subchain` (HMAC) → `SwarmReconstructor::reconstruct` → `dispatcher.dispatch` → `DeterminismChecker` (mismatch d'`output_hash`) → `ReplayReport`. **Échecs** : `NotFound`, `ChainIntegrityBroken`, `MissingFullInput`, dispatch error, output mismatch.

### 1.4 `efe-server` — chaîne d'audit Postgres + rotation de clés
- `catalog/audit_hmac.rs` : chaîne per-tenant (`audit_events`, `pg_advisory_xact_lock`, `prev_hmac` 64-zéros en genèse).
- `catalog/audit_keys.rs` : `HmacKey` (zéro-on-drop via `write_volatile`), fichier de rotation `EFE_AUDIT_SIGNING_KEYS_FILE` (1ʳᵉ ligne = clé active, suivantes = vérification pendant rotation), `key_id_of` = `hex(SHA-256(key))[..8]`.

### 1.5 Le substrat `efe-blackboard` — les **seams** d'instrumentation
```rust
// blackboard.rs — l'agent ET l'horloge sont DÉJÀ capturés à chaque write
pub struct BlackboardValue { pub value: Value, pub version: u64,
                             pub written_by: AgentId, pub written_at: SystemTime }
pub struct BlackboardWrite { pub pointer: String, pub version: u64,
                             pub agent_id: AgentId, pub at: SystemTime, pub bytes: usize }
pub fn write(&self, pointer: &str, value: Value, agent_id: AgentId) -> Result<u64, _>;     // version++ atomique
pub fn write_if(&self, pointer, expected_version, value, agent_id) -> Result<u64, _>;       // CAS
// sink.rs — LE seam passif : callback SYNCHRONE à CHAQUE write
pub trait WriteSink: Send + Sync + 'static { fn on_write(&self, write: &BlackboardWrite); fn flush(&self){} }
// eventbus.rs
pub struct SwarmEvent { pub swarm_id: SwarmId, pub published_by: AgentId,
                        pub topic: String, pub payload: Value, pub published_at: SystemTime }
```
- **`version` est l'ordre déterministe atomique** (clé de tri) ; `at`/`published_at` = `SystemTime::now()` direct ⇒ **non déterministe** (le gap §3.5).
- Traits d'agent (les frontières d'appel) — `efe-swarm-topology/src/traits.rs` + `consensus/mod.rs` :
```rust
async fn generate(&self, task: &Value, variant: usize) -> Value;     // Generator
async fn process(&self, input: &Value) -> Vec<Value>;                // Stage
async fn decompose(&self, task: &Value) -> Vec<Value>;               // Decomposer
async fn solve(&self, subtask: &Value) -> Value;                     // Solver
async fn merge(&self, task: &Value, results: Vec<Value>) -> Value;   // Merger
async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64>; // Judge
fn score(&self, task: &Value, candidate: &Value) -> f64;             // Verifier (sync)
```
- Sites d'appel `ConsensusTopology` : génération `g.generate(..)` → `bb.write("/cand/{i}", .., "generator")` (l.119) ; juges `judge.assess(..)` → `bb.write("/ballot/{j}", ..)` (l.157) + `router.publish(SwarmEvent{..})` (l.158).

**Bilan** : on a (a) le hash BLAKE3 par appel-LLM, (b) la chaîne HMAC + HKDF + rotation + zéro-on-drop, (c) le rejeu par-swarm, (d) les seams substrat qui capturent **déjà** agent+version. **Le manque est l'atome « appel d'agent » de l'essaim et son chaînage par-swarm.**

---

## 2. L'écart — *gap analysis* chirurgical (ce que Phase 4 ajoute, et **rien de plus**)

| # | Manque | Conséquence | Comblé par |
|---|--------|-------------|------------|
| G1 | **Aucune trace par-agent intra-swarm** (Mode A muet) | l'essaim — majoritairement Mode A — est **invisible** à l'audit | `AgentCallRecord` + capture aux 2 seams (§3.4) |
| G2 | **Pas de chaîne couvrant le DAG d'un swarm** | impossible de prouver l'intégrité d'**un run complet** | chaîne HMAC **par-swarm** réutilisant `compute_hmac` (§3.3) |
| G3 | **Pas de signature/hash dans les crates d'essaim** | greenfield | `efe-swarm-provenance` (canon + BLAKE3) (§3.2) |
| G4 | **`SystemTime::now()` ⇒ rejeu non déterministe** | le rejeu ne peut pas être bit-exact | clé sur la **`version`** ; `wall_at` advisory, **hors hash** (§3.5, ADR-007) |
| G5 | **Lien Mode A ↔ Mode C absent** | un juge LLM et un générateur Rust ne sont pas dans la même piste | `llm_provenance_id: Option<ProvenanceId>` dans le record (pont N17) |
| G6 | **Pas de vérificateur autonome** | le moat (« tiers indépendant ») n'est pas matérialisé | `verify_swarm` sans coopération provider (§3.6) |

> **Hors-scope explicite** (discipline anti-dette) : pas de base de données nouvelle (réutilise le store in-memory + le pattern PG existant), pas de signatures asymétriques en v1 (HMAC suffit pour la tamper-evidence ; ed25519 = upgrade gaté §8), pas d'ancrage externe (timestamping/blockchain = roadmap §8).

---

## 3. Le design — crate `efe-swarm-provenance`

### 3.0 Principe directeur
> **Capturer sans réécrire les agents.** L'instrumentation est **externe** (décorateur + sink). Un agent reste un `Generator`/`Judge` ignorant de la provenance. La provenance est une **couche transverse**, activable/désactivable, à **coût mesuré** (gate §7).

### 3.1 L'atome — `AgentCallRecord` (« qui a dit quoi, quand, à partir de quoi »)
```rust
/// Un appel d'agent dans l'essaim, hashé et chaîné. Tous les champs servant à
/// la VÉRITÉ sont dans `payload_hash` ; `wall_at` est ADVISORY (hors hash, ADR-007).
pub struct AgentCallRecord {
    // ── identité (QUI) ───────────────────────────────────────────────
    pub record_id: Uuid,                 // identité de CE record (frais)
    pub swarm_id: SwarmId,               // run d'essaim (déjà dans le substrat)
    pub tenant_id: TenantId,             // isolation cryptographique (clé HKDF)
    pub agent_id: AgentId,               // l'agent (validé 3..64, [a-z0-9_-])
    pub agent_role: AgentRole,           // Generator|Verifier|Stage|Decomposer|Merger|Solver|Judge
    pub agent_kind: AgentKind,           // ModeA (pur Rust) | ModeC (LLM)
    // ── causalité (À PARTIR DE QUOI) ─────────────────────────────────
    pub input_hash: Blake3Hash,          // BLAKE3 de l'entrée canonique (task/inputs)
    pub reads: Vec<CellRef>,             // cellules lues : {pointer, version} (DAG causal)
    pub chain_parent_hmac: Option<[u8;32]>, // chaînage tamper-evidence (tip précédent)
    pub causal_parents: Vec<Uuid>,       // record_ids des producteurs des cellules lues
    // ── effet (QUOI) ─────────────────────────────────────────────────
    pub output_hash: Blake3Hash,         // BLAKE3 de la sortie canonique
    pub writes: Vec<CellRef>,            // cellules écrites : {pointer, version}
    pub variant: Option<u64>,            // pour Generator(variant) — clé de reproductibilité
    // ── ordre (QUAND) ────────────────────────────────────────────────
    pub seq: u64,                        // ordre LOGIQUE déterministe (monotone/swarm)
    pub wall_at: SystemTime,             // ADVISORY (hors payload_hash)
    // ── ponts ────────────────────────────────────────────────────────
    pub llm_provenance_id: Option<ProvenanceId>, // lien N17 si Mode C (G5)
    // ── intégrité (calculés) ─────────────────────────────────────────
    pub payload_hash: [u8;32],           // SHA-256 du corps canonique (tout sauf wall_at/hmac)
    pub hmac: [u8;32],                   // HMAC-SHA256(parent_hmac ‖ payload_hash, clé tenant)
}

pub struct CellRef { pub pointer: String, pub version: u64 }
pub enum AgentRole { Generator, Verifier, Stage, Decomposer, Merger, Solver, Judge }
pub enum AgentKind { ModeA, ModeC }
```
**Lecture** : `(agent_id, agent_role)` = *qui* ; `output_hash`+`writes` = *quoi* ; `seq`/`version` = *quand* (déterministe) ; `input_hash`+`reads`+`causal_parents` = *à partir de quoi*. Le triple `(payload_hash, hmac, chain_parent_hmac)` = *re-prouvable*.

### 3.2 Encodage canonique + hash (réutilise la doctrine `efe-nano::payload`)
- `canon.rs` : sérialisation JSON **déterministe** — **clés triées**, formatage stable des nombres/UUID/`SystemTime` (RFC 3339), **exclusion** des champs advisory (`wall_at`, `hmac`). Identique à `compute_payload_hash` d'`efe-nano` (pour que les chaînes soient **cross-vérifiables**).
- `hash.rs` : `blake3_hash(bytes) -> Blake3Hash` (réutilise `efe_nano_sdk::Blake3Hash`). `payload_hash` = `Sha256(canon(record_sans_advisory))` (aligné HMAC-SHA256 d'`efe-nano`).
> **Décision d'archi (factorisation)** : extraire `compute_payload_hash`/`compute_hmac`/`HmacKeySource`/HKDF dans un crate partagé **`efe-audit-core`** (déplacement depuis `efe-nano::audit`, ré-export pour compat). Évite la duplication et **garantit l'identité bit-à-bit** des deux chaînes. *Alternative bas-risque si refactor trop large* : `efe-swarm-provenance` **dépend de `efe-nano`** et appelle ses fonctions `pub`. Recommandation : `efe-audit-core` (propreté ; un seul endroit où vit la crypto).

### 3.3 La chaîne — DAG causal **+** chaîne HMAC linéaire **par-swarm**
Deux structures **complémentaires** :
- **(a) DAG causal** (sémantique) : `reads`/`causal_parents` relient chaque record aux producteurs de ses entrées. Reconstruit « le générateur G3 a produit `/cand/3` que le juge J2 a lu pour son ballot ».
- **(b) Chaîne HMAC linéaire** (tamper-evidence) : ordonnée par `seq`, **genèse par `swarm_id`** (`chain_parent_hmac = None` ⇒ `b"genesis"`), `hmac = compute_hmac(parent, payload_hash, key_tenant)`. **Toute mutation/suppression/réordonnancement casse la chaîne.**
- **Le tip de chaîne = un engagement unique** vers tout le run : `TopologyOutput` gagne un `provenance_root: Option<[u8;32]>` (le dernier `hmac`) — **un seul hash prouve l'essaim entier**.

```rust
pub struct SwarmChain { tenant: TenantId, swarm_id: SwarmId, tip: Option<[u8;32]>, next_seq: u64 }
impl SwarmChain {
    pub fn append(&mut self, partial: AgentCallDraft, key: &[u8]) -> AgentCallRecord; // calcule seq, payload_hash, hmac, met à jour tip
    pub fn root(&self) -> Option<[u8;32]>;     // engagement courant
}
```

### 3.4 La capture — **2 seams, zéro réécriture d'agent**
- **Seam B (actif, niveau appel) — `ProvenancedAgent<A>` (décorateur)** : enveloppe n'importe quel trait d'agent. Avant l'appel : `input_hash = hash(canon(task))` + snapshot des `reads`. Après : `output_hash = hash(canon(out))`. Émet un `AgentCallDraft`. **C'est le seul endroit qui voit le lien entrée→sortie.**
```rust
pub struct ProvenancedAgent<A> { inner: A, role: AgentRole, kind: AgentKind, rec: Arc<ProvenanceRecorder> }
#[async_trait] impl<G: Generator> Generator for ProvenancedAgent<G> {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let input_hash = self.rec.hash_input(task);
        let out = self.inner.generate(task, variant).await;
        self.rec.capture(/* agent_id, role, kind, input_hash, variant, */ &out);
        out
    }
} // idem Judge::assess, Stage::process, Solver::solve, Decomposer::decompose, Merger::merge
```
- **Seam A (passif, substrat) — `ProvenanceSink: WriteSink`** : à **chaque** `bb.write`, reçoit `BlackboardWrite{pointer, version, agent_id, at, bytes}` et **ancre** le record à la cellule durable (remplit `writes`/`reads`/`version`). Capture **100 %** des agents qui écrivent — même ceux non décorés (filet de sécurité ⇒ métrique **traced_pct**).
```rust
pub struct ProvenanceSink { rec: Arc<ProvenanceRecorder> }
impl WriteSink for ProvenanceSink { fn on_write(&self, w: &BlackboardWrite) { self.rec.anchor(w); } }
```
- **Composition** : le décorateur fournit la **sémantique** (input→output), le sink fournit l'**ancrage durable** (cellule+version) ; corrélés par `(agent_id, swarm_id, pointer, version)`. `ProvenanceRecorder` fusionne les deux en `AgentCallRecord` complets et les `append` à la `SwarmChain`.

### 3.5 Rejeu déterministe — **fermer le gap horloge** (ADR-SWARM-007)
- **Problème** : `SystemTime::now()` est non déterministe ⇒ exclu de `payload_hash` (advisory). L'**ordre canonique** est la **`version` du blackboard** (atomique, déjà déterministe) reflétée dans `seq`.
- **`LogicalClock`** injectable via `SwarmContext` : `seq` monotone par swarm. Les agents qui ont besoin de « temps » lisent l'horloge logique, pas l'horloge murale.
- **`replay_swarm(records, agents)`** : re-exécute chaque agent depuis son `input` reconstruit (ordre `seq`), recalcule `output_hash`, compare. Réutilise le pattern `DeterminismChecker`/`ReplayReport` d'`efe-nano`. **Déterministe ssi** l'agent est une fonction pure de `(input, variant)` : Mode A l'est ; Mode C exige `sampling.seed` épinglé (déjà dans `LlmProvenanceRecord`).
```rust
pub async fn replay_swarm(records: &[AgentCallRecord], fleet: &AgentFleet) -> ReplayReport;
// ReplayReport { ops: usize, mismatched: usize, first_mismatch: Option<Uuid>, deterministic: bool }
```

### 3.6 Le **vérificateur indépendant** — l'artefact du moat (§0)
`verify_swarm` ne prend **que** les records + la clé HMAC du tenant (server-side). **Aucune coopération provider.**
```rust
pub fn verify_swarm(records: &[AgentCallRecord], key: &[u8]) -> SwarmProvenanceReport;
pub struct SwarmProvenanceReport {
    pub traced_pct: f64,            // % d'écritures couvertes par un record (cible 100 %)
    pub chain_intact: bool,         // tous les HMAC recalculés concordent (tamper-evidence)
    pub content_intact: bool,       // tous les output_hash recalculés concordent
    pub dag_consistent: bool,       // chaque read réfère une version produite ANTÉRIEUREMENT
    pub first_break: Option<Uuid>,  // 1er record en faute (forensique)
    pub root: Option<[u8;32]>,      // engagement (tip)
}
```
Quatre invariants : **(1)** chaîne HMAC (modification *après* enregistrement), **(2)** `output_hash` (substitution de sortie), **(3)** `input_hash` + DAG (forge d'entrée / lecture d'une cellule inexistante), **(4)** couverture (100 %). Optionnel **(5)** rejeu (§3.5).

---

## 4. Gestion des clés — **je ne touche jamais de clé**
- **Réutilise `HmacKeySource`** : `EnvHkdf` lit `EFE_NANO_AUDIT_HMAC_MASTER` (≥32 o) **côté serveur** ; HKDF par tenant (salt = UUID tenant, info versionnée). **Le harness/tests** : `StaticMaster` (jamais en prod).
- **Câble le placeholder `VaultTransit { path }`** (explicitement « Phase 4 ») : dérivation via Vault Transit côté serveur. **Aucun secret en clair, aucun secret manipulé par l'assistant.**
- **Rotation** : réutilise le format fichier `EFE_AUDIT_SIGNING_KEYS_FILE` (1ʳᵉ ligne active, suivantes = vérification) + `HmacKey` zéro-on-drop. Le vérificateur (§3.6) accepte un **trousseau** (clé active + historiques) pendant la rotation.

---

## 5. Carte d'intégration — où ça se branche (sans casser l'existant)
- **`SwarmContext`** gagne `provenance: Option<Arc<ProvenanceRecorder>>` (None ⇒ comportement actuel, **coût nul**).
- **`Topology::run`** : quand `provenance` est `Some`, le `Blackboard` reçoit `ProvenanceSink` (`set_sink`) et les agents sont enveloppés en `ProvenancedAgent`. `TopologyOutput.provenance_root` = tip.
- **`ConsensusTopology`** (cas pilote) : `generate()` (générateurs) et `run_judges()` (juges) passent par les décorateurs ; les writes `/cand/*` et `/ballot/*` sont ancrés par le sink. **Un run consensus complet ⇒ une chaîne vérifiable + un root.**
- **`AgentFleet`** : option `fleet.provenanced(recorder)` qui renvoie une flotte dont chaque agent récupéré est pré-enveloppé (la provenance suit l'assemblage **par nom** §3 de Phase 2).
- **Pont N17** : un juge Mode C (`LlmJudge`) renseigne `llm_provenance_id` ⇒ la piste d'essaim **référence** la piste d'appel-LLM. Une seule lignée, deux granularités.

---

## 6. Modèle de menace — **honnêteté sur ce qui est prouvé** (et ce qui ne l'est pas)
| Attaque | Défendue ? | Mécanisme |
|---|---|---|
| Mutation d'un record *après* enregistrement | ✅ | chaîne HMAC (casse en aval) |
| Suppression / réordonnancement | ✅ | `seq` + chaînage (genèse `b"genesis"`) |
| Substitution de sortie | ✅ | `output_hash` recalculable |
| Forge d'entrée / lecture fantôme | ✅ | `input_hash` + DAG (version antérieure requise) |
| Inférence inter-tenant | ✅ | clé HKDF par tenant (isolation cryptographique) |
| **Enregistreur malveillant au moment T** | ❌ (honnête) | le HMAC prouve « non modifié **après** », **pas** « l'enregistreur fut honnête » |
| Provider mentant sur la version de modèle | ⚠️ partiel | `model_hash_verified` (N17) le détecte *si* hash attendu connu |

> **Déclaration honnête** (doctrine EFE) : la v1 prouve l'**intégrité post-enregistrement** et la **reconstructibilité**, pas l'honnêteté de l'enregistreur. La **mitigation gatée** (§8) : signatures **ed25519 par-agent** (l'agent signe sa propre sortie avec une clé éphémère) + **ancrage externe** (timestamping tiers). On ne les construit **que si** un Lot le justifie.

---

## 7. Le **gate** — Lot P4 (avant de déclarer le moat construit)
**Question** : *un enregistrement par-agent atteint-il 100 % de couverture + rejeu déterministe, à overhead < budget ?*
- **Harnais honnête** : **crypto réelle** (BLAKE3 + HMAC + chaîne + HKDF) sur un **essaim simulé** (les topologies T1/T-consensus déjà construites). Seuls les agents sont simulés — **même doctrine** que S2/S3 (« math/crypto réelles, agents simulés »).
- **Bras** : run **sans** provenance (`None`) vs **avec** (`Some`). Splits répétés ±σ.
- **Métriques / SLO** :
  - **`traced_pct == 100 %`** (toute écriture d'agent ⇒ un record). *Assertion dure.*
  - **`deterministic == true`** : rejeu d'un swarm complet ⇒ `output_hash` bit-identiques (`mismatched == 0`).
  - **`chain_intact && content_intact && dag_consistent`** sur un run sain ; **et** un test de **falsification** (muter 1 record ⇒ `first_break` pointe dessus).
  - **Overhead** : **latence < X %** (cible initiale **< 5 %**), **octets/agent < Y** (cible **< 2 Ko**). X/Y figés au lancement du Lot.
- **GO** : 100 % tracé **ET** déterministe **ET** overhead sous budget ⇒ moat construit, ADR-006 *Accepted*.
- **NO-GO / escalades** (cost-aware, esprit §5 Phase 3) : sink **asynchrone** (`ChannelSink`, ne bloque pas l'agent) ; **échantillonnage** (tracer 100 % Mode C, échantillon Mode A) ; **compaction** (Merkle des records pour réduire les octets). On n'escalade **que** si le budget est dépassé.

---

## 8. Évolutions **gatées** (anti-dette — 0 LOC tant que non prouvé)
- **Signatures ed25519 par-agent** (clé éphémère par agent) : passe de « intègre après enregistrement » à « **l'agent atteste sa propre sortie** ». Gate : un cas exige la non-répudiation par-agent (litige/réglementaire). *Spec gelée, 0 LOC.*
- **Ancrage externe** (timestamping RFC 3161 / transparence log) : preuve d'**antériorité** indépendante de notre horloge. Gate : exigence de preuve tierce de l'instant. *0 LOC.*
- **Compaction Merkle** : un seul hash racine par swarm + preuves d'inclusion. Gate : volume d'octets dépasse Y (déclenché par Lot P4). *0 LOC.*

---

## 9. Phasage (≈3-4 semaines · 80 %→95 %)
- **Inc.1 — l'atome (≈ sem. 1)** : crate `efe-swarm-provenance` (+ factor `efe-audit-core`) ; `AgentCallRecord`, `canon.rs`, `hash.rs`, `payload_hash`. Oracles : canonicalisation stable (round-trip serde), BLAKE3 déterministe, `payload_hash` insensible à `wall_at`. **~25 tests.**
- **Inc.2 — la chaîne + le vérificateur (≈ sem. 2)** : `SwarmChain` (HMAC par-swarm, genèse), `verify_swarm`, réutilise `HmacKeySource`. Oracle de **falsification** (muter ⇒ break détecté au bon record). `ProvenanceSink: WriteSink`.
- **Inc.3 — capture & câblage (≈ sem. 2-3)** : `ProvenancedAgent<A>` (tous les traits), `ProvenanceRecorder` (fusion sink+décorateur), `SwarmContext`/`AgentFleet` wiring, intégration `ConsensusTopology`. **Assertion `traced_pct == 100 %`.** Exemple `provenance_swarm.rs`.
- **Inc.4 — rejeu + gate (≈ sem. 3-4)** : `LogicalClock`, `replay_swarm`, `ReplayReport`, vérificateur autonome `verify_swarm.rs`. **Lot P4** (`lot_p4.rs`, overhead avec/sans ±σ) → ADR-SWARM-006 (GO/NO-GO) + ADR-SWARM-007 (horloge logique). Résultats → `scripts/swarm/EFE_SWARM_P4_<date>.{json,md}`.

---

## 10. Esquisse d'API publique (gelée pour Inc.1-4)
```rust
// crates/efe-swarm-provenance/src/lib.rs
pub use record::{AgentCallRecord, AgentCallDraft, CellRef, AgentRole, AgentKind};
pub use canon::canonical_bytes;
pub use hash::{blake3_hash, payload_hash};
pub use chain::{SwarmChain};
pub use sink::ProvenanceSink;
pub use decorator::ProvenancedAgent;
pub use recorder::{ProvenanceRecorder, RecorderConfig};
pub use clock::LogicalClock;
pub use replay::{replay_swarm, ReplayReport};
pub use verify::{verify_swarm, SwarmProvenanceReport};
pub use metrics::ProvenanceMetrics; // traced_total, traced_pct, overhead_ns, bytes_per_call (Prometheus, sans dépendance)
```
- **Dépendances** : `efe-blackboard`, `efe-nano-sdk` (`Blake3Hash`, `AgentId`, `SwarmId`, `TenantId`, `ProvenanceId`), `efe-audit-core` (HMAC/HKDF/canon), `blake3`, `serde`/`serde_json`, `async-trait`, `tokio[sync,rt]`. **Pas** de nouveau driver DB (in-memory + pattern PG existant).

---

## 11. Métriques & SLO (le tableau de bord du moat)
| Métrique | Cible | Mesure |
|---|---|---|
| `traced_pct` | **100 %** | records / écritures d'agent (assertion Lot P4) |
| `replay_determinism` | **1.0** | `1 − mismatched/ops` sur rejeu complet |
| `chain_verify` | **pass** | `verify_swarm.chain_intact` sur run sain + détection de falsification |
| `overhead_latency` | **< 5 %** | p50/p95 avec vs sans provenance (±σ) |
| `overhead_bytes` | **< 2 Ko/agent** | taille sérialisée moyenne d'`AgentCallRecord` |

Surfaces Prometheus (réutilise le pattern **sans-dépendance** de `SubstrateMetrics`) : `efe_swarm_prov_records_total`, `efe_swarm_prov_traced_ratio`, `efe_swarm_prov_overhead_ns`, `efe_swarm_prov_bytes`, `efe_swarm_prov_chain_breaks_total`.

---

## 12. Arborescence cible
```
crates/efe-audit-core/                 # (factor) canon + HMAC + HKDF + HmacKeySource — partagé
crates/efe-swarm-provenance/
  Cargo.toml
  src/{lib, record, canon, hash, chain, sink, decorator, recorder, clock, replay, verify, metrics}.rs
  examples/{provenance_swarm.rs, verify_swarm.rs, lot_p4.rs}
docs/docs/adr/ADR-SWARM-006-per-agent-provenance-is-the-moat.md
docs/docs/adr/ADR-SWARM-007-deterministic-replay-keys-on-version.md
scripts/swarm/EFE_SWARM_P4_<date>.{json,md}
```

---

## 13. Risques & parades
| Risque | Parade |
|---|---|
| Sink synchrone ralentit les agents (G overhead) | `ChannelSink` async ; le hash hors chemin critique (escalade Lot P4) |
| Non-déterminisme Mode C (sampling) | épingler `sampling.seed` (N17) ; sinon marquer `deterministic=false` **honnêtement** |
| Refactor `efe-audit-core` trop large | repli : dépendre de `efe-nano` et appeler ses `pub` (cross-vérif garantie) |
| Canonicalisation divergente des deux chaînes | **un seul** code canon (la factorisation §3.2) — test d'identité bit-à-bit |
| « On prouve trop » (sur-vente) | §6 déclare explicitement la limite (enregistreur honnête) — pas de marketing au-delà de la preuve |

---

## 14. Définition de « fait » (Phase 4 close à 95 %)
1. `efe-swarm-provenance` construit ; `AgentCallRecord` hashé+chaîné ; `verify_swarm` autonome. ✅ tests verts.
2. `ConsensusTopology` instrumentée ; **`traced_pct == 100 %`** sur l'exemple.
3. **Rejeu déterministe** d'un swarm complet (output_hash bit-identiques) démontré.
4. **Lot P4** exécuté (overhead < budget) → **ADR-SWARM-006** GO + **ADR-SWARM-007**.
5. Évolutions (ed25519, ancrage, Merkle) **spécifiées & gatées** (0 LOC).
6. Tout **dans le dossier**, 0 warning, harnais honnête (crypto réelle / agents simulés).

> **Le livrable du moat** : *un tiers, avec seulement les records et la clé du tenant, peut re-prouver qui a dit quoi, quand, à partir de quoi — et rejouer l'essaim. Sans nous croire. Sans croire le provider.*
