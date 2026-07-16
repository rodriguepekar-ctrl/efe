# FIL-B — Servir un VRAI petit modèle (SLM) comme reformulateur-traducteur entre modèles frontière

**Statut :** architecture / préparation (aucun code n'est modifié par ce document).
**Date :** 2026-07-14 · **Base :** monorepo `efe` @ HEAD `665ead0` (+ working tree).
**Auteur :** R. Pekar — avec Claude Code (Opus 4.8).
**Règle cardinale :** NE JAMAIS SUR-VENDRE. Chaque affirmation ci-dessous est ancrée `fichier:ligne` ;
chaque revendication future (« relais médié par un SLM ») n'est vraie **que par hop, que si le SLM a
réellement tourné et réussi**, et toute dégradation vers le reformulateur-règles doit être **estampillée**.

---

## 0 · Le but exact (et ce que Fil B EST / n'est PAS)

**But du user :** prendre de gros agents frontière (Claude/Anthropic, Gemini, GPT) et intercaler un
**petit MODÈLE** qui **traduit/normalise** la sortie d'un gros modèle avant qu'un autre la consomme —
un petit modèle comme **traducteur de dialecte** entre les gros. C'est la variante « petit MODÈLE » (par
opposition à la variante (A) « petite étape déterministe », déjà servie via `CanonicalReformulator`).

**Fil B EST :** remplacer, sur le chemin servi des missions `relay`/`vote`/`debate`, le reformulateur
**déterministe Rust** par le **SLM réel** `ReformulatorClient` (Qwen3-0.6B, GBNF), quand il est configuré
et vivant, avec repli fail-closed vers le déterministe sinon.

**Fil B N'EST PAS :** (a) « un fil » — le SLM cible un vocabulaire de schémas **différent** de celui des
missions (§2), donc il faut un asset SLM neuf ; (b) mettre un LLM comme **participant du méta-essaim** —
c'est un **non-but assumé** (le doc Couche-2 s'y oppose ; le participant méta est Tensor→Tensor pur).
Fil B vit **entièrement** dans la pile `efe-swarm-gateway / efe-multi-llm / efe-coordinator /
efe-nano-app-coordinator`.

---

## 1 · Vérité-terrain vérifiée (ce qui existe aujourd'hui)

### 1.1 · Le seam de reformulation, DANS les missions
Les trois missions reçoivent **déjà** `coord: &Coordinator` mais construisent le reformulateur **en dur**,
localement :

| Mission | Signature | Reformulateur | Appel |
|---|---|---|---|
| `run_relay` | `(coord: &Coordinator, req) ` (`relay.rs:21`) | `CanonicalReformulator::new()` (`relay.rs:29`) | `reformulator.route(text, dialect, CanonicalSchemaId::PlanV1/CodeV1/SynthesisV1, provider)` (`relay.rs:64`) |
| `run_vote_3llm` | `(coord, req)` (`vote.rs:33`) | `…::new()` (`vote.rs:41`) | `reformulator.route(text, dialect, CanonicalSchemaId::ClaimV1, …)` (`vote.rs:80`) |
| `run_debate` | `(coord, req)` (`debate.rs:19`) | `…::new()` (`debate.rs:41`) | `reformulator.to_argument(raw, stance, dialect)` (`debate.rs:150,157`) |

Le seam unifié est **`CanonicalReformulator::route`** (`reformulator_ext/canonical.rs:307`) :

```rust
pub fn route(&self, raw: &str, from_dialect: SourceDialect,
             target: CanonicalSchemaId, llm_source: &str) -> MultiLlmResult<serde_json::Value>
```

Il **dispatche** vers `to_plan/to_code/to_claim/to_argument/to_synthesis` (`canonical.rs:44..303`) et
**retourne `serde_json::Value`** — la structure canonique typée sérialisée. `debate` n'utilise pas `route`
(il passe une **stance** réelle à `to_argument`). Fait décisif pour le seam : **`route` renvoie exactement
le même type (`serde_json::Value`) que la sortie du SLM** (§1.2).

`SourceDialect` (`reformulator_ext/mod.rs:10`) : `{Claude, Gpt, Gemini, …}` + `from_provider_id(id)` +
`tag()`. `CanonicalSchemaId` (multi-llm) : `{PlanV1, CodeV1, ClaimV1, ArgumentV1, SynthesisV1}` + `asset_id()`.

### 1.2 · Le petit MODÈLE réel : `ReformulatorClient`
`crates/efe-nano-app-coordinator/src/clients/reformulator.rs` — un vrai SLM (Qwen3-0.6B Q5), contraint par
grammaire GBNF via `LlamaServerClient` :

```rust
ReformulatorClient::from_assets(endpoint) -> RobotResult<Self>   // charge grammars/reformulator_v1.gbnf + prompts/reformulator.txt, 1024 tok, temp 0.1
impl NanoRobotClient { async fn invoke(&self, input: ReformulatorInput, ctx: &CallContext) -> RobotResult<ReformulatorOutput> }
```

Types I/O (`types/reformulator.rs`) :

```rust
struct ReformulatorInput  { raw_text: String, source_dialect: LlmDialect, target_schema_id: SchemaId }
struct ReformulatorOutput { canonical: serde_json::Value, schema_id: SchemaId, extraction_warnings: Vec<String> }
enum   LlmDialect { ClaudeXml, Gpt4Json, GeminiMarkdown, MistralMixed, Unknown }        // ← connaît déjà Claude/Gemini/GPT
enum   SchemaId   { CitationV1, FactV1, ToolCallV1, ClaimV1, SourceRefV1 }              // ← vocabulaire DIFFÉRENT (voir §2)
```

`invoke` : valide l'input, sérialise en user-prompt, appelle `inner.complete(&input, &user_text, ctx)`
(décodage contraint par GBNF), valide l'output. **`invoke` est `async`** ; `route` (règle) est **sync** — §2.

### 1.3 · Le porteur : `CoordinatorClients.reformulator` + `CoordinatorBuilder.clients()`
`efe-coordinator/src/coordinator.rs` :

```rust
struct CoordinatorClients { …, reformulator: Option<Arc<ReformulatorClient>>, …, slm_circuits: HashMap<String, SlmCircuitBreaker> }  // :37-62
impl CoordinatorBuilder { pub fn clients(mut self, c: CoordinatorClients) -> Self { … }  // :259  ← le seam d'injection existe
                          pub fn registry(…); pub fn policies(…); pub fn build(…) }
```

`efe-coordinator` **dépend déjà** de `efe-nano-app-coordinator` (donc de `ReformulatorClient`), et
`efe-multi-llm` dépend des **deux** — **aucun cycle, aucune frontière à franchir**. Le champ
`reformulator` existe, défaut `None`, jamais renseigné sur le chemin servi. Note : le garde du `build`
(`coordinator.rs:444`) exige des circuits **seulement** si analyzer/compressor/verifier sont présents —
**le reformulateur n'y est pas** (donc l'ajouter seul ne déclenche pas l'exigence de circuit ; §5 décide
d'en ajouter un quand même).

### 1.4 · Le chemin servi (gateway) et où il se branche
`efe-swarm-gateway/src/frontier.rs:42` — `coordinator_from_env()` construit :
```rust
Coordinator::builder().registry(registry).policies(policies).build()   // :79  ← AUCUN .clients(...) : clients = default (tous None)
```
`efe-swarm-gateway/src/routes/llm.rs` — les handlers servis appellent bien les missions avec ce coordinator :
`run_vote_3llm(coord, req)` (`:218`), `run_relay(coord, req)` (`:246`), etc., derrière l'`egress_gate`
+ `clamp_budget` + la feature `frontier-llm` + le flag runtime (sinon `501 NotImplemented`, `:203`).
**⟹ Le point d'injection servi de Fil B est `coordinator_from_env`**, pas ailleurs.

### 1.5 · `build_slm_clients` — hors périmètre servi
`efe-preview-server/src/lib.rs:337` construit `analyzer/compressor/verifier/router` (PAS le reformulateur,
PAS le filter) pour le serveur **preview (dev localhost)**. Ce **n'est pas** la surface frontière servie
(`efe-swarm-gateway`). Le brancher n'apporterait le SLM qu'au dev. (La synthèse de recherche pointait
`build_slm_clients` ; correction ici : le vrai point servi est `coordinator_from_env`.)

---

## 2 · LE PROBLÈME CENTRAL — désaccord de vocabulaire de schémas (+ sync/async + latence)

C'est ce qui transforme « un fil » en un vrai chantier. Trois écarts :

**(a) Vocabulaires de schémas disjoints.** Le SLM et les missions ne parlent pas des mêmes schémas :

| | SLM `ReformulatorClient` (`SchemaId`) | Missions multi-llm (`CanonicalSchemaId`) |
|---|---|---|
| Schémas | Citation, Fact, ToolCall, **Claim**, SourceRef | Plan, Code, **Claim**, Argument, Synthesis |
| Recouvrement | **`Claim` seulement (par le nom)** | idem |
| Même `Claim` ? | `{claim_text, supporting_evidence[], confidence}` | `CanonicalClaim {claim, evidence[ClaimEvidence], confidence, domain}` |

Le seul schéma commun **par le nom** (`Claim`) a des **champs différents** (`claim_text` vs `claim`,
`supporting_evidence` vs `evidence`, pas de `domain` côté SLM). Et le SLM **ne connaît ni Plan, ni Code,
ni Argument, ni Synthesis**. ⟹ **Le SLM, avec sa grammaire actuelle (`reformulator_v1.gbnf`, orientée
citations/facts), ne peut PAS produire les formes que `relay`/`vote`/`debate` attendent.** Réutiliser
`ReformulatorClient` tel quel donnerait du JSON hors-schéma que `plan_json`/`CanonicalClaim` ne
désérialiseraient pas.

**Conséquence de conception :** Fil B = **(seam Rust) + (asset SLM neuf)**. L'asset (grammaire GBNF +
prompt) doit émettre les **5 formes `CanonicalX` du multi-llm**, discriminées par `target_schema_id`. Pas
d'entraînement (GBNF = décodage **contraint** ; la grammaire *définit* la forme, le prompt guide) — mais
authorat de grammaire réel + tests live.

**(b) Asymétrie sync/async.** `CanonicalReformulator::route` est **sync** ; `ReformulatorClient::invoke`
est **async** (I/O vers llama-server). ⟹ le trait d'abstraction (§3.1) doit être **async**, et les
call-sites des missions passent de `reformulator.route(…)?` à `reformulator.route(…).await?`.

**(c) Latence & budget.** Le SLM ajoute un aller-retour `llama-server /completion` **par hop de
reformulation** : `relay` = 3 (plan/code/synth), `vote` = N (un par fournisseur), `debate` = 2/round. À
comptabiliser dans `deadline`/budget (Qwen3-0.6B est rapide, mais ce sont N appels réels de plus).

---

## 3 · La décision de conception

### 3.1 · L'abstraction : un trait `Reformulator` async, object-safe
Nouveau trait dans `efe-multi-llm` (crate qui possède déjà `CanonicalReformulator` et les types
`CanonicalX`). Object-safe via `#[async_trait]` (dépendance déjà présente, cf. `NanoRobotClient`).

```rust
// crates/efe-multi-llm/src/reformulator_ext/mod.rs (nouveau)
#[async_trait::async_trait]
pub trait Reformulator: Send + Sync {
    /// Le seam unifié (relay + vote). Retourne la forme canonique typée sérialisée.
    async fn route(&self, raw: &str, from: SourceDialect,
                   target: CanonicalSchemaId, llm_source: &str) -> MultiLlmResult<serde_json::Value>;

    /// Debate a besoin de la stance ; méthode dédiée (pas dans `route`).
    async fn to_argument_value(&self, raw: &str, stance: ArgumentStance,
                               from: SourceDialect) -> MultiLlmResult<serde_json::Value>;

    /// Étiquette de provenance : quel moteur a effectivement produit la dernière sortie.
    fn engine_tag(&self) -> ReformulatorEngine;   // Rule | Slm | SlmDegradedToRule
}
```

`route` renvoie `serde_json::Value` (aligné sur `CanonicalReformulator::route` **et** sur
`ReformulatorOutput.canonical`). `engine_tag` sert la provenance (§6).

### 3.2 · Deux implémentations
**(1) `CanonicalReformulator`** (déjà là) implémente `Reformulator` : `route`/`to_argument_value`
appellent les méthodes inhérentes existantes (travail sync enveloppé dans un futur immédiat) ;
`engine_tag = Rule`. **Zéro changement de comportement.** C'est le **défaut et le repli**.

**(2) `SlmReformulator`** (nouveau) — l'adapter du vrai SLM, avec repli intégré :
```rust
pub struct SlmReformulator {
    client:   Arc<ReformulatorClient>,   // le Qwen3-0.6B
    fallback: CanonicalReformulator,     // le repli déterministe, toujours dispo
    circuit:  Option<SlmCircuitBreaker>, // §5
    last:     ...,                        // pour engine_tag (Slm vs SlmDegradedToRule)
}
```
`route(raw, from, target, src)` :
1. Si le SLM ne **supporte pas** `target` (aujourd'hui : tout sauf éventuellement Claim ; §2), OU circuit
   ouvert → **fallback.route(...)** + `engine = SlmDegradedToRule` (dégradation par capacité).
2. Sinon : `ReformulatorInput { raw_text: raw, source_dialect: map(from), target_schema_id: map(target) }`
   → `client.invoke(input, ctx).await`.
3. Valider que `output.canonical` **désérialise** dans le `CanonicalX` attendu
   (`serde_json::from_value::<CanonicalX>(&v)`) — défense en profondeur au-delà du GBNF.
4. Succès → renvoyer `output.canonical`, `engine = Slm`. Erreur/timeout/JSON invalide → circuit.record() +
   **fallback.route(...)** + `engine = SlmDegradedToRule`.

`to_argument_value` : idem, mais la **stance** doit atteindre le SLM. Deux options (§4) : (i) préambule
dans `raw_text`, ou (ii) champ additif `stance_hint: Option<ArgumentStance>` dans `ReformulatorInput`
(non-breaking via `#[serde(default)]`). Recommandé : (ii), plus honnête et testable.

**Mapping `SourceDialect → LlmDialect`** (déjà quasi-isomorphe) : `Claude→ClaudeXml`, `Gpt→Gpt4Json`,
`Gemini→GeminiMarkdown`, autre→`Unknown`. **Mapping `CanonicalSchemaId → SchemaId`** : nécessite les
variantes multi-llm côté SLM (§3.6).

### 3.3 · Politique de sélection & dégradation : fail-closed-vers-dégradé, jamais silencieux
Principe : le SLM est un **traducteur de forme**, pas un juge. Une panne du SLM ne doit **jamais** faire
échouer une mission qui pourrait aboutir en déterministe — MAIS la dégradation doit être **visible**
(estampillée dans le reçu, §6). C'est un fail-closed **d'honnêteté** (on ne revendique pas « SLM » quand
c'était la règle), pas un fail-closed de refus (le service continue en dégradé). Table :

| Condition | Comportement | `engine_tag` |
|---|---|---|
| `clients.reformulator = None` (SLM non câblé) | reformulateur = `CanonicalReformulator` pur | `Rule` |
| SLM câblé, schéma supporté, invoke OK + désérialise | sortie SLM | `Slm` |
| SLM câblé, schéma **non supporté** | repli règle | `SlmDegradedToRule` |
| SLM invoke erreur / timeout / JSON invalide | circuit.record() + repli règle | `SlmDegradedToRule` |
| Circuit ouvert | repli règle (pas d'appel SLM) | `SlmDegradedToRule` |

### 3.4 · Le seam de construction : `Coordinator::reformulator()`
Ajouter un accesseur sur `Coordinator` qui **choisit** l'impl selon le câblage — les missions ne
construisent plus `new()` en dur :
```rust
impl Coordinator {
    pub fn reformulator(&self) -> Box<dyn Reformulator> {
        match &self.clients.reformulator {
            Some(c) => Box::new(SlmReformulator::new(c.clone(),
                          self.clients.slm_circuit("reformulator"))),  // §5
            None    => Box::new(CanonicalReformulator::new()),
        }
    }
}
```
Chaque mission : `let reformulator = coord.reformulator();` puis `reformulator.route(…).await?`. **Quand
`reformulator = None` (défaut, y compris tout le chemin servi actuel), le comportement est byte-identique
à aujourd'hui** (même `CanonicalReformulator`, juste derrière un `Box<dyn>` + `.await` sur un futur
immédiat).

### 3.5 · Le wiring servi : `coordinator_from_env`
Derrière la feature `frontier-llm` **et** un flag/URL dédiés, construire le SLM et l'injecter :
```rust
// frontier.rs (esquisse)
let mut clients = CoordinatorClients::default();
if let Ok(url) = std::env::var("EFE_SLM_REFORMULATOR_URL") {          // nouvel opt-in explicite
    match ReformulatorClient::from_assets(url) {
        Ok(c)  => { clients.reformulator = Some(Arc::new(c));
                    clients.slm_circuits.insert("reformulator".into(),
                        SlmCircuitBreaker::new(3, Duration::from_secs(10))); }  // §5
        Err(e) => tracing::warn!(%e, "SLM reformulateur configuré mais from_assets a échoué — reste en déterministe"),
    }
}
Coordinator::builder().registry(registry).policies(policies).clients(clients).build()
```
**Gardes :** URL absente ⟹ pas de SLM (déterministe, inchangé). `from_assets` échoue (assets manquants /
robot injoignable au boot) ⟹ **warn + déterministe**, jamais un boot cassé. `EFE_SLM_REFORMULATOR_URL`
est un **opt-in explicite**, additionnel à la triade frontière déjà exigée.

### 3.6 · L'asset SLM (le cœur « petit MODÈLE ») — B.1
Pour que le SLM émette réellement les formes multi-llm, il faut des **assets neufs** (aucun entraînement) :
- **Étendre `SchemaId`** (efe-nano-app-coordinator, additif) avec `PlanV1, CodeV1, ArgumentV1,
  SynthesisV1` (ou définir un `SchemaId` multi-llm dédié pour ne pas polluer l'app provenance ; décision
  ouverte §11). Le `ClaimV1` du SLM doit être **réaligné** sur `CanonicalClaim` (champs `claim`/`evidence`/
  `domain`) — ou le mapping fait la conversion `claim_text→claim`.
- **Nouvelle grammaire GBNF** `reformulator_multillm_v1.gbnf` : une **union** des 5 formes `CanonicalX`,
  discriminée par le `target_schema_id` porté dans le prompt (le client charge **une** grammaire au
  `from_assets`, donc l'union est nécessaire ; alternative §11 = 5 clients).
- **Nouveau prompt** `reformulator_multillm.txt` : instruit « convertis la sortie <dialecte> vers la forme
  canonique <schema> », few-shots par (dialecte × schéma).
- **Schéma JSON** `reformulator_multillm_v1.json` (miroir des `CanonicalX`) pour la validation.
B.1 est **la vraie substance « petit modèle entre gros modèles »** ; B.0 (le seam Rust) est le prérequis
qui la rend enfichable et sûre.

---

## 4 · Contrats d'interface (signatures proposées, exactes)

```rust
// efe-multi-llm/src/reformulator_ext/engine.rs (nouveau)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ReformulatorEngine { Rule, Slm, SlmDegradedToRule }

// efe-multi-llm/src/reformulator_ext/mod.rs (trait ci-dessus §3.1)

// efe-multi-llm/src/reformulator_ext/slm.rs (nouveau)
impl SlmReformulator {
    pub fn new(client: Arc<ReformulatorClient>, circuit: Option<SlmCircuitBreaker>) -> Self;
    fn supports(target: CanonicalSchemaId) -> bool;     // capacité (dépend des assets B.1)
    fn map_dialect(d: SourceDialect) -> LlmDialect;
    fn map_schema(s: CanonicalSchemaId) -> Option<SchemaId>;
}

// efe-nano-app-coordinator/src/types/reformulator.rs (additif, non-breaking)
pub struct ReformulatorInput { …, #[serde(default)] pub stance_hint: Option<String> }  // pour debate
pub enum SchemaId { …, PlanV1, CodeV1, ArgumentV1, SynthesisV1 }                        // B.1, additif

// efe-coordinator/src/coordinator.rs (additif)
impl Coordinator { pub fn reformulator(&self) -> Box<dyn efe_multi_llm::Reformulator>; }
```

> Note de dépendance : `Coordinator::reformulator()` renvoie un type de `efe-multi-llm`, or `efe-multi-llm`
> **dépend de** `efe-coordinator` (pas l'inverse) ⟹ **cycle**. Décision : le trait `Reformulator` et les
> impls vivent dans `efe-multi-llm` ; l'accesseur `reformulator()` vit **aussi** dans `efe-multi-llm`
> comme fonction libre `pub fn reformulator_for(coord: &Coordinator) -> Box<dyn Reformulator>` (lecture de
> `coord.clients.reformulator`), **pas** comme méthode de `Coordinator`. Évite tout cycle. (À valider §11.)

---

## 5 · Comportement fail-closed / dégradation + circuit breaker

- **Circuit dédié `"reformulator"`** ajouté à `slm_circuits` (aujourd'hui le reformulateur n'est pas
  protégé, cf. `coordinator.rs:101`). Seuil 3 échecs / reset 10 s (aligné sur les 3 autres SLM). Ouvre ⟹
  repli règle sans appel réseau.
- **Timeout dur** par invoke (deadline dérivée du budget mission restant) — un SLM lent ne doit pas
  manger la deadline des gros modèles.
- **Validation de sortie** : même GBNF-contraint, on re-désérialise dans `CanonicalX` avant de faire
  confiance (défense en profondeur) ; échec ⟹ dégradation.
- **Jamais fail-open** : une sortie SLM invalide n'est jamais renvoyée telle quelle ; on dégrade vers la
  règle (connue-bonne) et on **estampille**.

---

## 6 · Provenance / honnêteté sur le fil

Le reçu re-vérifiable des missions (Couche-2) doit désormais porter, **par hop de reformulation**, le
`ReformulatorEngine` effectif (`Rule` / `Slm` / `SlmDegradedToRule`). Concrètement : les événements
`publish_reformulation(...)` (déjà émis, `relay.rs:76`) gagnent un champ `engine`. Ainsi :
- On ne peut revendiquer « **relais médié par Qwen3-0.6B** » que pour les hops estampillés `Slm`.
- Une exécution où le SLM était down apparaît `SlmDegradedToRule` — impossible de la présenter comme
  SLM-médiée. **C'est le garde-fou d'honnêteté central de Fil B.**

---

## 7 · Change-set précis (préparation — fichiers & fonctions)

| Sous-jalon | Fichier | Changement |
|---|---|---|
| **B.0** trait+adapter | `efe-multi-llm/src/reformulator_ext/mod.rs` | `trait Reformulator` (async) + `ReformulatorEngine` |
| | `…/reformulator_ext/canonical.rs` | `impl Reformulator for CanonicalReformulator` (délègue aux méthodes existantes) |
| | `…/reformulator_ext/slm.rs` (neuf) | `SlmReformulator` (adapter + repli + circuit + validation) |
| | `efe-multi-llm/src/lib.rs` | `reformulator_for(coord: &Coordinator) -> Box<dyn Reformulator>` |
| | `…/workflows/{relay,vote,debate}.rs` | remplacer `CanonicalReformulator::new()` par `reformulator_for(coord)` ; `.await` sur `route`/`to_argument_value` |
| **B.1** asset SLM | `efe-nano-app-coordinator/src/types/reformulator.rs` | `SchemaId` += {Plan/Code/Argument/Synthesis} ; `ReformulatorInput.stance_hint` (additifs) |
| | `efe-nano-app-coordinator/assets/grammars/reformulator_multillm_v1.gbnf` (neuf) | union GBNF des 5 `CanonicalX` |
| | `…/assets/prompts/reformulator_multillm.txt` (neuf) | prompt few-shot par (dialecte × schéma) |
| | `efe-nano-app-coordinator/src/clients/reformulator.rs` | `from_assets_multillm(endpoint)` (charge les assets B.1) |
| **B.2** wiring servi | `efe-swarm-gateway/src/frontier.rs` | `coordinator_from_env` : `EFE_SLM_REFORMULATOR_URL` → `clients.reformulator` + circuit + `.clients(...)` |
| | `efe-swarm-gateway/Cargo.toml` | s'assurer que la feature tire les assets/déps SLM nécessaires |
| **B.3** provenance | `efe-multi-llm/src/events_ext.rs` | `publish_reformulation(..., engine)` ; sceller `engine` dans le reçu |
| **B.4** résilience | `efe-coordinator/src/coordinator.rs` | (rien de structurel ; circuit ajouté via builder en B.2) |

---

## 8 · Jalons (chacun fail-closed, testable, byte-identique quand SLM off)

- **B.0 — Seam Rust (SLM path dormant). ✅ IMPLÉMENTÉ & VÉRIFIÉ (2026-07-14, commit-pending).** Trait +
  2 impls (dont `SlmReformulator` qui, faute d'assets, `supports()==false` pour tout ⟹ dégrade toujours
  vers la règle) + threading dans les missions + provenance `engine`. **Invariant :** sans
  `clients.reformulator`, sortie strictement inchangée (le déterministe, juste boxé/awaité). **10 fichiers
  dans `crates/efe-multi-llm/`** (7 modifiés + `reformulator_ext/engine.rs`, `reformulator_ext/slm.rs`,
  `tests/unit_reformulator_pluggable.rs`). Vérif : `efe-multi-llm` 0-fail (lib 101 dont slm/engine +
  non-régression) ; `efe-swarm-gateway --features frontier-llm` 0-fail (114 + e2e). Note d'impl : les
  méthodes du trait sont `to_canonical` / `to_argument_value` (distinctes des inhérentes `route` /
  `to_argument`) pour éviter l'ambiguïté de résolution ; non-régression valeur-égale testée sur
  `ClaimV1`/`CodeV1` (déterministes) car `CanonicalPlan` porte un `Uuid` frais par appel.
- **B.1 — Assets SLM multi-llm. ✅ IMPLÉMENTÉ & VÉRIFIÉ (2026-07-14, commit-pending).** Grammaire-union
  `reformulator_multillm_v1.gbnf` (l'enveloppe LIE chaque `schema_id` à sa forme `CanonicalX` exacte —
  mismatch impossible par construction ; non-vacuité `json-string-1` sur claim/thesis/summary/action,
  alignée sur le contrat minLength 1 des schémas canoniques maison) + prompt + schéma JSON avec 5
  exemples = contrat inter-crates testé (`unit_slm_assets.rs` les désérialise dans les vrais
  `CanonicalX`). `SchemaId` +4 variants (additif ; le validateur provenance refuse explicitement le
  profil multi-llm — `CanonicalError::MultiLlmProfile`). `ReformulatorProfile {ProvenanceV1, MultiLlmV1}`
  + `from_assets_multillm` (n_predict 1536, **timeout 30 s** aligné sur le budget — revue B.1 #2) ;
  `schema_version()` délégué à l'inner (revue #4/6/9). Backend réveillé : `supports()` = profil
  multi-llm ; `reformulate()` = input+clamp(15 500 chars = sémantique `chars` de validator 0.18)+
  `CallContext`+invoke+écho-de-schéma. Dégradations **observables** (`tracing::warn` par raison) + seam
  d'audit `from_client_with_audit` (B.3). Testé hors-ligne jusqu'à l'écho et au clamp via un **faux
  llama-server** (revue #7) ; la qualité de contenu reste **live-only** (`#[ignore]`, llama-server +
  poids requis). Revue adverse : 10 findings bruts → 9 confirmés (dont 3 doublons) → **7 corrigés**.
- **B.2 — Wiring servi opt-in.** `coordinator_from_env` construit le SLM depuis l'env, derrière feature +
  URL. Défaut = déterministe, inchangé.
- **B.3 — Provenance estampillée.** `engine` par hop dans le reçu ; revendication « SLM-médié » gardée.
- **B.4 — Circuit + timeouts.** Résilience du hop reformulation.

Ordre imposé : B.0 (sûr) → B.1 (substance modèle) → B.2 (servi) → B.3/B.4 (honnêteté + résilience).

---

## 9 · Stratégie de test

- **Unit (B.0)** : `SlmReformulator` avec un **client mock** (double du `NanoRobotClient`) — succès →
  `Slm` ; erreur → `SlmDegradedToRule` + sortie = celle de la règle ; schéma non supporté → dégradation ;
  `engine_tag` correct. `CanonicalReformulator impl Reformulator` : parité avec les méthodes inhérentes.
- **Invariant de non-régression** : mission `run_relay`/`vote`/`debate` avec `reformulator = None` ⟹
  sortie **identique** à la version pré-Fil-B (fixer un golden ; provider mockés).
- **Integration** : un `MockLlmProvider` pour les gros + un `SlmReformulator` sur client mock renvoyant
  du `CanonicalPlan` valide ⟹ `relay` bout-en-bout estampille `Slm` sur les 3 hops.
- **Live (B.1, `#[ignore]`)** : llama-server réel + assets multi-llm ⟹ GBNF émet des `CanonicalX`
  désérialisables pour chaque (dialecte × schéma).
- **Suites servies** : `efe-swarm-gateway` — `no_llm_routes_when_frontier_off` inchangé ; nouveau
  `reformulator_slm_stamped_when_url_set` (avec SLM mock d'intégration).

---

## 10 · Modèle d'honnêteté (ne jamais sur-vendre)

- Le SLM fait de la **normalisation dialecte→schéma** (traduction de forme), **pas** du raisonnement ni du
  fact-checking. Le dire ainsi.
- « Relais médié par un petit modèle (Qwen3-0.6B) entre Claude/GPT/Gemini » n'est vrai **que pour les hops
  estampillés `Slm`**. Toute dégradation (`SlmDegradedToRule`) doit être dite ; ne jamais présenter une
  exécution dégradée comme SLM-médiée.
- Prérequis à énoncer à Kovant : **poids non embarqués** (llama-server à faire tourner) ; **grammaire
  exercée seulement en tests `#[ignore]`** tant qu'un runner GPU/Metal n'est pas en CI ; **latence** +N
  appels par mission ; la couverture réelle dépend de **quels schémas B.1 a livrés** (au départ, aucun —
  d'où B.0 dégrade tout).
- Fil B **n'ajoute pas** de robustesse d'agrégation (le vote reste une pluralité/Borda ; cf. la dette
  connue) ni ne rend le LLM participant du méta-essaim (non-but).

---

## 11 · Risques & questions ouvertes

1. **Cycle de crates.** L'accesseur `Coordinator::reformulator()` créerait un cycle (`efe-coordinator` →
   `efe-multi-llm`). **Décision retenue :** fonction libre `reformulator_for(coord)` dans `efe-multi-llm`.
   À revérifier au build.
2. **Union GBNF vs 5 clients.** Une grammaire-union (un client) est plus simple à câbler mais plus dure à
   écrire ; 5 clients (un par schéma) simplifient la grammaire mais changent la structure
   `Option<Arc<ReformulatorClient>>` en map. **Recommandation :** union GBNF d'abord (garde le champ
   unique) ; réévaluer si la grammaire devient ingérable.
3. **Pollution du `SchemaId` de l'app provenance.** Ajouter Plan/Code/… au `SchemaId` de
   `efe-nano-app-coordinator` mélange deux domaines. **Alternative :** un enum `MultiLlmSchemaId` dédié +
   un second type de client. Décider en B.1.
4. **Réalignement de `ClaimV1`.** Faut-il réaligner le `ClaimV1` du SLM sur `CanonicalClaim`, ou mapper ?
   Mapper est moins invasif (n'affecte pas l'app provenance) — **recommandé**.
5. **Stance de debate.** Champ additif `stance_hint` vs préambule prompt — le champ est plus propre et
   testable.
6. **CI live.** Sans runner llama-server en CI, B.1 reste `#[ignore]` — la garantie « SLM émet du
   `CanonicalX` valide » est **property-testée hors-ligne sur assets figés**, pas vérifiée en CI. À dire.
7. **Déterminisme.** temp 0.1 + GBNF ⟹ quasi-déterministe mais **pas** bit-identique ; ne pas revendiquer
   un replay identique du hop SLM (contrairement au chemin latent Φ).

---

## 12 · Definition of Done

- **B.0** mergé : trait + 2 impls + threading + provenance `engine`, suite existante verte, **invariant
  byte-identique** (SLM off) prouvé par golden.
- **B.1** : assets multi-llm livrés ; `supports()` couvre ≥ {Plan, Claim, Synthesis} (relay + vote) ;
  tests live `#[ignore]` verts sur un llama-server local.
- **B.2** : `EFE_SLM_REFORMULATOR_URL` câble le SLM dans le coordinator servi derrière la feature ; défaut
  inchangé ; boot jamais cassé si assets/robot absents.
- **B.3** : reçu estampille l'`engine` par hop ; test `reformulator_slm_stamped_when_url_set`.
- **B.4** : circuit `"reformulator"` + timeouts ; test de dégradation (SLM down ⟹ `SlmDegradedToRule`,
  mission réussit).
- **Honnêteté** : aucune revendication « SLM-médié » non estampillée ; doc de présentation mise à jour avec
  les réserves du §10.

---

### Annexe — le chemin de l'information, après Fil B (relay, hop plan)

```
user_text
  └─(prompt planner)→  CLAUDE (gros)  ──texte dialecte ClaudeXml──┐
                                                                  ▼
                         reformulator_for(coord)  = SlmReformulator si EFE_SLM_REFORMULATOR_URL, sinon CanonicalReformulator
                                                                  │
                              ┌───────────────────────────────────┴───────────────────────────────┐
                        [SLM up + schéma supporté]                                       [sinon : dégradé]
                     ReformulatorInput{raw, ClaudeXml, PlanV1}                        CanonicalReformulator.route(...)
                     → Qwen3-0.6B (GBNF) → canonical: CanonicalPlan                    → parsing Rust → CanonicalPlan
                     engine = Slm                                                      engine = SlmDegradedToRule
                              └───────────────────────────────────┬───────────────────────────────┘
                                                                  ▼
                                          plan_json (CanonicalPlan)  ──(interpolé dans le prompt)→  GPT (gros)  → …
                                          reçu ⊕ {hop: plan, engine: <Slm|SlmDegradedToRule>}
```

Le « petit agent entre les gros » = **`SlmReformulator`** ; il ne **décide** rien, il **traduit** la forme
et **s'efface honnêtement** (dégradation estampillée) quand il n'est pas là.
