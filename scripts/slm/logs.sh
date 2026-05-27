#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/logs.sh — Affiche / tail les logs des llama-server.
#
# Usage :
#   ./scripts/slm/logs.sh analyzer            # affiche le log complet
#   ./scripts/slm/logs.sh analyzer --tail     # tail -f (suit en temps réel)
#   ./scripts/slm/logs.sh analyzer --last 50  # 50 dernières lignes
#   ./scripts/slm/logs.sh --all               # affiche les 4 (concat avec préfixe)
#   ./scripts/slm/logs.sh --all --tail        # tail -f multiplexé sur les 4

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

ROBOT=""
ALL_ROBOTS=0
FOLLOW=0
LAST_N=0

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] ROBOT_NAME

Affiche les logs des llama-server nano-robots Phase 1.

Options :
  --all, -a         Affiche les logs de tous les robots (préfixés)
  --tail, -f        Suit en temps réel (équivalent tail -f)
  --last N          Affiche les N dernières lignes seulement
  --help, -h        Aide

ROBOT_NAME : un parmi ${ROBOT_NAMES[*]} (requis sauf si --all).

Exemples :
  $0 analyzer                    # cat du log analyzer
  $0 analyzer --tail             # follow analyzer en live
  $0 analyzer --last 50          # 50 dernières lignes de analyzer
  $0 --all                       # cat des 4 logs préfixés
  $0 --all --tail                # tail -f des 4 logs en live

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --all|-a)     ALL_ROBOTS=1; shift ;;
        --tail|-f)    FOLLOW=1; shift ;;
        --last)       LAST_N="$2"; shift 2 ;;
        --help|-h)    usage; exit 0 ;;
        --*)          log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)            ROBOT="$1"; shift ;;
    esac
done

if (( ALL_ROBOTS == 0 )) && [[ -z "${ROBOT}" ]]; then
    log_err "Manque le nom du robot (ou utilise --all)."
    usage
    exit 1
fi

# ─── Cas --all ──────────────────────────────────────────────────────────────
if (( ALL_ROBOTS == 1 )); then
    log_files=()
    for i in "${!ROBOT_NAMES[@]}"; do
        log_file="$(robot_log_path "$i")"
        if [[ -f "${log_file}" ]]; then
            log_files+=("${log_file}")
        fi
    done

    if (( ${#log_files[@]} == 0 )); then
        log_warn "Aucun log trouvé dans ${EFE_SLM_LOGS_DIR}/"
        exit 1
    fi

    if (( FOLLOW == 1 )); then
        # tail -f multi-fichier : tail natif affiche déjà ==> filename <== en switch.
        exec tail -F "${log_files[@]}"
    else
        # cat avec préfixe par fichier (style multi-tail).
        for f in "${log_files[@]}"; do
            name="$(basename "${f}" .log)"
            echo "${EFE_C_BOLD}${EFE_C_CYAN}━━━ ${name} ━━━${EFE_C_RESET}"
            if (( LAST_N > 0 )); then
                tail -n "${LAST_N}" "${f}"
            else
                cat "${f}"
            fi
            echo ""
        done
        exit 0
    fi
fi

# ─── Cas robot unique ───────────────────────────────────────────────────────
idx="$(robot_index_by_name "${ROBOT}")" || exit 1
log_file="$(robot_log_path "${idx}")"

if [[ ! -f "${log_file}" ]]; then
    log_err "Log absent : ${log_file}"
    log_warn "Le robot n'a peut-être jamais été démarré. Lance : ./scripts/slm/start.sh ${ROBOT}"
    exit 1
fi

if (( FOLLOW == 1 )); then
    exec tail -F "${log_file}"
elif (( LAST_N > 0 )); then
    exec tail -n "${LAST_N}" "${log_file}"
else
    exec cat "${log_file}"
fi
