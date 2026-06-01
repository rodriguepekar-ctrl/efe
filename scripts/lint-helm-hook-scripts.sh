#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
# PR #15 — lint-helm-hook-scripts.sh.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 28.
#
# Pre-commit hook helper.  Greps every Helm template under
# `charts/efe-stack/charts/efe-meta-swarm/templates/hooks/*.yaml` for
# `command: [...]` / `command: |` blocks, extracts the shell script
# body, writes it to a temp file, and runs `shellcheck` against it.
#
# The challenge :
#   - Helm templates wrap shell scripts in YAML `command:` blocks
#     PLUS Helm template directives ({{ ... }}).
#   - We can't run shellcheck on the template directly — bash doesn't
#     understand {{ ... }}.
#   - Strategy : render the template once with `helm template`, then
#     `yq` out every `command:[2]` (the script body, after `sh -c`),
#     pipe each to shellcheck individually.
#
# Usage (called from pre-commit) :
#   scripts/lint-helm-hook-scripts.sh <yaml_path> [<yaml_path>...]
#
# Returns :
#   0 — all scripts clean
#   1 — at least one shellcheck violation
#   2 — tooling missing
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail

# ─── tool sanity ────────────────────────────────────────────────────
for tool in helm yq shellcheck ; do
  if ! command -v "$tool" >/dev/null 2>&1 ; then
    echo "✗ required tool not installed : $tool" >&2
    echo "  Install : brew install $tool   (or your package manager)" >&2
    exit 2
  fi
done

# Locate the chart root from the first argument's path.
if [ "$#" -eq 0 ] ; then
  echo "usage : $0 <yaml_path> [<yaml_path>...]" >&2
  exit 1
fi

# Convert the input list (pre-commit passes changed files relative
# to cwd) → unique set of (chart_root, scenario) pairs we need to
# render.
declare -a charts
for arg in "$@" ; do
  # Walk up until we find a Chart.yaml.
  dir="$(dirname "$(realpath "$arg")")"
  while [ "$dir" != "/" ] ; do
    if [ -f "$dir/Chart.yaml" ] ; then
      charts+=("$dir")
      break
    fi
    dir="$(dirname "$dir")"
  done
done

# De-duplicate.
mapfile -t charts < <(printf '%s\n' "${charts[@]}" | sort -u)

if [ "${#charts[@]}" -eq 0 ] ; then
  echo "ℹ️  no chart root found for inputs ; nothing to lint" >&2
  exit 0
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

overall_rc=0

for chart_root in "${charts[@]}" ; do
  echo "── shellcheck on hook scripts in : ${chart_root#$(pwd)/} ──"

  # Render every hook template into one big rendered YAML.
  rendered="$tmpdir/rendered.yaml"
  if ! helm template _shellcheck "$chart_root" \
        --namespace _shellcheck_ns \
        --set enabled=true \
        --set image.tag=shellcheck-only \
        --set hooks.enabled=true \
        > "$rendered" 2>/dev/null ; then
    echo "  ⚠ helm template failed (chart not renderable on its own ; skipping)" >&2
    continue
  fi

  # Extract every `command: [..., -c, "<script>"]` block where the
  # second element is `-c` and the third is the script body.
  # We use yq's recursive selector to grab every container's command.
  script_idx=0
  while IFS= read -r script ; do
    script_idx=$((script_idx + 1))
    [ -z "$script" ] && continue
    [ "$script" = "null" ] && continue

    script_file="$tmpdir/script-${script_idx}.sh"
    printf '%s\n' "$script" > "$script_file"

    # Run shellcheck.  -S warning : suppress info-level.  -x : follow
    # external sources (we don't have any, but this is the safe flag).
    if ! shellcheck -S warning -x "$script_file" ; then
      echo "  ✗ shellcheck failed for script #${script_idx} of $chart_root" >&2
      overall_rc=1
    else
      echo "  ✅ script #${script_idx} : clean"
    fi
  done < <(
    yq eval '.. | select(has("command")) | .command | select(type == "!!seq") | select(.[0] == "sh" and .[1] == "-c") | .[2]' \
      "$rendered" 2>/dev/null \
      | awk '/^---$/{print ""; next} {print}'
  )

  if [ "$script_idx" -eq 0 ] ; then
    echo "  ℹ️  no hook scripts found"
  fi
done

if [ "$overall_rc" -eq 0 ] ; then
  echo "✅ all hook scripts pass shellcheck"
else
  echo "❌ at least one hook script has shellcheck violations" >&2
fi

exit "$overall_rc"
