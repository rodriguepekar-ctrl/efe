# ADR-SWARM-012 : Inc.4 — `Network` réel via un **routeur server-side** (aucune clé manipulée)

- **Status** : Accepted · **Date** : 2026-06-10
- **Phase** : 5 (Productisation) · **Inc.** : 4 (Network — dernier mode)
- **Related** : ADR-SWARM-011 (self-host keyless est le mode phare) ; ADR-SWARM-009 (l'endpoint
  `verify`) ; M21 (`efe-llm-router`) ; `efe-swarm-sdk` (`ClientMode::Network`, garde-fou keyless) ;
  `efe-swarm-gateway` (`SwarmGatewayState::resolve_mode`)

## Contexte

Trois modes : `Keyless`, `SelfHost` (phare), `Network` (frontière, M21). Les deux premiers sont
**keyless** et tournaient déjà. `Network` — le **seul** mode qui appelle un modèle hébergé — était
**refusé** en in-process (`NetworkRequiresGateway`), car le SDK est **keyless par construction**
(invariant prouvé en continu par `tests/keyless_guardrails.rs` : aucune dépendance réseau/TLS, aucun
`_API_KEY`/`Bearer`/`env::var` dans le SDK).

La règle **absolue** : *je ne saisis, ne lis, ne stocke **aucune clé***. Comment alors rendre
`Network` réel sans violer ni cette règle, ni l'invariant keyless du SDK ?

## Décision

**`Network` est exécuté par la passerelle, qui le traduit en un appel vers un routeur frontière
server-side (`efe-llm-router`, M21) détenant les clés provider — la passerelle et le SDK restent
keyless.**

1. **Le SDK ne change pas** : `Network` reste **refusé** in-process (`NetworkRequiresGateway`).
   L'invariant keyless (aucune clé, aucun en-tête d'auth, aucune dépendance réseau) **tient**.
2. **La passerelle résout** (`resolve_mode`) :
   - `Keyless` / `SelfHost` → inchangés, **souverains** (`allow_remote = false`, egress refusé).
   - `Network { provider }` → si un routeur est câblé, **traduit** en
     `SelfHost { endpoint: <routeur>, model: <provider> }` avec `allow_remote = true` (Network fait
     de l'egress **par conception** — l'opposé du mode souverain). Sinon **501** (Network non câblé).
3. **Les clés ne touchent jamais notre code** :
   - La passerelle ne connaît que l'**URL** du routeur (non-secrète), lue depuis l'env **côté
     binaire** (`EFE_SWARM_ROUTER_ENDPOINT`), **pas** dans la lib (déterministe/testable).
   - Le SDK parle au routeur via `OpenAiLocalBackend` (OpenAI-compatible) **sans en-tête d'auth**.
   - Le **routeur** (`efe-llm-router`, M21) ajoute la clé provider **server-side** (env/Vault).
     L'auth passerelle↔routeur est **infra** (mTLS / NetworkPolicy / réseau interne), **pas** une clé
     applicative dans notre code.
4. **La provenance s'applique inchangée** : les réponses du routeur sont des appels **Mode C**
   captés (records), re-prouvés par `verify_swarm` ; le pont N17 s'y greffe (forensique).

## Pourquoi pas le phare

`Network` rompt G1 (clé, hébergée dans le routeur) et G2 (egress vers le provider) ; il **affaiblit**
G4 (le provider est dans la boucle). Il **sert des cas** (modèle frontière ponctuel) mais
**self-host keyless reste le défaut souverain recommandé** (ADR-011). `Network` est honnêtement le
mode « non souverain, opt-in, clés ailleurs ».

## Évidence (gateway, sans réseau)

| critère | vérification |
|---|---|
| modes souverains inchangés (egress refusé) | `state::resolve_mode_keeps_sovereign_modes_unchanged` |
| Network **sans** routeur → 501 | `state::resolve_mode_network_without_router_is_501` ; HTTP : `network_mode_without_router_is_501` |
| Network **avec** routeur → `SelfHost`-vers-routeur + `allow_remote` | `state::resolve_mode_network_with_router_routes_to_self_host` |
| SDK reste keyless (rien ne change) | `tests/keyless_guardrails.rs` (inchangé) |

> **Note d'honnêteté** : ces tests sont **écrits** ; leur exécution `cargo` a été **différée** (disque
> hôte plein au moment de l'implémentation — `target/` ~193 Go non purgé sur demande). À rejouer dès
> l'espace libéré : `cargo test -p efe-swarm-gateway`.

## Conséquences

- **Le triptyque de modes est complet** : Keyless (Mode A) · Self-host (phare, souverain) · Network
  (frontière, clés **dans le routeur**). Un seul point d'entrée — `swarm.run` / `POST /v1/swarm/run`.
- **Aucune clé dans EFE-swarm** : ni SDK, ni passerelle, ni dans ce dépôt. Le routeur (M21) en est
  l'unique dépositaire, server-side.
- **Provenance préservée** : un run Network est **autant re-prouvable** qu'un run self-host (mêmes
  records, même `verify_swarm`) ; la différence est la **souveraineté** (Network sort de l'hôte),
  pas la vérifiabilité.
- **Différé** : câblage du protocole réel d'`efe-llm-router` (mapping `provider`→modèle/clé, retries,
  circuit-breaker partagé) ; déploiement (sidecar routeur + `NetworkPolicy`).
