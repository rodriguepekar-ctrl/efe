#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  audit-prompt-validator.sh — validates a docs/lessons/PR-N-lessons.md
#  file against the schema documented in docs/lessons/_template.md.
#
#  Implements process invariant I.1 (PR #25 § 3.3) :
#    "Every lessons file MUST carry the 7 canonical sections and a
#    non-empty defect-index table."
#
#  Invocation :
#    bash scripts/audit-prompt-validator.sh <path-to-lessons.md>
#
#  Exit codes (stable contract — STABILITY § 25.1) :
#    0  validation passed
#    2  file argument missing or path doesn't exist
#    3  a required section header is missing
#    4  the defect-index table has zero rows
#    5  "Known residual gaps" section absent
#    6  the audit prompt's "audit ≥ 1" structural invariant violated
#
#  Locally :
#    Same usage.  Output is colourised when stdout is a TTY.
#
#  CI :
#    Invoked from .github/workflows/pr25-process-checks.yml job
#    `i1-audit-prompt`.  Stdout/stderr captured into the run log.
#
#  References :
#    docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 6.5
#    docs/lessons/_template.md
#    docs/lessons/pr24-lessons-learned.md  (golden example — must validate)
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail
IFS=$'\n\t'

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

# ─── messaging helpers ───────────────────────────────────────────────
log_err()  { printf '%b\n' "${C_RED}${C_BOLD}error:${C_RESET} $*" >&2; }
log_warn() { printf '%b\n' "${C_YELLOW}warn:${C_RESET}  $*" >&2; }
log_ok()   { printf '%b\n' "${C_GREEN}ok:${C_RESET}    $*"; }
log_info() { printf '%b\n' "${C_DIM}info:${C_RESET}  $*"; }

# ─── usage ────────────────────────────────────────────────────────────
usage() {
    cat <<'EOF'
audit-prompt-validator.sh — validate a lessons file against the I.1 schema

USAGE
    scripts/audit-prompt-validator.sh <PATH>

ARGUMENTS
    PATH    Path to docs/lessons/PR-<N>-lessons.md (or any markdown file
            that should follow the canonical schema).

EXIT CODES
    0   validation passed
    2   file argument missing or path doesn't exist
    3   a required section header is missing
    4   the defect-index table has zero rows
    5   "Known residual gaps" section absent
    6   the audit prompt structural invariant violated (≥ 1 pass)

EXAMPLE
    # Validate PR #24's lessons file (the golden reference).
    scripts/audit-prompt-validator.sh docs/lessons/pr24-lessons-learned.md

    # Validate a new lessons file before pushing.
    scripts/audit-prompt-validator.sh docs/lessons/PR-26-lessons.md

ENVIRONMENT
    NO_COLOR=1    Disable ANSI colour codes (default : auto-detect TTY).
    DEBUG=1       Print the section regexes being tested.

REFERENCES
    docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 6.5
    docs/lessons/_template.md
EOF
}

# ─── arg parsing ─────────────────────────────────────────────────────
if [ $# -ne 1 ] || [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ] ; then
    usage
    [ $# -ne 1 ] && exit 2
    exit 0
fi

readonly LESSONS_FILE="$1"

if [ ! -e "$LESSONS_FILE" ] ; then
    log_err "$LESSONS_FILE does not exist"
    exit 2
fi

if [ ! -f "$LESSONS_FILE" ] ; then
    log_err "$LESSONS_FILE is not a regular file"
    exit 2
fi

if [ ! -s "$LESSONS_FILE" ] ; then
    log_err "$LESSONS_FILE is empty"
    exit 2
fi

# ─── required-section regexes ─────────────────────────────────────────
# These map 1:1 to the schema in docs/lessons/_template.md.  Each entry
# is a (label, regex) pair ; the regex is grep -E syntax.  The labels
# are emitted in failure messages so the author knows exactly which
# header is missing.
#
# Convention : every section header is `## <N>. <Title>` ; we tolerate
# `## ` vs `### ` vs trailing whitespace by anchoring to a digit-dot
# prefix.
readonly REQUIRED_LABELS=(
    "1. Headline"
    "2. Timeline summary"
    "3. Defect taxonomy"
    "4. What worked"
    "5. Process changes proposed"
    "6. Defect index"
    "7. Known residual gaps"
)

# The regex for each label : '^##+[[:space:]]+<N>\.[[:space:]]+<Title>'.
# We anchor to start-of-line, tolerate any number of `#`, require at
# least one whitespace, then the literal section number-dot, then any
# whitespace + the title prefix (matches "## 1. Headline" exactly or
# "### 1. Headline more text").  Trailing characters are tolerated.

section_regex() {
    local label="$1"
    # Extract leading digit-dot.
    local num="${label%%.*}"
    # Extract title (everything after first ". ").
    local title="${label#*. }"
    # Quote regex metacharacters in title (very limited — titles are ASCII alpha + spaces).
    printf '^##+[[:space:]]+%s\\.[[:space:]]+%s' "$num" "$title"
}

# ─── checks ───────────────────────────────────────────────────────────

log_info "validating $LESSONS_FILE"

missing_sections=()
for label in "${REQUIRED_LABELS[@]}" ; do
    re="$(section_regex "$label")"
    [ "${DEBUG:-0}" = "1" ] && log_info "checking : $re"
    if ! grep -qE -- "$re" "$LESSONS_FILE" ; then
        missing_sections+=("$label")
    fi
done

if [ ${#missing_sections[@]} -gt 0 ] ; then
    log_err "missing required section(s) :"
    for s in "${missing_sections[@]}" ; do
        printf '       - %s\n' "$s" >&2
    done
    printf '\n' >&2
    log_err "see docs/lessons/_template.md for the canonical schema"
    exit 3
fi

log_ok "all 7 required sections present"

# ─── defect-index table : must have ≥ 1 row ──────────────────────────
# The table lives between the "## 6. Defect index" header and the
# "## 7. Known residual gaps" header.  Each row is a line starting
# with `|` that has ≥ 3 pipe-separated columns AND does not look like
# a header / separator (no `---`, no purely word-character cells).

table_rows=$(
    awk '
        BEGIN { inside = 0 }
        /^##+[[:space:]]+6\.[[:space:]]+Defect index/ { inside = 1; next }
        /^##+[[:space:]]+7\.[[:space:]]+Known residual gaps/ { inside = 0; exit }
        inside && /^\|/ {
            # Skip header rows ("| Code | Pass | ... |") and separator
            # rows ("|---|---|---|").  Heuristic : a real row has at
            # least one cell with a non-trivial value (digit or
            # 2+ letters that arent "Code|Pass|Class|description"-style header).
            if ($0 ~ /^\|[[:space:]]*-+[[:space:]]*\|/) next        # separator
            if ($0 ~ /^\|[[:space:]]*Code[[:space:]]*\|/) next     # header
            if ($0 ~ /^\|[[:space:]]*Pass[[:space:]]*\|/) next     # alt header
            print
        }
    ' "$LESSONS_FILE" | wc -l | tr -d '[:space:]'
)

# wc -l on macOS produces "  N" with leading whitespace ; tr stripped.
table_rows=${table_rows:-0}

if ! [[ "$table_rows" =~ ^[0-9]+$ ]] ; then
    log_err "could not parse defect-index table row count (got : '$table_rows')"
    exit 4
fi

if [ "$table_rows" -lt 1 ] ; then
    log_err "defect-index table (§ 6) has zero rows"
    log_err "Even a one-row entry ('D-X | ... | no defects found') is required"
    log_err "to demonstrate the audit happened.  See PR #24 lessons § 6."
    exit 4
fi

log_ok "defect-index table : $table_rows row(s)"

# ─── "Known residual gaps" section : must have a body ────────────────
# The section can say "None — audit converged."  But it must have a
# non-empty body (≥ 1 line of non-header content after the header).

gaps_body_lines=$(
    awk '
        BEGIN { inside = 0 }
        /^##+[[:space:]]+7\.[[:space:]]+Known residual gaps/ { inside = 1; next }
        # Next ## header ends the section.  Any level-2-or-deeper after
        # § 7 closes the read.
        inside && /^##+[[:space:]]/ { exit }
        inside && NF > 0 { print }
    ' "$LESSONS_FILE" | grep -vE '^[[:space:]]*$' | wc -l | tr -d '[:space:]'
)
gaps_body_lines=${gaps_body_lines:-0}

if ! [[ "$gaps_body_lines" =~ ^[0-9]+$ ]] || [ "$gaps_body_lines" -lt 1 ] ; then
    log_err "section 7 'Known residual gaps' has empty body"
    log_err "Even 'None — audit converged.' is a valid one-line body."
    exit 5
fi

log_ok "Known residual gaps section : $gaps_body_lines non-empty line(s)"

# ─── audit-prompt structural invariant : ≥ 1 pass mentioned ──────────
# The header says "<N>-pass chirurgical audit" or similar.  Verify
# the timeline summary table has ≥ 1 pass row.

pass_count=$(
    awk '
        BEGIN { inside = 0 }
        /^##+[[:space:]]+2\.[[:space:]]+Timeline/ { inside = 1; next }
        /^##+[[:space:]]+3\./ { inside = 0; exit }
        # A pass row : first cell starts with a digit (the pass number).
        # The cell may carry annotation text after the digit
        # ("| 1 (initial) |", "| 1 (Day 1 — foo) |").  Anchor only on
        # "| <whitespace>* <digit>".
        inside && /^\|[[:space:]]*[0-9]/ { print }
    ' "$LESSONS_FILE" | wc -l | tr -d '[:space:]'
)
pass_count=${pass_count:-0}

if ! [[ "$pass_count" =~ ^[0-9]+$ ]] || [ "$pass_count" -lt 1 ] ; then
    log_err "timeline summary (§ 2) has zero pass rows"
    log_err "An audit must have at least 1 pass row in the timeline table."
    log_err "Schema reminder : | Pass | Tests at end | Δ | Defects | Notable |"
    exit 6
fi

log_ok "timeline summary : $pass_count pass row(s)"

# ─── final summary ────────────────────────────────────────────────────
printf '\n'
log_ok "$LESSONS_FILE passes schema validation"
log_info "sections=7  defects=$table_rows  passes=$pass_count  gap-lines=$gaps_body_lines"

exit 0
