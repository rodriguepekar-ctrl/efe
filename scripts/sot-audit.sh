#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  sot-audit.sh — single-source-of-truth audit for parallel const lists.
#
#  Implements process invariant I.2 (PR #25 § 4.4) :
#    "Const lists of paths / filenames / flags with ≥ 5 elements MUST
#     have a single canonical declaration ; consumers reference by use."
#
#  This script is the HUMAN-READABLE companion to the rigorous Rust
#  test at bench/harness/tests/process_invariants.rs (PR #25 day 3).
#  It walks the workspace, extracts every `const NAME: &[&str]`
#  declaration, and reports :
#    1. Each declaration with its location + entry count.
#    2. Pairs of declarations whose entries overlap ≥ 80 %.
#
#  Invocation :
#    bash scripts/sot-audit.sh           # scan whole workspace
#    bash scripts/sot-audit.sh --json    # machine-readable output
#    bash scripts/sot-audit.sh --threshold 50    # custom overlap %
#
#  Exit codes :
#    0  scan completed (whether or not duplicates found — see --strict)
#    1  scan failed (workspace not found, tool missing, etc.)
#    2  --strict mode AND ≥ 1 duplicate pair detected
#
#  --strict mode :
#    By default this script is INFORMATIONAL ; --strict turns it into
#    a gate.  The CI workflow runs in non-strict mode and lets the
#    Rust test be the gate.  Operators can run with --strict locally
#    before opening a PR.
#
#  Limitations (documented honestly) :
#    - Heuristic-only : a regex-based extractor, not a real Rust parser.
#      A `const NAME: &[&str] = &[ ... ]` split across many lines is
#      handled, but exotic forms (e.g. `lazy_static`, `phf_map!`,
#      generated code) are NOT detected.  The Rust test catches what
#      this script misses.
#    - macOS BSD awk + Linux GNU awk both supported.  We avoid GNU
#      extensions.
#    - The threshold defaults to 80 % (matches PR #25 architecture § 4.4).
#
#  References :
#    docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 6.6
#    docs/process/sot-pattern.md
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail
IFS=$'\n\t'

# ─── default options ─────────────────────────────────────────────────
FORMAT=text          # text | json
STRICT=false
THRESHOLD=80         # percent overlap to flag as duplication
ROOT_DIR=""          # default : workspace root (derived below)
MIN_ENTRIES=5        # ignore lists with < N entries (per architecture § 4.4)

# ─── colour ──────────────────────────────────────────────────────────
if [ -t 1 ] && [ "${NO_COLOR:-}" = "" ] ; then
    C_RED='\033[31m'; C_YELLOW='\033[33m'; C_GREEN='\033[32m'
    C_BOLD='\033[1m'; C_DIM='\033[2m'; C_RESET='\033[0m'
else
    C_RED=''; C_YELLOW=''; C_GREEN=''; C_BOLD=''; C_DIM=''; C_RESET=''
fi
readonly C_RED C_YELLOW C_GREEN C_BOLD C_DIM C_RESET

# All log helpers write to STDERR.  STDOUT is reserved for the report
# (text or JSON) so callers can pipe the script's output cleanly.
log_err()  { printf '%b\n' "${C_RED}${C_BOLD}error:${C_RESET} $*" >&2; }
log_warn() { printf '%b\n' "${C_YELLOW}warn:${C_RESET}  $*" >&2; }
log_ok()   { printf '%b\n' "${C_GREEN}ok:${C_RESET}    $*" >&2; }
log_info() { printf '%b\n' "${C_DIM}info:${C_RESET}  $*" >&2; }

# ─── usage ────────────────────────────────────────────────────────────
usage() {
    cat <<'EOF'
sot-audit.sh — single-source-of-truth audit for parallel const &[&str] lists

USAGE
    scripts/sot-audit.sh [OPTIONS]

OPTIONS
    --json              emit a machine-readable JSON report
    --strict            exit non-zero if any parallel-list pair is found
    --threshold N       overlap percentage required to flag (default: 80)
    --root PATH         workspace root (default: derive from script path)
    --min-entries N     ignore lists with fewer entries (default: 5)
    -h, --help          show this help

EXIT CODES
    0  scan ran cleanly
    1  scan failed (workspace not found, etc.)
    2  --strict AND ≥ 1 duplication found

EXAMPLES
    scripts/sot-audit.sh                         # informational report
    scripts/sot-audit.sh --strict --threshold 90 # local pre-PR check
    scripts/sot-audit.sh --json | jq .findings   # CI integration

REFERENCES
    docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 6.6
EOF
}

# ─── arg parsing ──────────────────────────────────────────────────────
while [ $# -gt 0 ] ; do
    case "$1" in
        --json)         FORMAT=json ; shift ;;
        --strict)       STRICT=true ; shift ;;
        --threshold)    THRESHOLD="${2:?--threshold requires a number}" ; shift 2 ;;
        --root)         ROOT_DIR="${2:?--root requires a path}" ; shift 2 ;;
        --min-entries)  MIN_ENTRIES="${2:?--min-entries requires a number}" ; shift 2 ;;
        -h|--help)      usage ; exit 0 ;;
        *)              log_err "unknown option : $1" ; usage >&2 ; exit 1 ;;
    esac
done

# Validate numeric options.
if ! [[ "$THRESHOLD" =~ ^[0-9]+$ ]] || [ "$THRESHOLD" -gt 100 ] ; then
    log_err "--threshold must be an integer 0..100 (got '$THRESHOLD')"
    exit 1
fi
if ! [[ "$MIN_ENTRIES" =~ ^[0-9]+$ ]] ; then
    log_err "--min-entries must be a non-negative integer (got '$MIN_ENTRIES')"
    exit 1
fi

# ─── resolve workspace root ───────────────────────────────────────────
if [ -z "$ROOT_DIR" ] ; then
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

if [ ! -d "$ROOT_DIR" ] ; then
    log_err "workspace root not a directory : $ROOT_DIR"
    exit 1
fi

cd "$ROOT_DIR"

# ─── stage 1 : harvest every const &[&str] declaration ───────────────
# We use a tempdir for intermediate files.  Cleaned on exit.

TMP_DIR=$(mktemp -d -t sot-audit-XXXXXX)
trap 'rm -rf "$TMP_DIR"' EXIT

# Extract candidate files.  Skip target/ + .git/ + node_modules/ +
# generated build dirs.  Include workspace src dirs explicitly so a
# top-level scratch .rs doesn't sneak in.
find . \
    -path ./target -prune -o \
    -path ./.git -prune -o \
    -path ./node_modules -prune -o \
    -name '*.rs' -print 2>/dev/null > "$TMP_DIR/files.txt" || true

FILE_COUNT=$(wc -l < "$TMP_DIR/files.txt" | tr -d '[:space:]')
log_info "scanning $FILE_COUNT .rs files under $ROOT_DIR (threshold $THRESHOLD%, min $MIN_ENTRIES entries)"

# For each file, run an awk script that finds `const X: &[&str] = &[...];`
# spanning N lines.  Output format (one decl per line) :
#   <file>\t<line>\t<NAME>\t<count>\t<comma-separated-entries>

awk_extract='
BEGIN { inside = 0; name = ""; start_line = 0; items = "" }
function flush() {
    if (inside && name != "") {
        # Count items.
        n = split(items, arr, ",")
        # Trim whitespace + surrounding quotes from each item.
        out = ""
        for (i = 1; i <= n; i++) {
            v = arr[i]
            sub(/^[[:space:]]+/, "", v)
            sub(/[[:space:]]+$/, "", v)
            sub(/^"/, "", v); sub(/"$/, "", v)
            if (v != "") {
                out = (out == "") ? v : out "," v
                ki++
            }
        }
        if (out != "") {
            printf "%s\t%d\t%s\t%d\t%s\n", FILENAME, start_line, name, ki, out
        }
        ki = 0
    }
    inside = 0; name = ""; items = ""
}
# Match start of declaration on a single line.
/^[[:space:]]*(pub[[:space:]]+)?const[[:space:]]+[A-Z_][A-Z_0-9]*[[:space:]]*:[[:space:]]*&\[[[:space:]]*&str[[:space:]]*\][[:space:]]*=/ {
    # Extract NAME : after `const`, before `:`.
    line = $0
    sub(/^[[:space:]]*(pub[[:space:]]+)?const[[:space:]]+/, "", line)
    n_pos = index(line, ":")
    if (n_pos > 0) {
        name = substr(line, 1, n_pos - 1)
        sub(/[[:space:]]+$/, "", name)
    }
    start_line = NR
    inside = 1
    # Consume the `= &[...]` body.  The line contains TWO `&[` tokens :
    #   1. in the type annotation : `: &[&str]`
    #   2. in the value initialiser : `= &[...]`
    # We need the second.  Find the `=`, then search `&[` after it.
    eq_pos = index($0, "=")
    if (eq_pos > 0) {
        body_search = substr($0, eq_pos)
        body_pos_in_search = index(body_search, "&[")
        if (body_pos_in_search > 0) {
            # Absolute position in $0.
            body_pos = eq_pos + body_pos_in_search - 1
            rest = substr($0, body_pos + 2)
            # If `];` ends on same line, finalise.
            close_pos = index(rest, "]")
            if (close_pos > 0) {
                items = items substr(rest, 1, close_pos - 1)
                flush()
            } else {
                items = items rest
            }
        }
    }
    next
}
inside {
    # Continuation of multi-line declaration.
    close_pos = index($0, "]")
    if (close_pos > 0) {
        items = items substr($0, 1, close_pos - 1)
        flush()
    } else {
        items = items $0
    }
}
END { flush() }
'

# Run extractor.
> "$TMP_DIR/decls.txt"
while IFS= read -r f ; do
    awk "$awk_extract" "$f" >> "$TMP_DIR/decls.txt" || true
done < "$TMP_DIR/files.txt"

DECL_COUNT=$(wc -l < "$TMP_DIR/decls.txt" | tr -d '[:space:]')
log_info "extracted $DECL_COUNT const &[&str] declarations"

# Filter to ≥ MIN_ENTRIES.
awk -F'\t' -v min="$MIN_ENTRIES" '$4 >= min' "$TMP_DIR/decls.txt" > "$TMP_DIR/decls-filtered.txt"
FILTERED=$(wc -l < "$TMP_DIR/decls-filtered.txt" | tr -d '[:space:]')
log_info "$FILTERED declarations with ≥ $MIN_ENTRIES entries"

# ─── stage 2 : detect overlap pairs ──────────────────────────────────
# For every (i, j) pair with i < j, compute overlap percent =
# |entries_i ∩ entries_j| / max(|entries_i|, |entries_j|) * 100.

if [ "$FILTERED" -lt 1 ] ; then
    log_ok "no qualifying declarations — workspace is clean"
    if [ "$FORMAT" = "json" ] ; then
        printf '{"scanned":%d,"decls":%d,"filtered":0,"findings":[]}\n' \
               "$FILE_COUNT" "$DECL_COUNT"
    fi
    exit 0
fi

# Use awk to do the pairwise comparison.
awk_pairs='
BEGIN { FS="\t" ; n = 0 }
{
    n++
    files[n] = $1; lines[n] = $2; names[n] = $3; counts[n] = $4
    # Build a "set" of items as an associative array key.
    split($5, items, ",")
    for (k in items) delete items[k]   # no-op : reset
    split($5, items, ",")
    item_count[n] = 0
    for (k = 1; k in items; k++) {
        v = items[k]
        if (v != "") {
            set[n, v] = 1
            item_count[n]++
        }
    }
}
END {
    threshold = THRESHOLD + 0
    found = 0
    for (i = 1; i <= n; i++) {
        for (j = i + 1; j <= n; j++) {
            # Same file + same line → same decl, skip.
            if (files[i] == files[j] && lines[i] == lines[j]) continue
            # Compute intersection.
            common = 0
            max_count = (item_count[i] > item_count[j]) ? item_count[i] : item_count[j]
            if (max_count == 0) continue
            # Iterate the smaller set.
            if (item_count[i] <= item_count[j]) { small = i; large = j } else { small = j; large = i }
            for (key in set) {
                # key looks like "<small_idx>SUBSEP<item>".
                split(key, kk, SUBSEP)
                if (kk[1] + 0 == small) {
                    if ((large SUBSEP kk[2]) in set) common++
                }
            }
            overlap = int(common * 100 / max_count)
            if (overlap >= threshold) {
                found++
                printf "%s:%s\t%s (count=%s) ↔ %s:%s\t%s (count=%s)\toverlap=%d%%\n",
                       files[i], lines[i], names[i], counts[i],
                       files[j], lines[j], names[j], counts[j], overlap
            }
        }
    }
    exit (found > 0 ? 1 : 0)
}
'

set +e
awk -v THRESHOLD="$THRESHOLD" "$awk_pairs" "$TMP_DIR/decls-filtered.txt" > "$TMP_DIR/findings.txt"
PAIR_RC=$?
set -e

FINDING_COUNT=$(wc -l < "$TMP_DIR/findings.txt" | tr -d '[:space:]')

# ─── stage 3 : emit report ────────────────────────────────────────────

if [ "$FORMAT" = "json" ] ; then
    # JSON envelope.
    printf '{"scanned":%d,"decls":%d,"filtered":%d,"threshold":%d,"findings":[' \
           "$FILE_COUNT" "$DECL_COUNT" "$FILTERED" "$THRESHOLD"
    awk -F'\t' '
        BEGIN { first=1 }
        {
            if (!first) printf ","
            first = 0
            # Field shape : "<file>:<line>\t<name> (count=N)\t..."
            split($1, a, ":") ; file_a = a[1]; line_a = a[2]
            split($3, b, ":") ; file_b = b[1]; line_b = b[2]
            # Strip "name (count=N)" → name + count.
            match($2, /\(count=[0-9]+\)/)
            name_a = substr($2, 1, RSTART - 2)
            count_a = substr($2, RSTART + 7, RLENGTH - 8)
            match($4, /\(count=[0-9]+\)/)
            name_b = substr($4, 1, RSTART - 2)
            count_b = substr($4, RSTART + 7, RLENGTH - 8)
            # overlap field.
            split($5, o, "=") ; ov = o[2] ; sub(/%/, "", ov)
            printf "{\"a\":{\"file\":\"%s\",\"line\":%s,\"name\":\"%s\",\"count\":%s},\"b\":{\"file\":\"%s\",\"line\":%s,\"name\":\"%s\",\"count\":%s},\"overlap_pct\":%s}",
                   file_a, line_a, name_a, count_a, file_b, line_b, name_b, count_b, ov
        }
    ' "$TMP_DIR/findings.txt"
    printf ']}\n'
else
    # Human-readable report.
    printf '\n'
    if [ "$FINDING_COUNT" -eq 0 ] ; then
        log_ok "no parallel-list duplications found (threshold $THRESHOLD%)"
        log_info "$FILTERED qualifying declaration(s) scanned, all SOT-compliant"
    else
        log_warn "$FINDING_COUNT parallel-list pair(s) flagged (overlap >= $THRESHOLD%)"
        printf '\n'
        cat "$TMP_DIR/findings.txt"
        printf '\n'
        log_warn 'Resolution : consolidate the duplicated list to a single canonical'
        log_warn '             location ; reference from the other site by use ::path.'
        log_warn '             OR whitelist in tests/process_invariants.rs (with comment).'
        log_warn '             See docs/process/sot-pattern.md for the SOT decision tree.'
    fi
fi

# ─── exit policy ──────────────────────────────────────────────────────
if [ "$STRICT" = "true" ] && [ "$FINDING_COUNT" -gt 0 ] ; then
    exit 2
fi
exit 0
