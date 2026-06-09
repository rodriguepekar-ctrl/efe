# ADR-SWARM-007 : Le rejeu déterministe se cle sur la **version** du Blackboard, pas sur l'horloge

- **Status** : Accepted · **Date** : 2026-06-09
- **Phase** : 4 (Provenance & confiance par-agent — le *moat*) · **Inc.** : 4 (rejeu)
- **Related** : ADR-SWARM-006 (provenance par-agent = moat) ; `efe-swarm-provenance` (Inc.1-3) ; `efe-blackboard` (I13 : `snapshot()` trié par pointer)

## Contexte

La provenance par-agent (Phase 4) scelle chaque appel d'agent dans une **chaîne HMAC
par-swarm** dont le tip — le `provenance_root` — est *un seul hash qui prouve tout le run*
(Inc.2). Pour qu'un **vérificateur indépendant** puisse **rejouer** un essaim et obtenir
le **même** `provenance_root`, l'ordre des enregistrements dans la chaîne doit être
**reproductible**.

Or l'essaim s'exécute **massivement en parallèle** (`JoinSet`, K générateurs + J juges
concurrents). Deux faits en découlent :

1. **L'ordre d'achèvement est non déterministe** : d'un run à l'autre, les tâches finissent
   dans un ordre différent.
2. **`SystemTime::now()` (le champ `wall_at`) est non déterministe** : il reflète l'horloge
   murale au moment de la capture, qui varie d'un run/replay à l'autre.

Si l'ordre de la chaîne (ou le `payload_hash`) dépendait de l'un ou l'autre, le
`provenance_root` ne serait **pas reproductible** → pas de rejeu, pas de vérification
tierce, **pas de moat**.

## Décision

1. **`wall_at` est ADVISORY et EXCLU du `payload_hash`.** (Déjà garanti depuis Inc.1 : le
   corps canonique hashé ne contient pas `wall_at`.) L'horloge murale est conservée comme
   métadonnée d'audit *indicative*, jamais comme source de vérité ni d'ordre.

2. **L'ordre de la chaîne se cle sur les coordonnées DÉTERMINISTES du substrat** : la
   **cellule écrite** `(version, pointer)`. La `version` du Blackboard est **atomique et
   par-cellule** (incrémentée sous garde de shard) ; le `pointer` la complète. La clé d'ordre
   d'un enregistrement est :

   ```
   OrderKey = ( a_une_écriture? , version , pointer , payload_hash )
   ```

   — les enregistrements **avec** écriture passent avant ceux **sans** ; puis ordre par
   `(version, pointer)` de la cellule représentante (la `(version, pointer)` minimale) ;
   départage final par `payload_hash` (adresse de contenu). **`wall_at` n'intervient
   jamais.**

3. **Cet ordre est cohérent avec le substrat** : `Blackboard::snapshot()` est lui-même
   « sorted by pointer for replay determinism » (I13). On réutilise la **même** discipline.

4. **Aucune horloge logique n'est injectée dans le substrat.** Une `LogicalClock` a été
   envisagée (§3.5 de l'architecture) ; elle est **inutile** : les coordonnées existantes
   `(version, pointer)` + `payload_hash` suffisent à un ordre total déterministe. Choix
   **anti-invasif** (zéro modification d'`efe-blackboard`).

5. **Le scellement vivant (`ProvenanceRecorder::seal`) ET le rejeu (`replay`) partagent la
   MÊME clé d'ordre.** Conséquence : `seal().root == replay(records).root`. Le root produit
   en direct est *déjà* le root reproductible.

## Conséquences

### Positives
- **`provenance_root` reproductible** : même **ensemble** d'enregistrements → même racine,
  quel que soit l'ordre d'exécution parallèle. (`replay_root` / `replay` — prouvé par
  oracles : permutation/inversion de l'entrée ⇒ root identique.)
- **Rejeu d'exécution** (`replay_outputs`) : re-dérive les sorties d'agents **purs**
  (fonction de `(input, variant)`) et compare les `output_hash`. Déterministe pour **Mode A**.
- **Vérificateur indépendant** : un tiers, avec la seule clé tenant + les enregistrements,
  reconstruit le root sans coopération du provider — le *moat*.
- **Zéro dette substrat** : pas de nouvelle primitive dans `efe-blackboard`.

### Négatives / limites (honnêtes)
- **L'ordre de chaîne ≠ ordre causal.** Trier par `(version, pointer)` peut placer un
  `/ballot/*` avant un `/cand/*` (alphabétique). C'est **sans incidence** sur la tamper-
  evidence (n'importe quel ordre déterministe convient) ; la **causalité** reste portée par
  `causal_parents` (le DAG), pas par la position dans la chaîne.
- **Mode C non déterministe par défaut** : un juge/générateur LLM n'est rejouable bit-à-bit
  que si sa graine d'échantillonnage est épinglée (`sampling.seed`, champ N17). Sinon
  `replay_outputs` le marque `mismatched` — **honnêtement**, pas de faux positif.
- **Plusieurs écritures par agent** : la cellule représentante est la `(version, pointer)`
  minimale ; un agent qui écrit plusieurs cellules est ordonné par sa plus petite. Suffisant
  pour un ordre total déterministe.

## Alternatives écartées
- **Ordonner par `wall_at`** : non déterministe (rejeté — c'est le cœur du problème).
- **Ordonner par ordre d'achèvement / index de capture** : non déterministe sous
  parallélisme.
- **Injecter une `LogicalClock` dans le substrat** : invasif et redondant avec `(version,
  pointer)` ; écarté.
- **Tri topologique par `causal_parents`** : `record_id` est un UUID **aléatoire** (non
  reproductible entre runs indépendants) → inadapté comme clé d'ordre stable ; la causalité
  reste exploitable pour la *vérification* du DAG (Inc.4 `verify_swarm`), pas pour l'ordre.

## Références
- `crates/efe-swarm-provenance/src/replay.rs` — `OrderKey`, `deterministic_order`,
  `replay`, `replay_root`, `replay_outputs`.
- `crates/efe-swarm-provenance/src/recorder.rs` — `seal` partage la clé d'ordre.
- `crates/efe-swarm-provenance/examples/replay.rs` — démonstration (même root sous
  permutation ; `wall_at` indifférent ; détection d'altération).
- `crates/efe-blackboard/src/blackboard.rs` — `snapshot()` I13 (tri par pointer).
- ADR-SWARM-006 (moat) · `docs/architecture/EFE_SWARM_PHASE4_PROVENANCE_ARCHITECTURE.md` (§3.5).
