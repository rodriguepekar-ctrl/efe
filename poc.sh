#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# poc.sh — Orchestrateur UNE-COMMANDE du PoC EFE (Scout & Synthesize).
#
# Lance/arrête/inspecte la démo complète : 4 llama-server SLM + le bridge
# `efe-preview-server` (qui sert l'UI Preview Lab ET l'API SSE sur un seul port).
#
#   ./poc.sh up        # build + démarre tout → http://localhost:8700
#   ./poc.sh down      # arrêt propre (bridge + SLM)
#   ./poc.sh status    # santé des processus
#   ./poc.sh -h        # aide
#
# Config : ./poc.env (sourcé ici). Flags CLI de `up` : --real, --no-slm,
#          --no-ui, --rebuild-ui (surchargent poc.env).
#
# Conçu pour macOS bash 3.2 (pas de `mapfile`/bash4). Tous les chemins sont
# quotés (le dépôt peut être dans un dossier à espaces, ex. "jjjj copie/efe").
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_ROOT"

POC_PID_DIR="$REPO_ROOT/.poc_pids"
POC_LOG_DIR="$REPO_ROOT/logs/poc"
BRIDGE_PIDFILE="$POC_PID_DIR/bridge.pid"
BRIDGE_LOG="$POC_LOG_DIR/bridge.log"

# ── Couleurs (respecte NO_COLOR) ────────────────────────────────────────────
if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
    C_B=$'\033[1m'; C_G=$'\033[32m'; C_Y=$'\033[33m'; C_R=$'\033[31m'; C_D=$'\033[2m'; C_0=$'\033[0m'
else
    C_B=""; C_G=""; C_Y=""; C_R=""; C_D=""; C_0=""
fi
info() { echo "${C_D}·${C_0} $*"; }
ok()   { echo "${C_G}✓${C_0} $*"; }
warn() { echo "${C_Y}⚠${C_0} $*" >&2; }
err()  { echo "${C_R}✗${C_0} $*" >&2; }
step() { echo ""; echo "${C_B}── $* ──${C_0}"; }

# ── Config : env ambiant > poc.env > défaut intégré ────────────────────────
# poc.env utilise la forme `${VAR:-...}` → une variable déjà présente dans l'env
# (ex. `POC_BRIDGE_ADDR=127.0.0.1:8788 ./poc.sh up`) a PRIORITÉ sur le fichier.
if [[ -f "$REPO_ROOT/poc.env" ]]; then
    # shellcheck disable=SC1091
    source "$REPO_ROOT/poc.env"
fi
# Défauts intégrés (si poc.env absent ou variable non définie). `:-` ne clobbe
# jamais une valeur déjà fournie (env ambiant ou fichier).
POC_MODE="${POC_MODE:-keyless}"
POC_BRIDGE_ADDR="${POC_BRIDGE_ADDR:-127.0.0.1:8700}"
POC_LIVE_SLM="${POC_LIVE_SLM:-1}"
POC_BUILD_UI="${POC_BUILD_UI:-1}"
POC_RELEASE="${POC_RELEASE:-1}"
POC_CORS_ORIGIN="${POC_CORS_ORIGIN:-http://localhost:3000}"
POC_TOKEN="${POC_TOKEN:-}"
POC_SAVINGS_REPORT="${POC_SAVINGS_REPORT:-}"
POC_BRIDGE_HEALTH_TIMEOUT="${POC_BRIDGE_HEALTH_TIMEOUT:-30}"

BRIDGE_PORT="${POC_BRIDGE_ADDR##*:}"
BRIDGE_HOST="${POC_BRIDGE_ADDR%:*}"

# ── Helpers ─────────────────────────────────────────────────────────────────

# PID qui ÉCOUTE sur un port TCP (vide si libre). macOS/Linux via lsof.
port_pid() {
    lsof -ti "tcp:$1" -sTCP:LISTEN 2>/dev/null | head -1 || true
}

pid_alive() { kill -0 "$1" 2>/dev/null; }

# Attend qu'une URL réponde 200 (timeout en s). 0 = ok, 1 = timeout.
wait_health() {
    local url="$1" timeout="$2" elapsed=0
    while (( elapsed < timeout )); do
        if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then return 0; fi
        sleep 1; elapsed=$(( elapsed + 1 ))
        printf "."
    done
    return 1
}

require() {
    command -v "$1" >/dev/null 2>&1 || { err "Prérequis manquant : '$1' introuvable dans le PATH."; return 1; }
}

# ── up ──────────────────────────────────────────────────────────────────────
cmd_up() {
    # Flags (surchargent poc.env)
    local rebuild_ui=0
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --real)        POC_MODE="real"; shift ;;
            --keyless)     POC_MODE="keyless"; shift ;;
            --no-slm)      POC_LIVE_SLM="0"; shift ;;
            --no-ui)       POC_BUILD_UI="0"; shift ;;
            --rebuild-ui)  rebuild_ui=1; shift ;;
            *)             err "Option inconnue pour up : $1"; return 1 ;;
        esac
    done

    echo "${C_B}EFE PoC — démarrage (mode: ${POC_MODE})${C_0}"
    mkdir -p "$POC_PID_DIR" "$POC_LOG_DIR"
    require cargo || return 1
    require curl  || return 1

    # ── Couche LLM (mode) → variables EFE_* du bridge ──
    local features=""
    if [[ "$POC_MODE" == "keyless" ]]; then
        export EFE_PREVIEW_MOCK=1
        info "LLM = MOCK (déterministe, gratuit)."
    else
        unset EFE_PREVIEW_MOCK || true
        features="--features real-llm"
        if [[ -z "${ANTHROPIC_API_KEY:-}${OPENAI_API_KEY:-}${GOOGLE_API_KEY:-}" ]]; then
            warn "Mode real mais AUCUNE clé en env (ANTHROPIC_API_KEY/OPENAI_API_KEY/GOOGLE_API_KEY)."
            warn "Le bridge retombera en MOCK. Exporte tes clés AVANT, ou reste en keyless."
        else
            info "LLM = RÉEL (clés détectées en env ; jamais lues par poc.sh)."
        fi
    fi

    # ── Couche SLM (réutilise scripts/slm/start.sh : port-guard + health intégrés) ──
    if [[ "$POC_LIVE_SLM" == "1" ]]; then
        step "Démarrage des SLM (scripts/slm/start.sh)"
        if "$REPO_ROOT/scripts/slm/start.sh"; then
            export EFE_LIVE_SLM=1
            ok "SLM opérationnels (pré-mâchage réel)."
        else
            warn "Démarrage SLM échoué (modèles manquants ? → ./scripts/slm/download_models.sh)."
            warn "On continue en mode DÉGRADÉ : pré-mâchage simulé par le moteur (EFE_LIVE_SLM=0)."
            export EFE_LIVE_SLM=0
        fi
    else
        export EFE_LIVE_SLM=0
        info "SLM désactivé (POC_LIVE_SLM=0) — pré-mâchage simulé par le moteur."
    fi

    # ── UI (export statique Next.js, servi par le bridge via static_dir) ──
    if [[ "$POC_BUILD_UI" == "1" ]]; then
        local ui_dir="$REPO_ROOT/preview-lab"
        local out_dir="$ui_dir/out"
        if [[ ! -d "$out_dir" || "$rebuild_ui" == "1" ]]; then
            step "Build de l'UI (preview-lab)"
            if command -v pnpm >/dev/null 2>&1; then
                ( cd "$ui_dir" && pnpm install --ignore-workspace && pnpm build )
            elif command -v npm >/dev/null 2>&1; then
                ( cd "$ui_dir" && npm install && npm run build )
            else
                warn "Ni pnpm ni npm → UI non buildée. API seule. (installe Node, ou --no-ui)"
            fi
        else
            info "UI déjà buildée ($out_dir) — saute (./poc.sh up --rebuild-ui pour reconstruire)."
        fi
        if [[ -d "$out_dir" ]]; then
            export EFE_PREVIEW_STATIC_DIR="$out_dir"
            ok "UI servie par le bridge (same-origin)."
        fi
    else
        info "UI désactivée (POC_BUILD_UI=0) — API seule."
    fi

    # ── Bridge : env restant ──
    export EFE_PREVIEW_ADDR="$POC_BRIDGE_ADDR"
    export EFE_PREVIEW_CORS_ORIGIN="$POC_CORS_ORIGIN"
    [[ -n "$POC_TOKEN" ]] && export EFE_PREVIEW_TOKEN="$POC_TOKEN"
    [[ -n "$POC_SAVINGS_REPORT" ]] && export EFE_PREVIEW_SAVINGS_REPORT="$POC_SAVINGS_REPORT"

    # ── Port-guard anti-AddrInUse (la leçon de la session) ──
    local existing; existing="$(port_pid "$BRIDGE_PORT")"
    if [[ -n "$existing" ]]; then
        local ours=""
        if [[ -f "$BRIDGE_PIDFILE" ]]; then ours="$(cat "$BRIDGE_PIDFILE" 2>/dev/null || true)"; fi
        if [[ -n "$ours" && "$ours" == "$existing" ]]; then
            ok "Bridge déjà en cours (pid $existing) sur :$BRIDGE_PORT — rien à faire."
            print_ready
            return 0
        fi
        err "Port :$BRIDGE_PORT déjà occupé par le pid $existing (pas notre bridge)."
        err "Libère-le (./poc.sh down, ou kill $existing) puis relance."
        return 1
    fi

    # ── Build + lancement du bridge (binaire direct → PID propre) ──
    step "Build du bridge (efe-preview-server)"
    local profile_flag="" bin_dir="debug"
    if [[ "$POC_RELEASE" == "1" ]]; then profile_flag="--release"; bin_dir="release"; fi
    # shellcheck disable=SC2086
    cargo build --manifest-path "$REPO_ROOT/Cargo.toml" -p efe-preview-server $profile_flag $features
    local bridge_bin="$REPO_ROOT/target/$bin_dir/efe-preview-server"
    [[ -x "$bridge_bin" ]] || { err "Binaire bridge introuvable : $bridge_bin"; return 1; }

    step "Lancement du bridge sur http://$POC_BRIDGE_ADDR"
    [[ -f "$BRIDGE_LOG" ]] && mv -f "$BRIDGE_LOG" "$BRIDGE_LOG.prev"
    nohup "$bridge_bin" >"$BRIDGE_LOG" 2>&1 < /dev/null &
    local pid=$!
    echo "$pid" > "$BRIDGE_PIDFILE"
    info "PID $pid (log : $BRIDGE_LOG)"

    sleep 1
    if ! pid_alive "$pid"; then
        err "Le bridge a crashé immédiatement. 5 dernières lignes :"
        tail -5 "$BRIDGE_LOG" 2>/dev/null | sed 's/^/    /'
        rm -f "$BRIDGE_PIDFILE"
        return 1
    fi

    printf "  ${C_D}attente health${C_0}"
    if wait_health "http://$POC_BRIDGE_ADDR/healthz" "$POC_BRIDGE_HEALTH_TIMEOUT"; then
        printf "\r"; ok "Bridge READY (port $BRIDGE_PORT)."
    else
        printf "\r"; err "Bridge TIMEOUT (${POC_BRIDGE_HEALTH_TIMEOUT}s sans /healthz). Voir $BRIDGE_LOG"
        return 1
    fi

    print_ready
}

print_ready() {
    echo ""
    ok "${C_B}PoC opérationnel → http://$POC_BRIDGE_ADDR${C_0}"
    info "UI + API sur le même port. Prompts de démo : poc-demo-prompts.md"
    info "Arrêt : ./poc.sh down   ·   Santé : ./poc.sh status"
}

# ── down ────────────────────────────────────────────────────────────────────
cmd_down() {
    step "Arrêt du bridge"
    if [[ -f "$BRIDGE_PIDFILE" ]]; then
        local pid; pid="$(cat "$BRIDGE_PIDFILE" 2>/dev/null || true)"
        if [[ -n "$pid" ]] && pid_alive "$pid"; then
            kill -TERM "$pid" 2>/dev/null || true
            ok "Bridge arrêté (pid $pid)."
        else
            info "Bridge déjà arrêté."
        fi
        rm -f "$BRIDGE_PIDFILE"
    else
        info "Aucun bridge à arrêter (pas de pidfile)."
    fi

    step "Arrêt des SLM (scripts/slm/stop.sh)"
    if [[ -x "$REPO_ROOT/scripts/slm/stop.sh" ]]; then
        "$REPO_ROOT/scripts/slm/stop.sh" || warn "stop.sh a renvoyé une erreur (déjà arrêtés ?)."
    else
        info "scripts/slm/stop.sh absent — rien à faire côté SLM."
    fi
    ok "Arrêt terminé."
}

# ── status ──────────────────────────────────────────────────────────────────
cmd_status() {
    step "Bridge"
    local listener; listener="$(port_pid "$BRIDGE_PORT")"
    if [[ -n "$listener" ]] && curl -fsS --max-time 2 "http://$POC_BRIDGE_ADDR/healthz" >/dev/null 2>&1; then
        ok "Bridge UP sur http://$POC_BRIDGE_ADDR (pid $listener)."
    elif [[ -n "$listener" ]]; then
        warn "Quelque chose écoute sur :$BRIDGE_PORT (pid $listener) mais /healthz ne répond pas."
    else
        err "Bridge DOWN (rien sur :$BRIDGE_PORT)."
    fi

    step "SLM (scripts/slm/status.sh)"
    if [[ -x "$REPO_ROOT/scripts/slm/status.sh" ]]; then
        "$REPO_ROOT/scripts/slm/status.sh" || true
    else
        info "scripts/slm/status.sh absent."
    fi
}

# ── demo ──────────────────────────────────────────────────────────────────
# Joue les 3 scénarios (CHEWER / BYPASS / compromis) en headless contre le
# bridge en marche, et imprime un tableau de contraste. Prouve la démo sans UI.
cmd_demo() {
    require python3 || return 1
    local url="http://$POC_BRIDGE_ADDR"
    if ! curl -fsS --max-time 3 "$url/healthz" >/dev/null 2>&1; then
        err "Bridge injoignable sur $url — lance d'abord : ./poc.sh up"
        return 1
    fi
    python3 "$REPO_ROOT/scripts/poc/demo.py" --url "$url" "$@"
}

usage() {
    cat <<EOF
${C_B}poc.sh — orchestrateur une-commande du PoC EFE${C_0}

USAGE :
  ./poc.sh up [OPTIONS]   Build + démarre tout (SLM + bridge + UI) → http://$POC_BRIDGE_ADDR
  ./poc.sh demo           Joue les 3 scénarios en headless (tableau de contraste)
  ./poc.sh down           Arrêt propre (bridge + SLM)
  ./poc.sh status         Santé des processus
  ./poc.sh -h | --help    Cette aide

OPTIONS de 'up' (surchargent poc.env) :
  --real         Mode réel (vrais LLM ; clés en env requises)
  --keyless      Mode keyless (LLM mock gratuit + SLM réels) [défaut]
  --no-slm       Ne pas lancer les SLM (pré-mâchage simulé par le moteur)
  --no-ui        Ne pas builder/servir l'UI (API seule)
  --rebuild-ui   Force la reconstruction de l'export UI

CONFIG : ${C_D}poc.env${C_0}   ·   DOC : ${C_D}README_POC.md${C_0}   ·   PÉRIMÈTRE : ${C_D}SCOPE.md${C_0}
EOF
}

# ── Dispatch ─────────────────────────────────────────────────────────────────
cmd="${1:-}"
[[ $# -gt 0 ]] && shift || true
case "$cmd" in
    up)              cmd_up "$@" ;;
    demo)            cmd_demo "$@" ;;
    down)            cmd_down ;;
    status)          cmd_status ;;
    -h|--help|help|"") usage ;;
    *)               err "Commande inconnue : $cmd"; usage; exit 1 ;;
esac
