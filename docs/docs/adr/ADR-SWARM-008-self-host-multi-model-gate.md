# ADR-SWARM-008 : Lot P5 — self-host keyless **multi-modèles** tient ses garde-fous — GO

- **Status** : Accepted · **Date** : 2026-06-10
- **Phase** : 5 (Productisation) · **Gate** : Lot P5
- **Related** : ADR-SWARM-011 (self-host keyless est le mode phare) ; ADR-SWARM-006 (la provenance
  est le moat) ; ADR-SWARM-002 (vérificateur déterministe) ; ADR-SWARM-007 (rejeu déterministe) ;
  `crates/efe-swarm-sdk/examples/lot_p5.rs` ; `scripts/swarm/EFE_SWARM_LOTP5_2026-06-10.json`

## Contexte

ADR-SWARM-011 a posé que **self-host keyless est le mode phare** (souverain · local · vérifiable) et
que ses **garde-fous sont tenus**. **Lot P5** tranche par l'évidence mesurée : *ces garde-fous et la
vérifiabilité tiennent-ils **à travers plusieurs modèles** locaux — pas seulement sur un cas ?*

Insight d'architecture : les garde-fous du flagship (souveraineté/egress, keyless, budget I13,
**tamper-evidence**, couverture `traced_pct`) sont **model-independent** — ils tiennent pour *tout*
modèle **par construction** (crypto/provenance/garde structurel). La campagne les **mesure** sur des
runs **réels** : une arme déterministe (keyless Mode A, crypto/provenance réelles) **plus** trois
modèles **locaux réels** (Ollama), tous keyless.

## Décision

**GO — le mode phare self-host keyless tient ses garde-fous et reste vérifiable à travers les
modèles.** Le levier de valeur (souveraineté + re-preuve), étant model-independent, **ne dépend pas**
du modèle choisi : l'utilisateur garde le contrôle quel que soit le moteur local.

## Évidence (Lot P5 — `examples/lot_p5.rs`, crypto/provenance RÉELLES, **65 runs réels**)

**Garde-fous structurels** (réels, déterministes, model-independent) :

| garde-fou | mesure | verdict |
|---|---|---|
| G2 souveraineté/egress (endpoint non-local refusé par défaut) | `RemoteBackendDenied` | ✓ |
| health-check (backend injoignable → échec rapide) | `BackendUnavailable` | ✓ |
| keyless (aucune clé) | `is_keyless() == true` | ✓ |

**Armes mesurées** (verify = re-preuve `verify_swarm` ; tamper = falsification d'1 record détectée) :

| arme | runs | verify | traced_pct | budget (I13) | tamper | latence moy. |
|---|---|---|---|---|---|---|
| **keyless (Mode A déterministe)** | 50 | **100 %** | **100 %** | ✓ ≤ plafond | **100 %** | 0,413 ms |
| **self-host : gemma3:12b** (réel) | 5 | **100 %** | **100 %** | ✓ ≤ plafond | **100 %** | 6 729,8 ms |
| **self-host : qwen3-coder:30b** (réel) | 5 | **100 %** | **100 %** | ✓ ≤ plafond | **100 %** | 31 800,0 ms |
| **self-host : qwen3-vl:30b** (réel) | 5 | **100 %** | **100 %** | ✓ ≤ plafond | **100 %** | 54 620,8 ms |

- **65 runs réels, 0 échec.** Chaque run : re-prouvé (`provenance.verified`), 100 % ancré
  (`traced_pct`), dépense **≤ plafond** (I13), **falsification détectée** (muter 1 sortie scellée →
  `verify_swarm` casse). Sur l'arme déterministe **et** sur **trois modèles locaux réels**.
- **Latence** : informationnelle, dépend de la taille du modèle (0,4 ms keyless → ~6,7 s gemma3:12b →
  ~54,6 s qwen3-vl:30b sur le matériel de mesure). Le coût est **local** (0¢/appel), borné par I13.

## Critères de gate (tous remplis)
- `verify_ok == 1.0` · `traced_pct == 1.0` · `budget ≤ plafond` (I13) · `tamper_detected == 1.0`,
  sur **chaque** arme réelle ; **plus** les trois garde-fous structurels.

## Conséquences

- **Le moat est model-agnostic** : la souveraineté (rien ne sort) + la re-preuve (records + clé
  tenant) ne dépendent ni du modèle ni du provider. Changer de modèle local **ne change rien** aux
  garanties. *Ce qu'aucun fournisseur hébergé ne peut offrir.*
- **Reproductibilité honnête** (cf. ADR-011 §4) : l'arme **keyless Mode A** est reproductible bit-à-bit
  ; les armes **LLM** sont **tamper-evident** (100 % de falsifications détectées), pas reproductibles —
  c'est la **re-preuve forensique** qui est gatée, pas le rejeu de génération.
- **Différé** (sprints dédiés) : campagne **qualité** par modèle (justesse de classification — distincte
  des garde-fous) ; pont N17 par-appel ; provisioning offline ; Helm + `NetworkPolicy egress=deny` ;
  `efe-pii` opt-in.
- **Reproduire** : `cargo run --release --example lot_p5 -p efe-swarm-sdk` (l'arme keyless + les
  garde-fous structurels tournent **sans** Ollama ; les armes self-host se mesurent là où les modèles
  sont provisionnés, sinon honnêtement marquées « structurel »).
