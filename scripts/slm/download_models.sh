#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/download_models.sh — Télécharge les 4 modèles GGUF des nano-robots
# Phase 1 vers <repo>/models/<robot>.gguf.
#
# Usage :
#   ./scripts/slm/download_models.sh                  # télécharge les 4 manquants
#   ./scripts/slm/download_models.sh analyzer         # télécharge 1 seul
#   ./scripts/slm/download_models.sh --force          # re-télécharge tout
#   ./scripts/slm/download_models.sh --check          # vérifie la présence + taille
#   ./scripts/slm/download_models.sh -h               # aide
#
# Coût : ~4.1 GB cumulés (290+444+912+2500 MB).
# Idempotent : skip les modèles déjà présents avec taille ≥ 95% de la cible.

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

# ─── Parsing arguments ──────────────────────────────────────────────────────
FORCE=0
CHECK_ONLY=0
TARGET_ROBOT=""

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] [ROBOT_NAME]

Télécharge les modèles GGUF des nano-robots Phase 1.

Options :
  --force, -f       Re-télécharge même si le fichier existe
  --check, -c       Vérifie seulement la présence et la taille (pas de download)
  --help, -h        Affiche cette aide

ROBOT_NAME (optionnel) : un parmi ${ROBOT_NAMES[*]}.
Si absent, traite les 4 robots.

Exemples :
  $0                       # télécharge tous les modèles manquants
  $0 analyzer              # télécharge seulement analyzer.gguf
  $0 --force compressor    # re-télécharge compressor.gguf
  $0 --check               # rapport présence/taille des 4 modèles

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --force|-f)   FORCE=1; shift ;;
        --check|-c)   CHECK_ONLY=1; shift ;;
        --help|-h)    usage; exit 0 ;;
        --*)          log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)            TARGET_ROBOT="$1"; shift ;;
    esac
done

# ─── Sélection des robots à traiter ─────────────────────────────────────────
INDICES_TO_PROCESS=()
if [[ -z "${TARGET_ROBOT}" ]]; then
    for i in "${!ROBOT_NAMES[@]}"; do INDICES_TO_PROCESS+=("$i"); done
else
    idx="$(robot_index_by_name "${TARGET_ROBOT}")" || exit 1
    INDICES_TO_PROCESS+=("${idx}")
fi

# ─── Pre-flight ─────────────────────────────────────────────────────────────
require_cmd curl || exit 1
ensure_runtime_dirs

# ─── Helper : taille fichier en octets ──────────────────────────────────────
file_size_bytes() {
    local path="$1"
    if [[ ! -f "${path}" ]]; then echo 0; return; fi
    if [[ "${EFE_SLM_OS}" == "macos" ]]; then
        stat -f%z "${path}" 2>/dev/null || echo 0
    else
        stat -c%s "${path}" 2>/dev/null || echo 0
    fi
}

# ─── Mode --check ───────────────────────────────────────────────────────────
if (( CHECK_ONLY == 1 )); then
    log_step "Vérification présence modèles"
    print_config_summary
    echo ""
    all_ok=1
    for i in "${INDICES_TO_PROCESS[@]}"; do
        name="$(robot_name "$i")"
        path="$(robot_model_path "$i")"
        expected_mb="$(robot_size_mb "$i")"
        if [[ -f "${path}" ]]; then
            actual_mb="$(( $(file_size_bytes "${path}") / 1024 / 1024 ))"
            # Tolérance ±10% (quantizations exactes peuvent légèrement varier).
            min_mb="$(( expected_mb * 90 / 100 ))"
            max_mb="$(( expected_mb * 110 / 100 ))"
            if (( actual_mb >= min_mb && actual_mb <= max_mb )); then
                log_ok "${name}.gguf présent (${actual_mb} MB ≈ ${expected_mb} MB attendu)"
            else
                log_warn "${name}.gguf taille inattendue : ${actual_mb} MB (attendu ~${expected_mb} MB ±10%)"
                all_ok=0
            fi
        else
            log_err "${name}.gguf MANQUANT (attendu : ${path})"
            all_ok=0
        fi
    done
    echo ""
    if (( all_ok == 1 )); then
        log_ok "Tous les modèles requis sont présents."
        exit 0
    else
        log_err "Certains modèles manquent ou ont une taille suspecte. Run : $0"
        exit 1
    fi
fi

# ─── Mode download ──────────────────────────────────────────────────────────
log_step "Téléchargement des modèles GGUF nano-robots"
log_info "Destination : ${EFE_SLM_MODELS_DIR}"
log_info "Robots ciblés : ${#INDICES_TO_PROCESS[@]} sur ${#ROBOT_NAMES[@]}"
echo ""

# Estimation cumulée pour préview.
total_expected_mb=0
for i in "${INDICES_TO_PROCESS[@]}"; do
    total_expected_mb=$(( total_expected_mb + $(robot_size_mb "$i") ))
done
log_info "Téléchargement estimé : ~${total_expected_mb} MB cumulés"
echo ""

failures=()
skipped=()
downloaded=()

for i in "${INDICES_TO_PROCESS[@]}"; do
    name="$(robot_name "$i")"
    label="$(robot_label "$i")"
    url="$(robot_hf_url "$i")"
    path="$(robot_model_path "$i")"
    expected_mb="$(robot_size_mb "$i")"

    echo "${EFE_C_BOLD}━━━ ${name} ━━━${EFE_C_RESET}"
    echo "  Modèle  : ${label}"
    echo "  URL     : ${url}"
    echo "  Cible   : ${path}"
    echo "  Taille  : ~${expected_mb} MB"

    # Skip si déjà présent et pas --force.
    if [[ -f "${path}" ]] && (( FORCE == 0 )); then
        actual_mb="$(( $(file_size_bytes "${path}") / 1024 / 1024 ))"
        min_mb="$(( expected_mb * 95 / 100 ))"
        if (( actual_mb >= min_mb )); then
            log_ok "Déjà téléchargé (${actual_mb} MB). Skip (use --force pour re-télécharger)."
            skipped+=("${name}")
            echo ""
            continue
        else
            log_warn "Fichier existant trop petit (${actual_mb} MB < ${min_mb} MB). Re-download."
        fi
    fi

    # Téléchargement avec curl. -L suit les redirects HF, --progress-bar = barre simple.
    tmp_path="${path}.partial"
    log_info "Téléchargement en cours…"
    if curl -L --fail --progress-bar --retry 3 --retry-delay 5 \
           --connect-timeout 30 \
           -o "${tmp_path}" "${url}"; then
        # Vérification rapide de la taille.
        actual_mb="$(( $(file_size_bytes "${tmp_path}") / 1024 / 1024 ))"
        min_mb="$(( expected_mb * 90 / 100 ))"
        if (( actual_mb < min_mb )); then
            log_err "Fichier téléchargé trop petit (${actual_mb} MB < ${min_mb} MB). Probablement une page d'erreur HF."
            rm -f "${tmp_path}"
            failures+=("${name}")
        else
            mv -f "${tmp_path}" "${path}"
            log_ok "Téléchargé : ${path} (${actual_mb} MB)"
            downloaded+=("${name}")
        fi
    else
        log_err "curl a échoué pour ${name}. URL : ${url}"
        log_warn "Si l'URL ne répond plus, vérifie HuggingFace (le slug du modèle peut avoir changé)."
        rm -f "${tmp_path}"
        failures+=("${name}")
    fi
    echo ""
done

# ─── Résumé ──────────────────────────────────────────────────────────────────
log_step "Résumé téléchargement"
log_ok "Téléchargés (${#downloaded[@]})  : ${downloaded[*]:-aucun}"
log_info "Déjà présents (${#skipped[@]}) : ${skipped[*]:-aucun}"
if (( ${#failures[@]} > 0 )); then
    log_err "Échecs (${#failures[@]})       : ${failures[*]}"
    echo ""
    log_warn "Voir crates/efe-nano-app-coordinator/MODELS_DOWNLOAD.md pour les URLs canoniques + alternatives."
    exit 1
fi

echo ""
log_ok "Tous les modèles sont prêts dans ${EFE_SLM_MODELS_DIR}/"
log_info "Étape suivante : ./scripts/slm/start.sh"
