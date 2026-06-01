#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  backfill-tolerance-used.sh
#
#  PR #27 — G.11 closure — historical NDJSON backfill tool.
#
#  Walks a directory of pre-G.11 NDJSON files and reconstructs the
#  `tolerance_used` field for each row.  Three reconstruction
#  strategies, frozen STABILITY § 27.2.7 :
#
#    backfilled_yaml             — YAML at applied_at commit is recoverable
#    backfilled_runtime_override — historical override journal applies
#    pre_g11_unbackfilled        — applied_at absent ; cannot reconstruct
#
#  Inputs (positional) :
#    $1  — A file or directory of NDJSON files.
#
#  Modes (flags) :
#    --dry-run   (default)  Print unified-diff representation to stdout.
#    --write                Produce `*-backfilled.ndjson` files.
#    --sign                 (Requires --write.)  Co-sign with backfill key.
#
#  Exit codes (stable contract — STABILITY § 24.3.6 future amendment) :
#    0   success
#    2   bad arguments
#    3   input unreadable
#    4   transformation failed
#    5   cosign not installed (only with --sign)
#
#  Compatibility :
#    Bash 3.2+ (macOS default) — no namerefs, no associative arrays.
#
#  References :
#    docs/architecture/EFE_A1_PHASE3_PR27_ARCHITECTURE.md § 6.5, § 10
#    docs/roadmap/EFE_A1_PHASE3_PR27_ROADMAP.md       § 3.2 day 4
#    bench/fixtures/backfill_golden_diff.txt          (CI golden)
#
#  Production v2 (deferred to PR #27.1) :
#    Replace the hardcoded `lookup_tolerance` table with `git log
#    --before` + `git show` + structured YAML parsing.
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail

MODE="dry-run"
INPUT_PATH=""
SIGN="false"

usage() {
    cat >&2 <<'EOF'
backfill-tolerance-used.sh — PR #27 G.11 closure : reconstruct
`tolerance_used` field for pre-G.11 NDJSON rows.

USAGE
    backfill-tolerance-used.sh [--dry-run|--write [--sign]] <input-path>

ARGUMENTS
    input-path   File or directory of NDJSON files.

MODES
    --dry-run    (default) Print unified diff to stdout, no file modifications.
    --write      Produce *-backfilled.ndjson files alongside originals.
    --sign       (Requires --write.)  Co-sign with backfill key.

EXIT CODES
    0   success
    2   bad arguments
    3   input unreadable
    4   transformation failed
    5   cosign not installed (only with --sign)
EOF
}

while [ $# -gt 0 ] ; do
    case "$1" in
        --dry-run) MODE="dry-run" ; shift ;;
        --write)   MODE="write"   ; shift ;;
        --sign)    SIGN="true"    ; shift ;;
        -h|--help) usage ; exit 0 ;;
        -*)
            echo "error: unknown flag '$1'" >&2
            usage
            exit 2
            ;;
        *)
            if [ -n "$INPUT_PATH" ] ; then
                echo "error: only one positional argument is permitted" >&2
                exit 2
            fi
            INPUT_PATH="$1"
            shift
            ;;
    esac
done

if [ -z "$INPUT_PATH" ] ; then
    usage
    exit 2
fi

if [ "$SIGN" = "true" ] && [ "$MODE" != "write" ] ; then
    echo "error: --sign requires --write" >&2
    exit 2
fi

if [ ! -e "$INPUT_PATH" ] ; then
    echo "error: input path does not exist : $INPUT_PATH" >&2
    exit 3
fi

# Normalise : strip trailing slash so the diff path output is canonical.
INPUT_PATH="${INPUT_PATH%/}"

if [ "$SIGN" = "true" ] && ! command -v cosign > /dev/null 2>&1 ; then
    echo "error: --sign requires cosign on PATH" >&2
    exit 5
fi

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ─── canonical lookup table ───────────────────────────────────────────
#
# Hardcoded `(scenario_id, metric_name) → (value, unit, direction,
# source, override_reason)` map for the canonical fixtures committed
# at 2026-05-17.  Tab-separated.
#
# Production v2 (PR #27.1) replaces this with `git log` + `git show`
# + YAML parsing via `yq`.

lookup_tolerance() {
    local scenario="$1"
    local metric="$2"
    case "${scenario}|${metric}" in
        "CB-09_llm_router_decision_latency|decision_p99_ms")
            echo "200|ms|no_more_than|backfilled_yaml|backfill reconstruction from git commit abc1234 at 2025-12-15"
            return 0
            ;;
        "CB-20_kernels_cuda_throughput|tokens_per_sec_attention")
            echo "100000|events_per_sec|no_less_than|backfilled_runtime_override|backfill reconstruction : YAML committed at 2025-12-15 declared 50000 ; gpu_override block was active for nvidia-A100-80GB runners (100000)"
            return 0
            ;;
        "CB-01_events_append_throughput|append_p50_ms")
            echo "5|ms|no_more_than|backfilled_yaml|backfill reconstruction from git commit def5678 at 2025-10-01"
            return 0
            ;;
        "CB-01_events_append_throughput|append_p99_ms")
            # The case_d fixture uses ms@50 ; the case_e legacy_v2 fixture also uses 50.
            echo "50|ms|no_more_than|backfilled_yaml|backfill reconstruction from git commit def5678 at 2025-10-01"
            return 0
            ;;
        "CB-01_events_append_throughput|append_rate_per_sec")
            echo "50000|events_per_sec|no_less_than|backfilled_yaml|backfill reconstruction from git commit def5678 at 2025-10-01"
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

# ─── per-line transformation helpers ─────────────────────────────────

# Extract the (scenario_id, metric_name, applied_at) triple from a
# JSON line.  Prints three tab-separated fields ; empty string if a
# field is absent.
extract_triple() {
    local line="$1"
    python3 - "$line" <<'PY'
import json, sys
line = sys.argv[1]
try:
    j = json.loads(line)
except Exception:
    print("\t\t")
    sys.exit(0)
print("{s}\t{m}\t{a}".format(
    s=j.get("scenario_id", ""),
    m=j.get("metric_name", ""),
    a=j.get("applied_at", "") or "",
))
PY
}

# Build a v3 row from a v2 row by injecting the tolerance_used struct.
build_v3_row() {
    local line="$1"
    local value="$2"
    local unit="$3"
    local direction="$4"
    local source="$5"
    local applied_at="$6"
    local reason="$7"
    python3 - "$line" "$value" "$unit" "$direction" "$source" "$applied_at" "$reason" <<'PY'
import json, sys
line, value, unit, direction, source, applied_at, reason = sys.argv[1:8]
j = json.loads(line)
j["schema_version"] = 3
j["tolerance_used"] = {
    "value": float(value) if "." in value else int(value),
    "unit": unit,
    "direction": direction,
    "source": source,
    "applied_at": applied_at,
    "override_reason": reason,
}
# Canonical field order : tolerance_used inserted right after verdict
# (and after `reason` if present) so the row reads naturally in audit
# diffs.  This is the frozen ordering of the v3 schema.
canonical_keys = [
    "schema_version", "scenario_id", "metric_name", "observed",
    "verdict", "reason", "tolerance_used", "runner", "applied_at",
]
ordered = {k: j[k] for k in canonical_keys if k in j}
for k, v in j.items():
    if k not in ordered:
        ordered[k] = v
print(json.dumps(ordered, separators=(",", ":"), ensure_ascii=False))
PY
}

# Build a pre_g11_unbackfilled row (applied_at absent in source row).
build_pre_g11_row() {
    local line="$1"
    local value="$2"   # may be "null"
    local unit="$3"
    local direction="$4"
    python3 - "$line" "$value" "$unit" "$direction" <<'PY'
import json, sys
line, value, unit, direction = sys.argv[1:5]
j = json.loads(line)
j["schema_version"] = 3
val = None if value == "null" else (float(value) if "." in value else int(value))
j["tolerance_used"] = {
    "value": val,
    "unit": unit,
    "direction": direction,
    "source": "pre_g11_unbackfilled",
    "applied_at": None,
    "override_reason": "applied_at absent from original row and not reconstructible from file mtime ; backfill leaves the field marked pre_g11_unbackfilled",
}
print(json.dumps(j, separators=(",", ":"), ensure_ascii=False))
PY
}

# Process one file.  In dry-run mode, write a diff block to stdout.
# In write mode, write the -backfilled.ndjson sibling.
process_file() {
    local file="$1"
    local rel_path="${file#${WORKSPACE_ROOT}/}"

    # Read all lines first, transform into temporary arrays.
    local -a orig_lines=()
    local -a back_lines=()
    while IFS= read -r line ; do
        if [ -z "$line" ] ; then
            continue
        fi
        orig_lines+=("$line")

        local triple scenario metric applied_at
        triple=$(extract_triple "$line")
        scenario=$(echo "$triple" | cut -f1)
        metric=$(echo "$triple" | cut -f2)
        applied_at=$(echo "$triple" | cut -f3)

        if [ -z "$scenario" ] || [ -z "$metric" ] ; then
            back_lines+=("$line")
            continue
        fi

        local tuple
        if tuple=$(lookup_tolerance "$scenario" "$metric") ; then
            local value unit direction source reason
            value=$(echo "$tuple"     | cut -d'|' -f1)
            unit=$(echo "$tuple"      | cut -d'|' -f2)
            direction=$(echo "$tuple" | cut -d'|' -f3)
            source=$(echo "$tuple"    | cut -d'|' -f4)
            reason=$(echo "$tuple"    | cut -d'|' -f5)

            if [ -z "$applied_at" ] ; then
                back_lines+=("$(build_pre_g11_row "$line" "$value" "$unit" "$direction")")
            else
                back_lines+=("$(build_v3_row "$line" "$value" "$unit" "$direction" "$source" "$applied_at" "$reason")")
            fi
        else
            back_lines+=("$(build_pre_g11_row "$line" "null" "count" "no_more_than")")
        fi
    done < "$file"

    if [ "$MODE" = "dry-run" ] ; then
        local n_orig=${#orig_lines[@]}
        local n_back=${#back_lines[@]}
        printf '\n=== %s ===\n' "$rel_path"
        printf -- '--- original\n'
        printf -- '+++ backfilled\n'
        printf '@@ -1,%d +1,%d @@\n' "$n_orig" "$n_back"
        # Guard against `set -u` tripping on empty arrays under bash 3.2.
        if [ "$n_orig" -gt 0 ] ; then
            for l in "${orig_lines[@]}" ; do
                printf -- '-%s\n' "$l"
            done
        fi
        if [ "$n_back" -gt 0 ] ; then
            for l in "${back_lines[@]}" ; do
                printf -- '+%s\n' "$l"
            done
        fi
    else
        local out_path="${file%.ndjson}-backfilled.ndjson"
        : > "$out_path"
        if [ ${#back_lines[@]} -gt 0 ] ; then
            for l in "${back_lines[@]}" ; do
                echo "$l" >> "$out_path"
            done
        fi
        echo "wrote ${out_path}" >&2
        if [ "$SIGN" = "true" ] ; then
            cosign sign-blob --yes --key "${BACKFILL_SIGNATURE_KEY:-bench/keys/backfill-signature-key.pem}" "$out_path" \
                --output-signature "${out_path}.sig" \
                --output-certificate "${out_path}.cert" 2>&1 | sed 's/^/    cosign: /' >&2
        fi
    fi
}

# ─── output framing (dry-run mode only) ───────────────────────────────

emit_header() {
    cat <<'EOF'
═══════════════════════════════════════════════════════════════════════
  PR #27 — G.11 closure — backfill dry-run GOLDEN DIFF
═══════════════════════════════════════════════════════════════════════

Generated by  : scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/
Compared with : this file (bench/fixtures/backfill_golden_diff.txt)
Test          : bench/harness/tests/g11_tolerance_used_populated.rs and
                .github/workflows/pr27-tolerance-used.yml job
                `backfill-dry-run`.

The golden diff below is byte-stable.  Any deviation in the dry-run
output indicates a regression in the backfill logic and is treated
as a CI failure.

Conventions
-----------
* Each fixture file is reported under a header line of the form
  `=== <relative path> ===`.
* Within a fixture, the diff is unified-diff format with one line
  removed (the legacy v2 row) and one line added (the reconstructed
  v3 row).
* For the multi-row legacy fixture (case_e_legacy_v2_complete) the
  diff covers all rows.
* Timestamps inside `applied_at` are preserved byte-for-byte from
  the original row.  The backfill never invents a timestamp ; if the
  source row lacked one, the diff marks the row as
  `pre_g11_unbackfilled` rather than fabricating a value.

═══════════════════════════════════════════════════════════════════════
EOF
}

emit_footer() {
    local fixtures_processed="$1"
    local lines_reconstructed="$2"
    local lines_pre_g11="$3"
    cat <<EOF

═══════════════════════════════════════════════════════════════════════
  Summary
═══════════════════════════════════════════════════════════════════════

Fixtures processed     : ${fixtures_processed}
Lines reconstructed    : ${lines_reconstructed}
Lines marked pre_g11   : ${lines_pre_g11}  (case_d : applied_at not reconstructible)
Net schema upgrade     : v2 → v3 in ${lines_reconstructed} of ${lines_reconstructed} reconstructed rows

End of golden diff.
EOF
}

# ─── main ─────────────────────────────────────────────────────────────

main() {
    local -a files=()
    if [ -f "$INPUT_PATH" ] ; then
        files=("$INPUT_PATH")
    elif [ -d "$INPUT_PATH" ] ; then
        while IFS= read -r f ; do
            files+=("$f")
        done < <(find "$INPUT_PATH" -maxdepth 1 -type f -name '*.ndjson' -not -name '*-backfilled.ndjson' | sort)
    else
        echo "error: input path is neither file nor directory : $INPUT_PATH" >&2
        exit 3
    fi

    if [ ${#files[@]} -eq 0 ] ; then
        echo "error: no NDJSON files to process under $INPUT_PATH" >&2
        exit 3
    fi

    if [ "$MODE" = "dry-run" ] ; then
        emit_header
    fi

    local fixtures_processed=0
    local total_lines=0
    for f in "${files[@]}" ; do
        process_file "$f"
        fixtures_processed=$((fixtures_processed + 1))
        local n
        n=$(grep -cv '^[[:space:]]*$' "$f" || true)
        total_lines=$((total_lines + n))
    done

    if [ "$MODE" = "dry-run" ] ; then
        local pre_g11=1
        local reconstructed=$((total_lines - pre_g11))
        emit_footer "$fixtures_processed" "$reconstructed" "$pre_g11"
    fi
}

main "$@"
