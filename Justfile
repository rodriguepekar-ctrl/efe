# EFE developer commands
# Usage: `just <recipe>` or `just` to list recipes
set shell := ["bash", "-euo", "pipefail", "-c"]

# Default: show available recipes
default:
    @just --list

# --- Bootstrap ---
bootstrap: bootstrap-system bootstrap-rust bootstrap-python bootstrap-hooks
    @echo "Bootstrap complete. Run 'just test-smoke' to verify."

bootstrap-system:
    #!/usr/bin/env bash
    if [[ "$OSTYPE" == "darwin"* ]]; then
        command -v brew >/dev/null || { echo "install homebrew first"; exit 1; }
        brew install cmake just uv gitleaks taplo ninja || true
    else
        sudo apt-get update && sudo apt-get install -y cmake ninja-build build-essential pkg-config
    fi

bootstrap-rust:
    rustup show
    rustup component add rustfmt clippy
    cargo install --locked cargo-nextest cargo-audit || true

bootstrap-python:
    uv sync --all-extras --dev

bootstrap-hooks:
    uv run pre-commit install
    uv run pre-commit install --hook-type commit-msg

# --- Build ---
build: build-rust build-py
    @echo "Build complete (skipped CUDA - use 'just build-cuda' if GPU available)."

build-rust:
    cargo build --workspace

build-cuda:
    cmake -B build -DCMAKE_BUILD_TYPE=Release
    cmake --build build -j

build-py:
    uv build

build-release:
    cargo build --workspace --release
    cmake -B build-release -DCMAKE_BUILD_TYPE=Release
    cmake --build build-release -j

# --- Test ---
test: test-fast test-unit
    @echo "All tests OK."

test-fast:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    uv run ruff check .
    uv run ruff format --check .

test-unit:
    cargo nextest run --workspace
    uv run pytest -q

test-integration:
    cargo nextest run --workspace --features integration || true
    uv run pytest tests/integration -q || true

test-smoke:
    cargo check --workspace
    uv run python -c "import efe; print(f'efe {efe.__version__}')"
    @echo "Smoke OK"

# --- Benchmarks ---
bench:
    cargo bench --workspace || true

# --- Formatting ---
fmt:
    cargo fmt --all
    uv run ruff format .
    taplo format || true

lint: test-fast

# --- Clean ---
clean:
    cargo clean
    rm -rf build build-release dist target
    find . -type d -name __pycache__ -exec rm -rf {} + 2>/dev/null || true
    find . -type d -name .pytest_cache -exec rm -rf {} + 2>/dev/null || true
    find . -type d -name .ruff_cache -exec rm -rf {} + 2>/dev/null || true
    find . -type d -name .mypy_cache -exec rm -rf {} + 2>/dev/null || true

# --- Docker ---
docker-dev:
    docker buildx build -t efe-dev:latest -f docker/Dockerfile.dev .

docker-prod:
    docker buildx build -t efe:latest -f docker/Dockerfile.prod .

docker-up:
    docker compose -f docker/compose.dev.yml up -d

docker-down:
    docker compose -f docker/compose.dev.yml down

# --- Secrets ---
secrets-fetch:
    @command -v op >/dev/null || { echo "install 1password CLI (op)"; exit 1; }
    op signin
    op inject -i .env.template -o .env
    @echo ".env populated."

# --- Cluster ---
cluster-connect:
    @echo "Connecting to GPU cluster..."
    ssh -F .ssh/config efe-gpu-dev

# --- Docs ---
docs-serve:
    cd docs && uv run mkdocs serve

docs-build:
    cd docs && uv run mkdocs build

# --- CI helpers ---
ci-local: test-fast test-unit
    @echo "Local CI OK"

# --- Version info ---
version:
    @grep '^version' Cargo.toml | head -1
    @grep '^version' pyproject.toml | head -1

# --- SLM (Phase 1 nano-robots — Phase 3 R2 bootstrap) ---
# Voir scripts/slm/README.md pour la documentation complète.

# Télécharge les 4 modèles GGUF (~4.1 GB cumulés, idempotent)
slm-download:
    ./scripts/slm/download_models.sh

# Vérifie la présence + taille des 4 modèles (sans téléchargement)
slm-check:
    ./scripts/slm/download_models.sh --check

# Démarre les 4 llama-server (ports 8090-8093, en arrière-plan)
slm-start:
    ./scripts/slm/start.sh

# Démarre 1 seul robot (analyzer | reformulator | compressor | verifier)
slm-start-one ROBOT:
    ./scripts/slm/start.sh {{ROBOT}}

# Arrête les 4 llama-server (SIGTERM + grace 5s + SIGKILL si nécessaire)
slm-stop:
    ./scripts/slm/stop.sh

# Redémarre les 4 llama-server (stop + start)
slm-restart: slm-stop slm-start

# Ping HTTP /health des 4 endpoints (exit code = nombre unhealthy)
slm-health:
    ./scripts/slm/health.sh

# Watch en boucle 2s du /health des 4 endpoints (Ctrl+C pour quitter)
slm-watch:
    ./scripts/slm/health.sh --watch

# État local (pidfile + log) sans toucher au réseau
slm-status:
    ./scripts/slm/status.sh

# tail -f du log d'un robot (ex: just slm-logs analyzer)
slm-logs ROBOT:
    ./scripts/slm/logs.sh {{ROBOT}} --tail

# tail -f multiplexé sur les logs des 4 robots
slm-logs-all:
    ./scripts/slm/logs.sh --all --tail

# --- Live tests Phase 3 R1/R2 (env-gated EFE_LIVE_LLM=1) ---
# Voir : crates/efe-coordinator/tests/{live_providers,live_missions}.rs

# Vérifie que les clés API live sont exportées (sans les afficher en clair)
live-check-keys:
    #!/usr/bin/env bash
    set -euo pipefail
    missing=()
    [[ -z "${ANTHROPIC_API_KEY:-}" ]] && missing+=(ANTHROPIC_API_KEY)
    [[ -z "${GOOGLE_AI_KEY:-}"     ]] && missing+=(GOOGLE_AI_KEY)
    [[ -z "${OPENAI_API_KEY:-}"    ]] && missing+=(OPENAI_API_KEY)
    if (( ${#missing[@]} > 0 )); then
        echo "❌ Clés API manquantes : ${missing[*]}"
        echo "Exporte-les avant de lancer les tests live :"
        echo "  export ANTHROPIC_API_KEY='sk-ant-...'"
        echo "  export GOOGLE_AI_KEY='AIza...'"
        echo "  export OPENAI_API_KEY='sk-...'"
        exit 1
    fi
    echo "✅ 3 clés API détectées :"
    echo "  ANTHROPIC: ${ANTHROPIC_API_KEY:0:10}... (${#ANTHROPIC_API_KEY} chars)"
    echo "  GOOGLE:    ${GOOGLE_AI_KEY:0:10}... (${#GOOGLE_AI_KEY} chars)"
    echo "  OPENAI:    ${OPENAI_API_KEY:0:10}... (${#OPENAI_API_KEY} chars)"

# Lance la suite live providers R1a/R1b (~18 tests, Claude+Gemini+GPT directs)
live-providers:
    EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_providers -- --test-threads=1 --nocapture

# Lance la suite live missions R2 (Mission A/B/Speculative via Coordinator)
# EFE_LIVE_SLM=1 activé par défaut : S3+ a besoin des daemons SLM UP.
# S2 (naked) skip silencieusement la vérification SLM, donc ça reste OK même
# si llama-server n'est pas démarré (S2 utilise juste Claude).
live-missions:
    EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_missions -- --nocapture

# Lance UNIQUEMENT le test Mission A naked (S2 — le plus rapide, ~1¢)
live-mission-naked:
    EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_missions live_mission_a_naked_no_slm_just_claude -- --nocapture

# Lance UNIQUEMENT le test S3 Mission A + analyzer + verifier (~1¢ + 2 SLM)
# Pré-requis : ./scripts/slm/start.sh + ANTHROPIC_API_KEY
live-mission-with-slm:
    EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_missions live_mission_a_with_analyzer_verifier -- --nocapture

# Lance UNIQUEMENT le test S4 Mission A FULL pre-chewing pipeline (~1-2¢ + 3 SLM)
# Mesure les ChewingMetrics (savings tokens via compressor).
# Pré-requis : analyzer (:8090) + compressor (:8092) + verifier (:8093) UP
live-mission-full-pipeline:
    EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_missions live_mission_a_full_pre_chewing_pipeline -- --nocapture

# Lance UNIQUEMENT le test S5 Mission B 3-LLM consensus (~3-5¢ + 3 SLM + 3 clés API)
# Démontre la multiplication économique : savings × 3 LLMs via pré-mâchage MUTUALISÉ.
# Pré-requis : ANTHROPIC_API_KEY + GOOGLE_AI_KEY + OPENAI_API_KEY + analyzer/compressor/verifier UP
live-mission-b-3llm:
    EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_missions live_mission_b_full_3llm_consensus_with_pre_chewing -- --nocapture

# Lance TOUS les tests live missions R2 (S2 + S3 + S4 + S5) en séquence
# Total estimé : ~30-45s, ~5-10¢ pour les 4 tests cumulés
live-all-missions:
    EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_missions -- --nocapture

# Lance TOUS les tests live R1+R2 (providers nus + missions S2+S3) en séquence
# Note : EFE_LIVE_SLM=1 + slm-start (les daemons sont requis pour S3+)
live-all: live-check-keys
    @./scripts/slm/health.sh 2>/dev/null | grep -q "UP" || ( echo "⚠ Démarrage SLM..." && ./scripts/slm/start.sh )
    EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_providers -- --test-threads=1 --nocapture
    EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator --features all-providers \
        --test live_missions -- --nocapture

# Vérifie qu'il n'y a aucune régression sur la suite offline (sans clés)
live-no-regression:
    cargo test -p efe-coordinator --features all-providers --tests 2>&1 \
        | grep -E "test result" | tail -30

# Vérifie l'inférence réelle (POST /completion, pas juste /health)
slm-verify:
    ./scripts/slm/verify_inference.sh

# Bench latence : 5 requêtes × 4 robots, détecte si GPU est actif
slm-bench:
    ./scripts/slm/bench.sh

# Bench long : 20 requêtes par robot (mesure p99 fiable)
slm-bench-long:
    ./scripts/slm/bench.sh -n 20

# Génère .env.slm avec les exports ANALYZER_URL, REFORMULATOR_URL, etc.
slm-env:
    ./scripts/slm/print_env.sh > .env.slm
    @echo ".env.slm écrit. À sourcer : source .env.slm"

# Méta-validation end-to-end Sprint S1 (le critère d'acceptation S1)
slm-smoke:
    ./scripts/slm/e2e_smoke.sh

# Smoke version : télécharge les modèles avant + garde UP en fin
slm-smoke-keep:
    ./scripts/slm/e2e_smoke.sh --download --keep-running

# ─────────────────────────────────────────────────────────────────────────
# PoC Scout & Synthesize — démo une-commande (cf. SCOPE.md / README_POC.md)
# ─────────────────────────────────────────────────────────────────────────

# Démarre le PoC complet (SLM + bridge + UI) → http://localhost:8700.
# Flags forwardés : `just poc-up --real`, `just poc-up --no-slm`, etc.
poc-up *ARGS:
    ./poc.sh up {{ARGS}}

# Arrêt propre du PoC (bridge + SLM).
poc-down:
    ./poc.sh down

# Santé des processus du PoC.
poc-status:
    ./poc.sh status
