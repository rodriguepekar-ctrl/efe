# C.1 — LE SANDWICH SOUVERAIN : compressor et verifier comme hops scellés autour des appels frontière

**Statut :** architecture / préparation (aucun code n'est modifié par ce document).
**Date :** 2026-07-17 · **Base :** monorepo `efe` @ HEAD `02eccdb` (Fil B complet B.0→B.4 + audit).
**Auteur :** R. Pekar — avec Claude Code (Opus 4.8).
**Filiation :** premier jalon de l'assemblage « Fil C recentré sur B » (nano-robots × Fil B). Le patron
de jonction est celui du Fil B — gardes → petit moteur → validation → estampille + raison → sceau →
borne — instancié à deux nouvelles frontières : AVANT l'appel frontière (compression) et APRÈS
(vérification factuelle).
**Règle cardinale :** NE JAMAIS SUR-VENDRE. Chaque affirmation est ancrée `fichier:ligne` ; les gains
de compression sont des **estimations heuristiques mesurées**, jamais le « ×8 » des commentaires ;
le verdict du verifier est une **annotation d'un petit modèle**, pas une preuve de vérité.

---

## 0 · Le but exact (et ce que C.1 EST / n'est PAS)

**But :** encadrer chaque appel frontière coûteux d'intelligence locale prouvable — le **compressor**
(Llama-3.2-1B) condense le texte utilisateur AVANT les gros modèles (hop `prep`), le **verifier**
(Phi-4-mini) contrôle le résultat APRÈS (hop `check`) — et sceller ces deux hops **dans le même reçu**
que les hops de traduction du Fil B. Le reçu d'un relay raconte alors :
`prep(SLM) → claude → translate(SLM) → gpt → translate(SLM) → gemini → check(SLM)`.

**C.1 EST :** (a) la généralisation du type de hop (`kind ∈ {prep, translate, check}`, additif,
rétro-compatible) ; (b) deux adaptateurs sandwich dans `efe-multi-llm` qui consomment les clients
`CompressorClient`/`VerifierClient` EXISTANTS avec les gardes B.2/B.4 (circuit à sonde unique,
deadline de mission, capture d'audit B.3) ; (c) leur câblage servi opt-in dans la gateway
(`EFE_SLM_COMPRESSOR_URL` / `EFE_SLM_VERIFIER_URL` — les mêmes noms que le preview-server) ;
(d) la métrologie honnête des gains (tokens estimés avant/après, scellés par hop).

**C.1 N'EST PAS :** (a) un gate — le verdict du verifier n'interrompt ni ne fait rejouer aucune
mission (annotation pure, parité stricte avec les missions A/B, `mission_a.rs:636-661`) ; (b) une
compression des formes canoniques (déjà denses et typées — seul le texte utilisateur est comprimé) ;
(c) un prep sur `debate` (comprimer un sujet de débat est sémantiquement risqué — exclu, documenté) ;
(d) un claim de qualité — la compression est lossy par nature ; le reçu rend toute régression
**attribuable**, il ne la rend pas impossible ; (e) specialist/cascade (aucune reformulation là —
hors périmètre).

---

## 1 · Vérité-terrain vérifiée (ce qui existe aujourd'hui)

### 1.1 · Les deux robots sont réels, typés, et déjà protégés

**CompressorClient** (`efe-nano-app-coordinator/src/clients/compressor.rs`) :
- `from_assets(endpoint)` (`:32-45`) : robot_id `robot_ids::COMPRESSOR = "compressor"` (`lib.rs:104`),
  schema_version `"1.0.0"`, grammaire `grammars/compressor_v1.gbnf`, prompt `prompts/compressor.txt`,
  n_predict **1536**, température **0.0** (greedy ; env `EFE_COMPRESSOR_TEMPERATURE`).
- I/O (`types/compressor.rs`) : `CompressorInput { text[50..50_000], target_words[10..500],
  must_preserve(≤50), style ∈ {Extractive(défaut), Abstractive, ClaimsOnly} }` (`:11-32`) →
  `CompressorOutput { summary[1..2000 chars], word_count, preserved, omitted_for_brevity,
  compression_ratio[1..50] }` (`:35-56`) + `validate_preservation()` (`:60-70`).
- **Grammaire dynamique** : `dynamic_compressor_grammar` réécrit `char{1,500}` → `char{1,N}`
  (budget clampé [120,500]) par appel (`compressor.rs:123-151`).
- **Map-reduce au-delà de ~512 tokens estimés** : `compress_long` découpe (chunks 512,
  chevauchement 30, concurrence 4, ≤4 passes de reduce — `compressor.rs:308-341`) ⟹ **un invoke
  peut émettre PLUSIEURS AuditEntry** (une par appel `complete_with_grammar`).
- **Timeout par appel : 5 s** (`llama_server.rs:98` ; ni compressor ni verifier n'appellent
  `with_timeouts` — contraste avec le reformulateur multi-llm à 30 s).

**VerifierClient** (`clients/verifier.rs`) :
- `from_assets(endpoint)` (`:22-35`) : robot_id `"verifier"` (`lib.rs:106`), schema_version `"1.0.0"`,
  grammaire `grammars/verifier_v1.gbnf` (statique), prompt `prompts/verifier.txt`, n_predict **768**,
  température **0.1**.
- I/O (`types/verifier.rs`) : `VerifierInput { claim[1..2000], candidate_sources[1..50]
  (CandidateSource { id[1..64], text[1..8000] }), strictness ∈ {Lenient, Standard(défaut), Strict,
  Audit} }` (`:11-40`) → `VerifierOutput { verdict ∈ {SUPPORTED, UNSUPPORTED, CONTRADICTED,
  INSUFFICIENT_EVIDENCE}, best_source_id?, similarity[0..1], contradictions(≤5), severity ∈
  {INFO, WARN, ERROR}, explanation?(≤500) }` (`:43-69`) + `validate_consistency()` (`:73-83` :
  SUPPORTED ⟹ best_source_id présent ; UNSUPPORTED ⟹ absent).
- Le prompt biaise honnêtement : « en cas de doute, INSUFFICIENT_EVIDENCE » ; `similarity` est
  auto-déclarée « cosine-like » par le modèle — PAS une mesure externe.

**Les circuits existent déjà dans le vocabulaire** : `default_slm_circuits()` crée exactement
`{"analyzer","compressor","verifier"}` à seuil 3 / reset 10 s (`efe-coordinator/src/coordinator.rs:112-121`) ;
`slm_circuit_for` retombe sur la clé `"{robot}"` sans policy (`:511-524`). Les champs
`CoordinatorClients.compressor/.verifier` existent (`coordinator.rs:43,45`), défaut `None`,
**jamais renseignés sur le chemin servi de la gateway** (vérifié : `slm_clients_from` ne mute que
`.reformulator` + son circuit — `frontier.rs:117-155`).

### 1.2 · Le protocole d'usage éprouvé (missions A/B — la parité à respecter)

- **Gate métier de compression** : `should_compress(user_text)` = seuil **250 tokens estimés**
  (`estimate_tokens = ceil(chars/4)`, `metrics.rs:252-257`) — `mission_a.rs:370`, `mission_b.rs:189`.
- **Cible proportionnelle** : `proportional_target_words = estimate_tokens/8` clampé [30,500]
  (`workflows/mod.rs:143-145`).
- **Détection de sortie dégénérée** : summary vide ou contenant les placeholders
  `<the compressed text>` / `<source text>` ⟹ repli texte brut + trace
  `"compressor:degenerate_output"` (`workflows/mod.rs:50-56`).
- **Ré-injection des questions** : les phrases interrogatives du texte brut sont ré-injectées
  verbatim autour du résumé + préambule (`final_prompt_from_compression`, `mod.rs:46-89` ;
  `extract_questions`, `mod.rs:104-134`).
- **Verifier = annotation pure, sources fournies par l'appelant** : ne tourne que si
  `!req.candidate_sources.is_empty()` (`mission_a.rs:602`, `mission_b.rs:458`) ; claim = sortie LLM
  (A) ou texte du gagnant du vote (B, `mission_b.rs:478`) ; verdict → évènement + `FactCheck` dans
  la réponse — jamais de gate ni retry.
- **Métrologie existante** : `ChewingMetrics { input_tokens_raw, input_tokens_compressed,
  compression_ratio, savings_tokens, savings_ratio, compressor_latency_ms, verifier_latency_ms, … }`
  (`metrics.rs:43-…`), recalculée depuis les tokens estimés (`recompute_savings`, `metrics.rs:274-284`).

### 1.3 · Le wire de hop Fil B (l'existant à étendre, pas à casser)

- `ReformulationHop { hop, from_llm, target_schema, engine, slm_audit?, degrade_reason? }`
  (`engine.rs:113-139`) ; `SlmHopAudit` (`:60-85`) ; `Reformulated{…}.to_hop(...)` (`:184-193`).
- Les 3 réponses portent `#[serde(default)] reformulation_hops` (`request.rs:87,133,182`).
- Builder de reçu : `build_and_seal_mission(..., reformulation_hops: &[Value], output)` crée un nœud
  Mode-A `reform_{i}` par hop, `output_hash = blake3(JSON du hop)`, enfant du dispatch, parent du
  reduce (`mission_graph.rs:121-129, 188-208, 226-228`).
- Handlers : extraction typée `resp.reformulation_hops` → `Vec<Value>` (`llm.rs:263,293,327`) ;
  marqueur déclaratif scellé `reform_expected` (relay 3, debate 2·rounds, specialist/cascade 0,
  vote absent — `llm.rs:260,290,324,359,394`).
- Vue servie : `ReformulationView { hops, slm, rule, degraded, slm_mediated }` dérivée de la sortie
  scellée (`llm.rs:138-168`) ; checker : `reform_hops_recorded` (préfixe `reform_`) vs
  `reform_hops_expected` (marqueur) (`llm.rs:406-465`).
- Runtime de hop (Fil B) : `HopCapture = Arc<Mutex<Option<AuditEntry>>>` + `TeeAuditWriter`
  (partagé D'ABORD, capture ensuite — `slm.rs:81-120`), plancher `SLM_HOP_MIN_REMAINING = 1 s`
  (`slm.rs:196`), porte `try_call` 1-sonde, entrée TIMEOUT synthétisée sur coupe (B.4).
- **vote.rs porte déjà l'intention en commentaire mort** (`vote.rs:46-48` : « Étape 1 : Compress…
  ici on appelle directement les LLMs ») — C.1.b remplace ce commentaire par le vrai hop.

---

## 2 · LE PROBLÈME CENTRAL — trois écarts, plus simples que B.1 mais réels

**(a) Le sens du hop change.** Le wire `reformulation_hops` est scellé dans des reçus existants ;
`prep`/`check` ne sont pas des traductions (pas de `target_schema` canonique, pas de `from_llm`
émetteur). Il faut étendre le type SANS invalider un seul sceau passé ⟹ champ `kind` additif avec
défaut `Translate` (un JSON ancien sans `kind` se désérialise inchangé), et des conventions de
champs documentées pour les nouveaux kinds (§4).

**(b) Un hop peut être multi-appels.** Le map-reduce du compressor casse l'hypothèse implicite
« 1 hop = 1 AuditEntry » du tee B.3. La capture doit compter (`CaptureSlot { last, calls }`) et le
hop porter `slm_calls` quand ≠ 1 ; le writer partagé, lui, retient TOUTES les entrées (inchangé).

**(c) Les bornes du verifier mordent.** `claim ≤ 2000 chars`, `source.text ≤ 8000`, `sources ≤ 50`
(`types/verifier.rs`) : un `final_text` de relay peut dépasser. On ne vérifie JAMAIS en silence un
claim tronqué ⟹ clamps aux frontières de champ + drapeaux scellés (`claim_truncated`,
`sources_truncated`).

Ce qui est FACILE (et c'est la récompense du Fil B) : les clients existent et sont mono-profil
(pas de B.1 bis) ; les circuits par défaut portent déjà les bonnes clés ; le runtime de hop
(capture/tee/plancher/synthèse TIMEOUT) est écrit et revu — il suffit de l'extraire et de le
partager ; le builder de reçu prend des `&[Value]` (le kind voyage dans le JSON, la gateway
non-gatée ne dépend toujours pas d'efe-multi-llm).

---

## 3 · Design

### 3.1 · Le type de hop généralisé (additif, wire figé)

```rust
// efe-multi-llm/src/reformulator_ext/engine.rs (extension)
#[derive(..., Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HopKind { Prep, #[default] Translate, Check }

pub struct ReformulationHop {
    …champs existants inchangés…,
    #[serde(default, skip_serializing_if = "is_default_kind")]
    pub kind: HopKind,                       // absent ⟹ Translate (rétro-compat sceaux B.3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slm_calls: Option<u32>,              // Some(n) SSI n ≠ 1 (map-reduce compressor)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prep: Option<PrepFacts>,             // kind=Prep seulement
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckFacts>,           // kind=Check seulement
}

pub struct PrepFacts {
    pub est_tokens_in: u32,                  // ceil(chars/4) du texte brut — heuristique, dite
    pub est_tokens_out: u32,                 // idem sur le prompt final (résumé + questions + préambule)
    pub compression_ratio: f32,              // celle déclarée par le robot (CompressorOutput)
    pub style: String,                       // "EXTRACTIVE" | …
}

pub struct CheckFacts {
    pub verdict: String,                     // tag fermé GBNF : SUPPORTED | UNSUPPORTED | CONTRADICTED | INSUFFICIENT_EVIDENCE
    pub severity: String,                    // INFO | WARN | ERROR
    pub similarity: f32,                     // auto-déclarée par le SLM — annotation, pas preuve
    pub best_source_id: Option<String>,
    pub contradictions: Vec<String>,
    pub claim_truncated: bool,               // claim clampé à 2000 chars — dit, jamais silencieux
    pub sources_truncated: u32,              // nb de sources dont text a été clampé à 8000
}
```

Conventions de champs pour les nouveaux kinds (documentées sur le type) :
- `prep` : `hop = "prep"`, `from_llm = "user_text"`, `target_schema = "compressor/summary_v1"`.
- `check` : `hop = "check"`, `from_llm = <winner_llm | synthesizer_llm | jury_llm>` (l'auteur du
  claim vérifié), `target_schema = "verifier/verdict_v1"`.
- Moteur : `Slm` = le robot a produit et la sortie a été validée ; `SlmDegradedToRule` = **identité**
  pour `prep` (texte original conservé) / **non-vérifié** pour `check` — toujours avec
  `degrade_reason` (mêmes six tags B.4). `Rule` n'apparaît PAS sur prep/check (il n'y a pas de
  moteur-règle qui produise un résumé ou un verdict) — invariant documenté et épinglé par test.
- Client absent ⟹ **aucun hop** (l'invariant B.0 : le chemin par défaut reste byte-identique).
- `slm_mediated` (vue) reste **scopé aux hops Translate** — gel sémantique explicite (§6).

### 3.2 · Le runtime de hop partagé (extraction, pas réécriture)

Nouveau module `efe-multi-llm/src/reformulator_ext/hop_runtime.rs` — on **déplace** depuis `slm.rs`
(aucun changement sémantique, les tests B.2/B.3/B.4 restent verts) :
`TeeAuditWriter`, le plancher (`SLM_HOP_MIN_REMAINING`), et la capture étendue :

```rust
pub(crate) struct CaptureSlot { last: Option<AuditEntry>, calls: u32 }   // ex-HopCapture
pub(crate) type HopCapture = Arc<Mutex<CaptureSlot>>;
// TeeAuditWriter::record : shared.record(entry.clone()).await? ;   ← l'ordre B.4 (revue) est CONSERVÉ
//                          slot.last = Some(entry); slot.calls += 1;
// take_capture -> (Option<SlmHopAudit>, u32 /*calls*/)
```

Le chemin reformulateur (`slm.rs`) consomme le module extrait ; `calls` y vaut toujours 1
(contrat une-entrée-par-invoke du `LlamaServerClient`) ⟹ `slm_calls: None` sur les hops Translate —
**wire Translate inchangé à l'octet**.

### 3.3 · L'adaptateur `prep` (compressor)

Nouveau `efe-multi-llm/src/sandwich.rs` :

```rust
pub const SANDWICH_PREP_CIRCUIT_KEY:  &str = "compressor";   // = robot_ids::COMPRESSOR = clé default_slm_circuits
pub const SANDWICH_CHECK_CIRCUIT_KEY: &str = "verifier";

pub struct SandwichPrep { client: Arc<CompressorClient>, circuit: Option<SlmCircuitBreaker>,
                          audit: Arc<dyn AuditWriter>, tenant_id: TenantId }
pub fn prep_for(coord: &Coordinator) -> Option<SandwichPrep>   // à la reformulator_for (slm.rs:668-682)

pub struct PrepOutcome { pub prompt: String, pub hop: Option<ReformulationHop> }
impl SandwichPrep {
    pub async fn run(&self, user_text: &str, deadline: Option<Duration>) -> PrepOutcome
}
```

Séquence de `run` (l'ordre des gardes B.4, inchangé) :
1. **Gate métier** : `should_compress(user_text)` (seuil 250 — exporté d'efe-coordinator, §3.6).
   Faux ⟹ `PrepOutcome { prompt: user_text, hop: None }` — pas de hop, pas d'appel (texte court =
   non-évènement, comme un LLM non-répondant dans vote).
2. **Deadline** : `deadline < SLM_HOP_MIN_REMAINING` ⟹ identité + hop `SlmDegradedToRule` +
   `deadline_exhausted`, SANS appel ni consommation de sonde.
3. **Circuit** : `try_call()` sur la clé `"compressor"` — ouvert/sonde prise ⟹ identité + `circuit_open`.
4. **Invoke borné** : `tokio::time::timeout(reste, client.invoke(input, ctx_tee))` où
   `input = CompressorInput { text: clamp 50_000 chars, target_words: proportional_target_words,
   must_preserve: vec![], style: Extractive }`. Coupe en vol ⟹ entrée TIMEOUT **synthétisée** si la
   capture est vide (parité B.4, robot_id `"compressor"`) ; `slm_calls` = appels complétés.
5. **Validation mission-side** (défense en profondeur, en plus du schéma GBNF) : summary non vide,
   sans placeholders dégénérés (les motifs de `workflows/mod.rs:50-56`), `word_count ≥ 1`.
   Invalide ⟹ `record_failure` + identité + `invalid_output`.
6. **Assemblage du prompt** : `final_prompt_from_compression(summary, user_text)` (ré-injection des
   questions + préambule — exporté, §3.6). S'il signale `fell_back` ⟹ traité comme invalide (5).
7. Succès ⟹ `record_success` + hop `Slm` + `PrepFacts { est_tokens_in/out, ratio, style }` +
   `slm_audit` (dernière entrée = reduce) + `slm_calls`.

**Interaction 5 s / map-reduce, dite :** le timeout INTERNE de 5 s est PAR APPEL `complete` ; un
long texte = chunks×5 s potentiels — c'est la deadline de MISSION (externe) qui borne le hop entier.
Un timeout interne (un chunk qui pend) arrive en `RobotError::Timeout` ⟹ échec robot ⟹ circuit +
`robot_error`. Une coupe externe ⟹ `deadline_exhausted`, sans impact circuit. Les deux chemins
laissent leur trace d'audit.

### 3.4 · L'adaptateur `check` (verifier)

```rust
pub struct SandwichCheck { client: Arc<VerifierClient>, circuit, audit, tenant_id }
pub fn check_for(coord: &Coordinator) -> Option<SandwichCheck>
impl SandwichCheck {
    pub async fn run(&self, claim: &str, claim_author: &str,
                     sources: &[CandidateSourceReq], deadline: Option<Duration>)
        -> Option<ReformulationHop>          // None SSI sources vides (parité A/B)
}
```

1. `sources.is_empty()` ⟹ `None` (pas de hop — exactement `mission_a.rs:602`).
2. Deadline / circuit (`"verifier"`) : mêmes gardes ; dégradé ⟹ hop `SlmDegradedToRule`
   (= « non-vérifié ») + raison — le hop EXISTE (l'appelant a demandé une vérification : son
   absence doit être visible, contrairement au prep court).
3. **Clamps scellés** : claim → 2000 chars (`claim_truncated=true` si coupé) ; chaque source.text →
   8000 (`sources_truncated += 1`) ; sources → 50 max (troncature de liste comptée aussi).
4. Invoke borné (5 s interne / reste-de-mission externe, TIMEOUT synthétisé pareil).
5. Validation : schéma + `validate_consistency()` (`types/verifier.rs:73-83`) — incohérent ⟹
   `invalid_output` + circuit.
6. Succès ⟹ hop `Slm` + `CheckFacts` complet. **Jamais de gate** : le verdict est scellé, la
   réponse de mission est inchangée par lui.

### 3.5 · Les missions (threading)

- **vote** (`vote.rs`) : le commentaire mort `:46-48` devient
  `let PrepOutcome { prompt, hop } = prep…` ; `LlmCall.prompt = prompt` (levier ×N sur le fan-out) ;
  hop poussé EN TÊTE de `reformulation_hops`. Après le tally : `check` sur
  `(winner_text, winner_llm, req.candidate_sources)` → hop poussé EN QUEUE.
- **relay** (`relay.rs`) : prep une fois au départ ; le prompt compressé remplace `req.user_text`
  dans les TROIS prompts (les canoniques `plan_json`/`code_json` injectés ne sont PAS touchés) ;
  check sur `(final_text, synthesizer_llm, sources)`.
- **debate** : PAS de prep (non-but §0) ; check sur `(verdict, jury_llm, sources)` — le verdict du
  jury, seul hop qui manquait au débat, gagne ainsi une trace scellée sans être reformulé.
- **Requêtes** (additif, `#[serde(default)]`) : `Vote3LlmRequest`, `RelayRequest`, `DebateRequest`
  gagnent `candidate_sources: Vec<CandidateSourceReq { id, text }>` — vide = pas de check, parité A/B.
- Ordre scellé dans le reçu : `prep? · translate* · check?` — chronologique, lisible.

### 3.6 · Les exports anti-dérive (efe-coordinator)

`should_compress`, `proportional_target_words`, `final_prompt_from_compression` (+ ses motifs
dégénérés) et `estimate_tokens` passent `pub` depuis `efe-coordinator::workflows` / `metrics`
(aujourd'hui pub(crate) — `mod.rs:46-145`, `metrics.rs:252-257`). **Pas de copie** dans
efe-multi-llm : la leçon du garde de souveraineté dupliqué (dérive SSRF) s'applique — une seule
source de vérité, consommée par A/B ET par le sandwich.

### 3.7 · Le wiring servi (gateway)

`slm_clients_from` (`frontier.rs:117-155`) se généralise en fonction pure à trois opt-ins :

```rust
fn slm_clients_from(reformulator: Option<&str>, compressor: Option<&str>, verifier: Option<&str>)
    -> CoordinatorClients
```

Pour chacun : URL absente/vide ⟹ rien ; fournie ⟹ `from_assets` (lecture disque, **aucun réseau au
boot** — pas de pré-vol santé, contraste assumé avec le preview-server `lib.rs:350-368` : ici le
circuit gère les serveurs malades) + circuit `SlmCircuitBreaker::new(3, 10 s)` sous la clé du robot
+ warn de localité (`slm_url_looks_local`) + log de boot ; échec de construction ⟹ warn +
dégradé, **boot jamais cassé**. Env : `EFE_SLM_COMPRESSOR_URL` / `EFE_SLM_VERIFIER_URL` — les mêmes
noms que le preview-server (`efe-preview-server/src/lib.rs:309-311`), cohérence d'opérateur ;
opt-ins séparés = rayons de panne séparés.

### 3.8 · Le reçu (gateway, non-gaté — toujours sans dépendance à efe-multi-llm)

`build_and_seal_mission` : le préfixe du nœud suit le kind lu DANS le JSON du hop —
`prep_{i}` / `reform_{i}` / `check_{i}` (`hop.get("kind")`, absent ⟹ `reform`, legacy inchangé).
Même commitment (`output_hash = blake3(JSON complet du hop)` — PrepFacts/CheckFacts/flags scellés),
mêmes arêtes (enfant du dispatch, parent du reduce). Marqueurs scellés par le handler dans le résumé
de requête : `prep_expected: 0|1` (= client câblé ET `should_compress(user_text)` — la décision du
builder de confiance, scellée comme la décision cascade `llm.rs:394`) et `check_expected: 0|1`
(= client câblé ET sources non vides). Checker `/v1/llm/audit/:id` : `prep_hops_recorded` /
`check_hops_recorded` (préfixes) + expected + `*_under_declared` — marqueur absent (run pré-C.1) ⟹
`None`, **aucune accusation rétroactive** (la leçon B.3).

---

## 4 · Contrats d'interface (signatures exactes proposées)

```rust
// efe-multi-llm/src/reformulator_ext/engine.rs (additif)
pub enum HopKind { Prep, #[default] Translate, Check }            // SCREAMING_SNAKE sur le fil
pub struct PrepFacts  { est_tokens_in: u32, est_tokens_out: u32, compression_ratio: f32, style: String }
pub struct CheckFacts { verdict: String, severity: String, similarity: f32,
                        best_source_id: Option<String>, contradictions: Vec<String>,
                        claim_truncated: bool, sources_truncated: u32 }
// ReformulationHop += { kind, slm_calls?, prep?, check? }         // tous #[serde(default)]

// efe-multi-llm/src/reformulator_ext/hop_runtime.rs (extraction de slm.rs)
pub(crate) struct CaptureSlot { last: Option<AuditEntry>, calls: u32 }
pub(crate) fn take_capture(c: &HopCapture) -> (Option<SlmHopAudit>, u32)

// efe-multi-llm/src/sandwich.rs (nouveau)
pub const SANDWICH_PREP_CIRCUIT_KEY:  &str = "compressor";
pub const SANDWICH_CHECK_CIRCUIT_KEY: &str = "verifier";
pub fn prep_for(coord: &Coordinator)  -> Option<SandwichPrep>;
pub fn check_for(coord: &Coordinator) -> Option<SandwichCheck>;
impl SandwichPrep  { pub async fn run(&self, user_text: &str, deadline: Option<Duration>) -> PrepOutcome }
impl SandwichCheck { pub async fn run(&self, claim: &str, claim_author: &str,
                                      sources: &[CandidateSourceReq], deadline: Option<Duration>)
                     -> Option<ReformulationHop> }

// efe-multi-llm/src/request.rs (additif)
pub struct CandidateSourceReq { pub id: String, pub text: String }
// Vote3LlmRequest / RelayRequest / DebateRequest += #[serde(default)] candidate_sources: Vec<CandidateSourceReq>

// efe-coordinator (exports additifs — une seule source de vérité)
pub use workflows::{should_compress, proportional_target_words, final_prompt_from_compression};
pub use metrics::estimate_tokens;

// efe-swarm-gateway/src/frontier.rs
fn slm_clients_from(reformulator: Option<&str>, compressor: Option<&str>, verifier: Option<&str>) -> CoordinatorClients
```

> Note de dépendance : identique au Fil B — le sandwich vit dans `efe-multi-llm` (qui dépend
> d'efe-coordinator et d'efe-nano-app-coordinator, zéro cycle) ; la gateway non-gatée continue de
> manipuler des `&[Value]`.

---

## 5 · Fail-closed / dégradation (le tableau des verdicts)

| Condition | prep | check | Circuit | Raison scellée |
|---|---|---|---|---|
| Client absent | pas de hop, prompt = brut | pas de hop | — | — |
| Texte < 250 tokens est. | pas de hop (non-évènement) | n/a | — | — |
| Sources vides | n/a | pas de hop (parité A/B) | — | — |
| Deadline < plancher | identité, sans appel | « non-vérifié », sans appel | **non consommé** | `deadline_exhausted` |
| Circuit ouvert / sonde prise | identité, sans appel | « non-vérifié », sans appel | — | `circuit_open` |
| Invoke échoue (transport / 5 s interne) | identité | « non-vérifié » | `record_failure` | `robot_error` |
| Coupe externe (reste de mission) | identité + TIMEOUT synthétisé | idem | **aucun impact** | `deadline_exhausted` |
| Sortie dégénérée / incohérente | identité | « non-vérifié » | `record_failure` | `invalid_output` |
| Entrée inutilisable (texte < 50 chars pour le robot) | identité | « non-vérifié » | aucun impact | `client_side` |
| Succès validé | prompt compressé, `Slm` | verdict scellé, `Slm` | `record_success` | — |

Jamais fail-open : une sortie robot invalide n'est jamais servie ; jamais silencieux : chaque
dégradation porte sa raison ; jamais de gate : ni prep ni check ne font échouer une mission qui
aboutirait sans eux.

---

## 6 · Provenance / honnêteté sur le fil

- Chaque hop prep/check est un nœud scellé de PLEIN droit (blake3 du JSON complet — facts, flags,
  audit SLM, `slm_calls`) : maquiller un verdict ou gonfler un ratio après coup casse la re-preuve,
  localisée.
- **`slm_mediated` ne change PAS de sens** : il reste « tous les hops **Translate** sont SLM » —
  gel sémantique explicite (un consommateur du wire B.3 ne doit pas voir sa définition bouger).
  La vue gagne à côté : `prep_hops`, `check_hops`, `prep_est_saved_tokens` (somme des
  `est_tokens_in − est_tokens_out`, étiquetée *estimation heuristique chars/4*), `check_verdict`
  (celui du dernier hop check).
- Le COÛT du sandwich est visible dans le même reçu : les `slm_audit` des hops portent les tokens
  du compressor/verifier eux-mêmes — le lecteur peut faire la soustraction gains−coût, on ne la
  fait pas à sa place.
- La revendication autorisée : « prompt compressé par le petit modèle, N tokens estimés économisés
  (heuristique), vérifié SUPPORTED contre la source src-X — scellé, re-vérifiable ». La revendication
  INTERDITE : « ×8 », « qualité préservée », « fact-checké » sans mention du modèle et de la
  similarité auto-déclarée.

---

## 7 · Change-set précis (fichiers & fonctions)

| Jalon | Fichier | Changement |
|---|---|---|
| C.1.a | `efe-multi-llm/src/reformulator_ext/engine.rs` | `HopKind` + `slm_calls`/`prep`/`check` additifs + tests wire (ancien JSON ⟹ Translate) |
| C.1.a | `efe-multi-llm/src/reformulator_ext/hop_runtime.rs` | extraction `TeeAuditWriter`/plancher + `CaptureSlot{last,calls}` ; `slm.rs` consomme |
| C.1.a | `efe-coordinator/src/workflows/mod.rs`, `metrics.rs` | exports pub `should_compress`/`proportional_target_words`/`final_prompt_from_compression`/`estimate_tokens` |
| C.1.a | `efe-swarm-gateway/src/mission_graph.rs` | préfixe de nœud par `kind` (`prep_/reform_/check_`), défaut legacy `reform_` |
| C.1.b | `efe-multi-llm/src/sandwich.rs` | `SandwichPrep` + `prep_for` + gardes B.2/B.4 + validation dégénérée + facts |
| C.1.b | `efe-multi-llm/src/workflows/vote.rs`, `relay.rs` | threading prep (vote :46-55 remplacé ; relay en tête des 3 phases) |
| C.1.b | `efe-swarm-gateway/src/frontier.rs` | `slm_clients_from` à 3 opt-ins + circuit `"compressor"` + logs/warns |
| C.1.c | `efe-multi-llm/src/sandwich.rs` | `SandwichCheck` + clamps scellés + `check_for` |
| C.1.c | `efe-multi-llm/src/request.rs` + 3 workflows | `candidate_sources` additif + threading check (vote/relay/debate) |
| C.1.c | `efe-swarm-gateway/src/frontier.rs` | circuit `"verifier"` + wiring |
| C.1.d | `efe-swarm-gateway/src/routes/llm.rs` | marqueurs `prep_expected`/`check_expected` scellés + `LlmAuditView` + `ReformulationView` étendues |
| C.1.d | doc + mémoire | ce document mis à jour au réel + revue adverse globale |

---

## 8 · Jalons (l'ordre B : chaque tranche sûre seule)

- **C.1.a — Le socle.** Type de hop généralisé, runtime extrait, exports anti-dérive, préfixes de
  nœud. **Invariants prouvés :** défaut byte-identique (aucun client câblé ⟹ réponses inchangées à
  l'octet) ; rétro-compat wire (hop JSON B.3 sans `kind` ⟹ Translate) ; tests B.2/B.3/B.4 verts sans
  modification de sémantique.
- **C.1.b — Le pain du dessus (prep servi).** Adapter compressor + vote/relay + wiring + mesures.
  DoD : vote e2e faux-llama (enveloppe compressor) ⟹ hop `PREP`/`Slm` scellé avec facts, re-preuve
  verte ; texte court ⟹ zéro hop ; map-reduce ⟹ `slm_calls` correct ; coupe deadline ⟹ TIMEOUT
  synthétisé, circuit intact ; placeholders dégénérés ⟹ `invalid_output` + identité.
- **C.1.c — Le pain du dessous (check servi).** Adapter verifier + `candidate_sources` + clamps.
  DoD : verdict scellé + `validate_consistency` épinglée ; claim > 2000 ⟹ `claim_truncated=true`
  scellé ; sources vides ⟹ zéro hop ; jamais de gate (mission verte même verdict CONTRADICTED).
- **C.1.d — La surface.** Marqueurs scellés + checker + vue + doc + **revue adverse globale**
  (3 clusters : capture multi-appels, clamps/honnêteté du check, invariants du reçu étendu).

Ordre imposé : a (sûr) → b (valeur principale) → c (parité A/B) → d (preuve servie).

---

## 9 · Tests (au-delà des DoD par jalon)

- **Wire :** round-trip des 3 kinds ; ancien JSON (fixture figée du format B.3) ⟹ `Translate` ;
  `slm_calls` absent ⟹ 1 implicite.
- **Runtime partagé :** `CaptureSlot` compte ; l'ordre tee (partagé d'abord) re-épinglé ; la
  synthèse TIMEOUT porte le bon `robot_id` par adaptateur.
- **Prep :** grammaire dynamique respectée (résumé ≤ budget) ; questions ré-injectées présentes
  dans le prompt final ; `est_tokens_*` cohérents avec `estimate_tokens`.
- **Check :** SUPPORTED ⟹ `best_source_id` présent (consistency) ; troncatures comptées ;
  sources > 50 refusées proprement.
- **Reçu :** nœuds `prep_0`/`check_0` présents, falsification d'un `CheckFacts.verdict` casse la
  re-preuve ; audit endpoint compte par kind ; run legacy ⟹ `None`, pas d'accusation.
- **Non-régression :** batterie complète des 5 crates (le harnais du Fil B).

---

## 10 · Réserves à énoncer (Kovant — le vrai)

- **Gains = estimations heuristiques** (`chars/4`) hors serveur live ; les vrais tokens dépendent du
  tokenizer de chaque frontière. Le bench arithmétique valide la comptabilité, pas la facture.
- **La compression est lossy** : le reçu rend la perte *attribuable* (prep scellé), il ne la rend
  pas nulle. Style Extractive + ré-injection des questions = mitigations, pas garanties.
- **Le verdict du verifier est l'avis d'un Phi-4-mini contraint par grammaire** — similarity
  auto-déclarée, biais assumé vers INSUFFICIENT_EVIDENCE. C'est une annotation scellée, pas une
  vérité établie.
- **Timeout robot 5 s/appel hérité** (pas de `with_timeouts` en C.1) : un chunk lent = échec robot.
  Question ouverte §11 pour un override.
- **Ops** : trois URL = potentiellement trois llama-server (poids non embarqués, comme B) ; la
  localité n'est pas contrainte, seulement warnée (héritage B.4, dit tel quel).

## 11 · Risques & questions ouvertes

1. **`with_timeouts` pour le compressor ?** Map-reduce long × 5 s/appel : suffisant pour Extractive
   1B ? À mesurer en live avant d'ajouter un override env (`EFE_COMPRESSOR_TIMEOUT_S`).
2. **Opt-out par requête** (`compress: false`) : différé à C.1.d si un consommateur le demande —
   l'opt-in env est déjà un choix de déploiement explicite.
3. **Check à sources internes** (relay : `final_text` vs `plan_json`+`code_json` comme grounding
   interne) : sémantiquement séduisant, mais ce n'est PAS la parité A/B — différé, à décider sur
   pièces après C.1.c.
4. **`must_preserve`** : brancher les entités de l'analyzer (pheromones) comme entités à préserver —
   dépendrait d'un 4ᵉ robot câblé ; hors périmètre C.1, noté pour C.2+.
5. **Debate-prep** : réévaluer seulement avec un bench sémantique (le risque de distordre un sujet
   de débat est réel).

## 12 · Definition of Done (C.1 entier)

- Sans env : les cinq réponses de mission sont **byte-identiques** à HEAD 02eccdb ; zéro nouveau hop.
- `EFE_SLM_COMPRESSOR_URL` seul : vote/relay portent un hop `PREP` scellé (facts + audit + calls),
  la vue l'affiche, le checker le compte, la re-preuve passe, la falsification casse.
- `EFE_SLM_VERIFIER_URL` + sources fournies : hop `CHECK` scellé avec verdict/consistency/flags de
  troncature ; la mission n'est JAMAIS gatée par le verdict.
- `slm_mediated` : sémantique inchangée (Translate uniquement), épinglée par test.
- Boot jamais cassé ; batterie 5 crates verte ; revue adverse passée avec findings corrigés ;
- Honnêteté : aucune revendication de gain non chiffrée-heuristique-étiquetée ; aucune revendication
  de vérification sans mention du modèle vérificateur.

---

### Annexe — le chemin de l'information, après C.1 (vote, sandwich complet)

```
user_text (long)
  └─ prep(SLM compressor) ── prompt compressé + questions ré-injectées ──┐   [sinon : identité, estampillé]
                                                                          ▼
                                     fan-out N modèles frontière (LlmCall.prompt = compressé)
                                                                          │
                                     translate(SLM reformulateur) × N → claims canoniques → tally
                                                                          │
  candidate_sources (fournies) ──► check(SLM verifier) : winner_text vs sources → verdict scellé
                                                                          │
        reçu ⊕ { prep(PrepFacts) · claim:<llm>* · check(CheckFacts) } → racine re-vérifiable
```

Le gros modèle est un invité : accueilli par un résumé local, traduit par un traducteur local,
contrôlé par un vérificateur local — et chaque geste de l'accueil est scellé.
