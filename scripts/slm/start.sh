#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/slm/start.sh — Démarre les 4 instances llama-server (Phase 1 nano-robots)
# sur les ports 8090-8093, en arrière-plan, avec attente health-check.
#
# Convention :
#   - Les modèles GGUF doivent être présents dans <repo>/models/ (cf. download_models.sh)
#   - Les PIDs sont écrits dans <repo>/.llama_pids/<robot>.pid
#   - Les logs vont dans <repo>/logs/llama-server/<robot>.log
#   - Health-check polling : 30s timeout (les gros modèles prennent du temps à charger)
#
# Usage :
#   ./scripts/slm/start.sh                  # démarre les 4
#   ./scripts/slm/start.sh analyzer         # démarre 1 seul
#   ./scripts/slm/start.sh --dry-run        # affiche ce qui serait fait
#   ./scripts/slm/start.sh -h               # aide

source "$(dirname "${BASH_SOURCE[0]}")/lib_common.sh"

# ─── Parsing arguments ──────────────────────────────────────────────────────
DRY_RUN=0
TARGET_ROBOT=""
HEALTH_TIMEOUT="${EFE_SLM_HEALTH_TIMEOUT:-30}"   # secondes
NGL_OVERRIDE="${EFE_SLM_NGL:-999}"                # n-gpu-layers (Metal/CUDA)
HOST_BIND="${EFE_SLM_HOST:-127.0.0.1}"

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] [ROBOT_NAME]

Démarre les llama-server des 4 nano-robots Phase 1.

Options :
  --dry-run         N'exécute rien, affiche les commandes
  --timeout SEC     Timeout health-check par robot (défaut: ${HEALTH_TIMEOUT}s)
  --ngl N           Couches GPU (défaut: ${NGL_OVERRIDE} = toutes ; 0 = CPU only)
  --host ADDR       Adresse de bind (défaut: ${HOST_BIND}, mettre 0.0.0.0 si remote)
  --help, -h        Aide

Variables d'env reconnues :
  EFE_SLM_HEALTH_TIMEOUT   Timeout health-check (s)
  EFE_SLM_NGL              Couches GPU à offload
  EFE_SLM_HOST             Adresse de bind
  NO_COLOR=1               Désactive les couleurs

Exemples :
  $0                       # démarre les 4
  $0 verifier              # démarre verifier seul
  $0 --dry-run             # preview sans lancer
  EFE_SLM_NGL=0 $0         # CPU only (slow but works without GPU)

EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)    DRY_RUN=1; shift ;;
        --timeout)    HEALTH_TIMEOUT="$2"; shift 2 ;;
        --ngl)        NGL_OVERRIDE="$2"; shift 2 ;;
        --host)       HOST_BIND="$2"; shift 2 ;;
        --help|-h)    usage; exit 0 ;;
        --*)          log_err "Option inconnue : $1"; usage; exit 1 ;;
        *)            TARGET_ROBOT="$1"; shift ;;
    esac
done

# ─── Sélection des robots à démarrer ────────────────────────────────────────
INDICES_TO_START=()
if [[ -z "${TARGET_ROBOT}" ]]; then
    for i in "${!ROBOT_NAMES[@]}"; do INDICES_TO_START+=("$i"); done
else
    idx="$(robot_index_by_name "${TARGET_ROBOT}")" || exit 1
    INDICES_TO_START+=("${idx}")
fi

# ─── Signal trap : cleanup en cas d'interruption pendant le démarrage ──────
# Garde la liste des PIDs démarrés dans cette session pour cleanup si Ctrl+C.
STARTED_PIDS=()
SIGNAL_CLEANUP_DONE=0

cleanup_on_signal() {
    local sig="$1"
    if (( SIGNAL_CLEANUP_DONE == 1 )); then return; fi
    SIGNAL_CLEANUP_DONE=1
    echo ""
    log_warn "Signal ${sig} reçu. Cleanup des process partiellement démarrés…"
    local pid
    for pid in "${STARTED_PIDS[@]}"; do
        if pid_alive "${pid}"; then
            log_info "Kill ${pid}"
            kill -TERM "${pid}" 2>/dev/null || true
        fi
    done
    # Cleanup pidfiles écrits pendant cette session.
    for i in "${!ROBOT_NAMES[@]}"; do
        local pf; pf="$(robot_pid_path "$i")"
        if [[ -f "${pf}" ]]; then
            local pid_in_file; pid_in_file="$(cat "${pf}" 2>/dev/null || true)"
            for spid in "${STARTED_PIDS[@]}"; do
                if [[ "${spid}" == "${pid_in_file}" ]]; then
                    rm -f "${pf}"
                    break
                fi
            done
        fi
    done
    log_warn "Cleanup terminé. Si certains process ont survécu : ./scripts/slm/stop.sh --force"
    exit 130   # convention : 130 = INT
}

trap 'cleanup_on_signal INT' INT
trap 'cleanup_on_signal TERM' TERM

# ─── Pre-flight ─────────────────────────────────────────────────────────────
log_step "Pre-flight check"

# Étape 1 : binaire llama-server présent ET version compatible.
require_llama_server || exit 1
check_llama_server_version 9297 || exit 1

# Étape 2 : assets Phase 1 (grammars + prompts) présents.
log_info "Validation des assets Phase 1…"
check_assets_phase1 || exit 1
log_ok "Assets Phase 1 OK (4 grammars + 4 prompts dans ${EFE_SLM_ASSETS_DIR})"

ensure_runtime_dirs

# Vérifie tous les modèles AVANT de démarrer quoi que ce soit.
missing_models=()
for i in "${INDICES_TO_START[@]}"; do
    path="$(robot_model_path "$i")"
    if [[ ! -f "${path}" ]]; then
        missing_models+=("$(robot_name "$i")")
    fi
done
if (( ${#missing_models[@]} > 0 )); then
    log_err "Modèle(s) manquant(s) : ${missing_models[*]}"
    log_warn "Lance d'abord : ./scripts/slm/download_models.sh"
    exit 1
fi

# Vérifie qu'aucun robot ciblé n'est déjà UP, et qu'aucun port n'est volé.
already_up=()
port_conflicts=()
for i in "${INDICES_TO_START[@]}"; do
    name="$(robot_name "$i")"
    port="$(robot_port "$i")"
    pid_file="$(robot_pid_path "$i")"

    if existing_pid="$(read_pid_alive "${pid_file}")"; then
        already_up+=("${name} (pid=${existing_pid})")
        continue
    fi

    if ! port_is_free "${port}"; then
        owner="$(who_owns_port "${port}" || echo "?")"
        port_conflicts+=("${name}:${port} (pid=${owner})")
    fi
done

if (( ${#already_up[@]} > 0 )); then
    log_warn "Déjà démarrés (skip) : ${already_up[*]}"
    log_info "Pour redémarrer, lance d'abord : ./scripts/slm/stop.sh"
fi

if (( ${#port_conflicts[@]} > 0 )); then
    log_err "Conflit de port (un autre process écoute) : ${port_conflicts[*]}"
    log_warn "Libère ces ports ou modifie ROBOT_PORTS dans lib_common.sh."
    exit 1
fi

# Filtre les robots déjà UP de la liste à démarrer (idempotence).
to_start=()
for i in "${INDICES_TO_START[@]}"; do
    name="$(robot_name "$i")"
    pid_file="$(robot_pid_path "$i")"
    if ! read_pid_alive "${pid_file}" >/dev/null; then
        to_start+=("$i")
    fi
done

if (( ${#to_start[@]} == 0 )); then
    log_ok "Rien à démarrer (tout est déjà UP)."
    exit 0
fi

# Étape 3 : check RAM disponible pour la liste finale de robots à démarrer.
# Non bloquant : on warn et on continue (l'utilisateur sait ce qu'il fait).
check_ram_sufficient "${to_start[@]}" || true

log_ok "Pre-flight passé. ${#to_start[@]} robot(s) à démarrer."

# ─── Construction de la commande llama-server ───────────────────────────────
# Optimisations choisies :
#   --host             bind 127.0.0.1 par défaut (sécurité local-only)
#   --port             par robot
#   --model            GGUF
#   --ctx-size         taille KV cache (par robot)
#   --n-gpu-layers     offload GPU max (999 = toutes ; auto-fallback CPU)
#   --threads          calculé selon le hardware
#   --threads-batch    idem pour batch
#   --parallel         1 slot suffit pour Phase 3 R2 (1 requête à la fois)
#   --cont-batching    continuous batching (efficace même à parallel=1)
#   --metrics          expose /metrics Prometheus
#   --no-warmup        accélère le startup (warmup à la 1ère requête)
#   --alias            nom human-readable du modèle
#   --cache-reuse      reuse KV cache prefix (utile car prompt système commun)
#   --log-disable      pas de logs vers stdout (on les capture dans le fichier)
#
# Ne pas spécifier --grammar : les nano-robot clients envoient leur grammar
# par requête (cf. llama_server.rs:120-169).

build_llama_cmd() {
    local i="$1"
    local name="$(robot_name "$i")"
    local port="$(robot_port "$i")"
    local model="$(robot_model_path "$i")"
    local ctx="$(robot_ctx_size "$i")"

    cat <<EOF
llama-server \
  --host ${HOST_BIND} \
  --port ${port} \
  --model ${model} \
  --alias ${name} \
  --ctx-size ${ctx} \
  --n-gpu-layers ${NGL_OVERRIDE} \
  --threads ${EFE_SLM_THREADS_PER_ROBOT} \
  --threads-batch ${EFE_SLM_THREADS_PER_ROBOT} \
  --parallel 1 \
  --cont-batching \
  --cache-reuse 256 \
  --metrics \
  --no-warmup
EOF
}

# ─── Démarrage effectif ─────────────────────────────────────────────────────
log_step "Démarrage des llama-server"

started=()
failed=()

for i in "${to_start[@]}"; do
    name="$(robot_name "$i")"
    port="$(robot_port "$i")"
    log_file="$(robot_log_path "$i")"
    pid_file="$(robot_pid_path "$i")"

    cmd="$(build_llama_cmd "$i")"

    echo "${EFE_C_BOLD}▶ ${name} (port ${port})${EFE_C_RESET}"
    echo "${EFE_C_DIM}  cmd  : ${cmd//$'\n'/}${EFE_C_RESET}"
    echo "${EFE_C_DIM}  log  : ${log_file}${EFE_C_RESET}"

    if (( DRY_RUN == 1 )); then
        log_info "  [DRY-RUN] aucun process lancé."
        continue
    fi

    # Rotation : si log existe déjà, archive en .log.prev.
    if [[ -f "${log_file}" ]]; then
        mv -f "${log_file}" "${log_file}.prev"
    fi

    # Lance en arrière-plan, détaché du shell parent.
    # nohup + setsid pour survie même si terminal se ferme.
    # eval requis pour résoudre les \ multilignes de build_llama_cmd.
    if command -v setsid >/dev/null 2>&1; then
        # Linux : setsid sépare la session.
        setsid bash -c "${cmd}" >"${log_file}" 2>&1 < /dev/null &
    else
        # macOS : nohup suffit (pas de setsid de base).
        nohup bash -c "${cmd}" >"${log_file}" 2>&1 < /dev/null &
    fi
    pid=$!
    STARTED_PIDS+=("${pid}")
    echo "${pid}" > "${pid_file}"
    log_info "  PID  : ${pid} (écrit dans ${pid_file})"

    # Petit délai pour que le process commence à charger le modèle.
    sleep 1
    if ! pid_alive "${pid}"; then
        log_err "  ${name} a crashé immédiatement. Voir : ${log_file}"
        failed+=("${name}")
        rm -f "${pid_file}"
        continue
    fi

    started+=("${name}")
done

if (( DRY_RUN == 1 )); then
    log_info "DRY-RUN terminé."
    exit 0
fi

# ─── Attente health-check ───────────────────────────────────────────────────
if (( ${#started[@]} == 0 )); then
    log_err "Aucun process n'a démarré correctement."
    exit 1
fi

log_step "Attente health-check (timeout ${HEALTH_TIMEOUT}s par robot)"

healthy=()
unhealthy=()

for i in "${to_start[@]}"; do
    name="$(robot_name "$i")"
    port="$(robot_port "$i")"
    pid_file="$(robot_pid_path "$i")"

    # Si le process est mort entre temps, on skip.
    if ! pid="$(read_pid_alive "${pid_file}")"; then
        unhealthy+=("${name} (process mort)")
        continue
    fi

    printf "  ${EFE_C_DIM}polling ${name}:${port}${EFE_C_RESET}"
    elapsed=0
    while (( elapsed < HEALTH_TIMEOUT )); do
        if http_health_ok "${port}" 1; then
            printf "\r"
            log_ok "${name} READY (port ${port}, pid ${pid}, ${elapsed}s)"
            healthy+=("${name}")
            break
        fi
        printf "."
        sleep 1
        elapsed=$(( elapsed + 1 ))

        # Si le process meurt pendant le polling, abort.
        if ! pid_alive "${pid}"; then
            printf "\r"
            log_err "${name} CRASHED pendant le démarrage (pid ${pid} mort)"
            unhealthy+=("${name} (crash)")
            rm -f "${pid_file}"
            break
        fi
    done

    if [[ ! " ${healthy[*]} " == *" ${name} "* ]] && [[ ! " ${unhealthy[*]} " == *" ${name} "* ]]; then
        printf "\r"
        log_err "${name} TIMEOUT après ${HEALTH_TIMEOUT}s (port ${port} pas de /health)"
        unhealthy+=("${name} (timeout)")
    fi
done

# ─── Désactivation des traps après démarrage ────────────────────────────────
# À partir d'ici, les daemons sont stables. Si l'utilisateur Ctrl+C maintenant,
# c'est sur le récap final — on ne tue PAS les daemons légitimes.
trap - INT TERM

# ─── Résumé final ───────────────────────────────────────────────────────────
log_step "Résumé démarrage"
log_ok "Healthy (${#healthy[@]})    : ${healthy[*]:-aucun}"
if (( ${#unhealthy[@]} > 0 )); then
    log_err "Unhealthy (${#unhealthy[@]}) : ${unhealthy[*]}"
    echo ""
    log_warn "Inspecte les logs :"
    for i in "${to_start[@]}"; do
        echo "    ${EFE_C_DIM}tail -f $(robot_log_path "$i")${EFE_C_RESET}"
    done
    exit 1
fi

echo ""
log_ok "Tous les nano-robots Phase 1 sont opérationnels."
log_info "Endpoints :"
for i in "${!ROBOT_NAMES[@]}"; do
    echo "    $(robot_name "$i")  → http://${HOST_BIND}:$(robot_port "$i")"
done
echo ""
log_info "Étape suivante :"
echo "    ${EFE_C_DIM}./scripts/slm/health.sh${EFE_C_RESET}      # vérification continue"
echo "    ${EFE_C_DIM}./scripts/slm/stop.sh${EFE_C_RESET}        # arrêt propre"
echo "    ${EFE_C_DIM}cargo test -p efe-nano-app-coordinator --test live_smoke -- --nocapture${EFE_C_RESET}"
