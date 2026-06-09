# ADR-SWARM-006 : La provenance **par-agent** est le moat — GO (Lot P4)

- **Status** : Accepted · **Date** : 2026-06-09
- **Phase** : 4 (Provenance & confiance par-agent) · **Gate** : Lot P4
- **Related** : ADR-SWARM-007 (rejeu déterministe sur la version) ; N17 (`efe-llm-provenance`) ;
  audit HMAC (`efe-nano::audit`) ; `docs/architecture/EFE_SWARM_PHASE4_PROVENANCE_ARCHITECTURE.md`

## Contexte

L'essaim EFE génère du trafic en **N agents minuscules en parallèle**. La question qu'**aucun
provider ne peut résoudre** : *« quel agent a dit quoi, quand, à partir de quoi — et puis-je le
**re-prouver** sans faire confiance à celui qui l'a produit ? »*. Avant Phase 4, les agents **Mode A**
(Rust pur, le gros de l'essaim) étaient **muets** ; seul l'appel-LLM (Mode C / N17) laissait une trace.

Phase 4 a construit, sur **crypto réelle réutilisée** (BLAKE3 N17 + HMAC-SHA256/HKDF `efe-nano`) :
l'**atome** `AgentCallRecord` (Inc.1), la **chaîne HMAC par-swarm** dont le tip = `provenance_root`
(Inc.2), la **capture par 2 seams sans réécrire les agents** (décorateur + sink, Inc.3), le **rejeu
déterministe** (ADR-007), le **pont N17** (`llm_provenance_id`), et le **vérificateur indépendant**
`verify_swarm`. La `ConsensusTopology` (Phase 3) est instrumentée de bout en bout.

**Lot P4** tranche : *cette provenance par-agent atteint-elle 100 % de couverture + rejeu
déterministe + vérification tierce, à overhead < budget ?*

## Décision

**GO — la provenance par-agent est construite et paie son coût. C'est le moat.**

Le levier de valeur n'est ni le calcul (s'érode avec le prix des tokens) ni la coopération du
provider (impossible à obtenir) : c'est un **vérificateur indépendant** qui re-prouve l'essaim avec
les seuls enregistrements + la clé tenant (serveur). **Ce que les providers ne peuvent pas offrir.**

## Évidence (Lot P4 — `examples/lot_p4.rs`, crypto + ConsensusTopology RÉELLES, agents simulés, 200 runs/bras, ±σ)

| critère | mesure | budget | verdict |
|---|---|---|---|
| latence sans provenance | 82,2 ± 6,0 µs/run | — | — |
| latence avec provenance | 374,3 ± 13,8 µs/run | — | — |
| **overhead / agent** | **24,34 µs** | < 100 µs | ✓ |
| **octets / enregistrement** | **815 o** | < 2 048 o | ✓ |
| **`traced_pct`** | **100 %** | == 100 % | ✓ |
| **rejeu déterministe** | **oui** (root reproductible) | true | ✓ |
| **`verify_swarm`** (sain) | ok · chaîne · contenu · DAG | true | ✓ |
| **falsification** (1 sortie mutée) | **détectée** (rupture localisée) | true | ✓ |

**Lecture chirurgicale** :
- **L'overhead relatif (355 %) est SURÉVALUÉ** et trompeur : les agents simulés font ≈ **zéro
  travail**, donc la baseline (82 µs, dominée par le `spawn` tokio) est minuscule. La mesure RÉELLE
  est l'**absolu** : **24,34 µs/agent** — le coût de BLAKE3 + canon + SHA-256 + HMAC. Négligeable
  devant **tout** agent réel (un Mode A à travail réel, ou un Mode C en **millisecondes**). Honnêteté
  doctrine EFE : on rapporte les deux et on tranche sur l'absolu.
- **100 % tracé** : chaque appel d'agent (`generate`/`assess`) → un enregistrement ancré à sa cellule.
- **Déterministe + re-prouvable** : le `provenance_root` est reproductible (ADR-007) et un tiers
  reconstruit tout sans le provider ; muter une entrée casse la chaîne, détecté à l'entrée fautive.

→ **Verdict : GO.** Tous les SLO sont franchis avec marge.

## Justification

1. **Cohérence avec toute la chaîne de gates** : chaque phase a gaté son levier (Lot1 ⊘, R0 ⊘, S1
   vote ⊘, S2/S3 consensus GO) ; **P4 gate l'overhead du moat** — et il passe.
2. **Réutilisation, pas réinvention** : `efe-audit-core` réplique byte-à-byte la crypto `efe-nano`
   (golden vectors) ; les chaînes d'essaim et de trace sont mutuellement vérifiables. Zéro dette.
3. **Anti-invasif** : aucune horloge logique injectée dans le substrat (ADR-007) ; l'instrumentation
   de `ConsensusTopology` est **opt-in** (`with_write_sink`), coût nul par défaut.

## Portée & Caveats (honnêteté sur ce qui est prouvé)
- **Crypto + topologie RÉELLES ; agents SIMULÉS** (même doctrine que S2/S3). L'overhead crypto est
  réel ; la baseline triviale gonfle le %. La valeur RÉELLE = l'absolu/agent.
- **Ce que le moat prouve** : intégrité **post-enregistrement** + reconstructibilité + rejeu. **Pas**
  l'honnêteté de l'enregistreur au temps T (un enregistreur malveillant peut mentir de façon
  cohérente). **Mitigations gatées** (0 LOC tant que non justifiées) : signatures **ed25519
  par-agent** (non-répudiation), **ancrage externe** (antériorité tierce), **compaction Merkle**
  (si le volume d'octets dépasse le budget — non déclenché : 815 o ≪ 2 048).
- **Mode C** rejouable bit-à-bit seulement si `sampling.seed` épinglé (sinon marqué `mismatched`).

## Conséquences

### Positives
- **Le moat est construit et mesuré** : provenance par-agent à **24 µs/agent**, 100 % tracée,
  re-prouvée par un tiers. Le territoire « vérificateur indépendant » est ouvert.
- **Phase 4 close à ~95 %** : atome + chaîne + 2 seams + rejeu + pont N17 + `verify_swarm` +
  instrumentation `ConsensusTopology` + métriques Prometheus + ce gate.

### Négatives
- ~~L'overhead absolu reste à confirmer sur un backend réel.~~ **CONFIRMÉ en RÉEL (2026-06-09).**

## Confirmation RÉELLE (Ollama `gemma3:12b`, local/keyless — `examples/swarm_llama.rs`)

La `ConsensusTopology` (K=3 + J=3) **instrumentée**, rejouée avec des **agents LLM réels** :

| métrique | valeur |
|---|---|
| latence/run (agents réels) | **4 240 ms** |
| scellement provenance/run | **148 µs** |
| **overhead provenance / latence LLM** | **0,0035 %** |
| tracé 100 % · `verify_swarm` ok | **oui / oui** |
| accuracy consensus | **3/3** |

→ La thèse est **confirmée hors simulation** : la provenance par-agent (≈150 µs) est **noyée** dans la
latence d'un agent réel (4,2 s) — **0,0035 %**. Le caveat « relatif surévalué par les agents
simulés » du Lot P4 est ainsi levé : sur agents réels, l'overhead est **indétectable**. Le moat paie
son coût **en conditions réelles**. (Et le juge réel atteint **v=1,0** — cf. ADR-SWARM-004.)
Résultats : `scripts/swarm/EFE_SWARM_LLAMA_2026-06-09.json`.

## Références
- `crates/efe-swarm-provenance/` — atome/chaîne/seams/rejeu/pont/`verify_swarm`/métriques.
- `crates/efe-swarm-provenance/examples/lot_p4.rs` — le gate (crypto+topologie réelles).
- `scripts/swarm/EFE_SWARM_P4_2026-06-09.json` — résultats.
- `crates/efe-audit-core/` — HMAC + HKDF partagés (byte-identiques `efe-nano`).
- ADR-SWARM-007 (rejeu déterministe) · ADR-SWARM-004/005 (Phase 3) · ADR-SWARM-001 (le pari aggregator).
