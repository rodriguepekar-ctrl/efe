#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  postmortem-validator.sh — validates a docs/postmortems/INC-YYYY-NNN.md
#  file against the schema documented in docs/postmortems/_template.md.
#
#  Implements process invariant G.15 (PR #26 § 5.2) :
#    "Every closed incident issue MUST carry a structurally valid
#     post-mortem file before its deadline expires."
#
#  This validator checks STRUCTURE only.  It does NOT check the
#  semantic content of any section (that is reserved for I.7 in
#  Phase-3 Q1-2027).  The structural check is six sequential gates ;
#  the first failing gate produces the corresponding exit code and
#  the validator does not proceed.
#
#  Invocation :
#    bash scripts/postmortem-validator.sh <path-to-postmortem.md>
#
#  Exit codes (stable contract — STABILITY § 26.3) :
#    0  validation passed
#    2  file argument missing or path doesn't exist
#    3  frontmatter missing, malformed, or any mandatory field bad
#    4  one or more canonical sections missing / out-of-order
#    5  Timeline section has fewer than 2 non-header rows
#    6  Action items section has fewer than 1 non-header row
#    7  Compliance checklist has un-addressed (i.e. neither ticked
#       nor N/A-annotated) checkboxes
#
#  Exit code 1 is reserved for unexpected runtime errors (broken
#  bash, missing tools).  Codes 8+ are reserved for future check
#  additions (I.7 will likely introduce them).
#
#  Allow-list :
#    The validator SKIPS files whose basename starts with `_`.  This
#    excludes the canonical template (`_template.md`) and the
#    reference example (`_VALIDATOR_REFERENCE.md`).  Grandfathered
#    pre-G.15 post-mortems that don't conform to the schema are
#    enumerated in the GRANDFATHERED array below and also skipped.
#
#  Locally :
#    Same usage as the CI workflow.  Output is colourised when
#    stdout is a TTY.
#
#  CI :
#    Invoked from .github/workflows/pr26-postmortem-checks.yml job
#    `validate-files` (file-level mode) for every file in
#    docs/postmortems/ on every PR, and from job `check-deadlines`
#    (deadline-level mode) nightly for each closed-incident-issue
#    that has reached its deadline window.
#
#  References :
#    docs/architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md § 5.2
#    docs/postmortems/_template.md                      (canonical schema)
#    docs/postmortems/_VALIDATOR_REFERENCE.md           (worked example)
#    docs/STABILITY_CONTRACT.md § 26.3                  (stable exit-code contract)
#    scripts/audit-prompt-validator.sh                  (sibling reference impl)
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail
IFS=$'\n\t'

# ─── grandfathered pre-G.15 post-mortems (skipped) ───────────────────
# Files explicitly listed here are pre-PR-26 post-mortems whose
# section structure does not conform to the canonical schema.  They
# are scheduled for rewriting under the canonical schema in Q1-2027 ;
# until then, they are accepted as-is.  Add new entries here ONLY
# with a STABILITY-bump PR — silently skipping a file is a privilege
# the team grants itself sparingly.
readonly GRANDFATHERED=(
    # (empty as of PR #26 — both pre-existing non-conforming files
    #  identified by the SRE audit referenced in the architecture
    #  doc § 1.3 will be migrated as part of the G.15 backfill plan
    #  before PR #26 itself merges.)
)

# ─── colour helpers (TTY only) ───────────────────────────────────────
if [ -t 1 ] && [ "${NO_COLOR:-}" = "" ] ; then
    readonly C_RED='\033[31m'
    readonly C_YELLOW='\033[33m'
    readonly C_GREEN='\033[32m'
    readonly C_BOLD='\033[1m'
    readonly C_DIM='\033[2m'
    readonly C_RESET='\033[0m'
else
    readonly C_RED=''
    readonly C_YELLOW=''
    readonly C_GREEN=''
    readonly C_BOLD=''
    readonly C_DIM=''
    readonly C_RESET=''
fi

log_err()  { printf '%b\n' "${C_RED}${C_BOLD}error:${C_RESET} $*" >&2; }
log_warn() { printf '%b\n' "${C_YELLOW}warn:${C_RESET}  $*" >&2; }
log_ok()   { printf '%b\n' "${C_GREEN}ok:${C_RESET}    $*"; }
log_info() { printf '%b\n' "${C_DIM}info:${C_RESET}  $*"; }

# ─── usage ────────────────────────────────────────────────────────────
usage() {
    cat <<'EOF'
postmortem-validator.sh — validate a post-mortem file against the
G.15 canonical schema (STABILITY § 26).

USAGE
    scripts/postmortem-validator.sh <PATH>

ARGUMENTS
    PATH    Path to docs/postmortems/INC-YYYY-NNN.md (or any markdown
            file that should follow the canonical schema).

EXIT CODES
    0   validation passed
    2   file argument missing or path doesn't exist
    3   frontmatter missing, malformed, or any mandatory field bad
    4   one or more canonical sections missing / out-of-order
    5   Timeline has fewer than 2 non-header rows
    6   Action items have fewer than 1 non-header row
    7   Compliance checklist has un-addressed checkboxes

EXAMPLE
    # Validate the reference example (must pass).
    scripts/postmortem-validator.sh docs/postmortems/_VALIDATOR_REFERENCE.md

    # Validate a new post-mortem before pushing.
    scripts/postmortem-validator.sh docs/postmortems/INC-2026-007.md

    # Note : files whose basename starts with `_` are intentionally
    # skipped (templates + reference example).  Run the validator on
    # them anyway and you will get the SKIP message + exit 0.

ENVIRONMENT
    NO_COLOR=1    Disable ANSI colour codes (default : auto-detect TTY).
    DEBUG=1       Print the section regexes and intermediate parses.

REFERENCES
    docs/architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md § 5.2
    docs/postmortems/_template.md   (the canonical schema)
    docs/STABILITY_CONTRACT.md § 26.3 (the stable exit-code contract)
EOF
}

# ─── arg parsing ─────────────────────────────────────────────────────
if [ $# -ne 1 ] || [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ] ; then
    usage
    [ $# -ne 1 ] && exit 2
    exit 0
fi

readonly PM_FILE="$1"

if [ ! -e "$PM_FILE" ] ; then
    log_err "$PM_FILE does not exist"
    exit 2
fi

if [ ! -f "$PM_FILE" ] ; then
    log_err "$PM_FILE is not a regular file"
    exit 2
fi

if [ ! -s "$PM_FILE" ] ; then
    log_err "$PM_FILE is empty"
    exit 2
fi

# ─── allow-list : skip underscore-prefixed files + grandfathered ─────
readonly PM_BASENAME=$(basename "$PM_FILE")
if [[ "$PM_BASENAME" == _* ]] ; then
    log_info "skipping $PM_FILE (underscore-prefixed : template or reference)"
    exit 0
fi
# Guard the loop against the bash quirk where "${arr[@]:-}" on an
# empty array iterates once with an empty entry under `set -u`.
if [ ${#GRANDFATHERED[@]} -gt 0 ] ; then
    for entry in "${GRANDFATHERED[@]}" ; do
        if [ "$PM_BASENAME" = "$entry" ] ; then
            log_info "skipping $PM_FILE (grandfathered pre-G.15 post-mortem)"
            exit 0
        fi
    done
fi

log_info "validating $PM_FILE"

# ─── Check 2 : frontmatter parses ────────────────────────────────────
# The frontmatter is the YAML block bracketed by `---` at the head of
# the file.  We extract it with awk, parse the mandatory fields with
# regex (bash doesn't ship a YAML parser by default, and we don't want
# to pin a third-party dependency for this tiny job), and verify each
# field's value matches its expected shape.

# Extract everything between the first `---` and the second `---`.
frontmatter=$(
    awk '
        BEGIN { state = "before" }
        /^---[[:space:]]*$/ {
            if (state == "before") { state = "inside" ; next }
            if (state == "inside") { state = "after" ; next }
        }
        state == "inside" { print }
    ' "$PM_FILE"
)

if [ -z "$frontmatter" ] ; then
    log_err "frontmatter missing or empty"
    log_err "file must begin with a YAML block bracketed by '---' lines"
    log_err "see docs/postmortems/_template.md for the canonical shape"
    exit 3
fi

# Helper : extract the value of a scalar field from the frontmatter.
# We accept the `key: value` shape on a single line.  For the
# `reviewers:` list we count list-item lines separately.
field_value() {
    local key="$1"
    # shellcheck disable=SC2155
    local line=$(printf '%s\n' "$frontmatter" | grep -E "^${key}:[[:space:]]+" || echo "")
    if [ -z "$line" ] ; then
        echo ""
    else
        # Strip the key + colon, trim whitespace.
        echo "${line#${key}:}" | sed -E 's/^[[:space:]]+//' | sed -E 's/[[:space:]]+$//'
    fi
}

# 2.1 incident_id : INC-YYYY-NNN (single) OR INC-YYYY-NNN+NNN (multi).
incident_id=$(field_value "incident_id")
if [ -z "$incident_id" ] ; then
    log_err "frontmatter : incident_id missing"
    exit 3
fi
if ! [[ "$incident_id" =~ ^INC-[0-9]{4}-[0-9A-Z]+(\+[0-9A-Z]+)*$ ]] ; then
    log_err "frontmatter : incident_id has bad shape (got '$incident_id')"
    log_err "expected : INC-YYYY-NNN  (or INC-YYYY-NNN+MMM for multi-incident)"
    exit 3
fi
[ "${DEBUG:-0}" = "1" ] && log_info "incident_id : $incident_id"

# 2.2 severity : S1 | S2 | S3.
severity=$(field_value "severity")
case "$severity" in
    S1|S2|S3) ;;
    *)
        log_err "frontmatter : severity must be S1, S2, or S3 (got '$severity')"
        exit 3
        ;;
esac
[ "${DEBUG:-0}" = "1" ] && log_info "severity : $severity"

# 2.3 timestamps : ISO-8601 UTC (YYYY-MM-DDTHH:MM:SSZ).
readonly ISO8601_RE='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'
for ts_field in detected_at mitigated_at resolved_at ; do
    ts_value=$(field_value "$ts_field")
    if [ -z "$ts_value" ] ; then
        log_err "frontmatter : $ts_field missing"
        exit 3
    fi
    if ! [[ "$ts_value" =~ $ISO8601_RE ]] ; then
        log_err "frontmatter : $ts_field has bad shape (got '$ts_value')"
        log_err "expected : ISO-8601 UTC like 2026-03-14T09:07:00Z"
        exit 3
    fi
done

# 2.4 author : non-empty string (GitHub-username-ish).
author=$(field_value "author")
if [ -z "$author" ] ; then
    log_err "frontmatter : author missing"
    exit 3
fi
if ! [[ "$author" =~ ^[a-zA-Z0-9._-]+$ ]] ; then
    log_err "frontmatter : author has bad shape (got '$author')"
    log_err "expected : a GitHub-style identifier"
    exit 3
fi

# 2.5 reviewers : list of at least 2 list-item lines.
reviewer_count=$(
    printf '%s\n' "$frontmatter" \
        | awk '
            BEGIN { in_list = 0 }
            /^reviewers:[[:space:]]*$/ { in_list = 1 ; next }
            in_list && /^[a-zA-Z_]+:/ { in_list = 0 }
            in_list && /^[[:space:]]+-[[:space:]]+/ { count++ }
            END { print count + 0 }
        '
)
if [ "$reviewer_count" -lt 2 ] ; then
    log_err "frontmatter : reviewers must list at least 2 entries (got $reviewer_count)"
    exit 3
fi
[ "${DEBUG:-0}" = "1" ] && log_info "reviewers : $reviewer_count entries"

# 2.6 github_issue : URL.
gh_issue=$(field_value "github_issue")
if [ -z "$gh_issue" ] ; then
    log_err "frontmatter : github_issue missing"
    exit 3
fi
if ! [[ "$gh_issue" =~ ^https?://github\.com/[^/]+/[^/]+/issues/[0-9]+ ]] ; then
    log_err "frontmatter : github_issue must be a github.com issue URL (got '$gh_issue')"
    exit 3
fi

# 2.7 status : DRAFT | REVIEW | PUBLISHED  (PUBLISHED_RETROSPECTIVE
# accepted for backfilled post-mortems per the architecture doc § 8.2).
status=$(field_value "status")
case "$status" in
    DRAFT|REVIEW|PUBLISHED|PUBLISHED_RETROSPECTIVE) ;;
    *)
        log_err "frontmatter : status must be DRAFT, REVIEW, PUBLISHED, or PUBLISHED_RETROSPECTIVE (got '$status')"
        exit 3
        ;;
esac

log_ok "frontmatter parses : $incident_id / $severity / status=$status"

# ─── Check 3 : eleven canonical sections present, in order ───────────
# Each section is matched against `^##+[[:space:]]+<N>\.[[:space:]]+<Title>`
# with a word boundary at the end of the title so that authors can add
# trailing parenthetical caveats (e.g. "## 1. Summary (= 3-4 sentences)").

readonly REQUIRED_SECTIONS=(
    "1. Summary"
    "2. Timeline"
    "3. Root cause"
    "4. Contributing factors"
    "5. What went well"
    "6. What went wrong"
    "7. Action items"
    "8. Prevention"
    "9. Customer impact"
    "10. Compliance ramifications"
    "11. Cross-references"
)

section_regex() {
    local label="$1"
    local num="${label%%.*}"
    local title="${label#*. }"
    printf '^##+[[:space:]]+%s\\.[[:space:]]+%s' "$num" "$title"
}

missing_sections=()
order_violations=()
last_line_number=0
for label in "${REQUIRED_SECTIONS[@]}" ; do
    re="$(section_regex "$label")"
    [ "${DEBUG:-0}" = "1" ] && log_info "checking : $re"
    line_number=$(grep -nE -- "$re" "$PM_FILE" | head -n1 | cut -d: -f1 || echo "")
    if [ -z "$line_number" ] ; then
        missing_sections+=("$label")
        continue
    fi
    if [ "$line_number" -le "$last_line_number" ] ; then
        order_violations+=("$label (at line $line_number, prev section ended at $last_line_number)")
    fi
    last_line_number=$line_number
done

if [ ${#missing_sections[@]} -gt 0 ] ; then
    log_err "missing required section(s) :"
    for s in "${missing_sections[@]}" ; do
        printf '       - %s\n' "$s" >&2
    done
    printf '\n' >&2
    log_err "see docs/postmortems/_template.md for the canonical schema"
    exit 4
fi

if [ ${#order_violations[@]} -gt 0 ] ; then
    log_err "section(s) out of canonical order :"
    for v in "${order_violations[@]}" ; do
        printf '       - %s\n' "$v" >&2
    done
    exit 4
fi

log_ok "all 11 canonical sections present, in order"

# ─── Check 4 : Timeline has ≥ 2 non-header rows ──────────────────────
# Rows between "## 2. Timeline" and the next "## N." heading.  A
# valid row is a pipe-bracketed line that is neither the header
# (`| Time | Event |`) nor the separator (`|---|---|`).

timeline_rows=$(
    awk '
        BEGIN { inside = 0 }
        /^##+[[:space:]]+2\.[[:space:]]+Timeline/ { inside = 1 ; next }
        /^##+[[:space:]]+3\./ { inside = 0 ; exit }
        inside && /^\|/ {
            if ($0 ~ /^\|[[:space:]]*-+[[:space:]]*\|/) next            # separator
            if ($0 ~ /^\|[[:space:]]*Time[[:space:]]*\|/) next         # header
            print
        }
    ' "$PM_FILE" | wc -l | tr -d '[:space:]'
)
timeline_rows=${timeline_rows:-0}

if [ "$timeline_rows" -lt 2 ] ; then
    log_err "Timeline (§ 2) has only $timeline_rows row(s)"
    log_err "Minimum 2 rows required.  Even a two-row timeline (first signal + all-clear)"
    log_err "establishes the chronology a reader needs."
    exit 5
fi
log_ok "Timeline (§ 2) : $timeline_rows non-header row(s)"

# ─── Check 5 : Action items has ≥ 1 non-header row ───────────────────
action_rows=$(
    awk '
        BEGIN { inside = 0 }
        /^##+[[:space:]]+7\.[[:space:]]+Action items/ { inside = 1 ; next }
        /^##+[[:space:]]+8\./ { inside = 0 ; exit }
        inside && /^\|/ {
            if ($0 ~ /^\|[[:space:]]*-+[[:space:]]*\|/) next            # separator
            if ($0 ~ /^\|[[:space:]]*Action[[:space:]]*\|/) next       # header
            print
        }
    ' "$PM_FILE" | wc -l | tr -d '[:space:]'
)
action_rows=${action_rows:-0}

if [ "$action_rows" -lt 1 ] ; then
    log_err "Action items (§ 7) has no rows"
    log_err "Minimum 1 row required.  If you genuinely have no action items,"
    log_err "use a single row like 'No action items — root cause resolved fully | n/a | — | — | DONE'."
    exit 6
fi
log_ok "Action items (§ 7) : $action_rows non-header row(s)"

# ─── Check 6 : Compliance checklist fully addressed ──────────────────
# Each `- [ ]` checkbox must be either ticked (`- [x]`) OR annotated
# with the literal string "N/A" anywhere on the same line.
unaddressed=$(
    awk '
        BEGIN { inside = 0 ; count = 0 }
        /^##+[[:space:]]+10\.[[:space:]]+Compliance/ { inside = 1 ; next }
        /^##+[[:space:]]+11\./ { inside = 0 ; exit }
        inside && /^- \[ \]/ {
            if ($0 !~ /N\/A/) count++
        }
        END { print count + 0 }
    ' "$PM_FILE"
)
unaddressed=${unaddressed:-0}

if [ "$unaddressed" -gt 0 ] ; then
    log_err "Compliance ramifications (§ 10) : $unaddressed un-addressed checkbox(es)"
    log_err "Every '- [ ]' line must either be ticked '- [x]' OR be annotated 'N/A'."
    log_err "(Annotation can be anywhere on the same line.)"
    exit 7
fi
log_ok "Compliance ramifications (§ 10) : all checkboxes addressed"

# ─── final summary ────────────────────────────────────────────────────
printf '\n'
log_ok "$PM_FILE passes schema validation"
log_info "incident=$incident_id  severity=$severity  status=$status  timeline=$timeline_rows  actions=$action_rows"

exit 0
