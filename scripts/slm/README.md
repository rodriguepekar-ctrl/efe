# scripts/slm/ — Bootstrap des llama-server pour les nano-robots Phase 1

Cette suite de 6 scripts bash permet de **démarrer, monitorer et arrêter**
en une seule commande les 4 llama-server locaux requis pour faire tourner
les nano-robots Phase 1 (`analyzer`, `reformulator`, `compressor`, `verifier`)
sur les ports `8090`-`8093`.

C'est le **Sprint S1** de la Phase 3 R2 — Missions Réelles Live. Sans ces
scripts, impossible d'exécuter les tests live `live_smoke.rs` (Phase 1)
ni `live_missions.rs` (Phase 3 R2, à venir).

---

## Pré-requis

| Outil | Version | Installation macOS | Installation Linux |
|---|---|---|---|
| `bash` | ≥ 4.x recommandé | `brew install bash` | natif |
| `llama-server` | ≥ b9297 | `brew install llama.cpp` | compiler depuis [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) |
| `curl` | quelconque | natif | natif |
| `lsof` ou `ss` ou `nc` | quelconque | natif (`lsof`) | natif |

Pour Mac M-series : llama.cpp compilé avec Metal (cas par défaut Homebrew).
Pour Linux NVIDIA : recompiler avec `LLAMA_CUDA=1` pour bénéficier des GPU.

---

## Quick start

```bash
# 1. Télécharger les 4 modèles GGUF (~4.1 GB cumulés, à faire 1× par machine)
./scripts/slm/download_models.sh

# 2. Démarrer les 4 daemons llama-server (en arrière-plan, ports 8090-8093)
./scripts/slm/start.sh

# 3. Vérifier la santé
./scripts/slm/health.sh
#   analyzer     8090   12345   ● UP
#   reformulator 8091   12346   ● UP
#   compressor   8092   12347   ● UP
#   verifier     8093   12348   ● UP

# 4. (plus tard) Lancer les tests live qui dépendent des SLM
EFE_LIVE_SMOKE=1 cargo test -p efe-nano-app-coordinator --test live_smoke -- --nocapture

# 5. Arrêter quand tu as fini
./scripts/slm/stop.sh
```

---

## Détail des 6 scripts

### `lib_common.sh`
Bibliothèque sourcée par les autres scripts. Définit :
- La config des 4 robots (nom, port, modèle, URL HF, taille, ctx)
- Les chemins runtime (`models/`, `.llama_pids/`, `logs/llama-server/`)
- Les helpers : couleurs, logging, détection OS, vérification ports, lecture pidfile, health-check HTTP.
- Variables d'env exportées : `EFE_SLM_OS`, `EFE_SLM_NPROC`, `EFE_SLM_THREADS_PER_ROBOT`, `EFE_REPO_ROOT`, etc.

**Ne pas exécuter directement** : `source` only.

### `download_models.sh`
Télécharge les 4 modèles GGUF depuis HuggingFace vers `<repo>/models/<robot>.gguf`.

```bash
./scripts/slm/download_models.sh                 # tous (skip les présents)
./scripts/slm/download_models.sh analyzer        # un seul
./scripts/slm/download_models.sh --force         # re-télécharge tout
./scripts/slm/download_models.sh --check         # vérifie présence + taille
```

Idempotent : skip les fichiers déjà téléchargés avec taille ≥ 95 % de la cible.

URLs canoniques documentées dans `crates/efe-nano-app-coordinator/MODELS_DOWNLOAD.md`.

### `start.sh`
Démarre 4 daemons `llama-server` en arrière-plan + attend que `/health` retourne 200.

```bash
./scripts/slm/start.sh                  # démarre les 4
./scripts/slm/start.sh verifier         # un seul
./scripts/slm/start.sh --dry-run        # preview sans lancer
./scripts/slm/start.sh --timeout 60     # health-timeout par robot (défaut 30s)
./scripts/slm/start.sh --ngl 0          # CPU-only (slow but works without GPU)
./scripts/slm/start.sh --host 0.0.0.0   # bind toutes interfaces (remote)
```

Flags llama-server choisis (commentés dans `start.sh`) :
- `--host 127.0.0.1` (local-only par sécurité)
- `--ctx-size` par robot (2048-4096)
- `--n-gpu-layers 999` (offload Metal/CUDA max, fallback CPU)
- `--threads N` calculé selon le hardware
- `--parallel 1 --cont-batching --cache-reuse 256`
- `--metrics` (expose `/metrics` Prometheus)
- `--no-warmup` (warmup à la 1ʳᵉ requête, accélère startup)
- `--alias <robot>` (visible dans `/props`)

Idempotent : si un robot est déjà UP, skip avec warning. Si un port est volé par un autre process, abort.

PIDs écrits dans `.llama_pids/<robot>.pid`. Logs dans `logs/llama-server/<robot>.log` (rotation `.prev`).

### `stop.sh`
Arrête les 4 daemons proprement : SIGTERM → attente `--grace SEC` → SIGKILL si nécessaire.

```bash
./scripts/slm/stop.sh                  # tous
./scripts/slm/stop.sh analyzer         # un seul
./scripts/slm/stop.sh --force          # SIGKILL direct
./scripts/slm/stop.sh --grace 10       # attendre 10s avant SIGKILL
```

Nettoie automatiquement les pidfiles orphelins.

### `health.sh`
Ping HTTP `/health` des 4 endpoints. Exit code = nombre de robots unhealthy.

```bash
./scripts/slm/health.sh                # check une fois
./scripts/slm/health.sh --watch        # refresh toutes les 2s
./scripts/slm/health.sh --quiet        # silencieux, exit code only
./scripts/slm/health.sh --json         # output JSON (pour scripts)
./scripts/slm/health.sh --timeout 5    # timeout HTTP par ping
```

### `status.sh`
Vue locale (pas de réseau) : pidfile + état process + taille log + mtime.
Pour vérification HTTP, préférer `health.sh`.

```bash
./scripts/slm/status.sh                # vue d'ensemble
./scripts/slm/status.sh --verbose      # + dernière ligne du log
./scripts/slm/status.sh --json         # output JSON
```

### `logs.sh`
Affiche / tail les logs des llama-server.

```bash
./scripts/slm/logs.sh analyzer            # cat du log
./scripts/slm/logs.sh analyzer --tail     # tail -f
./scripts/slm/logs.sh analyzer --last 50  # 50 dernières lignes
./scripts/slm/logs.sh --all               # cat des 4
./scripts/slm/logs.sh --all --tail        # tail -f multiplexé
```

---

## Variables d'environnement reconnues

| Variable | Défaut | Effet |
|---|---|---|
| `EFE_SLM_HEALTH_TIMEOUT` | 30 | Timeout health-wait au démarrage (s) |
| `EFE_SLM_HEALTH_PING_TIMEOUT` | 2 | Timeout HTTP par ping (s) |
| `EFE_SLM_NGL` | 999 | `--n-gpu-layers` |
| `EFE_SLM_HOST` | `127.0.0.1` | `--host` |
| `EFE_SLM_GRACE` | 5 | Délai SIGTERM → SIGKILL (s) |
| `NO_COLOR` | (unset) | Si défini, désactive les couleurs ANSI |
| `FORCE_COLOR` | (unset) | Force les couleurs même hors TTY |

---

## Convention des chemins

| Chemin | Contenu | Gitignored ? |
|---|---|---|
| `<repo>/models/<robot>.gguf` | Modèles GGUF (4 fichiers, ~4.1 GB) | ✅ oui |
| `<repo>/.llama_pids/<robot>.pid` | PIDs des daemons en cours | ✅ oui |
| `<repo>/logs/llama-server/<robot>.log` | Stdout+stderr des daemons | ✅ oui |
| `<repo>/logs/llama-server/<robot>.log.prev` | Log précédent (rotation au démarrage) | ✅ oui |

---

## Troubleshooting

**`llama-server: command not found`**
→ `brew install llama.cpp` (Mac) ou compile depuis source (Linux).

**`Conflit de port`**
→ Un autre process écoute déjà sur 8090/8091/8092/8093. Trouve-le :
`lsof -nP -iTCP:8090 -sTCP:LISTEN`.

**`/health` ne répond jamais (timeout 30s)**
→ Le modèle est trop gros pour ta RAM. Vérifie : `tail -50 logs/llama-server/<robot>.log`.
Solution : utilise une quantization plus petite (Q4 au lieu de Q5), ou plus de threads, ou downgrade ctx-size.

**Process crashe immédiatement**
→ Vérifie le log. Causes fréquentes :
- Modèle corrompu → `./scripts/slm/download_models.sh --force <robot>`
- llama-server trop vieux → besoin b9297+

**`/health` répond mais les requêtes sont lentes (>5s pour 10 tokens)**
→ Pas de GPU offload. Vérifie `--n-gpu-layers` dans le log. Sur Mac M-series,
recompile/réinstalle llama.cpp avec Metal. Sur Linux, avec `LLAMA_CUDA=1`.

---

## Intégration Justfile

Targets disponibles depuis la racine du repo :

```bash
just slm-download    # = ./scripts/slm/download_models.sh
just slm-start       # = ./scripts/slm/start.sh
just slm-stop        # = ./scripts/slm/stop.sh
just slm-restart     # = stop + start
just slm-status      # = ./scripts/slm/status.sh
just slm-health      # = ./scripts/slm/health.sh
just slm-logs ROBOT  # = ./scripts/slm/logs.sh ROBOT --tail
```

---

## Hors-scope S1 (pour info)

- **Docker** : un `docker-compose.slm.yml` est envisagé en Phase 4 (mais llama.cpp est plus simple en natif sur Mac M-series).
- **Init scripts systemd / launchd** : optionnel, à faire si déploiement persistant.
- **Health-check qui sonde réellement une inférence** (pas juste `/health`) : prévu en Sprint S2/S3 via les tests `live_smoke`.
- **Auto-restart en cas de crash** : non implémenté (préférer un orchestrateur externe si besoin prod).
