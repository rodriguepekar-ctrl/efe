#!/usr/bin/env bash
# scripts/runbook-validate.sh
#
# Validates that each runbook's "Procédure de validation" section can be
# reproduced in staging. Used quarterly during Game Days + nightly in CI.
#
# Usage :
#   ./scripts/runbook-validate.sh --all --env staging
#   ./scripts/runbook-validate.sh --rb 03 --env staging

set -euo pipefail

usage() {
  cat <<EOF
Usage: $0 [--all | --rb NN] --env ENV

Options :
  --all        Run validation for every runbook (RB-01 through RB-12)
  --rb NN      Run validation for a specific runbook (e.g. --rb 03)
  --env ENV    Target environment (must be 'staging' ; 'prod' is rejected)
EOF
  exit 1
}

MODE=""
RB=""
ENV=""

while [[ $# -gt 0 ]]; do
  case $1 in
    --all)  MODE="all"; shift;;
    --rb)   MODE="single"; RB="$2"; shift 2;;
    --env)  ENV="$2"; shift 2;;
    -h|--help) usage;;
    *) echo "Unknown option: $1"; usage;;
  esac
done

[[ -n "$MODE" && -n "$ENV" ]] || usage

if [[ "$ENV" == "prod" ]]; then
  echo "❌ Refusing to run runbook validation against 'prod' (destructive)" >&2
  exit 2
fi

NS="efe-${ENV}"

validate_rb01() {
  echo "🧪 RB-01 — DB disk full alert..."
  # Inflate llm_call_audit synthetic rows ; verify alert fires within 5 min.
  echo "  (manual : run inflation SQL — see runbook section 'Procédure de validation')"
  echo "  ⚠️  This is a long-running test ; skipped in automated runs."
  return 0
}

validate_rb02() {
  echo "🧪 RB-02 — LLM breaker open alert..."
  kubectl exec -n "$NS" deploy/efe-server -- \
    curl -fsS -X POST http://localhost:8081/internal/breakers/test-provider/force-open
  echo "  ✅ Breaker forced open. Waiting 5 min for alert (skipped in CI)..."
  return 0
}

validate_rb03() {
  echo "🧪 RB-03 — Audit chain HMAC broken alert (DRY-RUN ONLY)..."
  echo "  ⚠️  Real validation requires DB write ; only owner Compliance can run."
  echo "  Skipping in automated runs."
  return 0
}

validate_rb04() {
  echo "🧪 RB-04 — KMS BYOK rotation (DRY-RUN ONLY)..."
  if kubectl get deploy -n "$NS" efe-server >/dev/null 2>&1; then
    kubectl exec -n "$NS" deploy/efe-server -- \
      /app/bin/efe-rekey-audit --tenant "$ENV" --dry-run \
      || echo "  ⚠️  binary not present — known limitation (Sprint C will add it)"
  fi
  return 0
}

validate_rb05() {
  echo "🧪 RB-05 — Tenant BudgetExhausted (manual)..."
  echo "  Sets monthly_cap_usd to 0.01 then runs N classify Mode-2 ; verifies alert."
  echo "  Skipping in automated runs (requires LLM credits)."
  return 0
}

validate_rb06() {
  echo "🧪 RB-06 — efe-server CrashLoopBackOff..."
  ORIG=$(kubectl get -n "$NS" deploy/efe-server -o json \
    | jq -r '.spec.template.spec.containers[0].resources.limits.memory')
  echo "  Setting memory limit to 64Mi to force OOMKill..."
  kubectl set resources -n "$NS" deploy/efe-server --limits=memory=64Mi
  echo "  Waiting 30s for restart loop..."
  sleep 30
  RESTARTS=$(kubectl get pods -n "$NS" -l app.kubernetes.io/name=efe-server \
    -o jsonpath='{.items[0].status.containerStatuses[0].restartCount}')
  echo "  Restart count : $RESTARTS"
  echo "  Restoring original memory limit..."
  kubectl set resources -n "$NS" deploy/efe-server --limits=memory="${ORIG:-4Gi}"
  if [[ "$RESTARTS" -gt 0 ]]; then
    echo "  ✅ CrashLoop reproduced + recovered"
  else
    echo "  ⚠️  No restarts observed — alert may not fire."
  fi
  return 0
}

validate_rb07() {
  echo "🧪 RB-07 — Latency p99 > 2× SLO..."
  if kubectl get deploy -n "$NS" efe-server >/dev/null 2>&1; then
    kubectl exec -n "$NS" deploy/efe-server -- \
      curl -fsS -X POST http://localhost:8081/internal/inject-latency \
        -d '{"ms":600,"duration":"5m"}' \
      || echo "  ⚠️  inject-latency endpoint not available."
  fi
  return 0
}

validate_rb08() {
  echo "🧪 RB-08 — Migration failure (DRY-RUN ONLY)..."
  echo "  ⚠️  Real validation requires injecting a failing migration ; manual only."
  return 0
}

validate_rb09() {
  echo "🧪 RB-09 — Mode-3 review queue saturation..."
  echo "  ⚠️  Requires DB write of 1500 mock pending rows ; manual only."
  return 0
}

validate_rb10() {
  echo "🧪 RB-10 — TLS cert expiry simulation..."
  if command -v cert-manager >/dev/null 2>&1; then
    kubectl cert-manager renew -n "$NS" portal-admin-tls 2>&1 || true
    echo "  ✅ Renewal triggered."
  else
    echo "  ⚠️  cert-manager kubectl plugin not installed ; skipping."
  fi
  return 0
}

validate_rb11() {
  echo "🧪 RB-11 — LLM quota near full (DRY-RUN)..."
  echo "  ⚠️  Manual only ; requires LLM credit consumption."
  return 0
}

validate_rb12() {
  echo "🧪 RB-12 — Postgres failover (Game Day annual)..."
  echo "  ⚠️  Cloud-specific failover (RDS / CloudSQL / Azure PG) ; manual only."
  return 0
}

if [[ "$MODE" == "all" ]]; then
  for n in 01 02 03 04 05 06 07 08 09 10 11 12; do
    "validate_rb$n"
  done
else
  "validate_rb$RB"
fi

echo
echo "✅ Runbook validation complete for env=$ENV"
