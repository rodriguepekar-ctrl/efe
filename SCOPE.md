# EFE — Périmètre du PoC (Proof of Concept)

> **But du PoC** : une démo **lançable en une commande par un non-développeur**,
> qui montre **live** l'orchestration multi-LLM + le pré-mâchage gate-protégé,
> avec des métriques honnêtes, en mode **keyless gratuit** et **réel**.
>
> Ce document **fige le périmètre**. Le dépôt contient bien plus (47 crates,
> trois projets empilés) ; le PoC n'en active **qu'une tranche démontrable**.

## Dans le PoC (le seul code activé pour la démo)

| Composant | Rôle |
|---|---|
| `crates/efe-preview-server` | Bridge HTTP/SSE — sert l'UI **et** l'API sur `:8700` |
| `crates/efe-coordinator` | Orchestrateur DAG + blackboard + flux SSE |
| `crates/efe-nano-app-coordinator` | 6 clients SLM (analyzer, reformulator, compressor, verifier, router, filter) |
| `crates/efe-multi-llm` | Workflows multi-LLM |
| `preview-lab/` | UI Next.js (export statique servi par le bridge) |
| `scripts/slm/` | Cycle de vie des 4 llama-server (`:8090-8093`) |
| `poc.sh` + `poc.env` | Orchestrateur une-commande + config unique |

## Hors PoC (présent au dépôt, **gelé** — pas dans la démo)

- Les ~30 crates SaaS entreprise (Sprints M21→M50) : `efe-billing`, `efe-pii`,
  `efe-multimodal`, `efe-region`, `efe-vector`, `efe-retrieval`, `efe-kg`,
  `efe-graphrag`, `efe-cache`, `efe-events`, `efe-saga`, `efe-metering`,
  `efe-agent-runner`, `efe-gold-test-gate`, `efe-gateway`, `efe-meta-swarm*`,
  `efe-nano*`, `efe-compliance-wiring`, etc.
- **Mode 1** (espace latent partagé : `efe-core`, `efe-server`, `efe-kernels`) —
  code réel mais **valeur non prouvée** → ferait l'objet d'un **PoC distinct**
  conditionné à un benchmark d'accuracy.
- `efe-ffi` (bindings PyO3) — exclu : mismatch Python local (3.9 vs 3.10+ requis).

## Ce que la démo montre (honnête, mesuré)

Trois prompts scriptés (cf. `poc-demo-prompts.md`) illustrent le verdict mesuré
(campagnes H0·a / H1·a) :

1. **Prompt redondant/verbeux** → **CHEWER** : compression visible (+ gain
   coût/latence en mode réel).
2. **Prompt court** → **BYPASS** : le gate (`COMPRESS_MIN_TOKENS = 250`) protège,
   zéro overhead inutile.
3. **Prompt détaillé** → **compromis** : brièveté vs complétude (la limite, assumée).

## Non-goals (explicites)

- **Pas la validation ROI business** — elle est suspendue à la mesure du
  **trafic réel** (part de prompts redondants), hors-scope packaging.
- Pas de production : mono-nœud, `localhost`, pas de TLS, pas de multi-tenant à
  l'échelle.
- Pas de Docker pour les SLM : le compresseur tourne sur **Metal (GPU Apple)** via
  llama.cpp → exécution **native** (Docker macOS n'a pas d'accès GPU).

## Démarrage

```bash
./poc.sh up        # build + lance tout (SLM + bridge + UI) → http://localhost:8700
./poc.sh status    # santé des 6 processus
./poc.sh down      # arrêt propre
```

Détails : `README_POC.md`. Configuration : `poc.env`.
