# ADR-SWARM-009 : `GET /v1/swarm/verify/:id` — l'endpoint de re-preuve **est** le produit

- **Status** : Accepted · **Date** : 2026-06-09
- **Phase** : 5 (Productisation) · **Inc.** : 2 (passerelle HTTP)
- **Related** : ADR-SWARM-006 (la provenance par-agent est le moat) ; ADR-SWARM-007 (rejeu
  déterministe sur la version) ; `docs/architecture/EFE_SWARM_PHASE5_INC2_HTTP_GATEWAY_ARCHITECTURE.md` ;
  crate `efe-swarm-gateway` ; SDK `efe-swarm-sdk` (`run_traced`)

## Contexte

Phase 5 Inc.1 a livré le SDK en process : `swarm.run(task, topology, budget)` renvoie un `SwarmRun`
avec un `provenance_root` re-vérifiable. Mais le **moat** (ADR-006) n'a de valeur produit que s'il
est **exposé** : un régulateur ne lance pas un binaire Rust en process — il interroge une **URL**.

La question d'architecture : *comment exposer le SDK **et** le vérificateur indépendant sur HTTP,
sans (a) coupler le SDK léger au monolithe `efe-server` (redis/nats/sqlx/OIDC), (b) polluer le SDK
avec du serde de fil, (c) trahir la règle des clés (aucune clé manipulée côté nous) ?*

## Décision

**Un crate léger `efe-swarm-gateway` expose trois routes sur l'axum 0.7 existant ; la route
`GET /v1/swarm/verify/:id` est l'argument produit décisif.**

1. **`POST /v1/swarm/run`** exécute le SDK (`run_traced`, additif), **persiste les `AgentCallRecord`
   bruts** (l'évidence, serde Phase 4) et renvoie le `SwarmRun`.
2. **`GET /v1/swarm/verify/:id`** appelle `verify_swarm(records, clé_tenant)` qui **recalcule**
   chaîne + contenu + DAG depuis les records. **Même si le store a été altéré**, la chaîne casse →
   `verified=false`, `first_break` localise la faute. L'endpoint ne demande pas de *croire* le
   store ; il **re-prouve**. `GET …/records` exporte les records pour une re-preuve **hors-ligne**
   (indépendance maximale : ne croire même pas ce serveur).
3. **`GET /metrics`** rend le Prometheus des registres (provenance + substrat), sans dépendance.

**Découplage** : le crate expose un `axum::Router` **mergeable** (`.nest("/v1/swarm", swarm_router)`
+ `.merge(metrics_router)`) **ou** servable **standalone** (`router(state)` → souverain, self-host).
Les **DTO de fil** (Deserialize) vivent dans la passerelle et mappent vers les types SDK : le SDK
reste pur, le contrat HTTP évolue indépendamment.

**Clés** : la clé de provenance par tenant (HKDF) reste **server-side** (dans `SwarmGatewayState`) ;
la passerelle **dérive** pour sceller/re-prouver, n'expose **que** le `root`. Toutes les routes sont
**tenant-scopées** (run, verify, records). L'auth est `Caller` (dev : en-tête ; prod : `AuthCtx`
OIDC ES256, clés `EFE_OIDC_*` server-side) — même surface, handlers inchangés.

## Évidence (Inc.2 — `efe-swarm-gateway`, tests axum `oneshot`, keyless réel)

| critère | vérification | verdict |
|---|---|---|
| run → persiste les records → renvoie `SwarmRun` (`verified`, `root` 64 hex) | `run_then_verify_roundtrip_is_proven` | ✓ |
| `verify/:id` re-prouve (chaîne+contenu+DAG) et rend **le même root** qu'à l'exécution | idem | ✓ |
| **falsification** : muter 1 record stocké → `verified=false`, `content_intact=false`, `first_break` localisé | `verify_detects_tampering` | ✓ |
| **isolation tenant** : un run de tenant-A est **invisible** (404) pour tenant-B ; A re-prouve le sien | `run_is_tenant_scoped_against_verify` | ✓ |
| export `…/records` pour re-preuve hors-ligne (records sérialisés non vides) | `records_export_is_independently_verifiable` | ✓ |
| `/metrics` expose `efe_swarm_prov_*` agrégé sur tous les runs | `metrics_endpoint_exposes_prometheus` | ✓ |
| DTO de fil invalide → 400 ; run inconnu → 404 ; `Network` → 501 (routeur non câblé Inc.2) | `bad_topology_is_400`, `verify_unknown_run_is_404`, `error::tests` | ✓ |

19 tests verts (lib) + 2 doctests, **0 warning**. SDK : `run_traced` (`RunOutcome { run, sealed }`)
+ `SwarmBuilder::with_metrics` — additifs, comportement Inc.1 inchangé (`run()` = wrapper).

## Conséquences

- **Le livrable** : un dev fait `POST /v1/swarm/run` ; son régulateur fait `GET /v1/swarm/verify/:id`
  — et obtient une preuve qu'il **re-vérifie lui-même**, en ligne ou hors-ligne, sans nous croire ni
  croire le provider. *Le moat, exposé en une URL.*
- **Différé (Inc.2.d / Inc.3 / Inc.4)** : `PgRunStore` (write-once, prod) ; `AuthCtx` OIDC réel ;
  modèle **job** + SSE pour les runs LLM longs (dashboard live) ; `ClientMode::Network` réel (le
  serveur détient les clés du routeur, Vault) — la passerelle remplacera alors le `501`
  `NetworkRequiresGateway` que le SDK renvoie en in-process.
- **Risque tranché** : « verify fait-il vraiment confiance au store ? » → **non** : `verify_swarm`
  recalcule tout ; la falsification est testée ; l'export permet la vérif hors-ligne.
