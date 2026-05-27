#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/bench.sh — Mesure la latence de chaque nano-robot llama-server
# via N requêtes /completion (n_predict=16). Affiche min/p50/p99/max et
# détecte si Metal/CUDA fonctionne réellement.
#
# Cible le gap G8 du Sprint S1 R2 : sur Mac M-series, si la 1ʳᵉ requête prend
# >10s c'est que Metal ne tourne pas (fallback CPU silencieux). Ce bench le
# détecte sans ambiguïté.
#
# Usage :
#   ./scripts/slm/bench.sh                  # 5 requêtes × 4 robots
#   ./scripts/slm/bench.sh -n 10            # 10 requêtes par robot
#   ./scripts/slm/bench.sh analyzer         # 1 seul robot
#   ./scripts/slm/bench.sh --tokens 64      # 64 tokens par requête
#   ./scripts/slm/bench.sh --json           # output JSON pour scripts

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

NB_REQUESTS=5
N_PREDICT=16
JSON_OUTPUT=0
TARGET_ROBOT=""

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] [ROBOT_NAME]

Bench latence des nano-robots llama-server (POST /completion).

Options :
  -n, --requests N   Nombre de requêtes par robot (défaut: ${NB_REQUESTS})
  -t, --tokens N     n_predict par requête (défaut: ${N_PREDICT})
  --json             Sortie JSON (pour scripts)
  -h, --help         Aide

ROBOT_NAME : un parmi ${ROBOT_NAMES[*]} (défaut: tous).

Cibles de performance attendues :
  - GPU (Metal/CUDA) : p50 < 500 ms pour 16 tokens
  - CPU only         : p50 > 2000 ms pour 16 tokens (= problème de config GPU)

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n|--requests)  NB_REQUESTS="$2"; shift 2 ;;
        -t|--tokens)    N_PREDICT="$2"; shift 2 ;;
        --json)         JSON_OUTPUT=1; shift ;;
        -h|--help)      usage; exit 0 ;;
        --*)            log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)              TARGET_ROBOT="$1"; shift ;;
    esac
done

INDICES_TO_BENCH=()
if [[ -z "${TARGET_ROBOT}" ]]; then
    for i in "${!ROBOT_NAMES[@]}"; do INDICES_TO_BENCH+=("$i"); done
else
    idx="$(robot_index_by_name "${TARGET_ROBOT}")" || exit 1
    INDICES_TO_BENCH+=("${idx}")
fi

require_cmd curl || exit 1
require_cmd awk || exit 1

# ─── Payload bench : prompt court, n_predict configurable ───────────────────
make_bench_payload() {
    local n_tokens="$1"
    printf '{"prompt":"Continue this sentence: The quick brown fox","n_predict":%s,"temperature":0.0,"cache_prompt":true}' "${n_tokens}"
}

# ─── Mesure 1 requête, retourne latence en ms (entier) sur stdout ─────────
measure_one() {
    local port="$1"
    local payload; payload="$(make_bench_payload "${N_PREDICT}")"
    local start_ns end_ns elapsed_ms
    local http_code

    start_ns="$(ns_now)"

    http_code="$(curl -sS -o /dev/null -w '%{http_code}' \
        -m 60 \
        -H 'content-type: application/json' \
        -X POST "http://127.0.0.1:${port}/completion" \
        -d "${payload}" 2>/dev/null || echo "000")"

    end_ns="$(ns_now)"

    if [[ "${http_code}" != "200" ]]; then
        echo "FAIL:${http_code}"
        return
    fi

    # 1_000_000 NON portable (bash 3.2 macOS rejette les literals avec _) → 1000000.
    elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
    echo "${elapsed_ms}"
}

# ─── Stats sur N mesures ────────────────────────────────────────────────────
# Reçoit les latences en stdin (1 par ligne), retourne "min p50 p99 max moy".
compute_stats() {
    awk '
    {
        if ($0 ~ /^FAIL/) { fail++; next }
        a[++n] = $1 + 0
    }
    END {
        if (n == 0) { printf "- - - - -"; exit }
        # Tri.
        for (i = 1; i <= n; i++) {
            for (j = i+1; j <= n; j++) {
                if (a[i] > a[j]) { t = a[i]; a[i] = a[j]; a[j] = t }
            }
        }
        min = a[1]
        max = a[n]
        p50_idx = int(n * 0.5); if (p50_idx < 1) p50_idx = 1
        p99_idx = int(n * 0.99); if (p99_idx < 1) p99_idx = 1
        p50 = a[p50_idx]
        p99 = a[p99_idx]
        sum = 0
        for (i = 1; i <= n; i++) sum += a[i]
        avg = int(sum / n)
        printf "%d %d %d %d %d", min, p50, p99, max, avg
    }
    '
}

# ─── Bench 1 robot ──────────────────────────────────────────────────────────
bench_one_robot() {
    local i="$1"
    local name="$(robot_name "$i")"
    local port="$(robot_port "$i")"

    # Skip si pas démarré.
    if ! http_health_ok "${port}" 2; then
        echo "${name}|${port}|SKIPPED|-|-|-|-|-|-"
        return
    fi

    local latencies=()
    local failures=0
    local r
    for (( r = 0; r < NB_REQUESTS; r++ )); do
        local res; res="$(measure_one "${port}")"
        if [[ "${res}" == FAIL:* ]]; then
            failures=$(( failures + 1 ))
            continue
        fi
        latencies+=("${res}")
    done

    # Calcule stats.
    local stats
    stats="$(printf "%s\n" "${latencies[@]}" | compute_stats)"
    local min p50 p99 max avg
    read -r min p50 p99 max avg <<<"${stats}"

    echo "${name}|${port}|OK|${min}|${p50}|${p99}|${max}|${avg}|${failures}"
}

# ─── Verdict GPU/CPU ────────────────────────────────────────────────────────
classify_perf() {
    local p50="$1"
    if [[ "${p50}" == "-" ]]; then echo "-"; return; fi
    if   (( p50 < 500 ));  then echo "${EFE_C_GREEN}GPU (Metal/CUDA)${EFE_C_RESET}"
    elif (( p50 < 2000 )); then echo "${EFE_C_YELLOW}OK (slow GPU ?)${EFE_C_RESET}"
    else                        echo "${EFE_C_RED}CPU only (vérifie --n-gpu-layers)${EFE_C_RESET}"
    fi
}

# ─── Main ───────────────────────────────────────────────────────────────────
if (( JSON_OUTPUT == 0 )); then
    log_step "Bench latence (${NB_REQUESTS} requêtes × ${N_PREDICT} tokens)"
    printf "${EFE_C_BOLD}%-13s %-6s %-9s %-7s %-7s %-7s %-7s %-7s %-3s %s${EFE_C_RESET}\n" \
        "ROBOT" "PORT" "STATUS" "MIN" "P50" "P99" "MAX" "AVG" "FAIL" "VERDICT"
fi

results=()
for i in "${INDICES_TO_BENCH[@]}"; do
    res="$(bench_one_robot "$i")"
    results+=("${res}")

    if (( JSON_OUTPUT == 0 )); then
        IFS='|' read -r name port status min p50 p99 max avg fails <<<"${res}"
        verdict="$(classify_perf "${p50}")"
        printf "%-13s %-6s %-9s %-7s %-7s %-7s %-7s %-7s %-3s %b\n" \
            "${name}" "${port}" "${status}" "${min}" "${p50}" "${p99}" "${max}" "${avg}" "${fails}" "${verdict}"
    fi
done

if (( JSON_OUTPUT == 1 )); then
    n="${#results[@]}"; i=0
    printf '{"requests_per_robot":%s,"tokens_per_request":%s,"robots":[' "${NB_REQUESTS}" "${N_PREDICT}"
    for res in "${results[@]}"; do
        IFS='|' read -r name port status min p50 p99 max avg fails <<<"${res}"
        # Normalize "-" → null en JSON.
        normalize() { [[ "$1" == "-" ]] && echo "null" || echo "$1"; }
        printf '{"name":"%s","port":%s,"status":"%s","min_ms":%s,"p50_ms":%s,"p99_ms":%s,"max_ms":%s,"avg_ms":%s,"failures":%s}' \
            "${name}" "${port}" "${status}" \
            "$(normalize "${min}")" "$(normalize "${p50}")" "$(normalize "${p99}")" \
            "$(normalize "${max}")" "$(normalize "${avg}")" "${fails}"
        (( i < n - 1 )) && printf ','
        i=$(( i + 1 ))
    done
    printf ']}'
    echo ""
fi
