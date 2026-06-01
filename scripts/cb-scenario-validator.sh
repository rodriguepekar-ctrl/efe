#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  cb-scenario-validator.sh — validates a CB-*.yaml scenario file
#  against the schema frozen in STABILITY § 27.3.
#
#  Inputs (positional) :
#    $1  path to a bench/scenarios/CB-*.yaml file
#
#  Exit codes (stable contract — STABILITY § 27.3) :
#    0  validation passed
#    2  file argument missing or path doesn't exist
#    3  YAML malformed or top-level shape wrong
#    4  mandatory CB-specific field missing or of wrong shape
#    5  requires_meta_swarm is true (CB suite forbids MS dependency)
#    6  identifier mismatch (filename id prefix vs. id: field)
#    7  tolerance block missing or empty
#
#  Reference :
#    docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md § 8
#    docs/STABILITY_CONTRACT.md § 27.3
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail

if [ $# -ne 1 ] ; then
    cat <<'EOF' >&2
usage : cb-scenario-validator.sh <path-to-CB-NN_*.yaml>
EOF
    exit 2
fi

readonly CB_FILE="$1"

if [ ! -f "$CB_FILE" ] ; then
    echo "error: $CB_FILE does not exist or is not a regular file" >&2
    exit 2
fi
if [ ! -s "$CB_FILE" ] ; then
    echo "error: $CB_FILE is empty" >&2
    exit 2
fi

# Use python yaml to parse — it's the same parser CI uses elsewhere.
if ! python3 -c "import yaml; yaml.safe_load(open('$CB_FILE'))" 2>/dev/null ; then
    echo "error: $CB_FILE is not valid YAML" >&2
    exit 3
fi

# Extract field values via python (single subprocess, cheap).
read -r suite id family subsystem req_ms <<EOF
$(python3 - "$CB_FILE" <<'PY'
import sys, yaml
d = yaml.safe_load(open(sys.argv[1]))
print(d.get('suite', ''),
      d.get('id', ''),
      d.get('family', ''),
      d.get('subsystem', ''),
      d.get('requires_meta_swarm', 'NONE'))
PY
)
EOF

# Mandatory : suite == "CB".
if [ "$suite" != "CB" ] ; then
    echo "error: top-level 'suite:' must be 'CB' (got '$suite')" >&2
    exit 4
fi

# Mandatory : id matches CB-NN_* or CB-AN_* pattern.
if ! [[ "$id" =~ ^CB-(A?[0-9]+)_[a-z_]+ ]] ; then
    echo "error: top-level 'id:' must match 'CB-NN_*' or 'CB-AN_*' (got '$id')" >&2
    exit 4
fi

# Mandatory : family is one of the seven frozen families
# (STABILITY § 27.1).  The set was widened from five to seven in the
# deep-coverage expansion : "integration" covers FFI / MCP / eval /
# agent-core boundaries (subsystems that span the Rust↔external
# surface), and "long-run" covers soak + DR drills (workloads whose
# defining property is duration, not subsystem).
case "$family" in
    data-plane|control-plane|inference|compliance|integration|long-run|meta-bench) ;;
    *)
        echo "error: family must be one of {data-plane, control-plane, inference, compliance, integration, long-run, meta-bench} (got '$family')" >&2
        exit 4
        ;;
esac

# Mandatory : subsystem is non-empty.
if [ -z "$subsystem" ] ; then
    echo "error: 'subsystem:' must be non-empty" >&2
    exit 4
fi

# Mandatory : requires_meta_swarm must be set AND must be false (CB suite forbids MS).
if [ "$req_ms" = "NONE" ] ; then
    echo "error: 'requires_meta_swarm:' is mandatory in CB scenarios" >&2
    exit 4
fi
if [ "$req_ms" != "False" ] && [ "$req_ms" != "false" ] ; then
    echo "error: 'requires_meta_swarm: true' is forbidden in CB suite (STABILITY § 27.1)" >&2
    exit 5
fi

# Identifier consistency : filename id-prefix == id: value.
basename=$(basename "$CB_FILE" .yaml)
if [ "$basename" != "$id" ] ; then
    echo "error: filename '$basename' must match 'id: $id'" >&2
    exit 6
fi

# Tolerance block present + non-empty.
tolerance_keys=$(python3 - "$CB_FILE" <<'PY'
import sys, yaml
d = yaml.safe_load(open(sys.argv[1]))
t = d.get('tolerance', {})
print(len(t.keys()) if isinstance(t, dict) else 0)
PY
)
if [ "$tolerance_keys" -lt 1 ] ; then
    echo "error: 'tolerance:' block missing or empty (must declare at least one bound)" >&2
    exit 7
fi

echo "ok: $CB_FILE  · suite=$suite · id=$id · family=$family · subsystem=$subsystem · tolerance-keys=$tolerance_keys"
exit 0
