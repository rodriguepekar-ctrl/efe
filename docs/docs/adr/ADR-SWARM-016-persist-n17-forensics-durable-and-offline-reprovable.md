# ADR-SWARM-016 : Persister la forensique **N17** = preuve par-appel **durable** + `verify_bridge` **hors ligne** — chaîne N17 parallèle, re-preuve tierce sans provider

- **Status** : Accepted · **Date** : 2026-06-15
- **Phase** : la clôture du pont N17 (la forensique LLM par-appel devient durable, pas
  seulement vivante)
- **Related** : architecture `docs/reports/EFE_Archi_Persistance_N17_2026-06-15.html` (le
  build suit cette archi, commit `a6bab16`) ; `EFE_Partie7-8_Provenance_Moat_N17_2026-06-11`
  (le pont N17 en vol) ; ADR-SWARM-006 (la provenance par-agent **est** le moat) ;
  ADR-SWARM-009 (`verify/:id` **est** le produit) ; ADR-SWARM-007 (ordre canonique,
  rejeu sur version) ; `n17_chain.rs` · `store.rs` · `store_pg.rs` · `routes/verify.rs` ·
  `routes/records.rs` ; Lot N17PERSIST (artefact `scripts/swarm/EFE_SWARM_N17PERSIST_2026-06-15.json`)

## Contexte

Le pont N17 produisait, **en vol**, un `LlmProvenanceRecord` forensique par appel LLM
(Mode C), relié à son `AgentCallRecord` d'essaim par `llm_provenance_id`. Mais cette
forensique **mourait avec le process** : `StoredRun::from_outcome` **jetait**
`outcome.llm_records` à la persistance ; `GET /v1/swarm/verify/:id` ne rejouait que
`verify_swarm` (l'essaim), **jamais** le pont ; aucune colonne PG ne portait la
forensique. Résultat : sitôt le run terminé, `llm_provenance_id` devenait un **pointeur
pendant** — il désignait un record que plus personne ne détenait. Le moat (ADR-006/009)
re-prouvait l'essaim mais **pas** ce que chaque agent LLM avait réellement fait. Un
auditeur, des jours plus tard, ne pouvait pas re-prouver « ce modèle, ce prompt, ce coût,
cette sortie » : l'évidence Mode C n'avait pas survécu. Le propriétaire a ordonné le build
sur l'architecture du 2026-06-15.

## Décision

1. **Une chaîne N17 PARALLÈLE, pas une fusion** — le format d'`AgentCallRecord` reste
   **GELÉ** (zéro champ ajouté). On scelle une **seconde** chaîne HMAC, indépendante de la
   chaîne d'essaim, sur les `LlmProvenanceRecord` : feuille = `sha256(canonical_bytes(record))`
   (déterministe, tout le record), ordre **canonique par `provenance_id`**, racine `n17_root`
   = HMAC chaîné des feuilles sous **la même clé tenant** (HKDF). `n17_chain.rs`
   (~430 lignes, tests compris) : `seal_n17`, `verify_n17_chain`, `verify_run_offline`,
   `SealedN17`, `N17ChainVerdict`, `N17Break {RecordContentMutated, ChainRootMismatch,
   CountMismatch}`, `OfflineVerdict`.
2. **Sceller au SDK, là où vit la clé du run** — `RunOutcome` porte désormais
   `n17_sealed: SealedN17` (calculé dans `run_traced` par `seal_n17(&llm_records, &self.key)`,
   exactement comme `provenance_root`). La passerelle n'a pas à connaître la clé pour
   persister ; elle reçoit feuilles + racine déjà scellées. **Empreinte par-run assumée** :
   `n17_root` agrège des champs par-run (`recorded_at`, `trace_id` v4) — il n'est pas
   reproductible d'un run à l'autre, mais il est **re-prouvable** : les records persistés
   recomposent **exactement** la racine stockée.
3. **Persister la forensique à côté de l'évidence** — `StoredRun` conserve `llm_records`
   (serde), `n17_leaves` (hex, ordre canonique — la **localisation** d'un record muté) et
   `n17_root` (hex). PG : colonnes `llm_records JSONB DEFAULT '[]'`, `n17_leaves JSONB
   DEFAULT '[]'`, `n17_root TEXT`, posées par `migrate()` (idempotent,
   `ADD COLUMN IF NOT EXISTS`). **Compat ascendante gravée** : un run d'avant ADR-016 relit
   `[]`/`NULL` → traité comme **Mode A** (forensique vide), donc vérifié trivialement.
4. **`verify/:id` re-prouve TOUT, hors ligne** — l'endpoint appelle
   `verify_run_offline(records, llm_records, n17_leaves, n17_root, key)` =
   `verify_swarm` ⊕ `verify_n17_chain` ⊕ `verify_bridge`, **pur** : aucune async, aucun
   provider, aucun réseau. `VerifyResponse.verified` signifie désormais **`fully_proven`** :
   essaim intègre **ET** chaîne N17 intacte **ET** pont cohérent. Le bloc `n17` du DTO expose
   `{records, chain_intact, root, bridge_consistent, mode_c_consistent, mode_c_unlinked,
   mode_c_broken}`.
5. **L'export jumeau pour la vérification tierce** — `GET /v1/swarm/runs/:id/n17` rend
   `{run_id, llm_records, n17_leaves, n17_root}`. Avec `GET …/records` (l'essaim) et sa
   propre copie de la clé tenant, un tiers rejoue `verify_run_offline` sur **sa** machine,
   **sans rien croire** — pas même ce serveur. L'exemple `verify_offline` (binaire
   autonome : deux JSON + tenant → verdict, code de sortie `0`/`1`) en est l'incarnation
   exécutable.
6. **Le `LlmResolver` est la couture** — `verify_bridge` résout `llm_provenance_id` via un
   trait `LlmResolver` (`MapResolver::from_records` pré-charge la forensique persistée) :
   un store réel s'adapte derrière sans toucher la re-preuve. Un lien Mode C qui ne résout
   pas est **signalé `unlinked`** (pas silencieusement ignoré) ; un lien qui résout vers une
   forensique d'un autre swarm/tenant est **`broken`**.
7. **Mode A = no-op strict** — un run keyless (0 appel LLM) produit 0 `LlmProvenanceRecord`,
   donc `SealedN17` vide, `n17_root = None` : `verify_n17_chain` passe vacuousement, le pont
   n'a aucun Mode C à résoudre. La feature est **invisible** au chemin keyless (le flagship).

## Invariants introduits (gardiens nominatifs — 16 tests : 10 provenance · 5 passerelle · 1 Mode C réel ignoré)

**Provenance (`n17_chain.rs`, 10)** — `seal_then_verify_is_intact` ;
`canonical_order_is_deterministic_regardless_of_input_order` (ordre d'entrée neutre) ;
`mutated_n17_field_breaks_the_chain_and_localizes` (**I-PN2** : `first_break` pointe le
record muté) ; `mutated_seed_is_also_caught` ; `deleted_record_breaks_count`
(`CountMismatch`) ; `forged_leaves_and_root_break_at_chain_level` (feuille/racine forgée →
`ChainRootMismatch`, sans localisation fine — l'HMAC tranche) ; `wrong_key_breaks_the_chain`
(**I-PN5**, isolation par clé) ; `empty_n17_seals_and_verifies_vacuously` (**I-PN6**,
Mode A) ; `offline_verify_reproves_swarm_n17_and_bridge` (**I-PN3**, la re-preuve complète) ;
`offline_verify_catches_a_dangling_link_after_n17_loss` (**I-PN4** : forensique perdue → le
pont signale le lien pendant, `verified=false`).

**Passerelle (5)** — intégration HTTP keyless : `keyless_run_persists_zero_n17_and_stays_verified`
(**I-PN6** bout-en-bout), `n17_export_endpoint_round_trips_empty_for_mode_a`,
`n17_export_unknown_run_is_404_and_tenant_scoped` ; Mode C synthétique (lib) :
`mode_c_n17_is_persisted_and_reproven_over_http` (**I-PN1** : la forensique survit à la
persistance, le verdict global est `verified`), `tampered_n17_forensics_break_the_reproof`
(muter un champ forensique persisté → `verified=false`, chaîne d'essaim intacte mais chaîne
N17 cassée).

**Mode C RÉEL (`n17_persist_real.rs`, 1, ignoré sans Ollama)** —
`real_llm_forensics_survive_process_death` : un run `gemma3:12b` authentique, sa forensique
sérialisée puis l'`out` **jeté** (mort du process), re-prouvée depuis les seuls octets +
une re-dérivation de la clé tenant ; falsification d'un champ forensique relu → faute
**localisée**.

## Verdicts des gates (exécutés le jour du build — artefact daté)

- **Lot N17PERSIST : GO** (`scripts/swarm/EFE_SWARM_N17PERSIST_2026-06-15.json`) — crypto
  RÉELLE (BLAKE3+SHA-256+HMAC+HKDF) + `seal_n17`/`verify_run_offline` RÉELS ; appels LLM
  **synthétiques** (forensique fabriquée, **étiquetée**). 5 appels Mode C + 1 juge Mode A,
  scellés puis persistés par **aller-retour serde JSON** (la frontière `JSONB` exacte du
  store), l'objet vivant **jeté**, relu, re-prouvé : ① **aller-retour re-prouvé**
  (`fully_proven`, pont 5/5 cohérents) ; ② **Mode A** trivialement vérifié ; ③ **les 4
  vecteurs de falsification détectés** — champ forensique muté (**localisé**), record
  d'essaim muté, feuille forgée, record N17 retiré ; ④ **coût négligeable** : ~36 µs/record
  de re-preuve hors ligne (budget 2000), ~2,5 Ko/forensique (budget 8192). L'exemple
  `verify_offline` re-prouve l'export sain (sortie `0`) et **rejette** un `model_id`
  falsifié (sortie `1`, faute localisée à la `provenance_id` mutée).

## Conséquences

### Positives

- **Le moat couvre enfin le Mode C** : « ce modèle, ce prompt, ce coût, cette sortie » est
  durable et re-prouvable hors ligne, des jours après la mort du run.
- **Tamper-evidence à deux niveaux** : muter un champ forensique casse la chaîne N17 et
  **localise** le coupable ; forger les feuilles/la racine bute sur l'HMAC (clé tenant).
- **Format gelé, compat totale** : aucun champ neuf sur `AgentCallRecord` ; les anciens runs
  et les runs keyless restent vérifiés sans migration de données.
- **Indépendance maximale** : `/records` + `/n17` + la clé → un tiers re-prouve tout sans le
  serveur, sans provider, sans réseau (`verify_offline`, binaire autonome).

### Négatives / risques tenus (honnêteté de portée)

- **L'attestation du modèle À LA SOURCE n'est PAS prouvée** : `model_hash_verified = false`.
  On prouve que la forensique **déclarée** est intègre et durable, **pas** que le provider a
  réellement servi ce poids. C'est une borne assumée, gravée dans le record.
- **Alignement de domaine de hash `contenu ↔ sortie` différé** : `blake3_bytes` (essaim) vs
  `blake3_value` (forensique) ne sont pas encore réconciliés ; le pont vérifie la
  **cohérence structurelle** (lien résout, même tenant/swarm), pas l'égalité bit-à-bit
  sortie-essaim ↔ sortie-forensique.
- **`n17_root` est par-run** (non rejouable d'un run à l'autre) : c'est une **empreinte
  re-prouvable**, pas une racine déterministe à la ADR-007. Choix délibéré (la forensique
  porte `recorded_at`/`trace_id`).
- **`verify_run_offline` recharge toute la forensique en mémoire** : borné par run (quelques
  Ko/appel) ; le `LlmResolver` est la couture pour un store paresseux si un run devenait
  pathologiquement large.

## Références

- Architecture : `docs/reports/EFE_Archi_Persistance_N17_2026-06-15.html` (commit `a6bab16`)
- Code : `crates/efe-swarm-provenance/src/n17_chain.rs` · `crates/efe-swarm-sdk/src/sdk.rs`
  (scellement) · `crates/efe-swarm-gateway/src/store.rs` · `store_pg.rs` (persistance) ·
  `routes/verify.rs` · `routes/records.rs` (export `/n17`)
- Harnais / exemples : `crates/efe-swarm-provenance/examples/lot_n17persist.rs` ·
  `verify_offline.rs` · `crates/efe-swarm-sdk/tests/n17_persist_real.rs`
- Artefact : `scripts/swarm/EFE_SWARM_N17PERSIST_2026-06-15.json`
- Lignée : ADR-SWARM-006 (moat par-agent) · ADR-SWARM-009 (`verify/:id` = produit) ·
  ADR-SWARM-007 (ordre canonique)
