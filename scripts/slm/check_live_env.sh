#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/check_live_env.sh — Vérifie la présence des clés API requises
# pour les tests live Phase 3 R1/R2.
#
# Affiche un préfixe redacté (10 premiers caractères) + la longueur — ne
# montre JAMAIS la clé complète. Exit 0 si tout est OK, non-zéro sinon.
#
# Utilisation :
#   ./scripts/slm/check_live_env.sh                    # check toutes les 3 clés
#   ./scripts/slm/check_live_env.sh --required anthropic   # check 1 clé spécifique
#
# Variables vérifiées :
#   ANTHROPIC_API_KEY  (prefix sk-ant-)
#   GOOGLE_AI_KEY      (prefix AIza)
#   OPENAI_API_KEY     (prefix sk- ou sk-proj-)

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

REQUIRED="all"

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Vérifie la présence + préfixe des clés API requises pour les tests live R1/R2.

Options :
  --required PROVIDER   Ne check qu'une clé : anthropic|gemini|openai|all (défaut: all)
  --help, -h            Aide

Exit 0 si toutes les clés requises sont valides.
Exit 1 sinon (avec liste des manquantes).

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --required)   REQUIRED="$2"; shift 2 ;;
        --help|-h)    usage; exit 0 ;;
        *)            log_err "Option inconnue : $1"; usage; exit 1 ;;
    esac
done

# ─── Vérification d'une clé : prefix + length + non-vide ────────────────────
# Retourne 0 si OK, 1 sinon. Print la ligne d'info ou d'erreur.
check_key() {
    local name="$1"
    local expected_prefix="$2"
    local value="${!name:-}"   # indirection : value de la variable nommée par $name

    if [[ -z "${value}" ]]; then
        log_err "${name} : MANQUANTE"
        return 1
    fi

    if [[ -n "${expected_prefix}" ]] && [[ "${value}" != "${expected_prefix}"* ]]; then
        log_warn "${name} : préfixe inattendu (attendu '${expected_prefix}…', got '${value:0:8}…')"
        return 1
    fi

    local len="${#value}"
    local prefix_redacted="${value:0:10}"
    log_ok "${name} : ${prefix_redacted}... (${len} chars)"
    return 0
}

log_step "Vérification des clés API live"

missing=0

case "${REQUIRED}" in
    anthropic|all)
        check_key ANTHROPIC_API_KEY "sk-ant-" || missing=$(( missing + 1 ))
        ;;
esac

case "${REQUIRED}" in
    gemini|all)
        # Pas de garde de préfixe : Google émet des clés AI Studio sous
        # plusieurs formats (AIza…, AQ… sur les nouvelles). On exige seulement
        # une valeur non vide ; l'API Google valide l'authentification.
        check_key GOOGLE_AI_KEY "" || missing=$(( missing + 1 ))
        ;;
esac

case "${REQUIRED}" in
    openai|all)
        check_key OPENAI_API_KEY "sk-" || missing=$(( missing + 1 ))
        ;;
esac

echo ""
if (( missing > 0 )); then
    log_err "${missing} clé(s) manquante(s) ou invalide(s)."
    echo ""
    log_info "Pour exporter les clés (single quotes, pas de # commentaire) :"
    echo "  export ANTHROPIC_API_KEY='sk-ant-...'"
    echo "  export GOOGLE_AI_KEY='AIza...'"
    echo "  export OPENAI_API_KEY='sk-proj-...'"
    exit 1
fi

log_ok "Toutes les clés requises sont présentes et bien formées."
log_info "Tu peux lancer les tests live :"
echo "  ${EFE_C_DIM}just live-providers${EFE_C_RESET}       # Suite R1a/R1b (18 tests)"
echo "  ${EFE_C_DIM}just live-mission-naked${EFE_C_RESET}    # S2 only (1 test, ~1¢)"
echo "  ${EFE_C_DIM}just live-missions${EFE_C_RESET}        # Suite R2 missions"
