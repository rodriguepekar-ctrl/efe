#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/e2e_smoke.sh — Méta-script de validation Sprint S1.
#
# Cible le gap G2 : un seul appel qui prouve que S1 est livré en bout-en-bout.
# Le critère d'acceptation S1 (cf. architecture R2 section 3 S1) est :
#
#     $ ./scripts/slm/e2e_smoke.sh && echo "✅ Sprint S1 validé"
#     ✅ Sprint S1 validé
#
# Étapes exécutées (dans l'ordre, abort à la 1ʳᵉ erreur) :
#   1. Pre-flight     : llama-server installé + version + assets + RAM
#   2. Models         : --check (téléchargement implicite si --download)
#   3. Start          : démarrage des 4 daemons + health-wait
#   4. Health         : ping /health × 4 (un check post-start dur)
#   5. Inference      : POST /completion × 4 (vraie inférence, pas juste /health)
#   6. Bench          : 3 requêtes × 4 robots, latence raisonnable
#   7. Env exports    : génère .env.slm
#   8. (optionnel) Stop : --keep-running pour garder les daemons up
#
# Usage :
#   ./scripts/slm/e2e_smoke.sh                # full check, stop à la fin
#   ./scripts/slm/e2e_smoke.sh --keep-running # laisse les daemons UP en fin
#   ./scripts/slm/e2e_smoke.sh --download     # télécharge les modèles avant
#   ./scripts/slm/e2e_smoke.sh --quick        # skip le bench
#   ./scripts/slm/e2e_smoke.sh --verbose      # affiche tout, pas de filtrage

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

KEEP_RUNNING=0
DOWNLOAD=0
QUICK=0
VERBOSE=0
BENCH_REQUESTS=3

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Méta-validation Sprint S1 : vérifie en une commande que bootstrap llama-server
fonctionne end-to-end.

Options :
  --keep-running, -k     Laisse les daemons démarrés en fin de smoke
  --download, -d         Télécharge les modèles manquants avant de démarrer
  --quick, -q            Skip l'étape bench
  --verbose, -v          Affiche tous les détails (sans filtrage)
  --help, -h             Aide

Étapes :
  1. Pre-flight (binaire + version + assets + RAM)
  2. Models check (download si --download)
  3. Start 4 daemons
  4. Health × 4
  5. Inference × 4 (POST /completion réel)
  6. Bench (3 req × 4 robots, sauf --quick)
  7. Génère .env.slm
  8. Stop (sauf --keep-running)

Exit code : 0 si succès complet, non-zéro sinon (avec détail du gap).

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --keep-running|-k)   KEEP_RUNNING=1; shift ;;
        --download|-d)       DOWNLOAD=1; shift ;;
        --quick|-q)          QUICK=1; shift ;;
        --verbose|-v)        VERBOSE=1; shift ;;
        --help|-h)           usage; exit 0 ;;
        *)                   log_err "Option inconnue : $1"; usage; exit 1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Helper : exécute un sous-script et abort en cas d'échec.
run_step() {
    local label="$1"
    shift
    log_step "[${label}]"
    if (( VERBOSE == 1 )); then
        "$@"
        local rc=$?
    else
        # Mode condensé : redirige stdout, garde stderr pour erreurs.
        local tmp; tmp="$(mktemp)"
        "$@" >"${tmp}" 2>&1
        local rc=$?
        if (( rc != 0 )); then
            cat "${tmp}" >&2
        else
            # En mode silencieux, on affiche juste les lignes [OK]/[INFO].
            grep -E '\[(OK|INFO|WARN|ERR)\]' "${tmp}" | head -15 || true
        fi
        rm -f "${tmp}"
    fi
    if (( rc != 0 )); then
        log_err "Étape '${label}' a échoué (exit ${rc}). Smoke S1 STOPPED."
        exit "${rc}"
    fi
    return 0
}

# ─── Bannière ───────────────────────────────────────────────────────────────
echo ""
echo "${EFE_C_BOLD}${EFE_C_CYAN}╔════════════════════════════════════════════════════════╗${EFE_C_RESET}"
echo "${EFE_C_BOLD}${EFE_C_CYAN}║   EFE Phase 3 R2 — Sprint S1 End-to-End Smoke Test    ║${EFE_C_RESET}"
echo "${EFE_C_BOLD}${EFE_C_CYAN}╚════════════════════════════════════════════════════════╝${EFE_C_RESET}"
echo ""
log_info "Date          : $(date '+%Y-%m-%d %H:%M:%S')"
log_info "Repo          : ${EFE_REPO_ROOT}"
log_info "OS            : ${EFE_SLM_OS}"
log_info "Keep-running  : $([ "${KEEP_RUNNING}" -eq 1 ] && echo "YES" || echo "no (stop à la fin)")"
log_info "Download      : $([ "${DOWNLOAD}" -eq 1 ] && echo "YES" || echo "no (attend modèles présents)")"
log_info "Quick mode    : $([ "${QUICK}" -eq 1 ] && echo "YES (skip bench)" || echo "no (bench inclus)")"
echo ""

# ─── Étape 1 : Pre-flight ──────────────────────────────────────────────────
log_step "[1/8] Pre-flight environment"
if ! require_llama_server; then exit 1; fi
if ! check_llama_server_version 9297; then exit 1; fi
if ! check_assets_phase1; then exit 1; fi
log_ok "Pre-flight OK"

# ─── Étape 2 : Models ──────────────────────────────────────────────────────
log_step "[2/8] Vérification modèles GGUF"
if (( DOWNLOAD == 1 )); then
    run_step "Download modèles manquants" "${SCRIPT_DIR}/download_models.sh"
else
    if ! "${SCRIPT_DIR}/download_models.sh" --check >/dev/null 2>&1; then
        log_err "Modèles manquants. Relance avec --download ou exécute :"
        log_err "  ./scripts/slm/download_models.sh"
        exit 1
    fi
    log_ok "Modèles présents"
fi

# ─── Étape 3 : Start ───────────────────────────────────────────────────────
run_step "[3/8] Démarrage 4 daemons llama-server" "${SCRIPT_DIR}/start.sh"

# ─── Étape 4 : Health ──────────────────────────────────────────────────────
log_step "[4/8] Health-check HTTP /health (× 4)"
if ! "${SCRIPT_DIR}/health.sh"; then
    log_err "Health check FAILED — voir les logs."
    "${SCRIPT_DIR}/status.sh"
    exit 1
fi
log_ok "Health 4/4 OK"

# ─── Étape 5 : Inference réelle ────────────────────────────────────────────
run_step "[5/8] Inférence réelle (POST /completion × 4)" "${SCRIPT_DIR}/verify_inference.sh"

# ─── Étape 6 : Bench ───────────────────────────────────────────────────────
if (( QUICK == 0 )); then
    run_step "[6/8] Bench latence (${BENCH_REQUESTS} req × 4 robots)" \
        "${SCRIPT_DIR}/bench.sh" -n "${BENCH_REQUESTS}"
else
    log_info "[6/8] Skip bench (--quick)"
fi

# ─── Étape 7 : Génère .env.slm ─────────────────────────────────────────────
log_step "[7/8] Génération .env.slm (endpoints Coordinator)"
"${SCRIPT_DIR}/print_env.sh" > "${EFE_REPO_ROOT}/.env.slm"
log_ok "Écrit : ${EFE_REPO_ROOT}/.env.slm"
log_info "À sourcer dans le shell qui lance le Coordinator :  source .env.slm"

# ─── Étape 8 : Stop ────────────────────────────────────────────────────────
if (( KEEP_RUNNING == 0 )); then
    run_step "[8/8] Arrêt propre des daemons" "${SCRIPT_DIR}/stop.sh"
else
    log_info "[8/8] Daemons laissés UP (--keep-running)"
    "${SCRIPT_DIR}/status.sh"
fi

# ─── Verdict final ─────────────────────────────────────────────────────────
echo ""
echo "${EFE_C_BOLD}${EFE_C_GREEN}╔════════════════════════════════════════════════════════╗${EFE_C_RESET}"
echo "${EFE_C_BOLD}${EFE_C_GREEN}║         ✅ Sprint S1 validé end-to-end                 ║${EFE_C_RESET}"
echo "${EFE_C_BOLD}${EFE_C_GREEN}╚════════════════════════════════════════════════════════╝${EFE_C_RESET}"
echo ""
log_info "Prochaine étape : Sprint S2 (Mission A naked + Claude)"
echo "  ${EFE_C_DIM}voir docs/architecture/EFE_A1_PHASE3_R2_MISSIONS_REELLES_LIVE_ARCHITECTURE.md§3 S2${EFE_C_RESET}"
