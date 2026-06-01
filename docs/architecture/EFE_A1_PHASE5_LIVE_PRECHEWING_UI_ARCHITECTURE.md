<!-- SPDX-License-Identifier: UNLICENSED -->
# EFE · A1 · Phase 5 — Pré-mâchage live dans l'UI Preview Lab · Architecture

> **Statut** : architecture (ultra-think, chirurgicale)
> **Phase** : 5 — câblage des vrais nano-robots SLM dans le Preview Lab
> **Type** : livrable **visible** — l'UI montre enfin le pré-mâchage RÉEL se dérouler
> **Dépend de** : Phase 4 (bridge SSE + frontend), le moteur de pré-mâchage
> (`efe-coordinator` + `efe-nano-app-coordinator`), le garde anti-garbage
> (`workflows::final_prompt_from_compression`), le protocole `StreamEvent`
> (`EVENT_SCHEMA_VERSION = 1.0.0`)

---

## 1. Résumé exécutif

La Phase 4 a livré un Preview Lab qui visualise le cycle de vie d'une mission en
SSE. Mais en mode mock, le coordinator ne câble **aucun** nano-robot SLM : les
nœuds Analyzer/Compressor/Verifier restent « idle » et le `SavingsGauge` affiche
`0 %`. Le pré-mâchage — le cœur du projet — reste invisible.

**Le constat qui définit Phase 5 : le frontend est déjà prêt.** `Pipeline.tsx`
rend déjà les 5 nœuds avec statuts + latences, piloté par `state.robots[id]` et
`state.metrics`, eux-mêmes alimentés par les events `ROBOT_STARTED/FINISHED`,
`PHEROMONE_DEPOSITED`, `PRE_CHEWING_METRICS`. Ces events ne sont juste **jamais
émis** en mock, faute de robots câblés.

> **Phase 5 = câbler `.clients()` dans le coordinator du preview-server. Les
> events réels se mettent à couler dans le SSE (déjà streamé par le bridge), et
> l'UI existante s'allume toute seule. Zéro ligne de frontend (hors extension
> optionnelle des sources, §6.2).**

---

## 2. État actuel & le gap

```
main.rs build_coordinator(mock)
 ├─ mock=true  → mock_coordinator()  → registry(mocks) .metrics()   ❌ pas de .clients()
 └─ real-llm   → real_coordinator()  → registry(real)               ❌ pas de .clients()
```

Les deux chemins assemblent un `Coordinator` **sans robots SLM**. Le routeur
étant `None`, `mission_a` tombe sur `available_llms.first()` et saute
analyzer/compresseur/verifier. D'où `savings_ratio: 0` et les nœuds idle observés
en live mock.

Le template de câblage existe déjà et est **prouvé en live** :
`crates/efe-coordinator/tests/live_missions.rs::make_coord_full_pipeline` (l.787)
— analyzer/compresseur/verifier via `from_assets`, `RouterClient::new()`.

---

## 3. Idée directrice : deux couches orthogonales

Phase 5 décompose `build_coordinator` en **deux décisions indépendantes** :

| Couche | Contrôlée par | Options |
|---|---|---|
| **LLM** (gros modèle) | `EFE_PREVIEW_MOCK` | mock (gratuit) · réel (Claude/GPT, payant) |
| **SLM** (nano-robots) | `EFE_LIVE_SLM` + santé `/health` | câblés (réels) · absents |

Matrice de 4 modes, dont un **idéal** :

| Mode | LLM | SLM | Usage |
|---|---|---|---|
| `MOCK=1` | mock | — | démo gratuite actuelle (squelette) |
| `MOCK=1` + `LIVE_SLM=1` | mock | **réels** | ⭐ **démo gratuite du VRAI pré-mâchage** (compression réelle, LLM mocké → 0 $) |
| real-llm + clés | réel | — | mode réel actuel (sans mâchage) |
| real-llm + clés + `LIVE_SLM=1` | réel | **réels** | pré-mâchage complet réel (payant) |

**Le 2ᵉ mode est la pépite** : les robots SLM tournent en local (gratuit), le LLM
est mocké (gratuit), et l'UI montre la **vraie compression en direct** sans
dépenser un centime. Mode démo par défaut recommandé.

`EFE_PREVIEW_MOCK` contrôle la couche LLM, `EFE_LIVE_SLM` la couche SLM —
strictement orthogonal.

---

## 4. Le câblage backend (cœur de Phase 5)

### 4.1 Refactor de `build_coordinator` (async, deux couches)

```rust
// main.rs — esquisse architecturale (pas l'implémentation finale)
async fn build_coordinator(mock: bool) -> (Arc<Coordinator>, Vec<String>) {
    // (a) Couche LLM : registry + ids par défaut + metrics
    let (registry, default_llms, metrics) = if mock {
        (mock_registry(), mock_default_llms(), Arc::new(MetricsRegistry::new()))
    } else {
        real_registry()  // Claude/Gemini/GPT depuis l'env
    };

    // (b) Couche SLM : clients câblés si EFE_LIVE_SLM=1 ET robots joignables.
    let clients = if slm_enabled() {
        build_slm_clients().await   // health pre-flight + from_assets
    } else {
        CoordinatorClients::default()
    };

    let coord = Coordinator::builder()
        .registry(registry)
        .clients(clients)              // ← LE wiring qui manquait
        .with_default_slm_circuits()   // ← résilience R3.S2 (circuit breakers)
        .metrics(metrics)
        .build()?;
    (Arc::new(coord), default_llms)
}
```

`build_coordinator` devient **async** (health pre-flight HTTP) — trivial depuis
`#[tokio::main]`.

### 4.2 Helper `build_slm_clients()` (porté de `make_coord_full_pipeline`)

```rust
async fn build_slm_clients() -> CoordinatorClients {
    let mut c = CoordinatorClients::default();
    // Per-robot, tolérant : on câble ce qui répond /health.
    if health_ok(analyzer_url()).await {
        if let Ok(a) = AnalyzerClient::from_assets(analyzer_url()) {
            c.analyzer = Some(Arc::new(a));
        }
    }
    if health_ok(compressor_url()).await {
        if let Ok(k) = CompressorClient::from_assets(compressor_url()) {
            c.compressor = Some(Arc::new(k));
        }
    }
    if health_ok(verifier_url()).await {
        if let Ok(v) = VerifierClient::from_assets(verifier_url()) {
            c.verifier = Some(Arc::new(v));
        }
    }
    c.router = Some(Arc::new(RouterClient::new())); // in-process, pas de /health
    c
}
```

**Décision : câblage per-robot tolérant.** Chaque robot est `Option` ; un robot
down est laissé `None` (mission_a/b le sautent). Mieux que tout-ou-rien : si seul
le verifier est down, la compression marche quand même.

### 4.3 Dépendance déjà en place

`efe-preview-server/Cargo.toml` dépend **déjà** de `efe-nano-app-coordinator` →
`AnalyzerClient`/`CompressorClient`/`VerifierClient`/`RouterClient` importables
sans nouvelle dépendance.

---

## 5. Résolution des assets & caveat de portabilité

`assets_dir()` (`efe-nano-app-coordinator/src/clients/analyzer.rs:99`) =
`env!("CARGO_MANIFEST_DIR")/assets` → chemin **absolu baké à la compilation** vers
`crates/efe-nano-app-coordinator/assets/`.

- ✅ Le binaire preview-server charge grammars/prompts **quel que soit son CWD**.
- ⚠️ **Non-portable** : déplacer le binaire sur une autre machine casse le path.
  Acceptable pour un **outil dev local** (ce qu'est le Preview Lab). Pour un
  binaire distribuable → override `EFE_SLM_ASSETS_DIR` (travail futur, §11).

---

## 6. Le flux SSE (déjà fonctionnel) + 2 gates métier

### 6.1 Flux SSE — rien à faire

```
robots SLM (8090/8092/8093)
   │ invoke() émet sur le bus :
   ▼
StreamEvent::{RobotStarted, RobotFinished, PheromoneDeposited,
              PreChewingMetrics, VerdictReady}
   │ le bridge SSE possède le MissionStream → streame TOUS les events du swarm
   ▼
data: {json}\n\n  (text/event-stream)
   │ useMissionStream → reduceMissionState
   ▼
state.robots[id], state.metrics, state.verdicts, state.overlays
   │
   ▼
<Pipeline/> <SavingsGauge/> <Verdicts/> <BlackboardPanel/>   ← DÉJÀ codés
```

Le bridge streame déjà tous les `StreamEvent` du swarm (P4.S1). Les events robot
ne sont jamais émis en mock ; câblés, ils coulent et l'UI se peuple. **Zéro
changement bridge, zéro changement frontend.**

### 6.2 Deux nœuds restent éteints à cause de gates MÉTIER (pas du câblage)

**Compresseur — seuil 500 chars.** `mission_a` ne déclenche le compresseur que si
`user_text.len() > 500`. Sur un prompt court, Compressor reste idle.
→ **UX** : pré-remplir `MissionComposer` avec un exemple long (>500 chars) ou
afficher un hint. **Pas de changement moteur.**

**Verifier + Verdicts — `candidate_sources` absent du DTO.** `MissionADto`
(`dto.rs`) n'expose pas `candidate_sources` → `into_request` met `vec![]` → le
verifier ne se déclenche jamais → nœud Verifier + panneau Verdicts vides.
→ **Petite extension DTO** : ajouter `candidate_sources: Option<Vec<{id, text}>>`
à `MissionADto` + le mapper dans `into_request` + un champ optionnel « sources »
dans le composer. **Seul bout de frontend de Phase 5, et il est optionnel.**

---

## 7. Sécurité & résilience

| Risque | Mitigation (déjà en place) |
|---|---|
| Compresseur dégénère sur input long | **Garde anti-garbage** (`final_prompt_from_compression`) → fallback brut, jamais de garbage au LLM |
| Robot lent / timeout | **SlmCircuitBreaker** (`.with_default_slm_circuits()`) → 3 échecs → Open → skip <1ms, mission continue |
| Robot down au démarrage | Health pre-flight per-robot → laissé `None`, sauté proprement |
| Robot down en cours de run | Circuit breaker + event `RobotFinished{success:false}` → l'UI montre le robot en échec (overlay) |
| Clés LLM exposées | Restent **côté serveur** (le front ne voit que des `StreamEvent`) — inchangé |

Le garde anti-garbage est précisément ce qui **sécurise** Phase 5 : sans lui, un
compresseur dégénéré aurait pollué la démo live.

---

## 8. Surface de configuration (env vars)

```
EFE_PREVIEW_MOCK=1            # LLM mocké (gratuit). Omis + real-llm + clés → LLM réel.
EFE_LIVE_SLM=1               # ← NOUVEAU : câble les robots SLM (si /health OK)
EFE_SLM_ANALYZER_URL=...      # défaut http://127.0.0.1:8090
EFE_SLM_COMPRESSOR_URL=...    # défaut http://127.0.0.1:8092
EFE_SLM_VERIFIER_URL=...      # défaut http://127.0.0.1:8093
EFE_SLM_ASSETS_DIR=...        # (optionnel, futur) override du path assets
```

Convention alignée sur `scripts/slm/` et `live_missions.rs` (mêmes ports).

---

## 9. Plan d'implémentation (~1 sprint)

| Étape | Quoi | Effort |
|---|---|---|
| **P5.1** | Refactor `build_coordinator` async + couches LLM/SLM orthogonales | S |
| **P5.2** | `build_slm_clients()` (health pre-flight + from_assets per-robot) | S |
| **P5.3** | `.with_default_slm_circuits()` + `.metrics()` dans l'assemblage | XS |
| **P5.4** | Bannière de démarrage : mode LLM × mode SLM + robots vivants | XS |
| **P5.5** *(optionnel)* | DTO `candidate_sources` → active verifier + Verdicts | S |
| **P5.6** *(optionnel)* | Prompt long d'exemple dans `MissionComposer` (>500 chars) | XS |

**Frontend : 0** (sauf P5.5 optionnel = un champ sources).

---

## 10. Stratégie de test

- **Offline** (`bridge.rs`) : `build_slm_clients()` avec endpoints injoignables →
  `CoordinatorClients::default()` (tous None) → pas de panique, fallback propre
  (wiremock pour simuler `/health` up/down).
- **Live** : `./scripts/slm/start.sh` puis
  `EFE_PREVIEW_MOCK=1 EFE_LIVE_SLM=1 cargo run -p efe-preview-server` + curl
  Mission A avec un prompt > 500 chars → vérifier le SSE contient
  `ROBOT_STARTED/FINISHED` (analyzer, compressor) + `PRE_CHEWING_METRICS` avec
  `savings_ratio > 0`.
- **Garde** : déjà couvert par les 6 `guard_tests` (`workflows::tests`) — le
  chemin compresseur du preview passe par `final_prompt_from_compression`.

---

## 11. Risques & arbitrages

| Risque | Gravité | Réponse |
|---|---|---|
| RAM (4 GGUF ~3,9 Go + le serveur) | moyenne | Mode démo n'a besoin que de 3 robots ; `start.sh --robot` ciblé |
| Path assets baké → non-portable | faible (outil dev) | Documenté ; override `EFE_SLM_ASSETS_DIR` si besoin |
| Routing avec métadonnées placeholder | faible | Router câblé mais décision peu signifiante en preview (`preview_meta` à zéro) ; le nœud Router s'allume via `selected_llm` de toute façon. Routing « réel » = travail futur (vraies métadonnées dans le DTO). |
| Compresseur plafonne > ~1k tokens | connue | Le garde retombe sur brut → démo reste correcte (montre le fallback, honnête) |

---

## 12. Runbook

```bash
# 1. Lancer les robots SLM
./scripts/slm/start.sh

# 2. Lancer le bridge en mode "vrai pré-mâchage, LLM gratuit"
EFE_PREVIEW_MOCK=1 EFE_LIVE_SLM=1 cargo run -p efe-preview-server

# 3. Lancer le frontend (déjà existant)
cd preview-lab && pnpm dev

# 4. Ouvrir http://localhost:3000, taper un texte LONG (>500 chars)
#    → Analyzer/Compressor s'allument, le SavingsGauge affiche la vraie compression
```

---

## 13. Synthèse

| Aspect | Verdict |
|---|---|
| **Nature** | Backend quasi-pur (frontend déjà prêt) |
| **Cœur** | Câbler `.clients()` dans `build_coordinator` derrière `EFE_LIVE_SLM` |
| **Pépite** | `MOCK=1 + LIVE_SLM=1` = vrai pré-mâchage visible **gratuitement** |
| **Frontend** | 0 (Pipeline/SavingsGauge/Verdicts déjà codés) ; +1 champ optionnel (sources) |
| **Sécurité** | Le garde anti-garbage protège le chemin live |
| **Effort** | ~1 sprint, dont 4 étapes XS/S backend |
| **Risque** | Faible — tout le câblage est déjà prouvé en live via `live_missions.rs` |

**Phase 5 n'est pas une construction, c'est un branchement.** Le moteur sait
pré-mâcher (prouvé), le frontend sait l'afficher (déjà codé), le bridge sait
streamer (P4.S1). Il manque juste le `.clients()` entre les deux, derrière un
flag — et le mode `MOCK + LIVE_SLM` en fait une démo gratuite et spectaculaire du
cœur du projet.
