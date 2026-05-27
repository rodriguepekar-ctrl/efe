#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/lib_common.sh — Configuration et helpers partagés pour les 4
# nano-robot llama-server (Phase 1, ports 8090-8093).
#
# Source-only : NE PAS exécuter directement. Doit être sourcé par les autres
# scripts via `source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"`.
#
# Conventions :
#   - Repo root déterminé via `git rev-parse --show-toplevel`
#   - Modèles GGUF : <repo>/models/<robot>.gguf
#   - PIDs        : <repo>/.llama_pids/<robot>.pid
#   - Logs        : <repo>/logs/llama-server/<robot>.log
#   - Compatible bash 4+ et zsh (utilise arrays indexés simples, pas associatifs)

# Garde-fou : empêche l'exécution directe
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    echo "ERREUR : lib_common.sh doit être sourcé, pas exécuté directement." >&2
    exit 1
fi

# ─── Mode strict ────────────────────────────────────────────────────────────
set -euo pipefail

# ─── Détection de l'OS ──────────────────────────────────────────────────────
_OS_TYPE="$(uname -s)"
case "${_OS_TYPE}" in
    Darwin)  EFE_SLM_OS="macos"  ;;
    Linux)   EFE_SLM_OS="linux"  ;;
    *)       EFE_SLM_OS="unknown" ;;
esac
export EFE_SLM_OS

# ─── Détection nombre de cœurs (pour --threads) ─────────────────────────────
if [[ "${EFE_SLM_OS}" == "macos" ]]; then
    EFE_SLM_NPROC="$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"
else
    EFE_SLM_NPROC="$(nproc 2>/dev/null || echo 4)"
fi
# Cap : pas plus de 8 threads par robot (4 robots × 8 = 32, trop pour 4 robots simultanés).
# Sur Mac M-series 8-core, 2 threads par robot est plus que suffisant car Metal porte la charge.
EFE_SLM_THREADS_PER_ROBOT="$(( EFE_SLM_NPROC / 4 ))"
if (( EFE_SLM_THREADS_PER_ROBOT < 2 )); then
    EFE_SLM_THREADS_PER_ROBOT=2
fi
if (( EFE_SLM_THREADS_PER_ROBOT > 4 )); then
    EFE_SLM_THREADS_PER_ROBOT=4
fi
export EFE_SLM_THREADS_PER_ROBOT

# ─── Repo root ──────────────────────────────────────────────────────────────
if ! EFE_REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "ERREUR : impossible de détecter le repo git. Exécute les scripts depuis un clone EFE." >&2
    exit 1
fi
export EFE_REPO_ROOT

# ─── Dossiers conventionnels ────────────────────────────────────────────────
EFE_SLM_MODELS_DIR="${EFE_REPO_ROOT}/models"
EFE_SLM_PIDS_DIR="${EFE_REPO_ROOT}/.llama_pids"
EFE_SLM_LOGS_DIR="${EFE_REPO_ROOT}/logs/llama-server"
export EFE_SLM_MODELS_DIR EFE_SLM_PIDS_DIR EFE_SLM_LOGS_DIR

# ─── Couleurs ANSI ──────────────────────────────────────────────────────────
# Désactivable via `NO_COLOR=1` (convention `no-color.org`)
if [[ -z "${NO_COLOR:-}" ]] && [[ -t 1 || "${FORCE_COLOR:-}" == "1" ]]; then
    EFE_C_RESET=$'\033[0m'
    EFE_C_BOLD=$'\033[1m'
    EFE_C_DIM=$'\033[2m'
    EFE_C_RED=$'\033[31m'
    EFE_C_GREEN=$'\033[32m'
    EFE_C_YELLOW=$'\033[33m'
    EFE_C_BLUE=$'\033[34m'
    EFE_C_CYAN=$'\033[36m'
else
    EFE_C_RESET=""
    EFE_C_BOLD=""
    EFE_C_DIM=""
    EFE_C_RED=""
    EFE_C_GREEN=""
    EFE_C_YELLOW=""
    EFE_C_BLUE=""
    EFE_C_CYAN=""
fi
export EFE_C_RESET EFE_C_BOLD EFE_C_DIM EFE_C_RED EFE_C_GREEN EFE_C_YELLOW EFE_C_BLUE EFE_C_CYAN

# ─── Logging helpers ─────────────────────────────────────────────────────────
log_info()  { echo "${EFE_C_BLUE}[INFO]${EFE_C_RESET}  $*"; }
log_ok()    { echo "${EFE_C_GREEN}[ OK ]${EFE_C_RESET}  $*"; }
log_warn()  { echo "${EFE_C_YELLOW}[WARN]${EFE_C_RESET}  $*" >&2; }
log_err()   { echo "${EFE_C_RED}[ERR ]${EFE_C_RESET}  $*" >&2; }
log_step()  { echo ""; echo "${EFE_C_BOLD}${EFE_C_CYAN}━━━ $* ━━━${EFE_C_RESET}"; }

# ─── Configuration des 4 nano-robots ────────────────────────────────────────
# Arrays indexés parallèles (portable bash 3+/zsh).
#
# ROBOT_NAMES        — slug ASCII utilisé partout (filename, pidfile, alias)
# ROBOT_PORTS        — port HTTP du llama-server
# ROBOT_MODEL_LABELS — nom humain du modèle (pour logs et MODELS_DOWNLOAD.md)
# ROBOT_HF_URLS      — URL HuggingFace direct-download du GGUF
# ROBOT_SIZES_MB     — taille attendue du fichier GGUF (pour sanity-check)
# ROBOT_CTX_SIZES    — taille de contexte par défaut (KV cache)

ROBOT_NAMES=(analyzer reformulator compressor verifier)
ROBOT_PORTS=(8090 8091 8092 8093)
ROBOT_MODEL_LABELS=(
    "SmolLM2-360M-Instruct Q5_K_M"
    "Qwen3-0.6B Q5_K_M"
    "Llama-3.2-1B-Instruct Q5_K_M"
    "Phi-4-mini-instruct Q4_K_M"
)
ROBOT_HF_URLS=(
    "https://huggingface.co/HuggingFaceTB/SmolLM2-360M-Instruct-GGUF/resolve/main/smollm2-360m-instruct-q5_k_m.gguf"
    "https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q5_K_M.gguf"
    "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q5_K_M.gguf"
    "https://huggingface.co/bartowski/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-Q4_K_M.gguf"
)
ROBOT_SIZES_MB=(290 444 912 2500)
ROBOT_CTX_SIZES=(2048 2048 4096 4096)

# Sanity : tous les arrays doivent avoir 4 entrées.
_NB_ROBOTS=4
for _arr_name in ROBOT_NAMES ROBOT_PORTS ROBOT_MODEL_LABELS ROBOT_HF_URLS ROBOT_SIZES_MB ROBOT_CTX_SIZES; do
    eval "_count=\${#${_arr_name}[@]}"
    if (( _count != _NB_ROBOTS )); then
        echo "ERREUR INTERNE : tableau ${_arr_name} a ${_count} entrées, ${_NB_ROBOTS} attendues." >&2
        exit 1
    fi
done

# ─── Helpers d'accès aux configs par index ──────────────────────────────────
# Pourquoi des helpers ? Pour cacher la syntaxe array et garder les scripts lisibles.

robot_name()        { echo "${ROBOT_NAMES[$1]}"; }
robot_port()        { echo "${ROBOT_PORTS[$1]}"; }
robot_label()       { echo "${ROBOT_MODEL_LABELS[$1]}"; }
robot_hf_url()      { echo "${ROBOT_HF_URLS[$1]}"; }
robot_size_mb()     { echo "${ROBOT_SIZES_MB[$1]}"; }
robot_ctx_size()    { echo "${ROBOT_CTX_SIZES[$1]}"; }
robot_model_path()  { echo "${EFE_SLM_MODELS_DIR}/$(robot_name "$1").gguf"; }
robot_pid_path()    { echo "${EFE_SLM_PIDS_DIR}/$(robot_name "$1").pid"; }
robot_log_path()    { echo "${EFE_SLM_LOGS_DIR}/$(robot_name "$1").log"; }

# robot_index_by_name <name> → 0..3 sur stdout, exit 1 si nom inconnu.
robot_index_by_name() {
    local needle="$1"
    local i
    for i in "${!ROBOT_NAMES[@]}"; do
        if [[ "${ROBOT_NAMES[$i]}" == "${needle}" ]]; then
            echo "${i}"
            return 0
        fi
    done
    log_err "Robot inconnu : '${needle}'. Connus : ${ROBOT_NAMES[*]}."
    return 1
}

# ─── Vérifications environnement ────────────────────────────────────────────

# Vérifie qu'une commande est dans le PATH.
require_cmd() {
    local cmd="$1"
    if ! command -v "${cmd}" >/dev/null 2>&1; then
        log_err "Commande manquante : '${cmd}'. Installation requise."
        return 1
    fi
    return 0
}

# Vérifie que llama-server est installé et compatible.
# Retourne 0 si OK, 1 sinon.
require_llama_server() {
    if ! command -v llama-server >/dev/null 2>&1; then
        log_err "Binaire 'llama-server' introuvable dans \$PATH."
        cat >&2 <<'EOF'

Installation :
  • macOS Homebrew :   brew install llama.cpp
  • Linux (compilé) :  voir https://github.com/ggml-org/llama.cpp#build
  • Vérification :     llama-server --version

Version requise : b9297 ou supérieure (cf. llama_server.rs:4).
EOF
        return 1
    fi
    return 0
}

# Vérifie qu'un port TCP est libre sur 127.0.0.1.
# Retourne 0 si libre, 1 si occupé.
port_is_free() {
    local port="$1"
    # macOS : lsof. Linux : ss ou lsof. Fallback : nc.
    if command -v lsof >/dev/null 2>&1; then
        ! lsof -nP -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1
    elif command -v ss >/dev/null 2>&1; then
        ! ss -ltn "sport = :${port}" 2>/dev/null | grep -q ":${port}"
    elif command -v nc >/dev/null 2>&1; then
        ! nc -z 127.0.0.1 "${port}" >/dev/null 2>&1
    else
        log_warn "Aucun outil disponible pour vérifier le port ${port} (lsof/ss/nc absents). Skip check."
        return 0
    fi
}

# Affiche le PID qui occupe un port (best-effort, vide si rien trouvé).
who_owns_port() {
    local port="$1"
    if command -v lsof >/dev/null 2>&1; then
        lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null | head -1
    elif command -v ss >/dev/null 2>&1; then
        ss -ltnp "sport = :${port}" 2>/dev/null | grep -oE 'pid=[0-9]+' | head -1 | sed 's/pid=//'
    fi
}

# Vérifie qu'un PID existe (process actif).
pid_alive() {
    local pid="$1"
    [[ -n "${pid}" ]] && kill -0 "${pid}" >/dev/null 2>&1
}

# Lit un pidfile et retourne le PID si le process est encore vivant, vide sinon.
read_pid_alive() {
    local pid_file="$1"
    if [[ ! -f "${pid_file}" ]]; then
        return 1
    fi
    local pid
    pid="$(cat "${pid_file}" 2>/dev/null || true)"
    if pid_alive "${pid}"; then
        echo "${pid}"
        return 0
    fi
    return 1
}

# Ping HTTP /health avec timeout court. Retourne 0 si 200, 1 sinon.
http_health_ok() {
    local port="$1"
    local timeout="${2:-2}"
    curl -sf -m "${timeout}" "http://127.0.0.1:${port}/health" >/dev/null 2>&1
}

# Initialise les répertoires runtime (créés si absents, idempotent).
ensure_runtime_dirs() {
    mkdir -p "${EFE_SLM_MODELS_DIR}" "${EFE_SLM_PIDS_DIR}" "${EFE_SLM_LOGS_DIR}"
}

# Affiche un résumé de la configuration courante (utile en --help/--info).
print_config_summary() {
    echo "${EFE_C_BOLD}EFE SLM bootstrap configuration${EFE_C_RESET}"
    echo "  OS               : ${EFE_SLM_OS}"
    echo "  CPU cores        : ${EFE_SLM_NPROC}  (threads/robot : ${EFE_SLM_THREADS_PER_ROBOT})"
    echo "  Repo root        : ${EFE_REPO_ROOT}"
    echo "  Models dir       : ${EFE_SLM_MODELS_DIR}"
    echo "  PIDs dir         : ${EFE_SLM_PIDS_DIR}"
    echo "  Logs dir         : ${EFE_SLM_LOGS_DIR}"
    echo ""
    echo "${EFE_C_BOLD}Nano-robots${EFE_C_RESET}"
    printf "  %-13s %-6s %-32s %6s  %s\n" "NAME" "PORT" "MODEL" "SIZE" "CTX"
    local i
    for i in "${!ROBOT_NAMES[@]}"; do
        printf "  %-13s %-6s %-32s %4s MB  %s\n" \
            "$(robot_name "$i")" \
            "$(robot_port "$i")" \
            "$(robot_label "$i")" \
            "$(robot_size_mb "$i")" \
            "$(robot_ctx_size "$i")"
    done
}
