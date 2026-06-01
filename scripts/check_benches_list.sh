#!/usr/bin/env bash
# scripts/check_benches_list.sh — Sprint D5 catalog linter.
#
# Validates `scripts/benches.list` against the schema documented at
# the top of that file, and cross-checks every entry against the
# corresponding `Cargo.toml`. Designed to run :
#
#   - Locally before pushing a catalog change.
#   - In CI as a pre-flight check before the bench matrix kicks off.
#
# Exit codes:
#   0 = catalog is valid
#   1 = at least one validation error (full list printed)
#   2 = invocation error (missing benches.list, etc.)
#
# Checks performed:
#   1. Each row has exactly 7 colon-separated fields.
#   2. `name` is non-empty and unique across the catalog.
#   3. `crate` is one of the known crates (efe-core | efe-server).
#   4. `blas-aware` is "blas" or "no-blas" (no other typos accepted).
#   5. `tier` is "default" or "micro".
#   6. The bench `[[bench]] name = "<name>"` exists in the relevant
#      `crates/<crate>/Cargo.toml`. This is the most important check —
#      it catches drift between the catalog and the Cargo manifests.
#   7. The bench source file `crates/<crate>/benches/<name>.rs` exists
#      on disk.
#
# Out of scope (could be added later):
#   - Validate `slo-prefix` against parse_criterion_report.py.
#   - Validate `paths-watch` globs are syntactically correct.
#   - Verify alphabetical sort order within crate blocks.

set -euo pipefail

CRATE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CATALOG="$CRATE_DIR/scripts/benches.list"

if [ ! -f "$CATALOG" ]; then
  echo "ERROR: catalog not found at $CATALOG" >&2
  exit 2
fi

# shellcheck source=scripts/benches.list
source "$CATALOG"

errors=0
seen_names=""

# Known crates — extend this when a new crate gets benches.
# Sprint A1 P1 PR #4 added efe-meta-swarm.
KNOWN_CRATES=("efe-core" "efe-meta-swarm" "efe-server")

is_known_crate() {
  local c="$1"
  local k
  for k in "${KNOWN_CRATES[@]}"; do
    [ "$k" = "$c" ] && return 0
  done
  return 1
}

check_one() {
  local name="$1" crate="$2" sprint="$3" blas="$4" tier="$5" _slo="$6" _paths="$7"

  # Field 1: name
  if [ -z "$name" ]; then
    echo "FAIL: empty name in catalog row (crate=$crate sprint=$sprint)"
    errors=$((errors + 1))
    return
  fi

  # Uniqueness
  if [[ ",$seen_names," == *",$name,"* ]]; then
    echo "FAIL: duplicate name '$name'"
    errors=$((errors + 1))
  fi
  seen_names="${seen_names},${name}"

  # Field 2: crate
  if ! is_known_crate "$crate"; then
    echo "FAIL: $name has unknown crate '$crate' (expected one of: ${KNOWN_CRATES[*]})"
    errors=$((errors + 1))
    return
  fi

  # Field 4: blas-aware
  case "$blas" in
    blas|no-blas) ;;
    *)
      echo "FAIL: $name has invalid blas-aware '$blas' (expected: blas | no-blas)"
      errors=$((errors + 1))
      ;;
  esac

  # Field 5: tier
  case "$tier" in
    default|micro) ;;
    *)
      echo "FAIL: $name has invalid tier '$tier' (expected: default | micro)"
      errors=$((errors + 1))
      ;;
  esac

  # Field 6 (cross-check): [[bench]] in Cargo.toml
  local cargo_toml="$CRATE_DIR/crates/$crate/Cargo.toml"
  if [ ! -f "$cargo_toml" ]; then
    echo "FAIL: $name references crate '$crate' but $cargo_toml does not exist"
    errors=$((errors + 1))
    return
  fi
  # Look for `name = "<name>"` inside a `[[bench]]` block. We accept
  # both single-line and multi-line forms, matching:
  #   [[bench]]
  #   name = "swarm_e2e"
  # The grep is loose : `grep -B 5` of `name = "<name>"` followed by
  # a `[[bench]]` line within the preceding 5 lines. False positives
  # are theoretically possible (a `[package]` table with a
  # `name = "swarm_e2e"`), but no such collision exists in the
  # workspace today.
  if ! grep -B 5 "^name = \"$name\"$" "$cargo_toml" 2>/dev/null \
       | grep -q "^\[\[bench\]\]$"; then
    echo "FAIL: $name has no matching [[bench]] in $cargo_toml"
    errors=$((errors + 1))
  fi

  # Field 7 (cross-check): source file exists
  local src="$CRATE_DIR/crates/$crate/benches/$name.rs"
  if [ ! -f "$src" ]; then
    echo "FAIL: $name has no source file at $src"
    errors=$((errors + 1))
  fi
}

echo "==> Validating catalog $CATALOG"
run_bench_catalog check_one

if [ "$errors" -gt 0 ]; then
  echo ""
  echo "==> $errors validation error(s) — see messages above"
  exit 1
fi

# Count entries for the summary
total=$(printf "%s\n" "${EFE_BENCHES[@]}" | wc -l | tr -d ' ')
echo ""
echo "==> OK : $total catalog entries valid"
