#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/print_env.sh — Affiche les `export X=Y` à sourcer dans un shell
# pour que le Coordinator (Phase 3 R2) et les tests live trouvent les
# endpoints des 4 nano-robots Phase 1.
#
# Cible le gap G6 du Sprint S1 R2 : Mission A R2 a besoin de propager les
# 4 URLs au Coordinator. Ce script génère le bloc `export` à coller, à
# sourcer, ou à rediriger vers .env.slm.
#
# Usage :
#   ./scripts/slm/print_env.sh                # affiche sur stdout
#   eval "$(./scripts/slm/print_env.sh)"       # exporte dans le shell courant
#   ./scripts/slm/print_env.sh > .env.slm     # crée un fichier à sourcer
#   source .env.slm                            # active les vars
#
# Variables exportées :
#   ANALYZER_URL=http://127.0.0.1:8090
#   REFORMULATOR_URL=http://127.0.0.1:8091
#   COMPRESSOR_URL=http://127.0.0.1:8092
#   VERIFIER_URL=http://127.0.0.1:8093
#
# Note : on respecte la convention des tests Phase 1 `live_smoke.rs:39-41`
# qui lit `ANALYZER_URL` etc avec fallback hardcodé.

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Génère les exports d'env vars pour les endpoints nano-robots.

Options :
  --host ADDR       Override host (défaut: ${EFE_SLM_HOST:-127.0.0.1})
  --bare            Pas de commentaires, juste les exports
  --help, -h        Aide

Workflow recommandé :
  $0 > .env.slm
  source .env.slm
  cargo test -p efe-coordinator --features all-providers --test live_missions

Workflow one-shot :
  eval "\$($0)"
  cargo run --example mission_a_demo

EOF
}

BARE=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)       EFE_SLM_HOST="$2"; shift 2 ;;
        --bare)       BARE=1; shift ;;
        --help|-h)    usage; exit 0 ;;
        --*)          log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)            log_err "Argument inattendu : $1"; usage; exit 1 ;;
    esac
done

if (( BARE == 0 )); then
    print_coordinator_env_exports
else
    # Sans commentaires (utile pour eval direct).
    host="${EFE_SLM_HOST:-127.0.0.1}"
    for i in "${!ROBOT_NAMES[@]}"; do
        upname="$(robot_name "$i" | tr '[:lower:]' '[:upper:]')"
        echo "export ${upname}_URL=\"http://${host}:$(robot_port "$i")\""
    done
fi
