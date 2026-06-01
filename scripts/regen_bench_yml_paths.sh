#!/usr/bin/env bash
# scripts/regen_bench_yml_paths.sh — Sprint D5.
#
# Regenerates the `paths:` filter block of `.github/workflows/bench.yml`
# from `scripts/benches.list`. Output goes to STDOUT — copy/paste into
# `bench.yml` (or pipe to a file and diff vs the workflow).
#
# Why a generator rather than a runtime read of the catalog :
# GitHub Actions evaluates `on.pull_request.paths` statically when the
# workflow runs ; it cannot consume a file's contents to decide which
# paths to watch. So we generate the static block from the dynamic
# catalog, and a CI lint job can verify the generator output matches
# what's currently in `bench.yml`.
#
# Usage:
#   bash scripts/regen_bench_yml_paths.sh           # print to stdout
#   bash scripts/regen_bench_yml_paths.sh > /tmp/p  # save to file
#
# To verify drift:
#   diff <(bash scripts/regen_bench_yml_paths.sh) \
#        <(awk '/^      - / && /\b(crates|scripts|\.github)\b/ {print}' .github/workflows/bench.yml)
#
# Output format matches the workflow's indentation (6 spaces + `- `)
# and produces single-quoted globs (YAML-friendly, no escaping).

set -euo pipefail

# CRITICAL: disable glob expansion. The catalog's `paths-watch` field
# contains literal glob patterns like `crates/efe-core/src/compute/**`
# that we want to emit AS-IS into the workflow YAML — the GitHub
# runner will do the matching itself. Without `set -f`, bash would
# expand every `**` against the local filesystem and emit an exhaustive
# (and useless) file list.
set -f

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CATALOG="$SCRIPT_DIR/benches.list"

if [ ! -f "$CATALOG" ]; then
  echo "ERROR: $CATALOG not found" >&2
  exit 2
fi

# shellcheck source=scripts/benches.list
source "$CATALOG"

# Use a temp file to dedupe across rows. Bash 3 (macOS default)
# doesn't have associative arrays reliably, so we pipe through `sort -u`.
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

emit_paths() {
  local _name="$1" crate="$2" _sprint="$3" _blas="$4" _tier="$5" _slo="$6" paths="$7"
  # Each row's `paths-watch` is a space-separated list. Print one per
  # line for the sort/dedupe step below.
  for p in $paths; do
    echo "$p" >> "$tmp"
  done
  # Always-watch path: the crate's Cargo.toml, derived from the crate
  # name (rather than hardcoded). Adding a third crate's benches is
  # then a single catalog change.
  echo "crates/$crate/Cargo.toml" >> "$tmp"
  # Always-watch path: the crate's benches/ directory.
  echo "crates/$crate/benches/**" >> "$tmp"
}
run_bench_catalog emit_paths

# Static scaffolding paths — these aren't bench-specific but
# changing them MUST trigger a re-run.
cat <<'EOF' >> "$tmp"
scripts/aggregate_bench_reports.py
scripts/bench_runner.sh
scripts/benches.list
scripts/check_benches_list.sh
scripts/check_regression.py
scripts/fixtures/**
scripts/gen_fixtures_v2.py
scripts/parse_criterion_report.py
scripts/post_bench_pr_comment.py
scripts/regen_bench_yml_paths.sh
scripts/show_bench_report.py
.github/workflows/bench.yml
EOF

# Emit, sorted + deduped, with the workflow's indentation.
sort -u "$tmp" | while IFS= read -r path; do
  printf "      - '%s'\n" "$path"
done
