#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/stop.sh — Arrête proprement les llama-server des 4 nano-robots.
#
# Séquence : SIGTERM → wait 5s → SIGKILL si toujours vivant.
# Nettoie aussi les pidfiles orphelins.
#
# Usage :
#   ./scripts/slm/stop.sh                  # stoppe tous
#   ./scripts/slm/stop.sh analyzer         # stoppe 1 seul
#   ./scripts/slm/stop.sh --force          # SIGKILL directement (pas de SIGTERM)
#   ./scripts/slm/stop.sh -h               # aide

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

# ─── Parsing arguments ──────────────────────────────────────────────────────
FORCE_KILL=0
TARGET_ROBOT=""
GRACE_PERIOD="${EFE_SLM_GRACE:-5}"   # secondes avant SIGKILL après SIGTERM

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] [ROBOT_NAME]

Arrête les llama-server des 4 nano-robots Phase 1.

Options :
  --force, -f       SIGKILL direct (skip SIGTERM)
  --grace SEC       Délai SIGTERM → SIGKILL (défaut: ${GRACE_PERIOD}s)
  --help, -h        Aide

ROBOT_NAME (optionnel) : un parmi ${ROBOT_NAMES[*]}.
Si absent, stoppe les 4.

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --force|-f)   FORCE_KILL=1; shift ;;
        --grace)      GRACE_PERIOD="$2"; shift 2 ;;
        --help|-h)    usage; exit 0 ;;
        --*)          log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)            TARGET_ROBOT="$1"; shift ;;
    esac
done

# ─── Sélection des robots à stopper ─────────────────────────────────────────
INDICES_TO_STOP=()
if [[ -z "${TARGET_ROBOT}" ]]; then
    for i in "${!ROBOT_NAMES[@]}"; do INDICES_TO_STOP+=("$i"); done
else
    idx="$(robot_index_by_name "${TARGET_ROBOT}")" || exit 1
    INDICES_TO_STOP+=("${idx}")
fi

# ─── Stop séquentiel ────────────────────────────────────────────────────────
log_step "Arrêt des llama-server"

stopped=()
already_dead=()
forced=()

for i in "${INDICES_TO_STOP[@]}"; do
    name="$(robot_name "$i")"
    pid_file="$(robot_pid_path "$i")"

    if ! pid="$(read_pid_alive "${pid_file}")"; then
        # Soit pas de pidfile, soit process déjà mort.
        if [[ -f "${pid_file}" ]]; then
            log_warn "${name} : pidfile orphelin (process déjà mort), cleanup"
            rm -f "${pid_file}"
        else
            log_info "${name} : pas démarré (rien à faire)"
        fi
        already_dead+=("${name}")
        continue
    fi

    if (( FORCE_KILL == 1 )); then
        log_warn "${name} : SIGKILL (pid ${pid})"
        kill -KILL "${pid}" 2>/dev/null || true
        rm -f "${pid_file}"
        forced+=("${name}")
        stopped+=("${name}")
        continue
    fi

    # Graceful : SIGTERM puis attente, puis SIGKILL si nécessaire.
    log_info "${name} : SIGTERM (pid ${pid})"
    kill -TERM "${pid}" 2>/dev/null || true

    elapsed=0
    while (( elapsed < GRACE_PERIOD )) && pid_alive "${pid}"; do
        sleep 1
        elapsed=$(( elapsed + 1 ))
    done

    if pid_alive "${pid}"; then
        log_warn "${name} : pas mort après ${GRACE_PERIOD}s, SIGKILL"
        kill -KILL "${pid}" 2>/dev/null || true
        sleep 1
        forced+=("${name}")
    fi

    if pid_alive "${pid}"; then
        log_err "${name} : impossible de tuer le process (pid ${pid}). Investiguer manuellement."
        # On laisse le pidfile pour debug.
    else
        log_ok "${name} : arrêté"
        rm -f "${pid_file}"
        stopped+=("${name}")
    fi
done

# ─── Résumé ─────────────────────────────────────────────────────────────────
log_step "Résumé arrêt"
if (( ${#stopped[@]} > 0 )); then
    log_ok "Arrêtés (${#stopped[@]})       : ${stopped[*]}"
fi
if (( ${#forced[@]} > 0 )); then
    log_warn "Forcés SIGKILL (${#forced[@]}) : ${forced[*]}"
fi
if (( ${#already_dead[@]} > 0 )); then
    log_info "Déjà OFF (${#already_dead[@]})    : ${already_dead[*]}"
fi
