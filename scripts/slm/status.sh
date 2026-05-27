#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/status.sh — Vue rapide de l'état des 4 llama-server :
# PID, port, alive, log (taille + dernière mtime).
#
# Pour vérifier la santé HTTP, utiliser plutôt health.sh.
# status.sh est purement local (pidfile + filesystem), ne tape pas le réseau.
#
# Usage :
#   ./scripts/slm/status.sh                 # vue d'ensemble
#   ./scripts/slm/status.sh --verbose       # ajoute taille du log + dernière ligne
#   ./scripts/slm/status.sh --json          # output JSON

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

VERBOSE=0
JSON_OUTPUT=0

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

État local (pidfiles + logs) des 4 llama-server nano-robots.

Options :
  --verbose, -v     Ajoute taille du log + dernière ligne du log
  --json            Sortie JSON
  --help, -h        Aide

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --verbose|-v) VERBOSE=1; shift ;;
        --json)       JSON_OUTPUT=1; shift ;;
        --help|-h)    usage; exit 0 ;;
        --*)          log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)            log_err "Argument inattendu : $1"; usage; exit 1 ;;
    esac
done

# ─── Helper : taille humaine d'un fichier ───────────────────────────────────
# IMPORTANT : on n'utilise PAS `printf %.1f` car le bash printf builtin sur
# macOS dépend de LC_NUMERIC pour parser les floats — sur certaines locales
# il rejette "6.7" avec "invalid number". On reste donc en arithmétique
# entière pure : on calcule mb = bytes/MB et rem = (bytes*10/MB)%10 puis on
# affiche "%d.%dM". Portable bash 3+ / zsh.
file_size_human() {
    local path="$1"
    if [[ ! -f "${path}" ]]; then echo "-"; return; fi
    local bytes
    if [[ "${EFE_SLM_OS}" == "macos" ]]; then
        bytes="$(stat -f%z "${path}" 2>/dev/null || echo 0)"
    else
        bytes="$(stat -c%s "${path}" 2>/dev/null || echo 0)"
    fi

    local whole rem
    if (( bytes >= 1073741824 )); then
        whole=$(( bytes / 1073741824 ))
        rem=$(( (bytes * 10 / 1073741824) % 10 ))
        printf "%d.%dG" "${whole}" "${rem}"
    elif (( bytes >= 1048576 )); then
        whole=$(( bytes / 1048576 ))
        rem=$(( (bytes * 10 / 1048576) % 10 ))
        printf "%d.%dM" "${whole}" "${rem}"
    elif (( bytes >= 1024 )); then
        whole=$(( bytes / 1024 ))
        rem=$(( (bytes * 10 / 1024) % 10 ))
        printf "%d.%dK" "${whole}" "${rem}"
    else
        printf "%dB" "${bytes}"
    fi
}

# Mtime humain (ex: "2m ago").
file_mtime_age() {
    local path="$1"
    if [[ ! -f "${path}" ]]; then echo "-"; return; fi
    local mtime
    if [[ "${EFE_SLM_OS}" == "macos" ]]; then
        mtime="$(stat -f%m "${path}" 2>/dev/null || echo 0)"
    else
        mtime="$(stat -c%Y "${path}" 2>/dev/null || echo 0)"
    fi
    local now; now="$(date +%s)"
    local age=$(( now - mtime ))
    if   (( age < 60 ));    then echo "${age}s ago"
    elif (( age < 3600 ));  then echo "$(( age / 60 ))m ago"
    elif (( age < 86400 )); then echo "$(( age / 3600 ))h ago"
    else                         echo "$(( age / 86400 ))d ago"
    fi
}

# ─── Output texte ───────────────────────────────────────────────────────────
if (( JSON_OUTPUT == 0 )); then
    printf "${EFE_C_BOLD}%-13s %-7s %-9s %-8s %-9s %s${EFE_C_RESET}\n" \
        "ROBOT" "PORT" "PID" "STATE" "LOG SIZE" "LAST WRITE"
    for i in "${!ROBOT_NAMES[@]}"; do
        name="$(robot_name "$i")"
        port="$(robot_port "$i")"
        pid_file="$(robot_pid_path "$i")"
        log_file="$(robot_log_path "$i")"

        pid_display="-"
        state="${EFE_C_DIM}OFF${EFE_C_RESET}"

        if pid="$(read_pid_alive "${pid_file}")"; then
            pid_display="${pid}"
            state="${EFE_C_GREEN}ALIVE${EFE_C_RESET}"
        elif [[ -f "${pid_file}" ]]; then
            state="${EFE_C_RED}STALE${EFE_C_RESET}"
            pid_display="$(cat "${pid_file}" 2>/dev/null)"
        fi

        log_size="$(file_size_human "${log_file}")"
        log_age="$(file_mtime_age "${log_file}")"

        printf "%-13s %-7s %-9s %-19b %-9s %s\n" \
            "${name}" "${port}" "${pid_display}" "${state}" "${log_size}" "${log_age}"

        if (( VERBOSE == 1 )) && [[ -f "${log_file}" ]]; then
            local last_line
            last_line="$(tail -1 "${log_file}" 2>/dev/null | tr -d '\r' | cut -c1-120)"
            if [[ -n "${last_line}" ]]; then
                echo "  ${EFE_C_DIM}└─ ${last_line}${EFE_C_RESET}"
            fi
        fi
    done
    exit 0
fi

# ─── Output JSON ────────────────────────────────────────────────────────────
n="${#ROBOT_NAMES[@]}"
i=0
printf '{"robots":['
for idx in "${!ROBOT_NAMES[@]}"; do
    name="$(robot_name "$idx")"
    port="$(robot_port "$idx")"
    pid_file="$(robot_pid_path "$idx")"
    log_file="$(robot_log_path "$idx")"

    pid_json="null"
    state="off"
    if pid="$(read_pid_alive "${pid_file}")"; then
        pid_json="${pid}"
        state="alive"
    elif [[ -f "${pid_file}" ]]; then
        state="stale"
        pid_json="$(cat "${pid_file}" 2>/dev/null || echo null)"
    fi

    log_exists="false"
    log_size_bytes=0
    if [[ -f "${log_file}" ]]; then
        log_exists="true"
        if [[ "${EFE_SLM_OS}" == "macos" ]]; then
            log_size_bytes="$(stat -f%z "${log_file}" 2>/dev/null || echo 0)"
        else
            log_size_bytes="$(stat -c%s "${log_file}" 2>/dev/null || echo 0)"
        fi
    fi

    printf '{"name":"%s","port":%s,"pid":%s,"state":"%s","log_exists":%s,"log_size_bytes":%s}' \
        "${name}" "${port}" "${pid_json}" "${state}" "${log_exists}" "${log_size_bytes}"
    (( i < n - 1 )) && printf ','
    i=$(( i + 1 ))
done
printf ']}'
echo ""
