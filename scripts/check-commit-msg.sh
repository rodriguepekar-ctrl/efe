#!/usr/bin/env bash
# Enforces Conventional Commits for EFE.
# Invoked by pre-commit as commit-msg hook.
set -euo pipefail

MSG_FILE="${1:-}"
if [[ -z "$MSG_FILE" || ! -f "$MSG_FILE" ]]; then
    echo "ERROR: commit message file not provided"
    exit 1
fi

# First non-empty, non-comment line is the subject.
SUBJECT=$(grep -vE '^(#|$)' "$MSG_FILE" | head -n 1 || true)

# Allow merges and reverts to bypass.
if [[ "$SUBJECT" =~ ^Merge\  || "$SUBJECT" =~ ^Revert\  ]]; then
    exit 0
fi

# Conventional Commits pattern:
#   type(scope)?: subject
# scope is optional, kebab-case.
PATTERN='^(feat|fix|refactor|perf|chore|docs|test|build|ci)(\([a-z0-9-]+\))?!?: .+'

if ! [[ "$SUBJECT" =~ $PATTERN ]]; then
    echo "ERROR: commit message must follow Conventional Commits"
    echo ""
    echo "Got: $SUBJECT"
    echo ""
    echo "Required format:"
    echo "  <type>(<scope>): <subject>"
    echo ""
    echo "Allowed types: feat, fix, refactor, perf, chore, docs, test, build, ci"
    echo "Examples:"
    echo "  feat(m5): add swarm attention kernel"
    echo "  fix(m2): arena overflow check off-by-one"
    echo "  docs: update quickstart for uv 0.4"
    exit 1
fi

# Subject length check (no more than 72 chars)
SUBJECT_LEN=${#SUBJECT}
if (( SUBJECT_LEN > 72 )); then
    echo "WARNING: commit subject is $SUBJECT_LEN chars (recommended < 72)"
fi

exit 0
