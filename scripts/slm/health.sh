#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/health.sh — Vérifie l'état de santé des 4 llama-server (GET /health).
#
# Exit codes :
#   0  → 4/4 healthy
#   N  → N robots unhealthy (1 ≤ N ≤ 4)
#
# Sortie : tableau coloré + ligne résumé.
#
# Usage :
#   ./scripts/slm/health.sh                 # check les 4
#   ./scripts/slm/health.sh analyzer        # check 1 seul
#   ./scripts/slm/health.sh --watch         # check en boucle (2s)
#   ./scripts/slm/health.sh --quiet         # silence, exit code uniquement
#   ./scripts/slm/health.sh --json          # output JSON

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

# ─── Parsing arguments ──────────────────────────────────────────────────────
WATCH=0
QUIET=0
JSON_OUTPUT=0
TARGET_ROBOT=""
TIMEOUT="${EFE_SLM_HEALTH_PING_TIMEOUT:-2}"

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] [ROBOT_NAME]

Vérifie l'état des llama-server des 4 nano-robots Phase 1.

Options :
  --watch, -w       Refresh en boucle toutes les 2 secondes (Ctrl+C pour sortir)
  --quiet, -q       Aucune sortie, exit code uniquement
  --json            Sortie JSON (pour scripts)
  --timeout SEC     Timeout HTTP par ping (défaut: ${TIMEOUT}s)
  --help, -h        Aide

Exit codes :
  0   tous healthy
  1-4 nombre de robots unhealthy

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --watch|-w)   WATCH=1; shift ;;
        --quiet|-q)   QUIET=1; shift ;;
        --json)       JSON_OUTPUT=1; shift ;;
        --timeout)    TIMEOUT="$2"; shift 2 ;;
        --help|-h)    usage; exit 0 ;;
        --*)          log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)            TARGET_ROBOT="$1"; shift ;;
    esac
done

INDICES_TO_CHECK=()
if [[ -z "${TARGET_ROBOT}" ]]; then
    for i in "${!ROBOT_NAMES[@]}"; do INDICES_TO_CHECK+=("$i"); done
else
    idx="$(robot_index_by_name "${TARGET_ROBOT}")" || exit 1
    INDICES_TO_CHECK+=("${idx}")
fi

# ─── Helper : check un robot, retourne 0 si UP, 1 sinon, et exporte des vars ─
# Sortie via stdout : status_label  pid  port  health_ok(0/1)
check_one() {
    local i="$1"
    local name="$(robot_name "$i")"
    local port="$(robot_port "$i")"
    local pid_file="$(robot_pid_path "$i")"
    local pid
    local pid_status="-"
    local health_status=0   # 1 = OK

    if pid="$(read_pid_alive "${pid_file}")"; then
        pid_status="${pid}"
    fi

    if http_health_ok "${port}" "${TIMEOUT}"; then
        health_status=1
    fi

    # CSV : name pid port health
    echo "${name}|${pid_status}|${port}|${health_status}"
}

# ─── Rendu d'un cycle de check ─────────────────────────────────────────────
render_text() {
    local results=("$@")
    if (( QUIET == 1 )); then return; fi

    printf "${EFE_C_BOLD}%-13s %-7s %-7s %s${EFE_C_RESET}\n" "ROBOT" "PORT" "PID" "HEALTH"
    local r name pid port health
    for r in "${results[@]}"; do
        IFS='|' read -r name pid port health <<<"${r}"
        if (( health == 1 )); then
            local label="${EFE_C_GREEN}● UP${EFE_C_RESET}"
        elif [[ "${pid}" == "-" ]]; then
            local label="${EFE_C_DIM}○ OFF${EFE_C_RESET}"
        else
            local label="${EFE_C_RED}✗ UNHEALTHY${EFE_C_RESET}"
        fi
        printf "%-13s %-7s %-7s %b\n" "${name}" "${port}" "${pid}" "${label}"
    done
}

render_json() {
    local results=("$@")
    local n="${#results[@]}"
    local i=0
    printf '{"robots":['
    local r name pid port health
    for r in "${results[@]}"; do
        IFS='|' read -r name pid port health <<<"${r}"
        local healthy_str="false"
        (( health == 1 )) && healthy_str="true"
        local pid_str="null"
        [[ "${pid}" != "-" ]] && pid_str="${pid}"
        printf '{"name":"%s","port":%s,"pid":%s,"healthy":%s}' \
            "${name}" "${port}" "${pid_str}" "${healthy_str}"
        (( i < n - 1 )) && printf ','
        i=$(( i + 1 ))
    done
    printf ']}'
    echo ""
}

# ─── Boucle principale (1 itération ou --watch) ─────────────────────────────
run_one_pass() {
    local results=()
    for i in "${INDICES_TO_CHECK[@]}"; do
        results+=("$(check_one "$i")")
    done

    if (( JSON_OUTPUT == 1 )); then
        render_json "${results[@]}"
    else
        render_text "${results[@]}"
    fi

    # Compte les unhealthy.
    local nb_unhealthy=0
    local r
    for r in "${results[@]}"; do
        local health="${r##*|}"
        (( health == 0 )) && nb_unhealthy=$(( nb_unhealthy + 1 ))
    done

    # Cap exit code à 4 (max robots).
    (( nb_unhealthy > 4 )) && nb_unhealthy=4
    return "${nb_unhealthy}"
}

if (( WATCH == 1 )); then
    # Mode watch : clear + redraw toutes les 2s.
    trap 'echo ""; exit 0' INT TERM
    while true; do
        clear 2>/dev/null || printf '\033[H\033[2J'
        echo "${EFE_C_DIM}Watch mode — $(date '+%H:%M:%S') (Ctrl+C pour quitter)${EFE_C_RESET}"
        echo ""
        run_one_pass || true
        sleep 2
    done
else
    run_one_pass
    exit $?
fi
