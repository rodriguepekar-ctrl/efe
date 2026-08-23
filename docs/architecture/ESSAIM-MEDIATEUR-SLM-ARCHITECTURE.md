# Essaim Médiateur SLM — Architecture

> **Objet.** Élargir le Fil B *d'un seul hop de reformulation par arête* à *un essaim
> de petits agents SLM spécialisés* qui font transiter l'information entre agents LLM
> frontière (Claude / GPT / Gemini). Chaque petit agent reste un **hop scellé** ; une
> arête inter-LLM devient une **chaîne déclarée** de hops typés.
>
> **Statut.** Architecture ÉCRITE, **rien codé**. S'appuie sur le Fil B *codé* (B.0→B.4,
> commits `04759dd`→`6961f56`) et sur le C.1 Sandwich *conçu-non-codé*
> (`docs/architecture/C1-SANDWICH-SOUVERAIN-ARCHITECTURE.md`).
>
> **Références.** Fil B : `docs/architecture/FIL-B-reformulateur-slm-entre-frontiere-ARCHITECTURE.md`.
> Dessin de concept : `docs/reports/EFE_Essaim_Mediateur_SLM_Concept_2026-07-18.png`.
>
> **Jalons.** E.0 → E.4 (E = *Essaim*), plus E.5 futur (triage/SOMNIA). Ces jalons
> réutilisent la machinerie B.3 (provenance scellée) + B.4 (deadlines) et la
> généralisation `HopKind` du C.1.
>
> Dernière révision : 2026-07-18. Owner : R. Pekar.

---

## 0. Avertissement — il y a DEUX « essaims » dans le projet, ne pas les confondre

Le mot *essaim* désigne deux objets orthogonaux dans EFE :

1. **L'essaim latent** (nano-robots → Φ → méta-essaim) : des *lignes de tenseur* fusionnées
   par un point-fixe latent (`efe-core`, `efe-meta-swarm`). Il est **Tensor→Tensor sans
   décodeur** : il ne « passe » pas d'information linguistique, il *sélectionne* le
   consensus. C'est l'axe latent, mesuré (L̂_dyn 0,691 / L̂_fixe 0,366, β₁≈0,17 réfuté).

2. **L'essaim médiateur** — CE document. Des *petits modèles de langage* (Qwen3-0.6B et
   consorts) qui traduisent / compressent / vérifient / relaient l'information **entre**
   les gros modèles. C'est la famille **Fil B**, pas la famille latente.

Ce doc ne parle QUE du second. Aucun tenseur, aucun Φ, aucun décodeur manquant : ici tout
est texte structuré (JSON canonique) et sceaux HMAC. La seule chose commune aux deux
essaims est la *gateway* qui scelle les reçus.

---

## 1. Thèse et périmètre

**Thèse.** Une arête entre deux agents LLM n'est pas un fil nu : c'est un **interstice**
où l'on peut intercaler plusieurs petits agents, chacun faisant une opération bornée,
attribuable et scellée. Élargir Fil B, c'est passer de « un traducteur par arête » à
« une chaîne déclarée de petits agents par arête », **sans changer le contrat de reçu**
(re-vérifiable, `slm_mediated` honnête) et **sans introduire de routage émergent**.

**Ce que ça N'EST PAS** (périmètre négatif, à tenir) :

- Ce n'est **pas un routeur.** `efe-llm-router` reste rejeté : router = converger vers *un*
  fournisseur gagnant ; médier = relayer l'information *entre* eux. La distinction est le
  cœur du Fil B (cf. mémoire `efe-small-agents-between-frontier`).
- Ce n'est **pas un gain de vitesse.** Aucun benchmark temps/coût positif n'existe ;
  au contraire, le routage mesuré était **dominé** (2,68 s vs 2,50 s à qualité égale,
  rapport P2). Chaque hop supplémentaire *ajoute* de la latence. La justification est
  **fonctionnelle** (pont de dialectes, provenance, souveraineté), jamais la vitesse.
- Ce n'est **pas de l'agrégation robuste.** L'essaim médiateur ne fusionne rien ; il
  transporte. La fusion (médiane / trimmed / audit-weighted) a été réfutée ailleurs
  (HB.4, skew +8,72) et ne rentre pas ici.
- Ce n'est **pas émergent.** La chaîne de chaque arête est **déclarée** (un `EdgePlan`
  gelé par type de mission), jamais choisie à l'exécution par un modèle.

**Périmètre positif.** Traduction de dialectes (`ClaudeXml`↔`Gpt4Json`↔`GeminiMarkdown`
via une forme canonique), compression avant l'appel suivant, vérification annotée d'une
assertion, relais d'un modèle à l'autre — chaque opération portée par un petit modèle
distinct, chaînés dans un reçu unique.

---

## 2. Le substrat réel : ce que Fil B donne DÉJÀ

Point d'honnêteté capital : **l'essaim n'est pas un chantier neuf, c'est une
généralisation.** Le Fil B a déjà construit ~80 % de la plomberie. Un « essaim » est une
*chaîne* de hops, et Fil B produit déjà des *séquences* de hops (relay = 3 hops de
traduction, debate = 2·rounds). Ce qui manque n'est pas la mécanique du hop ; c'est la
*multiplicité des agents* et la *composition déclarée par arête*.

### 2.1 Le hop comme unité (déjà codé)

| Brique | Où | Rôle |
|---|---|---|
| `trait Reformulator` (async, object-safe, `deadline: Option<Duration>`) | `crates/efe-multi-llm/src/reformulator_ext/mod.rs:70` | Le contrat d'un petit agent : `to_canonical` / `to_argument_value`. |
| `enum ReformulatorEngine {Rule, Slm, SlmDegradedToRule}` | `.../reformulator_ext/engine.rs:22` | L'estampille de moteur, tags SCREAMING_SNAKE, `is_slm()`. |
| `struct Reformulated {value, engine, slm_audit?, degrade_reason?}` | `.../engine.rs:146` | La sortie d'un hop : valeur + estampille + audit + raison de dégradation. |
| `struct SlmHopAudit {entry_id, robot_id, schema_version, status, input_hash, output_hash, tokens_*, latency_ms}` | `.../engine.rs:61` | La capture par-hop, scellée dans le reçu (B.3). |
| `struct ReformulationHop {hop, from_llm, target_schema, engine, slm_audit?, degrade_reason?}` | `.../engine.rs:114` | Le hop tel qu'il voyage dans la mission. |
| `CanonicalReformulator` (déterministe, ~540 l.) | `.../reformulator_ext/canonical.rs:35` | Le repli règle-Rust : traduit sans modèle (parsing pur). |
| `SlmReformulator` + `trait ReformulatorBackend` + `NanoRobotBackend` | `.../reformulator_ext/slm.rs:401 / :155 / :211` | Le vrai petit MODÈLE (Qwen3-0.6B) + son backend mockable. |
| `reformulator_for(coord) -> Box<dyn Reformulator>` | `.../slm.rs:669` | Le sélecteur : SLM si `coord.clients.reformulator.is_some()`, sinon déterministe. |

### 2.2 Les gardes du hop (déjà codées, B.2/B.4)

L'ordre des gardes, à respecter **inchangé** dans l'essaim :
`supports(schema)` → `deadline ≥ SLM_HOP_MIN_REMAINING` → `circuit.try_call()`.

- `SLM_HOP_MIN_REMAINING = 1s` (plancher) : `.../slm.rs:196`, testé aux points de garde
  `.../slm.rs:488` et `:582`. Un hop sans temps ne consomme même pas le slot de sonde.
- `SlmCircuitBreaker::try_call()` (porte à acquisition 4-états, une seule sonde) :
  `crates/efe-nano-app-coordinator/src/clients/slm_circuit.rs:175`.
- Clé de circuit unique aujourd'hui : `SLM_REFORMULATOR_CIRCUIT_KEY = "reformulator"`
  (`.../slm.rs:67`), résolue via `coord.clients.slm_circuit(...)` (`.../slm.rs:675`).
- Les 6 `degrade_reason` stables : `deadline_exhausted`, `circuit_open`,
  `unsupported_schema`, `robot_error`, `invalid_output`, `client_side`.

### 2.3 Le reçu (déjà codé, B.3)

- `build_and_seal_mission(tenant, mission_id, key, topology, drained, reformulation_hops: &[Value], output)`
  (`crates/efe-swarm-gateway/src/mission_graph.rs:121`) crée **un nœud Mode-A `reform_{i}`
  par hop**, `output_hash = blake3(JSON du hop)`, `caused_by(dispatch_id)`, committé
  individuellement dans la chaîne HMAC.
- Le marqueur `reform_expected` scellé dans le résumé de requête déclare l'arité attendue
  (relay=3, debate=2·rounds, specialist/cascade=0, vote=absent). Absent ⟹ `None` ⟹ pas
  d'accusation rétroactive (leçon B.3).
- Le checker out-of-band `/v1/llm/audit/:id` **re-dérive l'arité déclarée** (jamais
  l'output producteur) : `crates/efe-swarm-gateway/src/store.rs:41`,
  `.../mission_graph.rs:348`.
- `slm_mediated = vrai SSI TOUS les hops de traduction sont SLM` (gelé aux hops Translate).

### 2.4 La surface servie (déjà codée, arc COUCHE2 + B.2)

- `POST /v1/llm/{vote,relay,debate,specialist,cascade}` montés via
  `.nest("/v1/llm", routes::llm::routes_llm())` (`crates/efe-swarm-gateway/src/lib.rs:122`),
  derrière feature `frontier-llm` + `EFE_LLM_FRONTIER_ENABLED=true` + triade de boot
  fail-closed (HMAC≥32o / OIDC aud=efe-llm / Postgres durable).
- Défaut = frontière NON servie : sans la feature, un POST `/v1/llm/*` répond 404
  (`.../lib.rs:1090`) ; sans clé, 501 (`.../state.rs:438`). **Invariant à préserver :
  l'essaim ne change RIEN au chemin servi par défaut.**
- Le petit modèle est câblé opt-in via `EFE_SLM_REFORMULATOR_URL` → `slm_clients_from(url)`
  dans `coordinator_from_env` (gateway, feature `frontier-llm`).

---

## 3. Le manque précis à combler

Trois écarts, et rien d'autre. Tout le reste (hop, gardes, sceau, missions, surface) est là.

**Écart 1 — un agent unique → un registre d'agents spécialisés.**
Aujourd'hui `coord.clients.reformulator` est **un seul** `Option<ReformulatorClient>` : un
Qwen qui fait la traduction de dialectes. L'essaim exige que l'arête soit servie par
*plusieurs* petits agents spécialisés (un traducteur, un compresseur, un vérificateur,
plus tard un trieur). Il faut donc remplacer le champ unique par un **registre par rôle**.

**Écart 2 — un hop par arête → une chaîne déclarée de hops typés.**
Aujourd'hui `reformulator_for(coord)` rend **un** `Box<dyn Reformulator>` et l'arête = un
hop. L'essaim veut qu'une arête inter-LLM soit une **chaîne** :
`[Translate(source→canonique), Prep(compresser), Check(vérifier), Translate(canonique→cible)]`.
C'est exactement la généralisation `HopKind {Prep, Translate, Check}` que le **C.1 a
conçue mais pas codée** (le grep confirme : aucun `enum HopKind` dans le tree). L'essaim
*consomme* `HopKind` et ajoute la notion d'arête-comme-chaîne.

**Écart 3 — la composition déclarée et scellée.**
Il faut un objet qui déclare, par arête, la liste ordonnée des hops, et le runtime qui
l'exécute en chaînant les sorties. Déclaré (pas émergent), scellé (le reçu reste
re-vérifiable), honnête (`slm_mediated` inchangé dans sa définition).

---

## 4. Le design

### 4.1 Rôles et HopKind

```rust
// NEUF (efe-multi-llm) — mappe 1:1 sur HopKind du C.1 pour le reçu.
pub enum MediationRole { Translate, Compress, Verify }   // + Triage (E.5, futur)
```

`MediationRole` correspond terme à terme au `HopKind {Prep, Translate, Check}` du C.1 :
`Translate`↔`Translate`, `Compress`↔`Prep`, `Verify`↔`Check`. On garde deux noms parce
que le C.1 raisonne « autour d'un appel frontière » (prep AVANT, check APRÈS) et l'essaim
raisonne « le long d'une arête » (rôle du petit agent). Le champ scellé dans le hop reste
`HopKind` (additif sur `ReformulationHop`, `.../engine.rs:114`), pour que les
consommateurs B.3 ne voient pas la définition bouger.

### 4.2 Le registre — remplace `Option<ReformulatorClient>`

```rust
// NEUF (efe-coordinator, sur coord.clients).
pub struct MediationRegistry {
    translate: Option<Box<dyn Reformulator>>,  // = l'actuel reformulator (Fil B)
    compress:  Option<Box<dyn Reformulator>>,  // = prep_for du C.1
    verify:    Option<Box<dyn Reformulator>>,  // = check_for du C.1
}
```

Réutilisation stricte : **le trait `Reformulator` (mod.rs:70) est déjà la bonne forme** —
async, object-safe, borné par deadline, rend un `Reformulated` estampillé. L'essaim
n'invente PAS de trait ; il enregistre *plusieurs implémenteurs par rôle*. Le traducteur
est l'agent `reformulator_for` (Fil B) ; le compresseur et le vérificateur sont les agents
`prep_for` / `check_for` (C.1). L'essaim est donc **l'union de Fil B et de C.1, organisée
en registre et composée par arête** — pas du code radicalement neuf.

**Invariant B.0 à préserver** : registre vide (aucun rôle enregistré) ⟹ aucun hop d'essaim
⟹ comportement Fil B mono-hop inchangé, sortie servie **byte-identique**. Le sélecteur
`reformulator_for` (slm.rs:669) reste le repli quand `translate` est `None`.

### 4.3 Le plan d'arête déclaré + marqueur `mediation_expected`

```rust
// NEUF. Déclaré, gelé par type de mission — JAMAIS choisi à l'exécution.
pub struct HopSpec { pub kind: HopKind, pub role: MediationRole }
pub struct EdgePlan { pub hops: Vec<HopSpec> }
```

L'`EdgePlan` généralise le marqueur `reform_expected` (aujourd'hui un simple entier :
relay=3). Il devient la **liste ordonnée** des `(HopKind, rôle)` attendus sur l'arête,
scellée dans le résumé de requête sous `mediation_expected`. Exemples de plans **gelés par
type de mission** (pas dynamiques) :

- `relay` (Claude→GPT→Gemini) : chaque arête = `[Translate]` aujourd'hui → l'essaim peut
  déclarer `[Translate, Compress]` si le prompt suivant est long, ou `[Translate, Verify,
  Translate]` si l'arête porte une assertion factuelle.
- `debate` : arête PRO/CONTRA = `[Translate]`. **Compress EXCLU** du debate (risque de
  dérive sémantique du sujet — même exclusion qu'en C.1 pour le prep).
- `vote` : `mediation_expected` **absent** ⟹ `None` ⟹ pas d'accusation rétroactive.

Absence de marqueur (runs legacy, vote) ⟹ `None` (leçon B.3, jamais d'accusation
rétroactive).

### 4.4 Le runtime de composition — `SwarmMediator::mediate`

```rust
// NEUF (efe-multi-llm). Exécute un EdgePlan, chaîne les sorties.
pub async fn mediate(
    reg: &MediationRegistry,
    circuits: &PerRoleCircuits,
    plan: &EdgePlan,
    from_llm: &str, to_llm: &str,
    input: serde_json::Value,
    edge_deadline: Option<Duration>,
) -> MediatedEdge; // { value, hops: Vec<ReformulationHop>, slm_mediated, degraded }
```

Déroulé pour une arête `A→B` de plan `[Translate(A→canon), Compress, Verify, Translate(canon→B)]` :

1. **Budget d'arête** = part du reste de mission (cf. §4.5).
2. Pour chaque `HopSpec` i du plan :
   a. deadline du hop = tranche du budget d'arête (cf. §4.5) ;
   b. sélection de l'agent par rôle dans le registre ; **absent ⟹ repli déterministe**
      estampillé `SlmDegradedToRule` (CanonicalReformulator pour Translate ; identité pour
      Compress ; « non-vérifié » pour Verify) ;
   c. **ordre des gardes inchangé** : `supports` → `deadline ≥ SLM_HOP_MIN_REMAINING`
      (slm.rs:196) → `circuits[role].try_call()` (slm_circuit.rs:175) ;
   d. exécution ; **validation typée** de la sortie (`serde_json::from_value::<CanonicalX>`
      avant de faire confiance au SLM) ; estampille `engine` + `degrade_reason` ; capture
      `SlmHopAudit` (via le `TeeAuditWriter` de B.3, ordre partagé-d'abord/capture-après) ;
   e. **la sortie devient l'entrée du hop i+1.**
3. Collecte des N hops → `Vec<ReformulationHop>` → passé tel quel à
   `build_and_seal_mission(reformulation_hops)` (mission_graph.rs:121).

`MediatedEdge.slm_mediated = hops.filter(kind==Translate).all(is_slm())` — **gelé aux hops
Translate** (décision B.3/C.1). Un hop Compress ou Verify dégradé est *enregistré* mais ne
fait PAS basculer `slm_mediated` : ce sont des annotations, pas la médiation elle-même.

### 4.5 Le partage du budget de deadline

B.4 donne à *un* hop `deadline = max_total_latency − elapsed`. Avec N hops par arête, il
faut **répartir**. Règle proposée (simple, honnête, révisable) :

- `edge_deadline` = reste de mission au moment d'entrer dans l'arête.
- Par hop : `edge_deadline` restant / nombre de hops restants (répartition uniforme
  décroissante), **plancher `SLM_HOP_MIN_REMAINING = 1s`** (slm.rs:196).
- Conséquence dite, pas cachée : **si le budget ne couvre pas la chaîne déclarée, les
  derniers hops dégradent** (estampillés `deadline_exhausted`). Une chaîne de 4 hops sous
  un budget de 3 s ne tiendra pas ses 4 hops SLM — elle en fera 2-3 puis dégradera en
  règle. C'est une contrainte réelle : **plus l'essaim est long, plus il exige du temps.**
- `BackendError::DeadlineExhausted` ne compte JAMAIS directement au circuit (impatience de
  mission ≠ maladie du robot — invariant B.4).

Le compresseur est **map-reduce au-delà de ~512 tokens** (C.1) ⟹ un hop Compress peut être
N sous-appels ⟹ son coût-temps est le plus lourd de la chaîne. La répartition uniforme est
donc un point de départ, pas un optimum (cf. §9).

### 4.6 Les circuits par rôle

Aujourd'hui **une** clé `"reformulator"` (slm.rs:67). L'essaim en veut **une par rôle** :
`"translate"`, `"compress"`, `"verify"` — pour qu'un compresseur malade ne fasse pas
tomber le traducteur. Réutilisation directe de `SlmCircuitBreaker::try_call()`
(slm_circuit.rs:175), une instance par clé, enregistrées comme les `default_slm_circuits`
du C.1 (compresseur 3/10 s, vérificateur 3/10 s). Zéro dérive : les clés sont des
constantes partagées (leçon du guard SSRF re-synchronisé, mémoire
`efe-sovereignty-guard-duplicated`).

### 4.7 La provenance — N hops scellés, contrat inchangé

Le mécanisme de sceau (mission_graph.rs:121) ne change **pas** : l'essaim produit
simplement *plus* de nœuds `reform_{i}`, chacun portant son `HopKind` (champ additif sur
`ReformulationHop`, sérialisé dans le JSON du hop). Le checker `/v1/llm/audit/:id`
(store.rs:41) est généralisé :

- `mediation_hops_recorded` vs `mediation_hops_expected` (re-dérivé du marqueur `EdgePlan`) ;
- `mediation_under_declared` si un hop déclaré manque au reçu ;
- `slm_mediated` re-calculé sur les seuls hops Translate ;
- marqueur absent ⟹ `None` ⟹ pas d'accusation (legacy/vote).

Falsifier un `HopKind` ou un stamp scellé casse la re-preuve HMAC (épinglé comme en B.3).

### 4.8 La sémantique de dégradation

Chaque hop qui ne peut pas être servi par son SLM **dégrade en repli estampillé**, la
chaîne **continue** (jamais d'abandon silencieux) :

- Translate absent/malade → `CanonicalReformulator` (déterministe), `SlmDegradedToRule`,
  `degrade_reason` renseignée. `slm_mediated` ⟹ faux (un hop Translate non-SLM).
- Compress absent/malade → **identité** (la valeur passe telle quelle), `SlmDegradedToRule`,
  `degrade_reason`. `slm_mediated` inchangé (Compress ne le gouverne pas).
- Verify absent/malade → verdict **« non-vérifié »** (annotation neutre, JAMAIS un gate —
  parité A/B stricte comme en C.1), `SlmDegradedToRule`, `degrade_reason`.

La chaîne est donc **fail-degraded, pas fail-closed** : une arête produit toujours une
sortie exploitable, et le reçu dit exactement *quels* hops ont dégradé et *pourquoi*.

---

## 5. Réutilisé tel quel vs neuf

| Réutilisé SANS modification | Où (ancré) |
|---|---|
| `trait Reformulator` (le contrat du petit agent) | `reformulator_ext/mod.rs:70` |
| `Reformulated` + `ReformulatorEngine` (estampilles) | `engine.rs:146` / `:22` |
| `SlmHopAudit` (capture par-hop, B.3) | `engine.rs:61` |
| `ReformulationHop` (le hop qui voyage) | `engine.rs:114` |
| Ordre des gardes + `SLM_HOP_MIN_REMAINING` | `slm.rs:196 / :488 / :582` |
| `SlmCircuitBreaker::try_call()` | `slm_circuit.rs:175` |
| Sceau du reçu `build_and_seal_mission` | `mission_graph.rs:121` |
| Checker `/v1/llm/audit/:id` (re-dérivation) | `store.rs:41` |
| Les 5 missions + surface `/v1/llm/*` | `lib.rs:122` |
| `LlmDialect` / `SchemaId` | `reformulator.rs:52 / :82` |
| `CanonicalReformulator` (repli Translate) | `canonical.rs:35` |

| NEUF à écrire | Nature |
|---|---|
| `HopKind` (conçu C.1, **pas codé**) + `MediationRole` | enum additif |
| `MediationRegistry` (remplace `Option<ReformulatorClient>`) | struct sur `coord.clients` |
| Les agents Compress / Verify (= `prep_for` / `check_for` C.1, **pas codés**) | 2 impls `Reformulator` |
| `EdgePlan` / `HopSpec` + marqueur scellé `mediation_expected` | déclaration gelée |
| `SwarmMediator::mediate` (runtime de composition) | fonction async |
| Partage de deadline sur la chaîne | politique |
| Circuits par rôle (`"translate"`/`"compress"`/`"verify"`) | 3 instances |
| Checker généralisé (`mediation_hops_recorded/expected`, `mediation_under_declared`) | extension du checker |

Bilan honnête : ~⅔ existe (le hop, ses gardes, le sceau, la surface). Le neuf est la
*multiplicité* (registre + circuits par rôle) et la *composition déclarée* (EdgePlan +
runtime). Aucun de ces éléments n'est un pari de recherche — ce sont des extensions
mécaniques d'un seam prouvé.

---

## 6. Où SOMNIA se branche (triage — E.5, futur, NON conçu ici en détail)

Le trieur SOMNIA (cf. `docs/reports/SOMNIA_dossier_complet_2026-07-18.pdf`, mémoire
`somnia-mlp-project`) devient un **`HopKind::Triage` en TÊTE de l'`EdgePlan`** :

- il calcule un **score OOD bon marché** sur une vue features de la modification/du message
  (métadonnées + stats d'embedding, PAS le texte brut) ;
- **in-distribution** ⟹ la chaîne saute le hop Verify coûteux (fast-path) ;
- **anormal** ⟹ la chaîne force le Verify complet OU escalade humaine (= Mode-3 d'EFE) ;
- il est lui-même un hop scellé (`TriageFacts { score, verdict, routed_to }` dans le reçu).

**Interdit dit d'emblée** : le triage **réduit le coût, il n'établit PAS la confiance** —
un poison in-distribution passe le fast-path. C'est de la défense-en-profondeur, jamais le
canal gold. Sa structure économique est identique à la loi de bascule du routeur mesurée
(`C > K/(1−f)`), mais sur la *vérification* au lieu de l'inférence. **Non conçu en détail
ici, non codé.** E.5 aura son propre doc.

---

## 7. Interdits d'honnêteté (à graver dans le code, pas seulement dans le pitch)

1. **Pas de routage émergent.** L'`EdgePlan` est déclaré et gelé par type de mission.
   Aucun modèle ne choisit la chaîne à l'exécution. `efe-llm-router` reste rejeté.
2. **Pas un gain de vitesse.** Aucun chiffre temps/coût positif ne doit être revendiqué.
   Le routage était dominé (2,68 s vs 2,50 s). Justification = fonction, jamais vitesse.
3. **Chaque hop est lossy mais attribuable.** La compression est *lossy et dite*
   (attribuable, pas nulle) ; jamais « sans perte ». Le verdict de vérification est
   l'avis d'un Phi-4-mini contraint (similarité auto-déclarée), **jamais un gate**.
4. **`slm_mediated` gelé aux hops Translate.** Compress/Verify dégradés sont enregistrés
   mais ne le font pas basculer. `slm_mediated = vrai` SSI *tous* les Translate sont SLM.
5. **Invariant byte-identique.** Registre vide ⟹ chemin servi inchangé (invariant B.0).
6. **Fail-degraded, pas fail-closed.** Une arête produit toujours une sortie ; le reçu dit
   quels hops ont dégradé et pourquoi (6 `degrade_reason` stables).
7. **Pas de nano-bot de sécurité fantôme.** Le trieur SOMNIA (E.5) n'est pas un garde de
   confiance ; c'est un réducteur de coût qui route la vérif. Ne jamais le présenter comme
   « il empêche l'empoisonnement ».
8. **Deux projets à zéro validation gold ne se dé-risquent pas en s'empilant.** Si E.5
   branche SOMNIA, dire que son canal gold est simulation-seule, comme celui d'EFE.

---

## 8. Jalons (E = Essaim ; élargissent B.0→B.4, réutilisent C.1)

- **E.0 — Seam registre.** `MediationRole` + `MediationRegistry` remplaçant le champ unique
  `Option<ReformulatorClient>` sur `coord.clients` ; sélection par rôle ; **byte-identique
  quand seul Translate est enregistré** (invariant B.0). Mergeable seul, aucun changement
  de comportement servi.
- **E.1 — Agents Compress + Verify.** Enregistrer les agents `prep_for` / `check_for` du
  C.1 sous les rôles Compress/Verify (dépend de C.1.b/c ; assets GBNF compresseur +
  vérificateur). Tests live `#[ignore]` (llama-server requis, poids non embarqués).
- **E.2 — Plan + composition.** `EdgePlan`/`HopSpec` + marqueur scellé `mediation_expected`
  + `SwarmMediator::mediate` (partage de deadline, circuits par rôle, gardes B.4 par hop,
  chaînage des sorties). Repli déterministe estampillé par rôle.
- **E.3 — Provenance N-hops.** Sceau de N nœuds `reform_{i}` avec `HopKind` ; checker
  `/v1/llm/audit/:id` généralisé (`mediation_hops_recorded/expected`,
  `mediation_under_declared`) ; épingler qu'un `HopKind` falsifié casse la re-preuve.
- **E.4 — Surface + revue.** Câbler les arêtes des missions relay/debate en chaînes
  médiées ; revue adverse (viser le pattern find→refute-by-default) ; audit d'honnêteté
  (les 8 interdits du §7) ; borne « fonction-pas-vitesse » dans la doc servie.
- **E.5 (futur) — `HopKind::Triage` / SOMNIA.** Doc séparé. Route fast-path / gold /
  humain. Non conçu ici.

---

## 9. Questions ouvertes (à trancher avant E.2)

1. **Politique de partage du deadline.** Uniforme (simple) vs pondérée par coût de rôle
   (le compresseur map-reduce est le plus lourd) ? La pondération est plus juste mais
   introduit un paramètre à calibrer — et rien n'est mesuré.
2. **Qui déclare l'`EdgePlan` ?** Défaut gelé par type de mission (relay vs debate ont des
   chaînes différentes) OU déclaration par l'auteur de la mission dans la requête ? Le
   défaut gelé est plus sûr (pas d'entrée émergente) ; la déclaration par requête est plus
   flexible mais rouvre la porte au routage déguisé.
3. **Ordre Compress / Verify dans la chaîne canonique.** Compresser AVANT vérifier perd de
   l'information que le vérificateur pourrait vouloir ; vérifier AVANT compresser vérifie
   une forme qui n'est pas celle qui atteint le modèle suivant. Tension réelle, à trancher
   par cas d'usage (probablement : Verify sur le canonique, Compress juste avant le
   Translate de sortie).
4. **Opt-out par arête.** Certaines arêtes ne doivent pas être compressées (debate =
   dérive sémantique du sujet, déjà exclu en C.1). Faut-il un opt-out par requête, ou
   suffit-il des défauts gelés par type de mission ?
5. **`with_timeouts` du compresseur.** Le compresseur a un timeout interne 5 s/appel
   (C.1) et non `with_timeouts` ; sous map-reduce (N sous-appels) le budget d'arête doit
   couvrir N×5 s au pire — à réconcilier avec le partage du §4.5.

---

## 10. Note de périmètre

Ce document décrit un **élargissement mécanique d'un seam prouvé**, pas une capacité
nouvelle non validée. La valeur défendable devant Kovant est : *« chaque échange entre
deux modèles frontière devient une chaîne de petits agents scellée, attribuable et bornée
en temps — un interstice instrumenté, pas un fil nu »*. La valeur NON défendable (et donc
interdite au pitch) est tout gain de vitesse, toute agrégation robuste, tout « ça empêche
l'empoisonnement ». Le reçu re-vérifiable est le produit ; la vitesse ne l'est pas.
