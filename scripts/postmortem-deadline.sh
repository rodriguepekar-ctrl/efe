#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  postmortem-deadline.sh — compute the post-mortem deadline for a
#  closed incident issue under the algorithm frozen in STABILITY § 26.4.
#
#  Inputs (positional) :
#    $1  severity            S1 | S2 | S3
#    $2  closed_at           ISO-8601 UTC (e.g. 2026-03-14T15:00:00Z)
#                            OR a plain YYYY-MM-DD date.
#
#  Output (stdout, one line) :
#    deadline in YYYY-MM-DD format, after applying weekend + holiday
#    shifts.
#
#  Algorithm (STABILITY § 26.4, frozen) :
#    base_window_days = match severity :
#      S1 → 7
#      S2 → 14
#      S3 → 30
#    naive_deadline = closed_at + base_window_days
#    while naive_deadline is Saturday, Sunday, or a recognised holiday :
#      naive_deadline := next calendar day
#    deadline := naive_deadline
#
#  Holiday source :
#    docs/process/efe-business-calendar.md — every line matching
#    `| YYYY-MM-DD |` is a recognised holiday.  Path is resolved
#    relative to the workspace root (CWD when the script runs).
#
#  Why this is a separate script (rather than inline in the workflow) :
#    * Testable.  bench/harness/tests/postmortem_schema.rs exercises
#      it directly with known inputs and expected outputs.
#    * Reusable.  Future workflows (e.g. I.7's hard-gate workflow in
#      Q1-2027) call the same script and get the same algorithm.
#    * Frozen surface.  STABILITY § 26.4 names the algorithm ; this
#      script IS the algorithm.  Drift between the doc and the code
#      becomes a code-level diff that review can catch.
#
#  Compatibility :
#    Works with both GNU date (Linux CI) and BSD date (macOS local).
#    Each `date` call has a `||` fallback to the BSD form.
#
#  Exit codes :
#    0  success — deadline printed to stdout
#    2  bad arguments (missing severity or closed_at, or invalid shape)
#
#  References :
#    docs/STABILITY_CONTRACT.md § 26.4
#    docs/process/efe-business-calendar.md
#    docs/architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md § 5.3.2
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail

if [ $# -ne 2 ] ; then
    cat <<'EOF' >&2
usage : postmortem-deadline.sh <severity> <closed_at>
    severity   one of S1 | S2 | S3
    closed_at  ISO-8601 UTC (2026-03-14T15:00:00Z) or YYYY-MM-DD
EOF
    exit 2
fi

readonly SEVERITY="$1"
readonly CLOSED_AT="$2"

# ─── window-days from severity ───────────────────────────────────────
case "$SEVERITY" in
    S1) WINDOW_DAYS=7  ;;
    S2) WINDOW_DAYS=14 ;;
    S3) WINDOW_DAYS=30 ;;
    *)
        echo "error: severity must be S1, S2, or S3 (got '$SEVERITY')" >&2
        exit 2
        ;;
esac

# ─── helper : add days to a YYYY-MM-DD or ISO-8601 date ──────────────
add_days() {
    local input="$1"
    local days="$2"
    # GNU date first (Linux CI).  BSD date second (macOS local).
    if date -u -d "${input} + ${days} days" +%Y-%m-%d 2>/dev/null ; then
        return 0
    fi
    # BSD : the parser format depends on the input shape.
    if [[ "$input" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] ; then
        date -u -j -v+"${days}"d -f %Y-%m-%d "$input" +%Y-%m-%d
    elif [[ "$input" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] ; then
        date -u -j -v+"${days}"d -f %Y-%m-%dT%H:%M:%SZ "$input" +%Y-%m-%d
    else
        echo "error: cannot parse closed_at '$input'" >&2
        exit 2
    fi
}

# ─── helper : day-of-week (0=Sun .. 6=Sat) for a YYYY-MM-DD ──────────
day_of_week() {
    local d="$1"
    date -u -d "$d" +%w 2>/dev/null \
        || date -u -j -f %Y-%m-%d "$d" +%w
}

# ─── helper : is date a recognised holiday ? ─────────────────────────
# Holiday source : docs/process/efe-business-calendar.md.  Each line
# `| YYYY-MM-DD | Name |` is a recognised holiday.  Path resolved
# relative to CWD (workflow always checks out at workspace root).
is_holiday() {
    local d="$1"
    local cal="docs/process/efe-business-calendar.md"
    if [ ! -f "$cal" ] ; then
        return 1   # No calendar file → no holidays.
    fi
    # The `|[[:space:]]*YYYY-MM-DD[[:space:]]*\|` pattern matches the
    # table-row leading cell exactly.  Trailing content is the holiday
    # name, which we don't need.
    if grep -qE "^\|[[:space:]]*${d}[[:space:]]*\|" "$cal" ; then
        return 0
    fi
    return 1
}

# ─── main : compute naive deadline, then shift ───────────────────────
deadline=$(add_days "$CLOSED_AT" "$WINDOW_DAYS")

# Apply the shift rule.  Bounded loop : 14 iterations is more than
# enough (a 14-day-consecutive holiday block does not exist on the
# calendar we ship).
iter=0
while [ "$iter" -lt 14 ] ; do
    iter=$((iter + 1))
    dow=$(day_of_week "$deadline")
    # Saturday (6) → +2 days to Monday.
    if [ "$dow" = "6" ] ; then
        deadline=$(add_days "$deadline" 2)
        continue
    fi
    # Sunday (0) → +1 day to Monday.
    if [ "$dow" = "0" ] ; then
        deadline=$(add_days "$deadline" 1)
        continue
    fi
    # Weekday — check holiday.
    if is_holiday "$deadline" ; then
        deadline=$(add_days "$deadline" 1)
        continue
    fi
    # Business day, not a holiday.  Done.
    break
done

printf '%s\n' "$deadline"
