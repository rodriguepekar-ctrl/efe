# ADR-SWARM-011 : Self-host **keyless** est le mode phare — souverain · local · vérifiable. Garde-fous tenus.

- **Status** : Accepted · **Date** : 2026-06-10
- **Phase** : 5 (Productisation)
- **Related** : ADR-SWARM-006 (la provenance par-agent est le moat) ; ADR-SWARM-009 (l'endpoint
  `verify` est le produit) ; ADR-SWARM-002 (vérificateur déterministe a-posteriori) ;
  `docs/architecture/EFE_SWARM_PHASE5_SELFHOST_KEYLESS_FLAGSHIP_ARCHITECTURE.md` ;
  `efe-swarm-sdk` (`ClientMode`, `backend/`, `BackendInfo`) ; N17 (`efe-llm-provenance`)

## Contexte

EFE expose trois modes (`Keyless`, `Self-host`, `Network`). La question de positionnement : **lequel
est le produit ?** Le calcul s'érode (le prix des tokens baisse), et la coopération d'un provider
hébergé est impossible à obtenir. Le levier durable est la **zone vérifiable et souveraine** (H-SWARM)
— ce qu'un fournisseur hébergé ne peut structurellement pas offrir : données qui ne sortent pas, zéro
secret, et une **preuve que le client re-vérifie lui-même**.

## Décision

**Self-host keyless est le mode phare** : les agents tournent sur un **modèle local**
(Ollama / serveur OpenAI-compatible local), **sans aucune clé**, et chaque run scelle une preuve que
le client (et son régulateur) **re-contrôle hors-ligne**. Le triptyque qu'aucun provider ne peut
égaler :

1. **Souverain** — tâche, états intermédiaires et sortie **ne quittent jamais l'hôte** (`127.0.0.1`).
   Air-gappable. Garde-fou **structurel** : `build_local_backend` **refuse un endpoint non-local par
   défaut** (`SwarmError::RemoteBackendDenied`) ; opt-in explicite `allow_remote_backend(true)`.
2. **Local / keyless** — `ClientMode::is_keyless()` ; **aucun en-tête `Authorization`**, **aucune
   dépendance réseau crate** (shell-out `curl` local). La **seule** clé est la HKDF tenant qui
   **signe** la provenance — elle n'authentifie aucun provider et ne quitte jamais le serveur.
3. **Vérifiable** — `verify_swarm` re-prouve chaîne + contenu + DAG + `root` depuis les seuls records
   + la clé tenant, **hors-ligne**, sans coopération du provider.

**Structure du mode phare** : Mode C local (générateurs/juges LLM) **+ ancre Mode A déterministe**
(le vérificateur reste toujours de la flotte, ADR-002 ; l'agrégation Vote/Borda/Condorcet est de la
mathématique pure). La **valeur** (le consensus) vit dans la zone déterministe re-prouvable.

**Abstraction backend** : un trait `LocalBackend` (keyless) couvre `OllamaBackend` (natif) **et**
`OpenAiLocalBackend` (llama.cpp-server / vLLM / TGI / LM Studio, OpenAI-compatible **local**),
enveloppés d'un **circuit-breaker** (coupe après N échecs → sentinel, protège l'hôte). Le
`BackendInfo` (moteur + modèle + endpoint) est **forensique** (pont N17), exposé dans `SwarmRun`.

## Le sens **exact** de « vérifiable » avec un LLM (honnêteté)

On **ne promet pas** la reproductibilité d'un LLM. On promet la **re-preuve forensique** :
- **Keyless (Mode A)** : rejeu **bit-à-bit** (ADR-007) → même records → même `root`. Reproductible.
- **Self-host (Mode C)** : l'échantillonnage (même *seedé*) peut varier ; « vérifiable » =
  **tamper-evidence + ancrage de contenu** (`verify_swarm` recalcule tout ; muter une sortie casse la
  chaîne) **+ pont N17** (quel modèle/params/hashs). L'**agrégation reste déterministe** : depuis les
  ballots captés, le verdict est **re-dérivable et re-prouvable** même quand la génération ne l'est pas.

C'est **plus fort** qu'une API hébergée, qui renvoie une réponse **sans aucune preuve** cryptographique.

## Les **garde-fous tenus** (câblés ET testés)

| # | garde-fou | mécanisme | évidence (test) |
|---|---|---|---|
| G1 | aucune clé applicative | keyless par construction ; HKDF tenant **signe** (≠ auth) | `client::keyless_modes_need_no_key` |
| G2 | souveraineté / egress=0 | `build_local_backend` refuse non-local par défaut ; opt-in explicite | `backend::build_denies_remote_by_default…`, `sdk::self_host_remote_endpoint_is_denied_by_default` |
| G3 | budget jamais dépassé (I13) + robustesse | `BudgetGuard` atomique ; circuit-breaker `Breaker` ; `--max-time` | `budget::i13_holds_under_concurrency`, `backend::breaker_opens_after_threshold_and_skips_inner` |
| G4 | provenance re-vérifiable | `verify_swarm` + pont N17 ; offline | `verify::*`, `gateway::verify_detects_tampering` |
| G5 | isolation tenant | HKDF par tenant ; run/verify/records/dashboard tenant-scopés | `store::get_is_tenant_scoped`, `registry::get_is_tenant_scoped` |
| G6 | déterminisme gradué | Mode A + agrégation reproductibles ; LLM = tamper-evident (déclaré) | `spec::auto_is_mode_aware` ; §ci-dessus |
| G7 | live ≠ audit | dashboard best-effort (`dropped` exposé) ; la chaîne est la vérité | ADR-SWARM-010 |
| G8 | bornes DoS | budget(#appels) + breaker + bornes substrat + TTL/quota SSE | `substrate_hardening`, `registry::ttl_purges_expired` |
| G9 | pas de réseau caché | `curl` local, aucune dépendance reqwest/TLS dans le SDK | `llm.rs`/`backend/*` (shell-out) |
| G10 | forensique du moteur | `BackendInfo` (engine/model/endpoint) dans `SwarmRun` (pont N17) | `sdk::*` (`backend.is_none()` en keyless) |

> « Garde-fous tenus » = invariants **par construction**, pas par discipline opérationnelle.

## Évidence (build self-host phare)
SDK 29 tests (trait + 2 backends keyless + circuit-breaker + garde-fou egress + sélecteur de moteur) ;
dashboard 15 (+3 intégration) ; gateway 22 (+1 ignored self-host RÉEL Ollama) ; topology 60 — doctests
OK, **0 warning**. Changements **additifs** : `ClientMode`/`SelfHost` inchangés ; `OllamaBackend`
refactoré derrière le trait ; `allow_remote_backend` opt-in ; `SwarmRun.backend` additif.

## Conséquences

- **Le livrable** : un client lance l'essaim sur **son** modèle local, **sans clé**, **sans egress** ;
  son régulateur re-prouve le run **hors-ligne**, sans nous croire ni croire un provider. Souverain,
  local, vérifiable — ce qu'aucun fournisseur hébergé ne peut offrir.
- **`Network` n'est PAS le phare** : il rompt G1 (clé), G2 (egress) et affaiblit G4 ; il sert des cas
  (modèle frontière ponctuel, clés routeur server-side, Inc.4). Self-host keyless est le **défaut
  souverain recommandé**.
- **Différé** : abstraction multi-backend en production (provisioning offline) ; Helm + sidecar Ollama
  (`NetworkPolicy egress=deny`) ; `efe-pii` opt-in par tenant ; **campagne Lot P5** multi-modèles +
  **ADR-008** (gate self-host).
- **Risque tranché** : survendre la « reproductibilité » LLM → on promet la **re-preuve forensique**
  (tamper-evidence + N17), pas le rejeu bit-à-bit ; Mode A + agrégation restent reproductibles.
