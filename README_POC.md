# EFE — PoC Scout & Synthesize (démarrage en 1 commande)

> Démo **live** d'orchestration multi-LLM + **pré-mâchage** (compression de prompt
> par petits modèles locaux). Une UI web montre chaque mission en temps réel
> (événements SSE), avec les métriques de coût / latence / tokens.
>
> **Périmètre** : `SCOPE.md`. **Config** : `poc.env`. **Verdict mesuré** : ce que
> la démo montre est honnête (cf. les 3 scénarios ci-dessous).

## 1. Prérequis

| Outil | Pourquoi | Vérifier |
|---|---|---|
| **Rust** (cargo) | build du bridge | `cargo --version` |
| **Node + pnpm** (ou npm) | build de l'UI | `pnpm --version` |
| **llama-server** (llama.cpp) | les 4 petits modèles (SLM) | `llama-server --version` |
| **curl** | health-checks | `curl --version` |
| ~6 Go disque | modèles GGUF (~4,1 Go) + builds | |

> **macOS recommandé** (les SLM tournent sur le GPU Metal). Sur une machine sans
> GPU, ça marche en CPU (plus lent) : `EFE_SLM_NGL=0 ./scripts/slm/start.sh`.

**Modèles SLM** (une seule fois, ~4,1 Go) :
```bash
./scripts/slm/download_models.sh
```

## 2. Lancer la démo (1 commande)

```bash
./poc.sh up
```

Cela : démarre les 4 SLM → build le bridge → build l'UI → sert le tout sur **un
seul port**. À la fin :

```
✓ PoC opérationnel → http://127.0.0.1:8700
```

Ouvre **http://localhost:8700** dans ton navigateur. C'est l'UI Preview Lab.

Par défaut, le mode est **keyless** : LLM **simulé (gratuit)** + SLM **réels** =
le pré-mâchage tourne pour de vrai, **sans dépenser un centime**.

## 3. Que regarder (les 3 scénarios)

Deux façons :
- **Dans l'UI** : copie-colle les prompts de **`poc-demo-prompts.md`**.
- **En headless** (sans UI, un tableau de contraste) : `./poc.sh demo`.

Tu verras :

1. **Prompt redondant** → le compresseur **CHEWER** : le prompt rétrécit, et la
   réponse reste correcte mais concise.
2. **Prompt court** → **BYPASS** : sous le seuil, aucun overhead — le système ne
   se sabote pas.
3. **Prompt détaillé** → **compromis** : plus concis, mais on voit la perte de
   complétude (la limite, assumée).

Chaque mission streame ses événements live : `SWARM_STARTED`,
`PRE_CHEWING_METRICS` (tokens entrée/sortie), `LLM_FINISHED` (coût, latence),
`SWARM_FINISHED`.

## 4. Mode réel (vrais LLM — optionnel, payant)

Tes clés restent **dans ton shell**, jamais dans un fichier :
```bash
export ANTHROPIC_API_KEY=sk-ant-...      # et/ou OPENAI_API_KEY / GOOGLE_API_KEY
./poc.sh up --real
```
Là, les réponses viennent du vrai modèle → tu vois le **gain réel** coût + latence
sur les prompts redondants.

## 5. Arrêt & santé

```bash
./poc.sh demo       # joue les 3 scénarios en headless (tableau de contraste)
./poc.sh status     # santé du bridge + des 4 SLM
./poc.sh down       # arrêt propre de tout
```

## 6. Dépannage

| Symptôme | Cause / solution |
|---|---|
| `Port :8700 déjà occupé (pas notre bridge)` | un autre process écoute → `./poc.sh down` ou `kill <pid>` |
| `Démarrage SLM échoué (modèles manquants)` | lance `./scripts/slm/download_models.sh` |
| UI non buildée | installe Node + pnpm, ou relance `./poc.sh up --rebuild-ui` |
| `provider mock-…` alors que tu voulais du réel | clés non exportées → cf. §4 (le bridge retombe en mock) |
| Tout est lent (CPU) | normal sans GPU ; `EFE_SLM_NGL=0` force le CPU |

## 7. Ce que le PoC NE prouve pas (honnêteté)

Le PoC démontre **la mécanique** (le pré-mâchage marche, est gate-protégé,
déterministe). Il ne prouve **pas le ROI business** : ça dépend de la part de
prompts redondants dans ton **trafic réel** — mesure à faire séparément. Voir
`SCOPE.md` § non-goals.
