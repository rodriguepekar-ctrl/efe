#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/verify_inference.sh — Vérifie que chaque robot répond à une vraie
# requête d'inférence (au-delà de /health qui répond OK dès que le modèle est en RAM).
#
# Cible le gap G1 du Sprint S1 R2 : /health ne prouve PAS que l'inférence + le
# grammar GBNF marchent — seul un POST réel /completion le prouve.
#
# Usage :
#   ./scripts/slm/verify_inference.sh                # vérifie les 4
#   ./scripts/slm/verify_inference.sh analyzer       # vérifie 1 seul
#   ./scripts/slm/verify_inference.sh --quiet        # exit code only
#   ./scripts/slm/verify_inference.sh --timeout 30   # timeout par requête
#
# Exit codes : 0 si tous OK ; N = nombre de robots avec inférence cassée.

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

QUIET=0
TARGET_ROBOT=""
TIMEOUT="${EFE_SLM_VERIFY_TIMEOUT:-20}"

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] [ROBOT_NAME]

Vérifie l'inférence réelle (pas juste /health) sur chaque llama-server.

Options :
  --quiet, -q       Aucune sortie, exit code uniquement
  --timeout SEC     Timeout par requête (défaut: ${TIMEOUT}s)
  --help, -h        Aide

Exit code = nombre de robots avec inférence cassée (0 = tous OK).

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --quiet|-q)   QUIET=1; shift ;;
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

require_cmd curl || exit 1

# ─── Payloads de test minimaux par robot ────────────────────────────────────
# Stratégie : on n'a PAS besoin d'envoyer la grammar / prompt complet ici —
# on veut juste prouver que :
#   1. l'endpoint POST /completion accepte un payload JSON
#   2. le modèle génère AU MOINS 1 token (n_predict=1)
#   3. la latence est raisonnable (timeout configurable)
#
# Note : on N'utilise PAS la grammar GBNF des nano-robots ici, car ça
# imposerait de respecter leur format JSON exact (analyzer attend
# {entities, intent, language, ...}), ce qui demande un modèle correctement
# tuned. Le but de verify_inference est juste de prouver que llama-server
# fonctionne, pas que les robots produisent le format Phase 1 exact.

# Payload générique : prompt court, 1 token de réponse max.
make_test_payload() {
    local prompt="Hello"
    # JSON compact, n_predict=1 = 1 token, temperature 0 = déterministe,
    # stop empty = pas de stop sequence (on accepte la fin sur n_predict).
    printf '{"prompt":"%s","n_predict":1,"temperature":0.0,"cache_prompt":false}' "${prompt}"
}

# Envoie 1 requête /completion et vérifie la réponse. Retourne 0 si OK.
test_robot_inference() {
    local i="$1"
    local name="$(robot_name "$i")"
    local port="$(robot_port "$i")"
    local payload="$(make_test_payload)"
    local started_at; started_at="$(date +%s)"

    # Vérifie d'abord que /health répond (skip si pas démarré).
    if ! http_health_ok "${port}" 2; then
        if (( QUIET == 0 )); then
            printf "  %-13s %-6s %b\n" "${name}" "${port}" "${EFE_C_DIM}○ NOT RUNNING (skipped)${EFE_C_RESET}"
        fi
        return 1
    fi

    # Envoie la requête. -w 'HTTPSTATUS:%{http_code}\nTIME:%{time_total}\n' sépare le code.
    local response http_status time_total body
    response="$(curl -sS -m "${TIMEOUT}" \
        -H 'content-type: application/json' \
        -X POST "http://127.0.0.1:${port}/completion" \
        -d "${payload}" \
        -w '\nHTTPSTATUS:%{http_code}\nTIME:%{time_total}\n' 2>&1 || true)"

    http_status="$(echo "${response}" | grep -oE 'HTTPSTATUS:[0-9]+' | head -1 | cut -d: -f2)"
    time_total="$(echo "${response}" | grep -oE 'TIME:[0-9.]+' | head -1 | cut -d: -f2)"
    body="$(echo "${response}" | sed '/^HTTPSTATUS:/,$d')"

    # Analyse.
    local ok=0
    local reason=""
    if [[ "${http_status}" == "200" ]]; then
        # llama-server retourne {"content": "...", ...}. Vérifie présence de "content".
        if echo "${body}" | grep -q '"content"'; then
            ok=1
        else
            reason="200 OK mais pas de champ 'content' dans la réponse"
        fi
    elif [[ -z "${http_status}" ]]; then
        reason="pas de réponse HTTP (timeout après ${TIMEOUT}s ?)"
    else
        reason="HTTP ${http_status}"
    fi

    # Latence en ms (formatage).
    local latency_ms="-"
    if [[ -n "${time_total}" ]]; then
        latency_ms="$(printf "%.0f" "$(echo "${time_total} * 1000" | bc 2>/dev/null || echo 0)")"
    fi

    if (( QUIET == 0 )); then
        if (( ok == 1 )); then
            printf "  %-13s %-6s %b  ${EFE_C_DIM}%s ms${EFE_C_RESET}\n" \
                "${name}" "${port}" "${EFE_C_GREEN}✓ inference OK${EFE_C_RESET}" "${latency_ms}"
        else
            printf "  %-13s %-6s %b  ${EFE_C_DIM}(%s)${EFE_C_RESET}\n" \
                "${name}" "${port}" "${EFE_C_RED}✗ INFERENCE FAILED${EFE_C_RESET}" "${reason}"
        fi
    fi

    if (( ok == 1 )); then return 0; else return 1; fi
}

# ─── Main ───────────────────────────────────────────────────────────────────
if (( QUIET == 0 )); then
    log_step "Vérification inférence des nano-robots"
    printf "${EFE_C_BOLD}  %-13s %-6s %s${EFE_C_RESET}\n" "ROBOT" "PORT" "STATUS"
fi

failed=0
for i in "${INDICES_TO_CHECK[@]}"; do
    if ! test_robot_inference "$i"; then
        failed=$(( failed + 1 ))
    fi
done

if (( QUIET == 0 )); then
    echo ""
    if (( failed == 0 )); then
        log_ok "Inférence vérifiée : ${#INDICES_TO_CHECK[@]}/${#INDICES_TO_CHECK[@]}"
    else
        log_err "Échecs inférence : ${failed}/${#INDICES_TO_CHECK[@]}"
        log_warn "Inspecte les logs :"
        for i in "${INDICES_TO_CHECK[@]}"; do
            echo "    ${EFE_C_DIM}./scripts/slm/logs.sh $(robot_name "$i") --last 50${EFE_C_RESET}"
        done
    fi
fi

# Cap à 255 pour exit code.
(( failed > 255 )) && failed=255
exit "${failed}"
