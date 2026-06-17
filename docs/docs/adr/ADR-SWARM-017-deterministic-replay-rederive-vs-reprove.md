# ADR-SWARM-017 : Rejeu déterministe — **R0 re-prouve la racine, R1 re-dérive les sorties** ; la `provenance_root` lie l'identité par-run

- **Status** : Accepted · **Date** : 2026-06-17
- **Phase** : l'auditabilité comme endpoint (après la re-preuve d'intégrité, la re-preuve de
  reproductibilité)
- **Related** : architecture `docs/reports/EFE_Archi_Replay_Deterministe_2026-06-17.html` ;
  ADR-SWARM-006 (la provenance par-agent est le moat) ; ADR-SWARM-007 (ordre canonique sur la
  version, socle du rejeu) ; ADR-SWARM-009 (`verify/:id` = produit) ; ADR-SWARM-014 (I-CP1 :
  largeur neutre) ; ADR-SWARM-016 (N17 persisté, `verify_run_offline`) ; `replay.rs`
  (`replay`/`replay_root`/`replay_outputs`) ; `routes/replay.rs` ; Lot REPLAY (artefact
  `scripts/swarm/EFE_SWARM_REPLAY_2026-06-17.json`)

## Contexte

`GET /v1/swarm/verify/:id` **re-prouve** qu'un run n'a pas été altéré : il recalcule la chaîne
HMAC, le DAG et la forensique N17 depuis les seuls records persistés + la clé tenant. Mais
`verify` répond à « ces records sont-ils **intègres** ? », pas à « cette computation est-elle
**reproductible** ? ». Le contrôle de contenu recalcule le `payload_hash` à partir des champs
du record **lui-même** — il atteste l'auto-cohérence, jamais la fidélité à une exécution. Un
record syntaxiquement parfait mais dont la sortie n'a **jamais** été produite par l'agent passe
`verify` sans broncher. La question d'un régulateur — « si je relance, j'obtiens la même
chose ? » — restait sans endpoint, alors que le socle existait : ADR-007 (ordre canonique clavé
sur la version du substrat, pas l'horloge) et `replay.rs` (`replay`/`replay_root`/`replay_outputs`).

**Découverte du build (corrige l'architecture).** La `provenance_root` **lie l'identité
par-run** : `body_value` (record.rs:137-168) hashe `record_id` (UUID v4 par-record),
`swarm_id` (`SwarmId::new()` par-run) et `causal_parents` (record-ids par-run). Deux exécutions
de la **même** computation produisent donc des racines **différentes** (mesuré : deux `POST /run`
identiques → deux racines distinctes). La racine est de **même nature que `n17_root`** :
**re-provable** (re-sceller les mêmes records → même racine) mais **non reproductible** par
re-exécution. L'architecture initiale réclamait à tort un « Mode A bit-à-bit de la racine » ; le
build l'a réfuté empiriquement et a tranché pour le modèle honnête ci-dessous.

## Décision

1. **Deux tiers, deux questions** — `/verify` **re-prouve** (intégrité), `/replay` **re-dérive**
   (reproductibilité). **R0** (`GET /v1/swarm/replay/:id`, zéro-input, hors-ligne, gratuit) :
   re-scelle les records scellés en ordre canonique (ADR-007, `replay()`) → la racine recomposée
   doit égaler la racine stockée ; localise la 1ʳᵉ rupture de contenu ; classe chaque record par
   `DeterminismLevel`. **R1** (`POST /v1/swarm/replay/:id { task, topology, … }`) : re-exécute
   le squelette **Mode A** sur un substrat vierge et compare les **sorties** aux scellées.
2. **La racine lie l'identité par-run → re-provable (R0), pas reproductible (re-exécution)** —
   R1 ne compare **jamais** une égalité de racine fraîche. Il compare l'**empreinte de calcul
   identité-libre** de chaque record : `(agent, rôle, mode, variant, input_hash, output_hash,
   lectures, écritures)` — tout **sauf** `record_id`/`swarm_id`/`causal_parents`. Le verdict R1
   est `outputs_reproduced` (la computation se reproduit), la racine fraîche n'étant
   qu'informative. C'est l'asymétrie honnête : **la computation se reproduit, l'empreinte non.**
3. **Mode A bit-exact des SORTIES ; Mode C honnêtement classé** — Mode A (essaim Rust pur) :
   `output_hash` + DAG re-dérivés à l'identique (I-RP1). Mode C : par appel,
   `cache_hit`/`replay_of` → `Match` (re-dérivable par identité), sinon `SkippedNonDeterministic`
   — **jamais** faux `Match`. Le re-appel provider est **hors-périmètre** (coûteux, non garanti
   bit-à-bit même à graine épinglée).
4. **N17 re-prouvée, pas re-produite** — R0/R1 re-vérifient la chaîne N17 **scellée**
   (`verify_run_offline`, feuilles relues) et ne recomputent **jamais** `n17_root` (qui dépend de
   `recorded_at`). Même asymétrie que la racine d'essaim.
5. **La task n'est pas persistée → R1 la re-fournit, avec garde anti-confusion** — `StoredRun`
   ne garde ni la task, ni la `TopologySpec` complète (nom seul) ; R1 prend un `RunRequest` (le
   caller re-soumet ce qu'il détient) plutôt qu'une migration de schéma lourde. **Garde §5.1** :
   si la task re-fournie ne correspond pas aux entrées scellées (les `input_hash` des
   générateurs divergent), R1 **ne produit PAS** un faux `outputs_reproduced: false` (qui
   laisserait croire le run non reproductible) — il signale `task_mismatch: true` avec
   `outputs_reproduced: null` (erreur d'opérateur, distincte d'une vraie non-reproduction).
   *(Affinement du build : un drapeau dans le rapport, plus diagnostique qu'un 400 opaque.)*
6. **Lecture-seule, tenant-scopé, largeur neutre** — le rejeu ne mute jamais le store (R1
   s'exécute sur un Blackboard vierge, ne persiste rien) ; un run d'un autre tenant est invisible
   (404) ; rejouer à une largeur différente re-dérive les mêmes sorties (I-CP1 héritée).

## Invariants introduits (famille I-RP — 10 gardiens : 9 passerelle · 1 Mode C réel ignoré ; + socle `replay.rs`)

- **I-RP1** — *Reproductibilité Mode A des SORTIES* : R1 re-exécute le squelette Mode A sur la
  task re-fournie → re-dérive les mêmes `output_hash` + flux de données. La racine, identité
  par-run, est re-prouvée par R0, pas reproduite. Gardien :
  `replay_reexecutes_mode_a_to_identical_outputs`.
- **I-RP2** — *Divergence détectée et localisée* : toute non-reproduction (record muté → R0
  content-break ; sortie non re-dérivée → R1 output-mismatch) identifie le 1ᵉʳ record divergent.
  Gardien : `replay_detects_and_localizes_divergence`.
- **I-RP3** — *R0 hors-ligne, zéro-input* : `GET /replay/:id` re-scelle les records scellés →
  racine reproduite, sans task/provider/réseau. Gardien : `replay_r0_reseals_to_same_root_offline`.
- **I-RP4** — *Intégrité préservée* : rejeu lecture-seule ; R1 sur Blackboard vierge ; aucun
  record persisté muté. Gardien : `replay_never_mutates_the_store`.
- **I-RP5** — *Isolation tenant* : run d'un autre tenant invisible (404). Gardien :
  `replay_is_tenant_scoped`.
- **I-RP6** — *Mode C honnêtement classé* : `cache_hit`/`replay_of` → `Match` ; éphémère/seeded →
  `SkippedNonDeterministic`, jamais faux `Match`. Gardien :
  `replay_classifies_mode_c_honestly_and_reverifies_n17`.
- **I-RP7** — *N17 re-prouvée, pas re-produite* : muter un record N17 persisté → le rejeu signale
  `n17.chain_intact=false` et localise la faute (`verify_run_offline` relit les feuilles), ALORS
  QUE la racine d'essaim reste **reproduite** — preuve que la N17 est re-vérifiée à part, jamais
  recomputée dans la racine. Gardien : `replay_reverifies_n17_without_recomputing_root`.
- **I-RP8** — *Largeur neutre* (I-CP1) : rejouer à une largeur différente re-dérive les mêmes
  sorties. Gardien : `replay_is_width_invariant`.
- **Garde du contrat R1** : `replay_rejects_task_mismatch` (une autre task → `task_mismatch: true`,
  `outputs_reproduced: null` — erreur d'opérateur, jamais un faux « non reproductible »).
- **Mode C RÉEL** (`replay_real.rs`, ignoré sans Ollama) : `replay_r0_reproves_root_of_real_mode_c_run`
  — sur un run `gemma3:12b` authentique, R0 re-prouve la racine + re-vérifie N17 (le provider
  n'est pas rejoué). Socle : `replay.rs` (9 unités — ordre déterministe, racine reproductible,
  wall_at advisory, falsification détectée, live==replay).

## Verdicts des gates (exécutés le jour du build — artefact daté)

- **Lot REPLAY : GO** (`scripts/swarm/EFE_SWARM_REPLAY_2026-06-17.json`) — crypto RÉELLE
  (BLAKE3+HMAC+HKDF) + `replay`/`replay_root`/`replay_outputs` RÉELS ; agents purs synthétiques
  (Mode A, **étiqueté**). ① **R0 re-prouve la racine** (re-seal en désordre → racine identique) ;
  ② **asymétrie identité démontrée** : deux runs de la même computation → racines **différentes**,
  sorties **identiques** (mêmes empreintes de calcul) ; ③ **R1 re-dérive 8/8 sorties**
  (`replay_outputs` déterministe) ; ④ **falsification localisée** (R0 content-break + R1
  output-mismatch) ; ⑤ **coût borné** : R0 ≈ 22 µs/record, R1 ≈ 1 µs/record (budget 2000).

## Conséquences

### Positives

- **L'auditabilité devient un endpoint** : « relancez, vous retrouverez le même verdict » —
  bit-à-bit des SORTIES pour le Mode A, honnêtement classé pour le Mode C.
- **Le rejeu attrape ce que `verify` ne voit pas** : le record auto-cohérent mais **fabriqué**
  (sortie qu'aucun agent n'aurait produite) — R1 le révèle par re-exécution.
- **Modèle honnête et cohérent** : la racine (et `n17_root`) sont des **empreintes par-run**
  re-prouvables, pas des constantes inter-runs ; R0 les re-prouve, R1 re-dérive la computation.
- **Zéro primitive nouvelle, zéro champ ajouté** : ADR-007/replay.rs réutilisés tels quels.

### Négatives / risques tenus (honnêteté de portée)

- **La racine n'est PAS reproductible par re-exécution** — par conception (elle lie
  `swarm_id`/`record_id`). On re-prouve la racine (R0) et on re-dérive les sorties (R1) ; on ne
  prétend jamais à une égalité de racine fraîche.
- **Mode C non déterministe** — la sortie LLM n'est pas reproductible bit-à-bit ; seul
  `cache_hit`/`replay_of` garantit le match. Le re-appel provider est hors-périmètre.
- **Task/spec non persistées** — R1 exige que le caller les re-fournisse (contrat API), pas une
  migration de `swarm_runs`.
- **R1 ne re-exécute que le Mode A keyless** — un run contenant du Mode C n'est pas re-exécuté
  (provider hors-périmètre) ; ses appels sont classés, et R0 + N17 restent disponibles.

## Références

- Architecture : `docs/reports/EFE_Archi_Replay_Deterministe_2026-06-17.html`
- Code : `crates/efe-swarm-provenance/src/replay.rs` · `crates/efe-swarm-gateway/src/routes/replay.rs`
  · `dto.rs` (ReplayReport) · `routes/mod.rs`
- Harnais / tests : `crates/efe-swarm-provenance/examples/lot_replay.rs` ·
  `crates/efe-swarm-sdk/tests/replay_real.rs` · gardiens I-RP dans `efe-swarm-gateway/src/lib.rs`
- Artefact : `scripts/swarm/EFE_SWARM_REPLAY_2026-06-17.json`
- Lignée : ADR-006 → 007 (ordre canonique) → 009 (verify) → 016 (N17) → **017 (rejeu déterministe)**
