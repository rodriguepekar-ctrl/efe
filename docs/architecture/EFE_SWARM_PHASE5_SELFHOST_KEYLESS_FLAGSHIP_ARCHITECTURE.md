# EFE — Phase 5 : **Self-host keyless, le mode phare** — souverain · local · vérifiable. Garde-fous tenus.

> **Statut** : Architecture (gelée pour build) · **Date** : 2026-06-10 · **Phase** : 5 (Productisation)
> **Sur** : l'existant — `efe-swarm-sdk` (`ClientMode`, `OllamaBackend`), provenance (Inc.1), passerelle HTTP (Inc.2), dashboard live (Inc.3).
> **Pré-requis** : Inc.1 (`swarm.run` + provenance auto) · Inc.2 (`verify/:id`) · Inc.3 (dashboard).

---

## 0. Thèse — ce qu'un provider **ne peut pas** offrir

EFE expose trois modes (`Keyless`, `Self-host`, `Network`). Un seul est le **mode phare** :

> **Self-host keyless** — les agents tournent sur un **modèle local** (Ollama/llama-server) sur la
> machine du client, **sans aucune clé**, et chaque run scelle une **preuve re-vérifiable** que le
> client (et son régulateur) re-contrôle **hors-ligne**, sans faire confiance à personne.

C'est le **triptyque qu'aucun fournisseur hébergé ne peut égaler** :
1. **Souverain** — la tâche, les états intermédiaires et la sortie **ne quittent jamais l'hôte**
   (boucle locale `127.0.0.1`). **Air-gappable**.
2. **Local / keyless** — **zéro secret applicatif** : rien à fuiter, rien à révoquer, aucune
   dépendance réseau ajoutée. Le binaire tourne **coupé d'Internet**.
3. **Vérifiable** — `verify_swarm` re-prouve la chaîne + le contenu + le DAG depuis les seuls
   enregistrements + la clé tenant. Une preuve **que le client détient**, pas la parole du provider.

Le levier de valeur n'est ni le calcul (le prix des tokens s'érode) ni la coopération du provider
(impossible) : c'est la **zone vérifiable et souveraine** (H-SWARM). Self-host keyless **est** cette
zone. Les autres modes le servent ; lui est le produit.

---

## 1. Le réel — ce qui existe déjà (on ne réinvente rien)

### 1.1 Les trois modes (`efe-swarm-sdk/src/client.rs`)
```rust
pub enum ClientMode {
    Keyless,                                   // Mode A : flotte déterministe en process (1¢/appel nominal)
    SelfHost { endpoint: String, model: String }, // Mode C : LLM LOCAL (Ollama/llama-server) — 0¢, keyless
    Network  { provider: String },             // frontière (M21) : clés SERVER-SIDE, déléguées au routeur
}
impl ClientMode { pub fn is_keyless(&self) -> bool { !matches!(self, Network { .. }) } }
```
- `Keyless` **et** `SelfHost` sont **keyless par construction** (`is_keyless() == true`). `SelfHost`
  facture **0¢** (compute local) ; seul `Network` n'est pas keyless (clés au routeur, jamais ici).

### 1.2 Le backend local RÉEL (`efe-swarm-sdk/src/llm.rs`)
```rust
pub struct OllamaBackend { endpoint: String, model: String }   // keyless
// appel : curl -s --max-time 120 {endpoint}/api/generate -d @-   (JSON par stdin, AUCUNE clé)
//         + tokio::task::spawn_blocking (compatible runtime async, aucune dépendance réseau ajoutée)
```
- En `SelfHost`, le **nom d'un agent** dans `TopologySpec` est une **instruction** en langage naturel
  (générateur/juge/étage/décomposeur/solveur/agrégateur). Mode C, prouvé bout-en-bout dans
  `efe-swarm-provenance/examples/swarm_all_phases_real.rs` (Ollama local).
- **Aucune clé, aucune dépendance crate réseau** : on shell-out `curl` vers `127.0.0.1`.

### 1.3 Les piliers déjà câblés
- **Provenance auto** (Inc.1) : chaque run scelle un `provenance_root` re-vérifiable (`verify_swarm`).
- **Budget I13** (`BudgetGuard`) : la dépense **ne dépasse JAMAIS** le plafond (charge atomique +
  revert), même self-host (le plafond borne le **nombre d'appels**, donc le **compute local**).
- **Passerelle HTTP** (Inc.2) : `POST /v1/swarm/run` + `GET /v1/swarm/verify/:id` + `/metrics`,
  **servable standalone souverain** (`axum::serve`).
- **Dashboard live** (Inc.3) : on **voit** l'essaim local penser ; flux best-effort, preuve durable.
- **Pont N17** (`efe-swarm-provenance/src/bridge.rs`) : relie un record Mode C à son
  `LlmProvenanceRecord` forensique (modèle, params, hashs prompt/réponse).

---

## 2. Pourquoi self-host keyless est le **mode phare** (positionnement)

| dimension | Hosted provider (API) | **Self-host keyless (EFE)** |
|---|---|---|
| résidence des données | sort de l'hôte (TLS vers le provider) | **reste sur l'hôte** (`127.0.0.1`), air-gappable |
| secret | clé API (à gérer, fuiter, révoquer) | **aucun** (keyless par construction) |
| dépendance réseau | obligatoire (egress) | **aucune** (fonctionne coupé d'Internet) |
| preuve de ce qui a été fait | la parole du provider (logs côté serveur) | **chaîne re-vérifiable** par le client (`verify_swarm`) |
| modèle/version certifiés | opaque, peut changer sans préavis | **pont N17** : modèle + params + hashs, forensiques |
| coût | par token (s'érode, mais perpétuel) | **compute local** (matériel amorti, 0¢/appel) |
| conformité (RGPD/IA Act/souveraineté) | dépend du provider | **souveraineté native** (rien ne sort) |

> Les colonnes que les providers **ne peuvent pas** cocher (résidence locale, zéro secret, re-preuve
> détenue par le client, air-gap) **sont** la proposition de valeur. C'est pourquoi self-host keyless
> est le **phare** — pas un mode parmi d'autres.

---

## 3. Anatomie du mode phare — Mode C local **+ ancre Mode A déterministe**

Self-host keyless **n'est pas** « juste des appels LLM locaux ». C'est l'**union** de deux familles
keyless, choisie par `TopologySpec::concretize(task, &SelfHost)` :

- **Agents génératifs / juges = Mode C** (LLM local, keyless, non-déterministe) — la puissance.
- **Vérificateur / agrégateur = Mode A** (Rust pur, **déterministe**, a-posteriori) — l'**ancre
  vérifiable**. `r.verifier(...)` est **toujours** de la flotte déterministe (`resolver.rs`) ;
  l'agrégation Vote/Borda/Condorcet est de la **mathématique pure**.

```text
   tâche ──► [K générateurs LLM locaux] ──candidats──► Blackboard (/cand/*)
                                                │
            [N juges LLM locaux] ───ballots────►┤  /ballot/*  (+ bus, Inc.3)
                                                ▼
            [agrégateur DÉTERMINISTE: Vote/Borda/Condorcet] ──► réponse  +  provenance_root
```
**Conséquence clé (vérifiabilité graduée)** :
- Les **ballots** (LLM) ne sont pas reproductibles bit-à-bit — mais ils sont **captés** (provenance).
- L'**agrégation** est **déterministe** : depuis les ballots captés, la décision est **re-dérivable**
  et **re-prouvable**. Le verdict (`Decided`, Inc.3) est donc **vérifiable** même quand la génération
  ne l'est pas. La valeur (le consensus) vit dans la zone déterministe.

`Auto` en self-host concrétise déjà ce schéma (consensus N-juges à instructions LLM).

---

## 4. Le sens **exact** de « vérifiable » avec un LLM (la nuance honnête)

> **Ne jamais promettre la reproductibilité d'un LLM.** On promet la **re-preuve forensique**.

- **Keyless (Mode A déterministe)** : rejeu **bit-à-bit** (ADR-SWARM-007, clé sur la *version* du
  Blackboard, pas l'horloge) → même ensemble de records → **même `provenance_root`**. Reproductible.
- **Self-host (Mode C LLM)** : l'échantillonnage (même *seedé* : `temperature`, `num_predict`, `seed`
  exposés dans `OllamaBackend`) peut varier selon version de modèle/matériel. Donc « vérifiable » =
  **tamper-evidence + ancrage de contenu** : `verify_swarm` **recalcule** chaîne (HMAC) + contenu
  (`payload_hash` = hashs entrée/sortie/reads/writes) + DAG depuis les records + la clé. Muter une
  sortie → la chaîne casse → `verified=false`, `first_break` localise. Le **pont N17** ajoute, par
  appel Mode C, un `LlmProvenanceRecord` (modèle, params, hashs prompt/réponse) : *quel modèle a dit
  quoi, à partir de quoi*. **Re-prouvable hors-ligne, sans le provider.**

C'est **plus fort** que ce qu'offre un provider : une API renvoie une réponse **sans aucune preuve**
cryptographique du modèle/version/prompt. Self-host keyless rend cette lignée **forensique et
re-vérifiable par le client**.

---

## 5. Abstraction backend local — **tous keyless** (généralisation d'`OllamaBackend`)

Aujourd'hui : `OllamaBackend` (curl → `/api/generate`). Demain, **un trait** (sans changer le SDK
public) pour couvrir l'écosystème local souverain, **tous sans clé** (localhost, pas d'en-tête
d'auth) :

```rust
// efe-swarm-sdk : trait interne, sélection par schéma d'endpoint/modèle.
#[async_trait] pub(crate) trait LocalBackend: Send + Sync {
    async fn complete(&self, prompt: &str, p: SamplingParams) -> Option<String>; // None = échec (sentinel)
    fn descriptor(&self) -> BackendDescriptor;   // { kind, model, endpoint } → pont N17
}
pub struct SamplingParams { pub max_tokens: u32, pub temperature: f64, pub seed: u64 }
pub enum BackendKind { Ollama, LlamaCpp, Vllm, Tgi, LmStudio }   // tous LOCAUX, tous KEYLESS
```
- **Ollama** : `POST /api/generate` (présent). **llama.cpp server** : `/completion`. **vLLM/LM
  Studio/TGI** : API OpenAI-compatible **locale** (`/v1/chat/completions`) — **sans** `Authorization`
  (souverain). Détection par `endpoint`/sonde de santé.
- **Garde-fous backend** : `--max-time` (timeout), **circuit-breaker** (N échecs → sentinel neutre,
  la topologie dégrade sans dépenser), **health-check** au démarrage, **seeding** (best-effort de
  reproductibilité). **Aucune** dépendance réseau crate ajoutée (shell-out `curl`/`reqwest` local).
- **Provisioning offline** : modèles pré-téléchargés dans le rootfs / volume (air-gap) ; aucun pull
  à l'exécution. Le `BackendDescriptor` alimente le pont N17 (forensique).

---

## 6. Déploiement souverain (air-gap, on-prem, zéro egress)

```text
┌─────────────────────────  HÔTE DU CLIENT (air-gappable)  ─────────────────────────┐
│  efe-swarm-gateway (standalone, Inc.2/3)   ──run/verify/metrics/dashboard──        │
│        │  swarm.run(SelfHost{127.0.0.1:11434, gemma3:12b})                          │
│        ▼                                                                            │
│   OllamaBackend ──curl 127.0.0.1──►  Ollama / llama-server  (modèle LOCAL)          │
│        │                                                                            │
│   provenance (HKDF tenant, server-side)  ──►  provenance_root  ──►  verify (offline)│
└────────────────────────────────────────────────────────────────────────────────────┘
   AUCUN egress.  La clé tenant SIGNE la chaîne (elle n'authentifie aucun provider).
```
- **Un binaire + Ollama** : démo souveraine en deux process. `examples/serve_standalone.rs` (Inc.2) +
  `/dashboard` (Inc.3) + `selfhost_ollama.rs` (Inc.1).
- **Conformité native** : résidence des données = l'hôte ; rien à exfiltrer ; preuve détenue
  localement. RGPD/IA Act/souveraineté **par construction**, pas par contrat.
- **Helm/Docker** (différé) : `efe-swarm-gateway` + sidecar Ollama, **NetworkPolicy egress=deny**.

---

## 7. Les **garde-fous tenus** (le cœur de la promesse)

Chaque garde-fou, son **mécanisme**, et son **évidence** (test/ADR) — *tenu* en self-host keyless :

| # | garde-fou | mécanisme | évidence |
|---|---|---|---|
| **G1** | **Aucune clé applicative** | keyless par construction (`is_keyless()`); on ne saisit/lit/stocke **jamais** de clé provider ; la **seule** clé est la HKDF tenant (server-side) qui **signe** la provenance, pas une auth | `client::keyless_modes_need_no_key` ; règle absolue de session |
| **G2** | **Souveraineté des données** | boucle locale `127.0.0.1` ; ni la tâche ni les sorties ne quittent l'hôte ; air-gappable ; verify **hors-ligne** | `OllamaBackend` (curl localhost) ; export `…/records` re-vérifiable offline |
| **G3** | **Budget jamais dépassé (I13)** | `BudgetGuard` : charge atomique + revert ; self-host=0¢ mais le plafond borne le **#appels** (Stop/Degrade/Error) ; `--max-time` borne chaque appel | `budget::i13_holds_under_concurrency`, `*_caps_spend`, `*_shrinks_to_fit` |
| **G4** | **Provenance re-vérifiable** | `verify_swarm` (chaîne+contenu+DAG+root) ; pont N17 (forensique Mode C) ; aucune coopération provider | `verify::*` (falsification détectée) ; `gateway::verify_detects_tampering` |
| **G5** | **Isolation tenant** | clé **HKDF par tenant** ; run/verify/records/dashboard/registry **tenant-scopés** | `store::get_is_tenant_scoped`, `gateway::run_is_tenant_scoped`, `registry::get_is_tenant_scoped` |
| **G6** | **Déterminisme là où il compte** | Mode A + agrégation = reproductibles (ADR-007) ; LLM = tamper-evident (déclaré, pas survendu) ; seeding best-effort | §4 ; `spec::auto_is_mode_aware` |
| **G7** | **Live ≠ audit** | dashboard best-effort/lossy (`dropped` exposé) ; la vérité durable = la chaîne | ADR-SWARM-010 ; `dash` lossy tests |
| **G8** | **Bornes substrat (DoS)** | Blackboard cap octets ; bus broadcast borné ; ring+**TTL** dashboard ; **quota SSE/tenant (429)** | `substrate_hardening` ; `registry::ttl_purges_expired` ; quota SSE (Inc.3) |
| **G9** | **PII / data-rights** | `efe-pii` (optionnel par tenant) sur previews/sorties ; air-gap ⇒ pas d'exfiltration de toute façon | crate `efe-pii` ; previews tronqués (Inc.3) |
| **G10** | **Pas de réseau caché** | self-host n'ajoute **aucune** dépendance réseau crate ; `curl` vers localhost ; tourne air-gap | `llm.rs` (shell-out local), `Cargo.toml` (pas de reqwest/TLS) |

> **« Garde-fous tenus »** = ces dix invariants ne sont pas des intentions : ils sont **câblés et
> testés**. Le mode phare est *sûr par construction*, pas par discipline opérationnelle.

---

## 8. Le `Network` en contraste (pourquoi il n'est PAS le phare)

`ClientMode::Network` (frontière, M21) délègue les clés au **routeur server-side** (env/Vault) — il
**peut** être utile (modèle frontière ponctuel) mais il **rompt** G1 (clé), G2 (egress) et affaiblit
G4 (le provider est dans la boucle). En in-process, le SDK renvoie honnêtement
`NetworkRequiresGateway` ; la passerelle (qui détiendrait les clés) le câblera en Inc.4. **Network
sert des cas ; self-host keyless est le défaut souverain recommandé.**

---

## 9. Métriques & SLO (souveraineté mesurable)
| métrique | cible |
|---|---|
| **egress réseau pendant un run self-host** | **0 octet** (assertion : aucune connexion hors `127.0.0.1`) |
| clé provider manipulée | **0** (revue + grep CI : aucun `*_API_KEY` lu) |
| `verify/:id` hors-ligne (sans Internet) | **succès** (records + clé tenant suffisent) |
| dépense vs plafond (self-host, Stop) | **≤ plafond** (I13), 0¢ nominal |
| latence run self-host (gemma3:12b, consensus k=3) | **secondes** (matériel-dépendant ; mesuré, pas promis) |
| couverture provenance (`traced_pct`) | **100 %** (ancrage 2 seams) |

---

## 10. Phasage
- **Construit** : 3 modes ; `OllamaBackend` (self-host RÉEL) ; provenance auto + verify ; budget I13 ;
  passerelle standalone + dashboard ; `Auto` mode-aware (self-host → consensus N-juges).
- **Suite (cette tranche)** :
  - **S.a — durcissement garde-fous** : assertion **egress=0** (test self-host air-gap) ; circuit-breaker
    backend + health-check ; seeding exposé.
  - **S.b — abstraction `LocalBackend`** : Ollama (présent) + llama.cpp/vLLM/TGI (OpenAI-local keyless) ;
    `BackendDescriptor` → pont N17.
  - **S.c — souveraineté ops** : Helm + sidecar Ollama, `NetworkPolicy egress=deny` ; provisioning
    offline des modèles ; `efe-pii` opt-in par tenant.
  - **S.d — Lot P5 / ADR-008** : campagne self-host multi-modèles (familles/tailles) + gate.
- **Différé** : `Network` réel (Inc.4, clés routeur server-side).

---

## 11. Risques & parades
| risque | parade |
|---|---|
| Survendre la « reproductibilité » d'un LLM | §4 : on promet la **re-preuve forensique** (tamper-evidence + N17), pas le rejeu bit-à-bit ; Mode A reste reproductible |
| Fuite de données via un backend mal configuré | **localhost only** par défaut ; health-check refuse un endpoint non-local en mode souverain (option `--allow-remote-backend` explicite) |
| Clé tenant confondue avec une clé d'auth provider | doc + types : la HKDF **signe** la provenance (jamais transmise) ; aucun en-tête d'auth backend |
| Dépendance réseau cachée tirée par un crate | self-host **shell-out** `curl` local ; CI vérifie l'absence de reqwest/TLS dans `efe-swarm-sdk` |
| DoS (essaim local sature l'hôte) | budget I13 (#appels) + timeout + circuit-breaker + bornes substrat (G8) |
| « Verify » suppose Internet | **faux** : records + clé tenant suffisent ; export `…/records` pour vérif **hors-ligne** |

---

## 12. Définition de « fait »
1. **Self-host keyless est le défaut documenté** : positionnement (souverain/local/vérifiable) +
   `Auto` qui concrétise un consensus N-juges local à ancre déterministe.
2. **Les 10 garde-fous (§7) sont câblés ET testés** — keyless, souveraineté, I13, verify, isolation,
   déterminisme gradué, live≠audit, bornes, PII, zéro réseau caché.
3. **`verify` fonctionne hors-ligne** (records + clé tenant) ; **egress=0** asserté en self-host.
4. La **nuance « vérifiable »** (§4) est **honnête** : forensique re-prouvable, pas reproductibilité LLM.
5. (Suite) abstraction `LocalBackend` multi-moteurs **keyless** ; campagne Lot P5 ; ADR-008.
6. **ADR-SWARM-011** : self-host keyless est le mode phare ; garde-fous tenus.

> **Le livrable** : *un client lance l'essaim sur **son** modèle local, **sans clé**, **sans
> egress** ; son régulateur re-prouve le run **hors-ligne**, sans nous croire ni croire un provider.
> Souverain, local, vérifiable — ce qu'aucun fournisseur hébergé ne peut offrir.*
