#!/usr/bin/env bash
# EFE monthly chargeback — exports tenant-level metrics to CSV.
#
# Invocation:
#   PROMETHEUS_URL=https://prom.internal \
#   MONTH=2026-04 \
#   bash deploy/scripts/chargeback.sh > chargeback_2026_04.csv
#
# Columns:
#   tenant, billing_account, requests, errors, rejects, cost_usd, reject_pct
#
# Cost formula (MVP): $0.00001 / classify call (tunable via COST_PER_CLASSIFY).
#
# Robustness note (vs. the previous version):
#   Each metric is queried separately and merged on (tenant,billing_account)
#   via a single-pass awk aggregation. This is tolerant to:
#     - A tenant present in requests but absent in errors.
#     - Ordering differences between Prometheus responses.
#     - Missing billing_account labels (treated as "-").

set -Eeuo pipefail

: "${PROMETHEUS_URL:?set PROMETHEUS_URL, e.g. https://prom.internal}"
: "${MONTH:?set MONTH, e.g. 2026-04}"

COST_PER_CLASSIFY="${COST_PER_CLASSIFY:-0.00001}"

log() { printf "\033[1;34m==>\033[0m %s\n" "$*" >&2; }

# Portable days-in-month computation (GNU + BSD date).
DAYS_IN_MONTH=$(date -d "$MONTH-01 +1 month -1 day" +%d 2>/dev/null || \
                date -j -f "%Y-%m-%d" "$MONTH-28" +%d)
RANGE="${DAYS_IN_MONTH}d"

log "Querying Prometheus for tenant usage in $MONTH ($RANGE window)"

# Run a PromQL query and emit `tenant\tbilling_account\tvalue` lines.
query_metric() {
  local metric="$1" q
  q="sum by (tenant, billing_account) (increase(${metric}[${RANGE}]))"
  curl -sSG "$PROMETHEUS_URL/api/v1/query" \
    --data-urlencode "query=${q}" \
  | jq -r '.data.result[]
           | [(.metric.tenant // "-"),
              (.metric.billing_account // "-"),
              (.value[1] // "0")]
           | @tsv'
}

REQUESTS_TSV=$(query_metric "efe_classify_requests_total" || true)
ERRORS_TSV=$(  query_metric "efe_classify_errors_total"   || true)
REJECTS_TSV=$( query_metric "efe_classify_reject_total"   || true)

log "Merging on (tenant, billing_account)"

# Emit the CSV header.
printf "tenant,billing_account,requests,errors,rejects,cost_usd,reject_pct\n"

# Single-pass awk merge keyed by tenant|billing_account.
#   marker 1: REQUESTS  (col 3 of the combined stream is the value)
#   marker 2: ERRORS
#   marker 3: REJECTS
# We prefix each line so awk can tell them apart in a single pass.
{
  echo "$REQUESTS_TSV" | awk 'NF { print "1\t"$0 }'
  echo "$ERRORS_TSV"   | awk 'NF { print "2\t"$0 }'
  echo "$REJECTS_TSV"  | awk 'NF { print "3\t"$0 }'
} | awk -F'\t' -v cost="$COST_PER_CLASSIFY" '
  BEGIN { OFS="\t" }
  {
    kind = $1
    key  = $2 SUBSEP $3
    val  = $4 + 0
    tenant[key]  = $2
    billing[key] = $3
    if (kind == "1") req[key] = val
    if (kind == "2") err[key] = val
    if (kind == "3") rej[key] = val
  }
  END {
    for (k in tenant) {
      r = (k in req) ? req[k] : 0
      e = (k in err) ? err[k] : 0
      j = (k in rej) ? rej[k] : 0
      c = r * cost
      p = (r > 0) ? (j / r * 100) : 0
      printf "%s,%s,%d,%d,%d,%.4f,%.2f\n", tenant[k], billing[k], r, e, j, c, p
    }
  }
' | sort -t, -k1,1 -k2,2

log "Done."
