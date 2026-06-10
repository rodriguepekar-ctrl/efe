# efe-swarm-gateway — Helm chart (déploiement souverain)

Déploie la **passerelle d'essaim EFE** en **self-host keyless** — le mode phare :
**souverain · local · vérifiable**, ce qu'aucun provider hébergé n'offre.

- `POST /v1/swarm/run` — exécute un essaim, persiste les records.
- `GET /v1/swarm/verify/:id` — **re-prouve** (le moat) sans faire confiance à l'opérateur.
- `GET /metrics` — Prometheus (provenance + substrat + dashboard).
- `GET /dashboard` — UI live 4 panneaux.

## La souveraineté, prouvée à deux couches (défense en profondeur)

| couche | mécanisme |
|---|---|
| **applicative** | `build_local_backend` **refuse** un endpoint backend non-local par défaut (egress interdit) ; circuit-breaker ; health-check. |
| **cluster** | **`NetworkPolicy` egress=deny** — le pod ne peut **pas** joindre l'extérieur. Le sidecar **Ollama** sert le modèle en **boucle locale** (`127.0.0.1`). |

Résultat : la tâche, les états et la sortie **ne quittent jamais le pod**. Air-gappable.

## Clés — règle absolue

**Aucune clé provider.** La seule clé est la **HKDF de provenance** (un `Secret`), qui **signe**
la chaîne — elle n'authentifie aucun provider. Fournissez-la via `hkdf.existingSecret` (Vault /
external-secrets recommandé) ; sinon le chart en génère une, **conservée** (`resource-policy: keep`)
pour que les preuves historiques restent re-vérifiables.

## Installer (souverain par défaut)

```bash
# Air-gap : pré-chargez le PVC de modèles, puis :
helm install swarm charts/efe-swarm-gateway -n efe --create-namespace \
  --set image.repository=ghcr.io/efe-ai/efe-swarm-gateway

# Non air-gap : laissez le Job de provisioning pull les modèles (egress temporaire) :
helm install swarm charts/efe-swarm-gateway -n efe --create-namespace \
  --set ollama.provision.enabled=true
```

Par défaut : `replicaCount=1`, `networkPolicy.egress.denyAll=true` (DNS cluster autorisé),
sidecar Ollama avec `gemma3:12b`, contextes de sécurité durcis (non-root, rootfs read-only,
caps `ALL` drop, seccomp `RuntimeDefault`).

## Modèles (provisioning)

Le runtime est **egress=deny** → il **ne peut pas** `ollama pull`. Deux options :
- **Air-gap** (`ollama.provision.enabled=false`, défaut) : pré-chargez le PVC
  `…-ollama-models` hors-bande (copie des poids).
- **Provisioning** (`ollama.provision.enabled=true`) : un **Job pre-install** (avec une
  NetworkPolicy d'egress **dédiée et temporaire**) pull les modèles dans le PVC. Le runtime
  reste souverain.

## Mode Network (frontière) — opt-in, NON souverain

`router.enabled=true` + `router.endpoint=<url-routeur>` : la passerelle route
`ClientMode::Network` vers le **routeur server-side** (`efe-llm-router`, qui détient les clés).
Ouvre l'egress vers `router.egressToNamespace`. À n'utiliser que sciemment — **self-host keyless
reste le défaut recommandé**.

## Observabilité

`serviceMonitor.enabled=true` (Prometheus Operator) scrape `/metrics` :
`efe_swarm_prov_*` (provenance), `swarm_*` (substrat), `efe_swarm_dash_*` (dashboard).

## Valeurs clés

| clé | défaut | rôle |
|---|---|---|
| `networkPolicy.egress.denyAll` | `true` | **souveraineté** : aucun egress |
| `networkPolicy.egress.allowDns` | `true` | autorise le DNS cluster (resserrez en air-gap) |
| `ollama.enabled` | `true` | sidecar modèle local |
| `ollama.models` | `[gemma3:12b]` | modèles attendus (pré-provisionnés) |
| `ollama.persistence.size` | `50Gi` | store de modèles (air-gap) |
| `hkdf.existingSecret` | `""` | Secret de la clé de provenance (prod : Vault) |
| `router.enabled` | `false` | mode Network (non souverain) |
| `serviceMonitor.enabled` | `false` | scrape Prometheus Operator |

## Vérifier le rendu (sans cluster)

```bash
helm lint charts/efe-swarm-gateway
helm template swarm charts/efe-swarm-gateway | kubectl apply --dry-run=client -f -
```
